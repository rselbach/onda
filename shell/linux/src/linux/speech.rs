//! CPU inference, live preview, and pinned GGUF downloads for Linux.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Instant;

use onda_asr::{AsrError, LoadProgress, ManifestFile, ModelManifest, SpeechEngine};
use onda_domain::languages::Language;
use onda_domain::models::{Catalog, ModelId};
use transcribe_cpp::{Backend, Model, ModelOptions, RunOptions, StreamOptions};

pub fn manifest(model: ModelId) -> Option<&'static ModelManifest> {
    match model {
        ModelId::ParakeetTdtV2 => Some(&ModelManifest {
            base_url: "https://huggingface.co/handy-computer/parakeet-tdt-0.6b-v2-gguf/resolve/07cee0616125a08ef619729bb47f40ef747e4bc4",
            files: &V2,
        }),
        ModelId::ParakeetUltra => Some(&ModelManifest {
            base_url: "https://huggingface.co/handy-computer/parakeet-ultra-gguf/resolve/a1539be1b849ea7463b88343e5277b373d0c4ea9",
            files: &ULTRA,
        }),
        ModelId::ParakeetJapanese => Some(&ModelManifest {
            base_url: "https://huggingface.co/ASRISEx/parakeet-tdt_ctc-0.6b-ja-gguf/resolve/9ac30ce2f51e603770b7b6b6cbfee77774286482",
            files: &JAPANESE,
        }),
        ModelId::ParakeetUnified => Some(&ModelManifest {
            base_url: "https://huggingface.co/handy-computer/parakeet-unified-en-0.6b-gguf/resolve/7e948f21b7bdbac698d3318db9d350f1096f3b6c",
            files: &UNIFIED,
        }),
        _ => None,
    }
}

const V2: [ManifestFile; 1] = [ManifestFile {
    path: "parakeet-tdt-0.6b-v2-Q8_0.gguf",
    size: 729_574_912,
    sha256: Some("f0d0e99cebb6d3b83f1f7069b82b5d3c2e39a54545b0da039cb4bafd9c4e5caa"),
}];
const ULTRA: [ManifestFile; 1] = [ManifestFile {
    path: "parakeet-ultra-Q8_0.gguf",
    size: 740_363_168,
    sha256: Some("007a59761e9258779f189df396b50d12b83bb5b79e66df4b955c230d2f2a0a59"),
}];
const JAPANESE: [ManifestFile; 1] = [ManifestFile {
    path: "parakeet-tdt_ctc-0.6b-ja-Q8_0.gguf",
    size: 732_523_296,
    sha256: Some("fb9b510f24beba192c13987207b124b907e8320629894e753faa2ba883075cf7"),
}];
const UNIFIED: [ManifestFile; 1] = [ManifestFile {
    path: "parakeet-unified-en-0.6b-Q8_0.gguf",
    size: 731_357_568,
    sha256: Some("4b50b6dd862bf6e346929aaf4f5eaacec003bfa3f56462d6c874b41ef2f38795"),
}];

// fewest CPUs that run the live preview. the stream encodes a 7.68 s window
// for every 1.04 s of speech, a little over 2 CPU-seconds per second of
// audio on the arm64 release baseline: two cores took 1.13 s for each window
// and fell behind the speaker, four took 0.6 s
const PREVIEW_MIN_CPUS: usize = 4;

// one stream chunk, 1.04 s. the stream encodes a window for every chunk a
// feed completes, and the final decode waits for the preview call in flight,
// so a preview that fell behind delays a stop by one window, not its backlog
const PREVIEW_FEED_SAMPLES: usize = 16_640;

/// CPUs this process may run on.
pub fn cpus() -> usize {
    std::thread::available_parallelism().map_or(1, usize::from)
}

/// The Speech page's note on CPU inference and live preview.
pub fn preview_note(cpus: usize) -> String {
    if cpus < PREVIEW_MIN_CPUS {
        return format!(
            "Linux uses CPU inference. Live preview needs {PREVIEW_MIN_CPUS} CPU cores, and \
             this computer has {cpus}."
        );
    }
    let catalog = Catalog::gguf();
    let streaming: Vec<&str> = catalog
        .models()
        .iter()
        .filter(|model| model.live_preview)
        .map(|model| model.name)
        .collect();
    format!(
        "Linux uses CPU inference. Live preview works with {}.",
        streaming.join(" and ")
    )
}

pub fn engine(id: ModelId, directory: &Path) -> Option<Box<dyn SpeechEngine>> {
    let manifest = manifest(id)?;
    Some(Box::new(Gguf {
        path: directory.join(manifest.files[0].path),
        previews: previews(id, cpus()),
        stream: None,
        model: None,
    }))
}

// whether `id` shows a live preview on a computer with `cpus` CPUs
fn previews(id: ModelId, cpus: usize) -> bool {
    cpus >= PREVIEW_MIN_CPUS
        && Catalog::gguf()
            .model(id)
            .is_some_and(|model| model.live_preview)
}

struct Gguf {
    path: PathBuf,
    previews: bool,
    // Drop the worker stream before unloading the model it borrows from.
    stream: Option<PreviewStream>,
    model: Option<Model>,
}

impl SpeechEngine for Gguf {
    fn load(&mut self, progress: &mut dyn FnMut(LoadProgress)) -> Result<(), AsrError> {
        if self.model.is_some() {
            return Ok(());
        }
        progress(LoadProgress {
            status: "Loading CPU speech model…".into(),
            fraction: 0.0,
        });
        transcribe_cpp::init_backends_default().map_err(|error| AsrError::LoadFailed {
            detail: error.to_string(),
        })?;
        self.model = Some(
            Model::load_with(
                &self.path,
                &ModelOptions {
                    backend: Backend::Cpu,
                    ..Default::default()
                },
            )
            .map_err(|error| AsrError::LoadFailed {
                detail: error.to_string(),
            })?,
        );
        Ok(())
    }

    fn unload(&mut self) {
        self.reset_preview();
        self.model = None;
    }
    fn supports_preview(&self) -> bool {
        self.previews
    }
    // the preview stream's text is not the offline decode's (the stream drops
    // the last period of jfk.wav), so the final transcript never comes from
    // it: with or without a preview, it is one offline decode of the recording
    fn transcribe(&mut self, samples: &[f32], _: Option<Language>) -> Result<String, AsrError> {
        // the library refuses an offline run while a stream is active
        self.reset_preview();
        let model = self.model.as_ref().ok_or(AsrError::NotReady)?;
        // A fresh session releases recording-sized inference scratch after each run.
        let mut session = model.session().map_err(failed)?;
        let transcript = session
            .run(samples, &RunOptions::default())
            .map_err(failed)?;
        Ok(transcript.text.trim().to_owned())
    }
    fn preview(&mut self, samples: &[f32], _: Option<Language>) -> Result<String, AsrError> {
        if !self.previews {
            return Err(AsrError::Unsupported {
                message: "Live preview is unavailable.".into(),
            });
        }
        // fewer samples than the last call: a new recording
        if self
            .stream
            .as_ref()
            .is_some_and(|stream| samples.len() < stream.heard)
        {
            self.reset_preview();
        }
        let model = self.model.as_ref().ok_or(AsrError::NotReady)?;
        let stream = match &mut self.stream {
            Some(stream) => stream,
            None => self.stream.insert(PreviewStream::start(model)?),
        };
        let started = Instant::now();
        let text = stream.feed(samples);
        match &text {
            Ok(text) => tracing::info!(
                audio_ms = samples.len() / 16,
                streamed_ms = stream.fed / 16,
                decode_ms = started.elapsed().as_millis() as u64,
                chars = text.chars().count(),
                "preview decode"
            ),
            // a failed stream takes no more audio; the next call starts over
            Err(_) => self.reset_preview(),
        }
        text
    }
    fn reset_preview(&mut self) {
        if let Some(stream) = self.stream.take() {
            stream.end();
        }
    }
}

/// One recording's streaming decode.
///
/// A library stream borrows its session, so both live on a thread of their
/// own that takes audio and answers with the text so far.
struct PreviewStream {
    audio: Option<mpsc::Sender<Vec<f32>>>,
    text: mpsc::Receiver<Result<String, transcribe_cpp::Error>>,
    worker: Option<JoinHandle<()>>,
    /// Samples of the recording the stream has taken.
    fed: usize,
    /// Samples the last call carried, which a capped feed leaves above `fed`.
    heard: usize,
}

impl PreviewStream {
    fn start(model: &Model) -> Result<PreviewStream, AsrError> {
        let mut session = model.session().map_err(failed)?;
        let (audio, heard) = mpsc::channel::<Vec<f32>>();
        let (say, text) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("onda-preview".into())
            .spawn(move || {
                let begun = session.stream(&RunOptions::default(), &StreamOptions::default());
                let mut stream = match begun {
                    Ok(stream) => stream,
                    Err(error) => {
                        let _ = say.send(Err(error));
                        return;
                    }
                };
                let _ = say.send(Ok(String::new()));
                for samples in heard {
                    let fed = if samples.is_empty() {
                        Ok(())
                    } else {
                        stream.feed(&samples).map(drop)
                    };
                    let _ = say.send(fed.map(|()| stream.text().full));
                }
            })
            .map_err(failed)?;
        let stream = PreviewStream {
            audio: Some(audio),
            text,
            worker: Some(worker),
            fed: 0,
            heard: 0,
        };
        stream.answer()?;
        Ok(stream)
    }

    /// Streams the part of `samples` the stream has not taken, up to
    /// [`PREVIEW_FEED_SAMPLES`], and returns the text of everything taken.
    fn feed(&mut self, samples: &[f32]) -> Result<String, AsrError> {
        self.heard = samples.len();
        let end = samples.len().min(self.fed + PREVIEW_FEED_SAMPLES);
        let audio = self
            .audio
            .as_ref()
            .ok_or_else(|| failed("preview stream ended"))?;
        audio
            .send(samples[self.fed..end].to_vec())
            .map_err(failed)?;
        let text = self.answer()?;
        self.fed = end;
        Ok(text.trim().to_owned())
    }

    fn answer(&self) -> Result<String, AsrError> {
        self.text.recv().map_err(failed)?.map_err(failed)
    }

    /// Ends the stream and waits until the library has let go of it, so the
    /// next offline run is not refused.
    fn end(self) {
        drop(self);
    }
}

impl Drop for PreviewStream {
    fn drop(&mut self) {
        self.audio.take();
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            tracing::error!("the preview stream panicked");
        }
    }
}

fn failed(error: impl std::fmt::Display) -> AsrError {
    AsrError::TranscriptionFailed {
        detail: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_linux_model_has_a_pinned_verified_download() {
        for model in onda_domain::models::Catalog::gguf().models() {
            let manifest = manifest(model.id).unwrap();
            let revision = manifest.base_url.rsplit('/').next().unwrap();
            assert_eq!(revision.len(), 40);
            assert!(revision.bytes().all(|byte| byte.is_ascii_hexdigit()));
            assert_eq!(manifest.files.len(), 1);
            let file = &manifest.files[0];
            assert!(file.path.ends_with(".gguf"));
            assert_eq!(
                model.size_hint,
                format!("~{}MB", (file.size as f64 / 1e6).round()),
                "{}",
                model.id
            );
            let digest = file.sha256.unwrap();
            assert_eq!(digest.len(), 64);
            assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn preview_needs_a_streaming_model_and_enough_cpus() {
        let cases = [
            ("v2 does not stream", ModelId::ParakeetTdtV2, 8, false),
            ("ultra does not stream", ModelId::ParakeetUltra, 8, false),
            (
                "japanese does not stream",
                ModelId::ParakeetJapanese,
                8,
                false,
            ),
            ("unified on 8 CPUs", ModelId::ParakeetUnified, 8, true),
            ("unified on 4 CPUs", ModelId::ParakeetUnified, 4, true),
            ("unified on 3 CPUs", ModelId::ParakeetUnified, 3, false),
            ("unified on 2 CPUs", ModelId::ParakeetUnified, 2, false),
        ];
        for (name, id, cpus, want) in cases {
            assert_eq!(previews(id, cpus), want, "{name}");
        }
    }

    #[test]
    fn speech_page_note_says_where_preview_runs() {
        let cases = [
            (
                8,
                "Linux uses CPU inference. Live preview works with Parakeet Unified.",
            ),
            (
                4,
                "Linux uses CPU inference. Live preview works with Parakeet Unified.",
            ),
            (
                2,
                "Linux uses CPU inference. Live preview needs 4 CPU cores, and this computer has 2.",
            ),
        ];
        for (cpus, want) in cases {
            assert_eq!(preview_note(cpus), want, "{cpus} CPUs");
        }
    }

    #[test]
    fn an_engine_without_preview_refuses_one() {
        let mut engine = Gguf {
            path: PathBuf::from("greendale.gguf"),
            model: None,
            previews: false,
            stream: None,
        };
        assert!(!engine.supports_preview());
        assert!(matches!(
            engine.preview(&[0.0; 16_000], None),
            Err(AsrError::Unsupported { .. })
        ));
    }

    /// The folder in `ONDA_TEST_MODEL_DIR`, holding the pinned GGUF files and
    /// whisper.cpp's `jfk.wav`. The tests that need it pass without running
    /// when the variable is not set, as in CI, which has no model files.
    fn fixtures() -> Option<PathBuf> {
        let folder = std::env::var_os("ONDA_TEST_MODEL_DIR").map(PathBuf::from);
        if folder.is_none() {
            eprintln!("skipped: set ONDA_TEST_MODEL_DIR to run against the real models");
        }
        folder
    }

    fn loaded(fixtures: &Path, id: ModelId, previews: bool) -> Gguf {
        let mut engine = Gguf {
            path: fixtures.join(manifest(id).unwrap().files[0].path),
            model: None,
            previews,
            stream: None,
        };
        engine.load(&mut |_| {}).unwrap();
        engine
    }

    fn jfk(fixtures: &Path) -> Vec<f32> {
        hound::WavReader::open(fixtures.join("jfk.wav"))
            .unwrap()
            .samples::<i16>()
            .map(|sample| f32::from(sample.unwrap()) / 32_768.0)
            .collect()
    }

    #[test]
    fn the_catalog_offers_preview_for_exactly_the_models_that_stream() {
        let Some(fixtures) = fixtures() else { return };
        for model in Catalog::gguf().models() {
            let engine = loaded(&fixtures, model.id, false);
            let streams = engine
                .model
                .as_ref()
                .unwrap()
                .capabilities()
                .unwrap()
                .supports_streaming;
            assert_eq!(model.live_preview, streams, "{}", model.name);
        }
    }

    #[test]
    fn preview_streams_the_recording_and_leaves_the_final_transcript_alone() {
        let Some(fixtures) = fixtures() else { return };
        let recording = jfk(&fixtures);
        let without_preview = loaded(&fixtures, ModelId::ParakeetUnified, false)
            .transcribe(&recording, None)
            .unwrap();
        assert!(without_preview.contains("country"), "{without_preview}");

        let mut engine = loaded(&fixtures, ModelId::ParakeetUnified, true);
        // the preview loop's ticks: the whole recording so far, 300 ms apart
        let ticks = || (8_000..recording.len()).step_by(4_800);
        let previews: Vec<String> = ticks()
            .map(|end| engine.preview(&recording[..end], None).unwrap())
            .collect();
        let halfway = &previews[previews.len() / 2];
        assert!(halfway.contains("fellow Americans"), "{halfway}");
        assert!(!halfway.contains("country"), "{halfway}");
        for pair in previews.windows(2) {
            assert!(pair[1].starts_with(&pair[0]), "{pair:?}");
        }
        assert_eq!(
            engine.transcribe(&recording, None).unwrap(),
            without_preview
        );

        // a shorter call is a new recording, with nothing of the last one
        assert_eq!(engine.preview(&recording[..8_000], None).unwrap(), "");
        let again: Vec<String> = ticks()
            .map(|end| engine.preview(&recording[..end], None).unwrap())
            .collect();
        assert_eq!(again, previews);
        engine.reset_preview();
        assert_eq!(
            engine.transcribe(&recording, None).unwrap(),
            without_preview
        );
    }

    #[test]
    fn a_preview_call_streams_one_chunk_at_most() {
        let Some(fixtures) = fixtures() else { return };
        let recording = jfk(&fixtures);
        let mut engine = loaded(&fixtures, ModelId::ParakeetUnified, true);
        // the whole recording at once, as after a long stall
        engine.preview(&recording, None).unwrap();
        assert_eq!(engine.stream.as_ref().unwrap().fed, PREVIEW_FEED_SAMPLES);
        let caught_up = (0..recording.len().div_ceil(PREVIEW_FEED_SAMPLES))
            .map(|_| engine.preview(&recording, None).unwrap())
            .last()
            .unwrap();
        assert_eq!(engine.stream.as_ref().unwrap().fed, recording.len());
        assert!(caught_up.contains("Ask what you"), "{caught_up}");

        // a new recording longer than what a capped stream took of the last
        engine.reset_preview();
        let behind = (0..3)
            .map(|_| engine.preview(&recording, None).unwrap())
            .last()
            .unwrap();
        assert!(behind.contains("And so"), "{behind}");
        let taken = engine.stream.as_ref().unwrap().fed;
        assert_eq!(
            engine.preview(&recording[..taken + 8_000], None).unwrap(),
            ""
        );
        assert_eq!(engine.stream.as_ref().unwrap().fed, PREVIEW_FEED_SAMPLES);
    }
}
