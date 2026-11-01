//! Speech-engine interface shared by every inference backend (02 "Inference
//! runtime").

mod engine;
mod manifest;

pub use engine::{AsrError, LoadProgress, SpeechEngine};
pub use manifest::{ManifestFile, ModelManifest};
