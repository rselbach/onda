//! The optional Apple Intelligence port as a UniFFI foreign trait, adapted
//! to `onda-app`'s [`OnDeviceModel`]. The adapter only converts types; the
//! core decides prompts, timeouts, and fallbacks.

use std::fmt;
use std::sync::Arc;

use onda_app::ports::OnDeviceModel;
use onda_domain::cleanup::OnDeviceStatus;

/// Whether the on-device model can run now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum OnDeviceAvailability {
    /// Ready to use.
    Available,
    /// This Mac cannot run Apple Intelligence.
    DeviceNotEligible,
    /// Apple Intelligence is turned off in System Settings.
    NotEnabled,
    /// The model is still downloading or otherwise not ready.
    NotReady,
}

impl From<OnDeviceAvailability> for OnDeviceStatus {
    fn from(availability: OnDeviceAvailability) -> Self {
        match availability {
            OnDeviceAvailability::Available => Self::Available,
            OnDeviceAvailability::DeviceNotEligible => Self::DeviceNotEligible,
            OnDeviceAvailability::NotEnabled => Self::NotEnabled,
            OnDeviceAvailability::NotReady => Self::NotReady,
        }
    }
}

/// Why the on-device model gave no answer.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum OnDeviceError {
    /// The OS's description, e.g. a refusal or a context overflow; never
    /// the transcript.
    Failed {
        /// What went wrong.
        message: String,
    },
}

impl fmt::Display for OnDeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for OnDeviceError {}

impl From<uniffi::UnexpectedUniFFICallbackError> for OnDeviceError {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Failed {
            message: error.reason,
        }
    }
}

/// Apple Intelligence's on-device model (macOS: the Foundation Models
/// framework). The core calls `respond` from blocking-pool threads, never
/// the main thread, and it blocks until the model answers.
#[uniffi::export(with_foreign)]
pub trait OnDeviceCleanupPort: Send + Sync {
    /// Whether the model can run now; read fresh, cheap.
    fn availability(&self) -> OnDeviceAvailability;

    /// Answers `prompt` in a new session with `instructions`, at
    /// temperature 0.
    fn respond(&self, instructions: String, prompt: String) -> Result<String, OnDeviceError>;
}

/// The shell's [`OnDeviceCleanupPort`] behind `onda-app`'s [`OnDeviceModel`].
pub(crate) struct ShellOnDevice(pub(crate) Arc<dyn OnDeviceCleanupPort>);

impl OnDeviceModel for ShellOnDevice {
    fn status(&self) -> OnDeviceStatus {
        self.0.availability().into()
    }

    fn respond(&self, instructions: &str, prompt: &str) -> Result<String, String> {
        self.0
            .respond(instructions.to_owned(), prompt.to_owned())
            .map_err(|error| error.to_string())
    }
}
