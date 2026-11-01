//! Speech-engine interface shared by every inference backend.

mod engine;
mod manifest;

pub use engine::{AsrError, LoadProgress, SpeechEngine};
pub use manifest::{ManifestFile, ModelManifest};
