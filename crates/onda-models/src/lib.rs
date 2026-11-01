//! Speech model manager: downloads, validates, and deletes model folders
//! under `<data>/Models/<model-id>/` (DL-1..DL-15).
//!
//! The manager knows nothing about inference runtimes. The backend owns the
//! file list of every model (MOD-1, DL-6) and injects it as an
//! [`onda_asr::ModelManifest`] lookup.

mod error;
mod event;
mod integrity;
mod manager;
mod remove;
mod validate;

#[cfg(test)]
mod tests;

pub use error::Error;
pub use event::{DownloadEvent, DownloadEventKind};
pub use integrity::VERIFIED_MARKER;
pub use manager::ModelManager;
pub use validate::validate_folder;
