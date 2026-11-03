//! Parakeet on CoreML behind [`SpeechEngine`]. The TDT models (v2, Ultra,
//! Japanese) are ported from FluidAudio's `AsrModels` and `AsrManager`;
//! Parakeet Unified streams through [`crate::unified`].

use std::path::{Path, PathBuf};
use std::time::Instant;

use onda_asr::{AsrError, LoadProgress, SpeechEngine};
use onda_domain::languages::Language;
use onda_domain::models::ModelId;

use crate::chunk::{self, FRAME_SAMPLES, Layout, MAX_MODEL_SAMPLES, MIN_SAMPLES, SAMPLE_RATE};
use crate::chunk::{PreviewWindows, WindowRequest};
use crate::coreml::{Array, ComputeUnits, Features, Model};
use crate::language::TokenFilter;
use crate::recovery::{self, LengthPolicy, RECOVERY_POLICIES};
use crate::tdt::{self, DecodeSpan, Hypothesis, JointDecision, TdtConfig, Token, TopK, Transducer};
use crate::tensor::EncoderFrames;
use crate::unified::UnifiedEngine;
use crate::vocab::Vocabulary;

const ENCODER_HIDDEN: usize = 1024;
const DECODER_HIDDEN: usize = 640;
const DECODER_LAYERS: usize = 2;

/// A Parakeet model on CoreML, loaded from FluidInference's conversion in
/// a model folder (MOD-1).
pub struct ParakeetEngine {
    inner: Inner,
}

enum Inner {
    Tdt(TdtEngine),
    Unified(UnifiedEngine),
}

impl ParakeetEngine {
    /// An unloaded engine for `model` whose files are in `dir`; `None` for
    /// models this backend does not run.
    pub fn new(model: ModelId, dir: impl Into<PathBuf>) -> Option<ParakeetEngine> {
        let dir = dir.into();
        let inner = match model {
            ModelId::ParakeetTdtV2 => Inner::Tdt(TdtEngine::new(&V2, dir)),
            ModelId::ParakeetUltra => Inner::Tdt(TdtEngine::new(&ULTRA, dir)),
            ModelId::ParakeetJapanese => Inner::Tdt(TdtEngine::new(&JAPANESE, dir)),
            ModelId::ParakeetUnified => Inner::Unified(UnifiedEngine::new(dir)),
            ModelId::SystemSpeech => return None,
        };
        Some(ParakeetEngine { inner })
    }

    fn engine(&mut self) -> &mut dyn SpeechEngine {
        match &mut self.inner {
            Inner::Tdt(engine) => engine,
            Inner::Unified(engine) => engine,
        }
    }
}

impl SpeechEngine for ParakeetEngine {
    fn load(&mut self, progress: &mut dyn FnMut(LoadProgress)) -> Result<(), AsrError> {
        self.engine().load(progress)
    }

    fn unload(&mut self) {
        self.engine().unload();
    }

    fn supports_preview(&self) -> bool {
        match &self.inner {
            Inner::Tdt(engine) => engine.supports_preview(),
            Inner::Unified(engine) => engine.supports_preview(),
        }
    }

    // FluidAudio rejects anything under 0.3 s; the unified stream keeps the
    // same floor so every Parakeet model judges short clips alike
    fn min_samples(&self) -> usize {
        MIN_SAMPLES
    }

    fn transcribe(
        &mut self,
        samples: &[f32],
        language: Option<Language>,
    ) -> Result<String, AsrError> {
        self.engine().transcribe(samples, language)
    }

    fn preview(&mut self, samples: &[f32], language: Option<Language>) -> Result<String, AsrError> {
        self.engine().preview(samples, language)
    }

    fn reset_preview(&mut self) {
        self.engine().reset_preview();
    }
}

/// What differs between the TDT conversions (FluidAudio `AsrModelVersion`).
struct TdtSpec {
    name: &'static str,
    /// Compiled models in load order: preprocessor, encoder, decoder,
    /// joint, with FluidAudio's compute units (the preprocessor's ops all
    /// map to the CPU; the rest prefer the Neural Engine).
    models: [(&'static str, ComputeUnits); 4],
    vocabulary: &'static str,
    /// The preprocessor output the encoder reads.
    mel: &'static str,
    config: fn() -> TdtConfig,
    layout: Layout,
    /// v3 family (`isV3Family`): script hint and empty-decode recovery.
    v3: bool,
    /// Whether pieces mark word starts (off for Japanese).
    splice_guard: bool,
}

static V2: TdtSpec = TdtSpec {
    name: "Parakeet TDT v2",
    models: [
        ("Preprocessor.mlmodelc", ComputeUnits::CpuOnly),
        ("Encoder.mlmodelc", ComputeUnits::CpuAndNeuralEngine),
        ("Decoder.mlmodelc", ComputeUnits::CpuAndNeuralEngine),
        ("JointDecision.mlmodelc", ComputeUnits::CpuAndNeuralEngine),
    ],
    vocabulary: "parakeet_vocab.json",
    mel: "mel",
    config: TdtConfig::v2,
    layout: Layout::V2,
    v3: false,
    splice_guard: true,
};

static ULTRA: TdtSpec = TdtSpec {
    name: "Parakeet Ultra",
    models: [
        ("Preprocessor.mlmodelc", ComputeUnits::CpuOnly),
        ("Encoder.mlmodelc", ComputeUnits::CpuAndNeuralEngine),
        ("Decoder.mlmodelc", ComputeUnits::CpuAndNeuralEngine),
        ("JointDecisionv3.mlmodelc", ComputeUnits::CpuAndNeuralEngine),
    ],
    vocabulary: "parakeet_vocab.json",
    mel: "mel",
    config: TdtConfig::v3,
    layout: Layout::V3,
    v3: true,
    splice_guard: true,
};

static JAPANESE: TdtSpec = TdtSpec {
    name: "Parakeet Japanese",
    models: [
        ("Preprocessor.mlmodelc", ComputeUnits::CpuOnly),
        ("Encoder.mlmodelc", ComputeUnits::CpuAndNeuralEngine),
        ("Decoderv2.mlmodelc", ComputeUnits::CpuAndNeuralEngine),
        ("Jointerv2.mlmodelc", ComputeUnits::CpuAndNeuralEngine),
    ],
    vocabulary: "vocab.json",
    mel: "mel_features",
    config: TdtConfig::japanese,
    layout: Layout::Japanese,
    v3: false,
    splice_guard: false,
};

/// A Parakeet TDT model. The final decode decodes the whole recording from
/// scratch; a preview decodes again only the windows that changed since the
/// last preview (ASR-17).
struct TdtEngine {
    spec: &'static TdtSpec,
    dir: PathBuf,
    loaded: Option<Loaded>,
    preview_windows: PreviewWindows,
}

impl TdtEngine {
    fn new(spec: &'static TdtSpec, dir: PathBuf) -> TdtEngine {
        TdtEngine {
            spec,
            dir,
            loaded: None,
            preview_windows: PreviewWindows::default(),
        }
    }

    /// Decodes `samples`, through the preview's windows when `preview`.
    /// Returns the text and how many windows ran.
    fn decode(
        &mut self,
        samples: &[f32],
        language: Option<Language>,
        preview: bool,
    ) -> Result<(String, usize), AsrError> {
        let loaded = self.loaded.as_mut().ok_or(AsrError::NotReady)?;
        if samples.len() < MIN_SAMPLES {
            return Err(AsrError::TranscriptionFailed {
                detail: format!("{} samples is shorter than 0.3 s", samples.len()),
            });
        }
        let filter = match language {
            Some(language) if self.spec.v3 => {
                TokenFilter::new(language, &loaded.vocab, loaded.runtime.config.blank)
            }
            Some(language) => {
                tracing::debug!(
                    language = language.code(),
                    model = self.spec.name,
                    "the model ignores the language"
                );
                None
            }
            None => None,
        };
        let runtime = &mut loaded.runtime;
        let mut decoded = 0;
        let decode = |request: &WindowRequest| {
            decoded += 1;
            runtime.decode_window(samples, request, filter.as_ref())
        };
        let (vocab, layout) = (&loaded.vocab, self.spec.layout);
        let tokens = match preview {
            true => self
                .preview_windows
                .transcribe(samples, language, vocab, layout, decode)?,
            false => chunk::transcribe(samples, vocab, layout, decode)?,
        };
        let ids: Vec<usize> = tokens.iter().map(|token| token.id).collect();
        Ok((vocab.detokenize(&ids), decoded))
    }
}

impl SpeechEngine for TdtEngine {
    fn load(&mut self, progress: &mut dyn FnMut(LoadProgress)) -> Result<(), AsrError> {
        if self.loaded.is_some() {
            return Ok(());
        }
        let started = Instant::now();
        let loaded = objc2::rc::autoreleasepool(|_| Loaded::load(self.spec, &self.dir, progress))?;
        tracing::info!(
            model = self.spec.name,
            load_ms = started.elapsed().as_millis() as u64,
            "model loaded"
        );
        self.loaded = Some(loaded);
        Ok(())
    }

    fn unload(&mut self) {
        self.loaded = None;
        self.reset_preview();
    }

    fn supports_preview(&self) -> bool {
        true
    }

    // the final decode ends the recording's preview stream
    fn transcribe(
        &mut self,
        samples: &[f32],
        language: Option<Language>,
    ) -> Result<String, AsrError> {
        self.reset_preview();
        let started = Instant::now();
        let (text, _) = self.decode(samples, language, false)?;
        tracing::debug!(
            audio_s = samples.len() as f64 / SAMPLE_RATE as f64,
            decode_ms = started.elapsed().as_secs_f64() * 1e3,
            "final decode"
        );
        Ok(text)
    }

    fn preview(&mut self, samples: &[f32], language: Option<Language>) -> Result<String, AsrError> {
        let started = Instant::now();
        let (text, windows_decoded) = self.decode(samples, language, true)?;
        tracing::info!(
            audio_s = samples.len() as f64 / SAMPLE_RATE as f64,
            decode_ms = started.elapsed().as_secs_f64() * 1e3,
            windows_decoded,
            "preview decode"
        );
        Ok(text)
    }

    fn reset_preview(&mut self) {
        self.preview_windows = PreviewWindows::default();
    }
}

/// A loaded model: the vocabulary and the CoreML runtime.
struct Loaded {
    vocab: Vocabulary,
    runtime: Runtime,
}

impl Loaded {
    fn load(
        spec: &'static TdtSpec,
        dir: &Path,
        progress: &mut dyn FnMut(LoadProgress),
    ) -> Result<Loaded, AsrError> {
        let mut models = Vec::with_capacity(spec.models.len());
        for (index, (name, units)) in spec.models.iter().enumerate() {
            models.push(load_model(
                dir,
                name,
                *units,
                index,
                spec.models.len(),
                progress,
            )?);
        }
        let [preprocessor, encoder, decoder, joint]: [Model; 4] = models
            .try_into()
            .map_err(|_| load_failed("models", "unexpected model count".to_owned()))?;
        let config = (spec.config)();
        let json = std::fs::read_to_string(dir.join(spec.vocabulary))
            .map_err(|error| load_failed(spec.vocabulary, error.to_string()))?;
        let mut vocab = Vocabulary::parse(&json, config.blank)
            .map_err(|error| load_failed(spec.vocabulary, error))?;
        if !spec.splice_guard {
            vocab = vocab.without_splice_guard();
        }

        let buffers = || -> Result<(Array, Array, Features, Array, StepIo), String> {
            let audio = Array::zeros_f32(&[1, MAX_MODEL_SAMPLES])?;
            let audio_length = Array::zeros_i32(&[1])?;
            let inputs =
                Features::new(&[("audio_signal", &audio), ("audio_length", &audio_length)])?;
            Ok((
                audio,
                audio_length,
                inputs,
                Array::zeros_i32(&[1])?,
                StepIo::new()?,
            ))
        };
        let (audio, audio_length, preprocessor_inputs, full_mel_length, steps) =
            buffers().map_err(|error| load_failed("input buffers", error))?;
        progress(LoadProgress {
            status: String::new(),
            fraction: 1.0,
        });
        Ok(Loaded {
            vocab,
            runtime: Runtime {
                spec,
                preprocessor,
                encoder,
                decoder,
                joint,
                config,
                audio,
                audio_length,
                preprocessor_inputs,
                full_mel_length,
                steps,
            },
        })
    }
}

/// Compiles one model, reporting it as step `index` of `count`.
pub(crate) fn load_model(
    dir: &Path,
    name: &str,
    units: ComputeUnits,
    index: usize,
    count: usize,
    progress: &mut dyn FnMut(LoadProgress),
) -> Result<Model, AsrError> {
    progress(LoadProgress {
        status: format!("Compiling {name}..."),
        fraction: index as f64 / count as f64,
    });
    let started = Instant::now();
    let model = Model::load(&dir.join(name), units).map_err(|error| load_failed(name, error))?;
    tracing::info!(
        model = name,
        load_ms = started.elapsed().as_millis() as u64,
        "compiled model"
    );
    Ok(model)
}

/// The four models plus input buffers reused across windows.
struct Runtime {
    spec: &'static TdtSpec,
    preprocessor: Model,
    encoder: Model,
    decoder: Model,
    joint: Model,
    config: TdtConfig,
    audio: Array,
    audio_length: Array,
    preprocessor_inputs: Features,
    full_mel_length: Array,
    steps: StepIo,
}

impl Runtime {
    /// Preprocessor, encoder, and greedy TDT decode of one window, retried
    /// with perturbed lengths when a v3 window of speech decodes to nothing
    /// (FluidAudio `executeMLInferenceWithTimings`). CoreML autoreleases
    /// objects, and engine calls run on long-lived worker threads that
    /// never drain a pool, so each window drains its own.
    fn decode_window(
        &mut self,
        samples: &[f32],
        request: &WindowRequest,
        filter: Option<&TokenFilter<'_>>,
    ) -> Result<Vec<Token>, AsrError> {
        objc2::rc::autoreleasepool(|_| {
            let window = samples
                .get(request.start..request.end)
                .ok_or_else(|| failed("window outside the audio"))?;
            let mut padded = vec![0.0f32; MAX_MODEL_SAMPLES];
            padded
                .get_mut(..window.len())
                .ok_or_else(|| failed("window longer than the model input"))?
                .copy_from_slice(window);
            let first = self.run_window(&padded, request, LengthPolicy::default(), filter)?;
            if !self.spec.v3
                || !first.is_blank()
                || !recovery::should_recover(&padded, request.audio_length)
            {
                return Ok(first.tokens);
            }
            for policy in RECOVERY_POLICIES {
                let retry = self.run_window(&padded, request, policy, filter)?;
                if recovery::is_credible(&retry) {
                    tracing::info!(
                        audio_s = request.audio_length as f64 / SAMPLE_RATE as f64,
                        tokens = retry.tokens.len(),
                        policy = ?policy,
                        "empty decode recovered"
                    );
                    return Ok(retry.tokens);
                }
            }
            Ok(first.tokens)
        })
    }

    fn run_window(
        &mut self,
        padded: &[f32],
        request: &WindowRequest,
        policy: LengthPolicy,
        filter: Option<&TokenFilter<'_>>,
    ) -> Result<Hypothesis, AsrError> {
        let mut declared = request.audio_length;
        let mut audio_frames = request.audio_frames;
        if policy.trimmed_tail {
            // the declared length alone leaves the trimmed samples' mel
            // frames in the input, so they are silenced too
            let trimmed = recovery::trimmed_length(request.audio_length);
            let mut audio = padded.to_vec();
            let silenced = trimmed..request.audio_length.min(audio.len());
            if let Some(tail) = audio.get_mut(silenced) {
                tail.fill(0.0);
            }
            self.audio.write_f32(&audio).map_err(failed)?;
            declared = trimmed;
            audio_frames = trimmed.div_ceil(FRAME_SAMPLES);
        } else {
            self.audio.write_f32(padded).map_err(failed)?;
        }
        if policy.preprocessor_full {
            declared = MAX_MODEL_SAMPLES;
        }
        let length = i32::try_from(declared).map_err(|_| failed("audio length overflow"))?;
        self.audio_length.write_i32(&[length]).map_err(failed)?;

        let mel = self
            .preprocessor
            .predict(&self.preprocessor_inputs)
            .map_err(failed)?;
        let encoded = if policy.encoder_full {
            let features = mel.array(self.spec.mel).map_err(failed)?;
            let frames = features
                .shape()
                .map_err(failed)?
                .last()
                .copied()
                .ok_or_else(|| failed("mel has no dimensions"))?;
            let frames = i32::try_from(frames).map_err(|_| failed("mel length overflow"))?;
            self.full_mel_length.write_i32(&[frames]).map_err(failed)?;
            let inputs = Features::new(&[
                (self.spec.mel, &features),
                ("mel_length", &self.full_mel_length),
            ])
            .map_err(failed)?;
            self.encoder.predict(&inputs).map_err(failed)?
        } else {
            self.encoder.predict(&mel).map_err(failed)?
        };
        let encoder_length = encoded
            .array("encoder_length")
            .and_then(|a| a.first_i32())
            .map_err(failed)?;
        let tensor = encoded
            .array("encoder")
            .and_then(|a| a.read_f32())
            .map_err(failed)?;
        let frames = EncoderFrames::new(
            &tensor,
            ENCODER_HIDDEN,
            usize::try_from(encoder_length).unwrap_or(0),
        )
        .map_err(failed)?;

        let span = DecodeSpan {
            encoder_frames: frames.len(),
            audio_frames,
            start_frame: request.start_frame,
            frame_offset: request.frame_offset,
            is_last: request.is_last,
            emit_after_frame: request.emit_after_frame,
        };
        let mut transducer = CoreMlTransducer {
            decoder: &self.decoder,
            joint: &self.joint,
            frames: &frames,
            io: &mut self.steps,
            hidden: vec![0.0; DECODER_LAYERS * DECODER_HIDDEN],
            cell: vec![0.0; DECODER_LAYERS * DECODER_HIDDEN],
            top_k: filter.is_some(),
        };
        tdt::decode(&mut transducer, span, &self.config, filter)
    }
}

/// The prediction network's projection for one token and the LSTM state
/// after it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Prediction {
    pub output: Vec<f32>,
    pub hidden: Vec<f32>,
    pub cell: Vec<f32>,
}

/// Inputs of the prediction (decoder) and joint networks.
pub(crate) struct StepIo {
    pub targets: Array,
    pub hidden_in: Array,
    pub cell_in: Array,
    pub decoder_inputs: Features,
    pub encoder_step: Array,
    pub decoder_step: Array,
    pub joint_inputs: Features,
}

impl StepIo {
    pub(crate) fn new() -> Result<StepIo, String> {
        let targets = Array::zeros_i32(&[1, 1])?;
        let target_length = Array::zeros_i32(&[1])?;
        target_length.write_i32(&[1])?;
        let state = [DECODER_LAYERS, 1, DECODER_HIDDEN];
        let hidden_in = Array::zeros_f32(&state)?;
        let cell_in = Array::zeros_f32(&state)?;
        let decoder_inputs = Features::new(&[
            ("targets", &targets),
            ("target_length", &target_length),
            ("h_in", &hidden_in),
            ("c_in", &cell_in),
        ])?;
        let encoder_step = Array::zeros_f32(&[1, ENCODER_HIDDEN, 1])?;
        let decoder_step = Array::zeros_f32(&[1, DECODER_HIDDEN, 1])?;
        let joint_inputs = Features::new(&[
            ("encoder_step", &encoder_step),
            ("decoder_step", &decoder_step),
        ])?;
        Ok(StepIo {
            targets,
            hidden_in,
            cell_in,
            decoder_inputs,
            encoder_step,
            decoder_step,
            joint_inputs,
        })
    }

    /// Runs the prediction network on `token` from state `(hidden, cell)`.
    pub(crate) fn predict(
        &self,
        decoder: &Model,
        token: usize,
        hidden: &[f32],
        cell: &[f32],
    ) -> Result<Prediction, AsrError> {
        let token = i32::try_from(token).map_err(|_| failed("token id overflow"))?;
        self.targets.write_i32(&[token]).map_err(failed)?;
        self.hidden_in.write_f32(hidden).map_err(failed)?;
        self.cell_in.write_f32(cell).map_err(failed)?;
        let output = decoder.predict(&self.decoder_inputs).map_err(failed)?;
        let read = |name: &str, len: usize| -> Result<Vec<f32>, AsrError> {
            let values = output
                .array(name)
                .and_then(|a| a.read_f32())
                .map_err(failed)?
                .to_row_major();
            if values.len() != len {
                return Err(failed(format!("{name} has {} values", values.len())));
            }
            Ok(values)
        };
        Ok(Prediction {
            output: read("decoder", DECODER_HIDDEN)?,
            hidden: read("h_out", DECODER_LAYERS * DECODER_HIDDEN)?,
            cell: read("c_out", DECODER_LAYERS * DECODER_HIDDEN)?,
        })
    }
}

/// Zeroed decoder LSTM state.
pub(crate) fn zero_state() -> Vec<f32> {
    vec![0.0; DECODER_LAYERS * DECODER_HIDDEN]
}

/// The decoder LSTM and joint network over one window, starting from a
/// zero state.
struct CoreMlTransducer<'a> {
    decoder: &'a Model,
    joint: &'a Model,
    frames: &'a EncoderFrames,
    io: &'a mut StepIo,
    hidden: Vec<f32>,
    cell: Vec<f32>,
    /// Read the joint's top-K candidates for the script hint.
    top_k: bool,
}

impl Transducer for CoreMlTransducer<'_> {
    fn predict(&mut self, token: usize) -> Result<(), AsrError> {
        let prediction = self
            .io
            .predict(self.decoder, token, &self.hidden, &self.cell)?;
        self.hidden = prediction.hidden;
        self.cell = prediction.cell;
        self.io
            .decoder_step
            .write_f32(&prediction.output)
            .map_err(failed)
    }

    fn joint(&mut self, frame: usize) -> Result<JointDecision, AsrError> {
        let encoder = self
            .frames
            .frame(frame)
            .ok_or_else(|| failed(format!("encoder frame {frame} out of range")))?;
        self.io.encoder_step.write_f32(encoder).map_err(failed)?;
        let output = self.joint.predict(&self.io.joint_inputs).map_err(failed)?;
        let token = output
            .array("token_id")
            .and_then(|a| a.first_i32())
            .map_err(failed)?;
        let probability = output
            .array("token_prob")
            .and_then(|a| a.first_f32())
            .map_err(failed)?;
        let duration = output
            .array("duration")
            .and_then(|a| a.first_i32())
            .map_err(failed)?;
        let top_k = if self.top_k {
            let ids = output
                .array("top_k_ids")
                .and_then(|a| a.read_i32s())
                .map_err(failed)?;
            let logits = output
                .array("top_k_logits")
                .and_then(|a| a.read_f32())
                .map_err(failed)?
                .to_row_major();
            if ids.len() != logits.len() {
                return Err(failed(format!(
                    "top-K length mismatch: {} ids, {} logits",
                    ids.len(),
                    logits.len()
                )));
            }
            let ids = ids
                .into_iter()
                .map(|id| usize::try_from(id).map_err(|_| failed(format!("top-K id {id}"))))
                .collect::<Result<_, _>>()?;
            Some(TopK { ids, logits })
        } else {
            None
        };
        Ok(JointDecision {
            token: usize::try_from(token).map_err(|_| failed(format!("token id {token}")))?,
            probability,
            duration_bin: usize::try_from(duration)
                .map_err(|_| failed(format!("duration bin {duration}")))?,
            top_k,
        })
    }
}

pub(crate) fn failed(detail: impl Into<String>) -> AsrError {
    AsrError::TranscriptionFailed {
        detail: detail.into(),
    }
}

pub(crate) fn load_failed(what: &str, error: String) -> AsrError {
    AsrError::LoadFailed {
        detail: format!("{what}: {error}"),
    }
}
