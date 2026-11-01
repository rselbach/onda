//! Transcript history store (prd/06): one folder per entry under
//! `<data>/History/`, each holding `metadata.json` and, when audio is kept,
//! `audio.wav` (HIS-5).
//!
//! The rules (metadata format, folder names, load-time cleanup, retention,
//! WAV encoding) live in [`onda_domain::history`]; this crate only reads and
//! writes the files and keeps the newest-first list in memory (HIS-9).

mod error;
mod fs;
mod store;

#[cfg(test)]
mod tests;

pub use error::HistoryError;
pub use store::{HistoryHooks, HistoryPreferences, HistoryStore};
