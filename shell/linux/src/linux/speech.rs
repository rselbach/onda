//! CPU inference and pinned GGUF downloads for Linux.

use std::path::{Path, PathBuf};

use onda_asr::{AsrError, LoadProgress, ManifestFile, ModelManifest, SpeechEngine};
use onda_domain::{languages::Language, models::ModelId};
use transcribe_cpp::{Backend, Model, ModelOptions, RunOptions};

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
const UNIFIED: [ManifestFile; 1] = [ManifestFile {
    path: "parakeet-unified-en-0.6b-Q8_0.gguf",
    size: 731_357_568,
    sha256: Some("4b50b6dd862bf6e346929aaf4f5eaacec003bfa3f56462d6c874b41ef2f38795"),
}];

pub fn engine(id: ModelId, directory: &Path) -> Option<Box<dyn SpeechEngine>> {
    let manifest = manifest(id)?;
    Some(Box::new(Gguf {
        path: directory.join(manifest.files[0].path),
        model: None,
    }))
}

struct Gguf {
    path: PathBuf,
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
        self.model = None;
    }
    fn supports_preview(&self) -> bool {
        false
    }
    fn transcribe(&mut self, samples: &[f32], _: Option<Language>) -> Result<String, AsrError> {
        let model = self.model.as_ref().ok_or(AsrError::NotReady)?;
        // A fresh session releases recording-sized inference scratch after each run.
        let mut session = model
            .session()
            .map_err(|error| AsrError::TranscriptionFailed {
                detail: error.to_string(),
            })?;
        let transcript = session
            .run(samples, &RunOptions::default())
            .map_err(|error| AsrError::TranscriptionFailed {
                detail: error.to_string(),
            })?;
        Ok(transcript.text.trim().to_owned())
    }
    fn preview(&mut self, _: &[f32], _: Option<Language>) -> Result<String, AsrError> {
        Err(AsrError::Unsupported {
            message: "Live preview is unavailable with the CPU backend.".into(),
        })
    }
    fn reset_preview(&mut self) {}
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
            assert!(file.size > 0);
            let digest = file.sha256.unwrap();
            assert_eq!(digest.len(), 64);
            assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
        }
    }
}
