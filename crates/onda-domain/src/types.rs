//! Vocabulary shared by more than one domain module.

use std::time::Duration;

/// Monotonic time measured from an arbitrary origin chosen by the caller.
///
/// Domain code never reads a clock (ARCH-2); callers pass "now" in.
pub type MonoTime = Duration;

/// How the dictation hotkey starts and stops a recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ActivationMode {
    /// Press and hold to record, release to stop.
    #[default]
    Hold,
    /// Tap once to start, tap again to stop.
    Toggle,
    /// Quick tap toggles; holding behaves like push-to-talk.
    Automatic,
}

/// Where a finished transcript goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OutputMode {
    /// Type into the target app (needs Accessibility on macOS).
    #[default]
    Type,
    /// Copy to the clipboard only.
    Clipboard,
}

/// Microphone permission as reported by the OS (PERM-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MicrophonePermission {
    /// The user allowed access.
    Granted,
    /// The system prompt has never been shown.
    NotDetermined,
    /// The user refused; only system settings can change it.
    Denied,
    /// Policy forbids access on this machine.
    Restricted,
}

/// State of the transcription engine (04, "States").
#[derive(Debug, Clone, PartialEq)]
pub enum ModelState {
    /// No model loaded and no load in progress.
    NotReady,
    /// Downloading or loading; `progress` is `None` when indeterminate.
    Loading {
        /// Status text such as "Loading model...".
        status: String,
        /// Fraction in `[0, 1]`, or `None` for an indeterminate bar.
        progress: Option<f64>,
    },
    /// A model is loaded and dictation would work.
    Ready,
    /// The last load failed.
    Error {
        /// Human-readable failure description.
        message: String,
    },
}

/// Phase of the single dictation session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SessionPhase {
    /// No session.
    #[default]
    Idle,
    /// Capturing audio. `provisional` is true for an unconfirmed Hold press.
    Recording {
        /// True until the Hold tap threshold confirms the recording.
        provisional: bool,
    },
    /// Running ASR, cleanup, replacements, history, and delivery.
    Transcribing,
}

/// How long history entries are kept (HIS-13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HistoryRetention {
    /// 7 days.
    Week,
    /// 30 days.
    #[default]
    Month,
    /// Never expire by age.
    Forever,
}

impl HistoryRetention {
    /// Maximum entry age, or `None` for [`HistoryRetention::Forever`].
    pub fn max_age(self) -> Option<Duration> {
        const DAY: u64 = 24 * 60 * 60;
        match self {
            Self::Week => Some(Duration::from_secs(7 * DAY)),
            Self::Month => Some(Duration::from_secs(30 * DAY)),
            Self::Forever => None,
        }
    }
}
