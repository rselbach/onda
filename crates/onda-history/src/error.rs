use std::fmt;
use std::path::{Path, PathBuf};

use onda_domain::history::{MISSING_RECORDING_ERROR, WavTooLarge};

/// Errors from the history store. The display text is what the History
/// page shows under "History Error" (HIS-27).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryError {
    /// The recording does not fit in a WAV file (HIS-11).
    AudioTooLarge,
    /// The entry has no recording, or its file is gone (HIS-24).
    MissingRecording,
    /// A path is not a strict descendant of the history folder, or reaches
    /// it through a symlink (ARCH-12).
    UnsafePath(PathBuf),
    /// A file-system operation failed; the text names the action and path.
    Io(String),
}

impl fmt::Display for HistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AudioTooLarge => WavTooLarge.fmt(f),
            Self::MissingRecording => f.write_str(MISSING_RECORDING_ERROR),
            Self::UnsafePath(path) => write!(
                f,
                "Refusing to use {}: it is outside the history folder.",
                path.display()
            ),
            Self::Io(description) => f.write_str(description),
        }
    }
}

impl std::error::Error for HistoryError {}

pub(crate) fn io_error(action: &str, path: &Path, error: &std::io::Error) -> HistoryError {
    HistoryError::Io(format!("Could not {action} {}: {error}", path.display()))
}
