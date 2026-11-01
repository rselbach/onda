use std::fmt;
use std::path::PathBuf;

use onda_domain::models::{ModelError, ModelId};

/// Errors from model management.
///
/// Cloneable because every caller that joined a download receives the same
/// outcome (DL-1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The user cancelled the download; partial data is kept (DL-3).
    Cancelled,
    /// A model error with domain copy (unknown model, incomplete download,
    /// timeout).
    Model(ModelError),
    /// The download failed after retries; the description is shown to the
    /// user (DL-4).
    Download(String),
    /// The model is downloading, so its folder cannot be deleted now.
    Downloading(ModelId),
    /// A path to delete is not a strict descendant of the models directory
    /// (ARCH-12).
    UnsafePath(PathBuf),
    /// A local file-system operation failed.
    Io(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Cancelled => f.write_str("Download cancelled"),
            Error::Model(error) => error.fmt(f),
            Error::Download(description) | Error::Io(description) => f.write_str(description),
            Error::Downloading(model) => write!(f, "Model '{model}' is downloading."),
            Error::UnsafePath(path) => write!(
                f,
                "Refusing to delete {}: it is outside the models folder.",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {}

impl From<ModelError> for Error {
    fn from(error: ModelError) -> Error {
        Error::Model(error)
    }
}

/// Describes an error and its causes, joined with ": ". reqwest's own text
/// leaves out the root cause, e.g. "connection refused".
pub(crate) fn describe(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        source = cause.source();
    }
    text
}
