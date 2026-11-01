//! Parakeet Unified streaming, ported from FluidAudio's
//! `StreamingUnifiedAsrManager`, `UnifiedStreamingWindower`, and
//! `UnifiedRnntDecoder` with Jabber's 70/7/7 context (ASR-17).
//!
//! The encoder is stateless: each step re-encodes a `[left | chunk |
//! right]` window (5.6 s / 0.56 s / 0.56 s) whose attention mask was baked
//! in at conversion, and decodes only the frames no earlier step decoded,
//! holding back the look-ahead frames for the next step. Only the RNNT
//! decoder state and the transcript persist between steps, so a preview
//! costs only the audio since the last one and the final decode only
//! flushes the tail (ASR-20).

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Instant;

use onda_asr::{AsrError, LoadProgress, SpeechEngine};
use onda_domain::languages::Language;
use onda_domain::preview::{
    StreamFeed, UNIFIED_CHUNK_FRAMES, UNIFIED_FRAME_SAMPLES, UNIFIED_LEFT_CONTEXT_FRAMES,
    UNIFIED_LOOKAHEAD_FRAMES, UnifiedStream,
};

use crate::coreml::{Array, ComputeUnits, Features, Model};
use crate::engine::{Prediction, StepIo, failed, load_failed, load_model, zero_state};
use crate::mel::{MelExtractor, N_MELS};
use crate::tensor::EncoderFrames;
use crate::vocab::{UNKNOWN, Vocabulary, WORD_BOUNDARY};

const SAMPLE_RATE: usize = 16_000;
const ENCODER_HIDDEN: usize = 1024;
const BLANK: usize = 1024;
const MAX_SYMBOLS_PER_FRAME: usize = 10;
const CHUNK_SAMPLES: usize = UNIFIED_CHUNK_FRAMES * UNIFIED_FRAME_SAMPLES;
const LOOKAHEAD_SAMPLES: usize = UNIFIED_LOOKAHEAD_FRAMES * UNIFIED_FRAME_SAMPLES;
/// The encoder window: 6.72 s.
const WINDOW_SAMPLES: usize =
    (UNIFIED_LEFT_CONTEXT_FRAMES + UNIFIED_CHUNK_FRAMES + UNIFIED_LOOKAHEAD_FRAMES)
        * UNIFIED_FRAME_SAMPLES;
const VOCABULARY: &str = "vocab.json";

/// FluidAudio runs the int8 encoder on the Neural Engine and the tiny
/// per-token decoder and joint on the CPU.
const MODELS: [(&str, ComputeUnits); 3] = [
    (
        "parakeet_unified_encoder_streaming_70_7_7_int8.mlmodelc",
        ComputeUnits::CpuAndNeuralEngine,
    ),
    ("parakeet_unified_decoder.mlmodelc", ComputeUnits::CpuOnly),
    (
        "parakeet_unified_joint_decision_single_step.mlmodelc",
        ComputeUnits::CpuOnly,
    ),
];

/// One encoder step: which samples to encode and which frames to hold
/// back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowPlan {
    /// Global sample range encoded; the window is zero-padded past it.
    start: usize,
    end: usize,
    /// Global encoder frame of the window's first frame.
    start_frame: usize,
    /// Look-ahead frames not decoded yet (0 on the final window).
    holdback: usize,
}

/// FluidAudio `UnifiedStreamingWindower`, after NeMo's
/// `StreamingBatchedAudioBuffer`: the first step waits for chunk plus
/// look-ahead, later steps for one chunk each.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Windower {
    /// Samples fed to the encoder so far.
    consumed: usize,
    /// Encoder frames decoded so far.
    decoded: usize,
    /// The final (holdback 0) window was planned; it is planned once.
    finished: bool,
}

impl Windower {
    /// The next window for a recording of `total` samples, or `None` when
    /// too little new audio arrived (or, when `is_final`, none is left).
    fn next_window(&mut self, total: usize, is_final: bool) -> Option<WindowPlan> {
        if self.finished {
            return None;
        }
        let feed = if self.consumed == 0 {
            CHUNK_SAMPLES + LOOKAHEAD_SAMPLES
        } else {
            CHUNK_SAMPLES
        };
        let consumed = if self.consumed + feed <= total {
            self.consumed + feed
        } else if is_final && total > 0 && total >= self.consumed {
            // the tail, or a stream that ended on a chunk boundary whose
            // held-back look-ahead still needs decoding
            total
        } else {
            return None;
        };
        let is_last = is_final && consumed >= total;
        self.finished = is_last;
        // frame-aligned upward so the window never exceeds its size
        let start = consumed
            .saturating_sub(WINDOW_SAMPLES)
            .next_multiple_of(UNIFIED_FRAME_SAMPLES);
        self.consumed = consumed;
        Some(WindowPlan {
            start,
            end: consumed,
            start_frame: start / UNIFIED_FRAME_SAMPLES,
            holdback: if is_last { 0 } else { UNIFIED_LOOKAHEAD_FRAMES },
        })
    }

    /// The window-local frames to decode given the encoder's valid length;
    /// advances the decoded position.
    fn decode_range(&mut self, encoder_length: usize, plan: &WindowPlan) -> Option<Range<usize>> {
        let local_start = self.decoded.checked_sub(plan.start_frame)?;
        let local_end = encoder_length.checked_sub(plan.holdback)?;
        if local_end <= local_start {
            return None;
        }
        self.decoded += local_end - local_start;
        Some(local_start..local_end)
    }
}

/// The decoder and single-step joint of the RNNT.
trait Rnnt {
    /// Feeds `token` to the prediction network from state `(hidden, cell)`.
    fn predict(
        &mut self,
        token: usize,
        hidden: &[f32],
        cell: &[f32],
    ) -> Result<Prediction, AsrError>;
    /// The joint's best token for encoder `frame` and a prediction output.
    fn joint(&mut self, frame: usize, prediction: &[f32]) -> Result<usize, AsrError>;
}

/// RNNT decoder state carried across steps: `(hidden, cell)` is the state
/// before `last_token` was consumed.
#[derive(Debug, Clone, PartialEq)]
struct DecoderState {
    hidden: Vec<f32>,
    cell: Vec<f32>,
    last_token: usize,
}

impl DecoderState {
    fn new() -> DecoderState {
        DecoderState {
            hidden: zero_state(),
            cell: zero_state(),
            last_token: BLANK,
        }
    }
}

/// Greedy RNNT over `frames` (FluidAudio `UnifiedRnntDecoder.decode`):
/// up to ten tokens per frame, a blank moves to the next frame. The state
/// is committed only when the whole pass succeeds.
fn decode_frames(
    rnnt: &mut impl Rnnt,
    state: &mut DecoderState,
    frames: Range<usize>,
) -> Result<Vec<usize>, AsrError> {
    let mut current = state.clone();
    let mut emitted = Vec::new();
    let mut step = rnnt.predict(current.last_token, &current.hidden, &current.cell)?;
    for frame in frames {
        for _ in 0..MAX_SYMBOLS_PER_FRAME {
            let token = rnnt.joint(frame, &step.output)?;
            if token == BLANK {
                break;
            }
            emitted.push(token);
            current = DecoderState {
                hidden: step.hidden,
                cell: step.cell,
                last_token: token,
            };
            step = rnnt.predict(token, &current.hidden, &current.cell)?;
        }
    }
    *state = current;
    Ok(emitted)
}

/// One recording's stream.
struct Session {
    stream: UnifiedStream,
    windower: Windower,
    decoder: DecoderState,
    text: String,
}

impl Session {
    fn new() -> Session {
        Session {
            stream: UnifiedStream::new(),
            windower: Windower::default(),
            decoder: DecoderState::new(),
            text: String::new(),
        }
    }

    /// Starts a new recording.
    fn restart(&mut self) {
        self.windower = Windower::default();
        self.decoder = DecoderState::new();
        self.text.clear();
    }

    fn append(&mut self, vocab: &Vocabulary, tokens: &[usize]) {
        for &token in tokens {
            match vocab.get(token) {
                Some(piece) if !piece.is_empty() && piece != UNKNOWN => {
                    self.text.extend(piece.chars().map(
                        |c| {
                            if c == WORD_BOUNDARY { ' ' } else { c }
                        },
                    ));
                }
                _ => {}
            }
        }
    }
}

/// Parakeet Unified: previews and the final decode share one stream.
pub(crate) struct UnifiedEngine {
    dir: PathBuf,
    loaded: Option<Loaded>,
}

impl UnifiedEngine {
    pub(crate) fn new(dir: PathBuf) -> UnifiedEngine {
        UnifiedEngine { dir, loaded: None }
    }

    /// Streams the part of `samples` the session has not seen and, when
    /// `is_final`, flushes the tail. Returns the transcript so far.
    fn stream(&mut self, samples: &[f32], is_final: bool) -> Result<String, AsrError> {
        let loaded = self.loaded.as_mut().ok_or(AsrError::NotReady)?;
        if loaded.session.stream.feed(samples.len()) == StreamFeed::Restart {
            loaded.session.restart();
        }
        let started = Instant::now();
        let mut windows = 0;
        while let Some(plan) = loaded.session.windower.next_window(samples.len(), is_final) {
            if let Err(error) = objc2::rc::autoreleasepool(|_| loaded.step(samples, &plan)) {
                // the stream no longer matches the audio: start over next call
                loaded.session.stream.reset();
                return Err(error);
            }
            windows += 1;
        }
        tracing::debug!(
            audio_s = samples.len() as f64 / SAMPLE_RATE as f64,
            windows,
            decode_ms = started.elapsed().as_secs_f64() * 1e3,
            is_final,
            "unified stream step"
        );
        Ok(loaded.session.text.trim().to_owned())
    }
}

impl SpeechEngine for UnifiedEngine {
    fn load(&mut self, progress: &mut dyn FnMut(LoadProgress)) -> Result<(), AsrError> {
        if self.loaded.is_some() {
            return Ok(());
        }
        let started = Instant::now();
        let loaded = objc2::rc::autoreleasepool(|_| Loaded::load(&self.dir, progress))?;
        tracing::info!(
            model = "Parakeet Unified",
            load_ms = started.elapsed().as_millis() as u64,
            "model loaded"
        );
        self.loaded = Some(loaded);
        Ok(())
    }

    fn unload(&mut self) {
        self.loaded = None;
    }

    fn supports_preview(&self) -> bool {
        true
    }

    fn transcribe(
        &mut self,
        samples: &[f32],
        language: Option<Language>,
    ) -> Result<String, AsrError> {
        log_ignored(language);
        let started = Instant::now();
        let text = self.stream(samples, true);
        if let Some(loaded) = self.loaded.as_mut() {
            loaded.session.stream.reset();
        }
        tracing::debug!(
            audio_s = samples.len() as f64 / SAMPLE_RATE as f64,
            decode_ms = started.elapsed().as_secs_f64() * 1e3,
            "final decode"
        );
        text
    }

    fn preview(&mut self, samples: &[f32], language: Option<Language>) -> Result<String, AsrError> {
        log_ignored(language);
        let started = Instant::now();
        let text = self.stream(samples, false)?;
        tracing::info!(
            audio_s = samples.len() as f64 / SAMPLE_RATE as f64,
            decode_ms = started.elapsed().as_secs_f64() * 1e3,
            "preview decode"
        );
        Ok(text)
    }

    fn reset_preview(&mut self) {
        if let Some(loaded) = self.loaded.as_mut() {
            loaded.session.stream.reset();
        }
    }
}

fn log_ignored(language: Option<Language>) {
    if let Some(language) = language {
        tracing::debug!(
            language = language.code(),
            model = "Parakeet Unified",
            "the model ignores the language"
        );
    }
}

/// The loaded models, reusable inputs, and the stream.
struct Loaded {
    vocab: Vocabulary,
    mel: MelExtractor,
    encoder: Model,
    decoder: Model,
    joint: Model,
    mel_input: Array,
    mel_length: Array,
    encoder_inputs: Features,
    steps: StepIo,
    session: Session,
}

impl Loaded {
    fn load(dir: &Path, progress: &mut dyn FnMut(LoadProgress)) -> Result<Loaded, AsrError> {
        let mut models = Vec::with_capacity(MODELS.len());
        for (index, (name, units)) in MODELS.iter().enumerate() {
            models.push(load_model(
                dir,
                name,
                *units,
                index,
                MODELS.len(),
                progress,
            )?);
        }
        let [encoder, decoder, joint]: [Model; 3] = models
            .try_into()
            .map_err(|_| load_failed("models", "unexpected model count".to_owned()))?;
        let json = std::fs::read_to_string(dir.join(VOCABULARY))
            .map_err(|error| load_failed(VOCABULARY, error.to_string()))?;
        let vocab =
            Vocabulary::parse(&json, BLANK).map_err(|error| load_failed(VOCABULARY, error))?;
        let mel = MelExtractor::new(WINDOW_SAMPLES);
        let buffers = || -> Result<(Array, Array, Features, StepIo), String> {
            let mel_input = Array::zeros_f32(&[1, N_MELS, mel.total_frames()])?;
            let mel_length = Array::zeros_i32(&[1])?;
            let inputs = Features::new(&[("mel", &mel_input), ("mel_length", &mel_length)])?;
            Ok((mel_input, mel_length, inputs, StepIo::new()?))
        };
        let (mel_input, mel_length, encoder_inputs, steps) =
            buffers().map_err(|error| load_failed("input buffers", error))?;
        progress(LoadProgress {
            status: String::new(),
            fraction: 1.0,
        });
        Ok(Loaded {
            vocab,
            mel,
            encoder,
            decoder,
            joint,
            mel_input,
            mel_length,
            encoder_inputs,
            steps,
            session: Session::new(),
        })
    }

    /// Encodes one window and decodes its new frames into the transcript.
    fn step(&mut self, samples: &[f32], plan: &WindowPlan) -> Result<(), AsrError> {
        let window = samples
            .get(plan.start..plan.end)
            .ok_or_else(|| failed("streaming window outside the audio"))?;
        let (mel, valid) = self.mel.features(window);
        self.mel_input.write_f32(&mel).map_err(failed)?;
        let valid = i32::try_from(valid).map_err(|_| failed("mel length overflow"))?;
        self.mel_length.write_i32(&[valid]).map_err(failed)?;
        let encoded = self.encoder.predict(&self.encoder_inputs).map_err(failed)?;
        let length = encoded
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
            usize::try_from(length).unwrap_or(0),
        )
        .map_err(failed)?;
        let Some(range) = self.session.windower.decode_range(frames.len(), plan) else {
            return Ok(());
        };
        let mut rnnt = CoreMlRnnt {
            decoder: &self.decoder,
            joint: &self.joint,
            frames: &frames,
            io: &self.steps,
        };
        let tokens = decode_frames(&mut rnnt, &mut self.session.decoder, range)?;
        self.session.append(&self.vocab, &tokens);
        Ok(())
    }
}

struct CoreMlRnnt<'a> {
    decoder: &'a Model,
    joint: &'a Model,
    frames: &'a EncoderFrames,
    io: &'a StepIo,
}

impl Rnnt for CoreMlRnnt<'_> {
    fn predict(
        &mut self,
        token: usize,
        hidden: &[f32],
        cell: &[f32],
    ) -> Result<Prediction, AsrError> {
        self.io.predict(self.decoder, token, hidden, cell)
    }

    fn joint(&mut self, frame: usize, prediction: &[f32]) -> Result<usize, AsrError> {
        let encoder = self
            .frames
            .frame(frame)
            .ok_or_else(|| failed(format!("encoder frame {frame} out of range")))?;
        self.io.encoder_step.write_f32(encoder).map_err(failed)?;
        self.io.decoder_step.write_f32(prediction).map_err(failed)?;
        let output = self.joint.predict(&self.io.joint_inputs).map_err(failed)?;
        let token = output
            .array("token_id")
            .and_then(|a| a.first_i32())
            .map_err(failed)?;
        usize::try_from(token).map_err(|_| failed(format!("token id {token}")))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    #[test]
    fn window_constants_match_the_70_7_7_export() {
        assert_eq!(WINDOW_SAMPLES, 107_520);
        assert_eq!(CHUNK_SAMPLES, 8_960);
        assert_eq!(MelExtractor::new(WINDOW_SAMPLES).total_frames(), 673);
    }

    /// Feeds growing totals, then a final call, and returns every plan.
    fn plans(totals: &[usize], final_total: Option<usize>) -> Vec<WindowPlan> {
        let mut windower = Windower::default();
        let mut plans = Vec::new();
        let mut run = |windower: &mut Windower, total: usize, is_final: bool| {
            while let Some(plan) = windower.next_window(total, is_final) {
                // pretend the encoder covers every whole frame of audio
                let length = (plan.end - plan.start).div_ceil(UNIFIED_FRAME_SAMPLES);
                windower.decode_range(length, &plan);
                plans.push(plan);
            }
        };
        for &total in totals {
            run(&mut windower, total, false);
        }
        if let Some(total) = final_total {
            run(&mut windower, total, true);
        }
        plans
    }

    #[test]
    fn windower_streams_chunks_and_flushes_the_tail() {
        struct Case {
            name: &'static str,
            totals: Vec<usize>,
            final_total: Option<usize>,
            want_ends: Vec<usize>,
        }
        let cases = [
            Case {
                name: "first step waits for chunk plus look-ahead",
                totals: vec![17_919],
                final_total: None,
                want_ends: vec![],
            },
            Case {
                name: "then one chunk per step",
                totals: vec![17_920, 26_879, 26_880, 40_000],
                final_total: None,
                want_ends: vec![17_920, 26_880, 35_840],
            },
            Case {
                name: "final call flushes the tail once",
                totals: vec![20_000],
                final_total: Some(30_000),
                want_ends: vec![17_920, 26_880, 30_000],
            },
            Case {
                name: "stream ending on a chunk boundary re-encodes the look-ahead",
                totals: vec![26_880],
                final_total: Some(26_880),
                want_ends: vec![17_920, 26_880, 26_880],
            },
            Case {
                name: "short final recording is one window",
                totals: vec![],
                final_total: Some(5_000),
                want_ends: vec![5_000],
            },
            Case {
                name: "empty final recording plans nothing",
                totals: vec![],
                final_total: Some(0),
                want_ends: vec![],
            },
        ];
        for tc in cases {
            let ends: Vec<usize> = plans(&tc.totals, tc.final_total)
                .iter()
                .map(|p| p.end)
                .collect();
            assert_eq!(ends, tc.want_ends, "{}", tc.name);
        }
    }

    #[test]
    fn windows_slide_and_decode_every_frame_once() {
        let total = 400_000;
        let totals: Vec<usize> = (4_800..total).step_by(4_800).collect();
        let plans = plans(&totals, Some(total));
        let mut windower = Windower::default();
        let mut decoded = Vec::new();
        for plan in &plans {
            assert!(plan.end - plan.start <= WINDOW_SAMPLES, "{plan:?}");
            assert_eq!(plan.start % UNIFIED_FRAME_SAMPLES, 0, "{plan:?}");
            // replay the decode ranges to collect the global frames
            let length = (plan.end - plan.start).div_ceil(UNIFIED_FRAME_SAMPLES);
            if let Some(range) = windower.decode_range(length, plan) {
                decoded.extend(range.map(|f| f + plan.start_frame));
            }
        }
        let want: Vec<usize> = (0..total.div_ceil(UNIFIED_FRAME_SAMPLES)).collect();
        assert_eq!(decoded, want);
        assert_eq!(plans.last().map(|p| p.holdback), Some(0));
    }

    /// Replays joint tokens per frame and records decoder calls.
    struct Scripted {
        /// Tokens the joint returns, in call order; blank once exhausted.
        joints: VecDeque<usize>,
        predicted: Vec<usize>,
    }

    impl Rnnt for Scripted {
        fn predict(
            &mut self,
            token: usize,
            hidden: &[f32],
            _cell: &[f32],
        ) -> Result<Prediction, AsrError> {
            self.predicted.push(token);
            // the state counts consumed tokens, to check what is committed
            let consumed = hidden.first().copied().unwrap_or(0.0) + 1.0;
            Ok(Prediction {
                output: vec![token as f32],
                hidden: vec![consumed],
                cell: vec![0.0],
            })
        }

        fn joint(&mut self, _frame: usize, prediction: &[f32]) -> Result<usize, AsrError> {
            assert_eq!(prediction.len(), 1);
            Ok(self.joints.pop_front().unwrap_or(BLANK))
        }
    }

    #[test]
    fn rnnt_decode_carries_state_across_steps() {
        let mut rnnt = Scripted {
            // frame 0: Troy Barnes, blank; frame 1: blank; frame 2: Abed, blank
            joints: [5, 6, BLANK, BLANK, 7, BLANK].into(),
            predicted: Vec::new(),
        };
        let mut state = DecoderState {
            hidden: vec![0.0],
            cell: vec![0.0],
            last_token: BLANK,
        };
        let tokens = decode_frames(&mut rnnt, &mut state, 0..3).expect("decode");
        assert_eq!(tokens, vec![5, 6, 7]);
        assert_eq!(rnnt.predicted, vec![BLANK, 5, 6, 7]);
        // the committed state is the one before the last token was consumed
        assert_eq!(state.last_token, 7);
        assert_eq!(state.hidden, vec![3.0]);

        // the next step re-derives the prediction from the last token
        rnnt.predicted.clear();
        let tokens = decode_frames(&mut rnnt, &mut state, 0..1).expect("decode");
        assert!(tokens.is_empty());
        assert_eq!(rnnt.predicted, vec![7]);
    }

    #[test]
    fn rnnt_caps_symbols_per_frame() {
        let mut rnnt = Scripted {
            joints: std::iter::repeat_n(9, 25).collect(),
            predicted: Vec::new(),
        };
        let mut state = DecoderState::new();
        let tokens = decode_frames(&mut rnnt, &mut state, 0..2).expect("decode");
        assert_eq!(tokens.len(), 2 * MAX_SYMBOLS_PER_FRAME);
    }

    #[test]
    fn session_restarts_on_a_shorter_recording() {
        let vocab = Vocabulary::new(
            ["<unk>", "▁Troy", "▁Bar", "nes"]
                .map(str::to_owned)
                .to_vec(),
        );
        let mut session = Session::new();
        assert_eq!(session.stream.feed(32_000), StreamFeed::Restart);
        session.append(&vocab, &[1, 0, 2, 3]);
        session.windower.consumed = 32_000;
        assert_eq!(session.text.trim(), "Troy Barnes");
        assert_eq!(
            session.stream.feed(40_000),
            StreamFeed::Continue { start: 32_000 }
        );
        assert_eq!(session.stream.feed(16_000), StreamFeed::Restart);
        session.restart();
        assert_eq!(session.text, "");
        assert_eq!(session.windower, Windower::default());
        assert_eq!(session.decoder, DecoderState::new());
    }
}
