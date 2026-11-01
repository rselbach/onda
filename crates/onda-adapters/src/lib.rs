//! Implements the session engine's core-side seams
//! ([`onda_app::seams`]) with Onda's portable crates, so every composition
//! root (the macOS FFI layer, the dev CLI) wires the same code.
//!
//! The inference backend is not chosen here: [`Models`] takes an engine
//! factory, so the platform-specific backend stays in the composition root
//! (ARCH-1).

mod audio;
mod history;
mod models;
mod secrets;
mod settings;

use std::path::{Path, PathBuf};

pub use audio::{Microphone, forward_audio_events};
pub use history::History;
pub use models::{
    DownloadListener, EngineFactory, Models, download_update, forward_download_events,
};
pub use secrets::{Keychain, OpenRouter};
pub use settings::SettingsAdapter;

/// Where Onda keeps its files under the platform data directory (02 "Data
/// on disk").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataPaths {
    root: PathBuf,
}

impl DataPaths {
    /// Paths under `root`, e.g. `~/Library/Application Support/Onda`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The data directory itself.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `<data>/settings.json` (SET-1).
    pub fn settings_file(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    /// `<data>/History` (HIS-5).
    pub fn history_dir(&self) -> PathBuf {
        self.root.join("History")
    }

    /// `<data>/Models` (02 "Data on disk").
    pub fn models_dir(&self) -> PathBuf {
        self.root.join("Models")
    }
}
