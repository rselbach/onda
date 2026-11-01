//! User messages: the copy catalog, channel routing (MSG-1), the per-kind
//! notice rate limit (DICT-2, CLN-13), and launch-at-login status and copy
//! (LOGIN-1, LOGIN-2).
//!
//! Copy that names the tray icon takes the platform's term as a parameter
//! ("menu bar icon" on macOS, "tray icon" elsewhere).
//! Copy owned by a feature module (speech gate, clock, cleanup, hotkey,
//! downloads) is delegated to it so each string has one home.

use std::time::Duration;

use crate::cleanup::CleanupNotice;
use crate::hotkey::{REGISTRATION_FAILURE_TITLE, RegistrationFailureNotice};
use crate::privacy_naming::PrivacyNaming;
use crate::types::{MonoTime, OutputMode};
use crate::{clock, download_progress, speech_gate};

/// Title of the alert the shell shows when the core fails to start (MSG-3).
/// It appears before the core exists, so the shell renders it with the
/// start error as its text.
pub const START_FAILURE_TITLE: &str = "Onda Could Not Start";

/// The start-failure alert's only button; it quits Onda.
pub const START_FAILURE_BUTTON: &str = "Quit";

/// How a message class reaches the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageClass {
    /// A reaction to something the user just did: always the overlay notice.
    Feedback,
    /// Something that happened in the background: a notification when
    /// authorized, else the overlay notice.
    Background,
    /// Blocking modal alert with "OK". Nothing in v1 uses it.
    Critical,
}

/// Notification authorization as reported by the OS (MSG-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotificationAuthorization {
    /// The user allowed notifications.
    Authorized,
    /// Quiet delivery allowed without asking.
    Provisional,
    /// Allowed for a limited time.
    Ephemeral,
    /// Never asked.
    NotDetermined,
    /// The user refused.
    Denied,
    /// No notification support, e.g. an unbundled dev build.
    Unsupported,
}

impl NotificationAuthorization {
    /// Authorized, provisional, and ephemeral count as authorized (MSG-1).
    pub fn allows_posting(self) -> bool {
        match self {
            Self::Authorized | Self::Provisional | Self::Ephemeral => true,
            Self::NotDetermined | Self::Denied | Self::Unsupported => false,
        }
    }
}

/// Where one message goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    /// The overlay notice (OVL-12).
    OverlayNotice,
    /// A system notification: banner with sound, never replacing another
    /// (MSG-2). A failed post falls back to [`Channel::on_post_failure`].
    Notification,
    /// A blocking modal alert with an "OK" button.
    Alert,
}

impl Channel {
    /// Where to send the message when posting on this channel failed. Only
    /// notifications fall back, to the overlay notice (MSG-1; Deviation:
    /// Jabber only logs a failed post).
    pub fn on_post_failure(self) -> Option<Channel> {
        match self {
            Self::Notification => Some(Self::OverlayNotice),
            Self::OverlayNotice | Self::Alert => None,
        }
    }
}

/// Routes a message (MSG-1). Check authorization on every send: the user
/// can revoke it after granting.
pub fn route(class: MessageClass, authorization: NotificationAuthorization) -> Channel {
    match class {
        MessageClass::Critical => Channel::Alert,
        MessageClass::Feedback => Channel::OverlayNotice,
        MessageClass::Background if authorization.allows_posting() => Channel::Notification,
        MessageClass::Background => Channel::OverlayNotice,
    }
}

/// Minimum spacing between two rate-limited notices of one kind (DICT-2,
/// CLN-13).
pub const NOTICE_RATE_LIMIT: Duration = Duration::from_millis(1500);

/// Notices limited to one per [`NOTICE_RATE_LIMIT`] each (DICT-2, CLN-13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RateLimitedNotice {
    /// "Model Not Ready" or "Model Not Downloaded".
    ModelNotReady,
    /// "Still Transcribing".
    StillTranscribing,
    /// "Couldn't Refine Transcript".
    CleanupFailed,
    /// "Audio Processing Error" for a buffer that failed to convert while
    /// recording. Other audio processing errors are not limited, so
    /// [`UserMessage::rate_limit`] never returns this; the session asks for it
    /// explicitly.
    AudioConversionFailed,
}

impl RateLimitedNotice {
    fn index(self) -> usize {
        match self {
            Self::ModelNotReady => 0,
            Self::StillTranscribing => 1,
            Self::CleanupFailed => 2,
            Self::AudioConversionFailed => 3,
        }
    }
}

/// Per-kind rate limiter for notices (DICT-2, CLN-13).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoticeRateLimiter {
    last_shown: [Option<MonoTime>; 4],
}

impl NoticeRateLimiter {
    /// A limiter that has shown nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a notice of `kind` may show at `now`; records it when allowed.
    /// A suppressed notice does not restart the window.
    pub fn allow(&mut self, kind: RateLimitedNotice, now: MonoTime) -> bool {
        let last = &mut self.last_shown[kind.index()];
        if let Some(previous) = *last
            && now.saturating_sub(previous) <= NOTICE_RATE_LIMIT
        {
            return false;
        }
        *last = Some(now);
        true
    }
}

/// Every titled message in the copy catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserMessage {
    /// DICT-1: the model is still preparing.
    ModelNotReady,
    /// DICT-1: the user declined a migration download.
    ModelNotDownloaded,
    /// DICT-1: a dictation is still transcribing.
    StillTranscribing,
    /// DICT-1: microphone denied.
    MicrophonePermissionRequired,
    /// DICT-1: Accessibility missing for type-into-app output.
    AccessibilityPermissionRequired,
    /// GATE-5: the recording had no speech.
    NoSpeechDetected,
    /// Audio errors: the microphone could not be used.
    MicrophoneUnavailable {
        /// Audio error text.
        error: String,
    },
    /// Audio errors: processing failed.
    AudioProcessingError {
        /// Error text.
        error: String,
    },
    /// OUT-1: copied because Accessibility is missing.
    TranscriptCopiedToClipboard,
    /// OUT-4: paste could not be simulated.
    PasteFailed,
    /// OUT-5: the previous clipboard could not be restored.
    ClipboardRestoreFailed,
    /// OUT-6: copying failed.
    CopyFailed,
    /// CLN-13 / CLN-14: cleanup fell back to the raw transcript.
    Cleanup {
        /// Which cleanup notice.
        notice: CleanupNotice,
        /// Where the raw transcript went.
        output_mode: OutputMode,
    },
    /// DICT-22: the final decode failed; the audio is kept.
    TranscriptionFailed {
        /// Error text.
        error: String,
    },
    /// DICT-8: the 15-minute limit stopped the recording.
    RecordingLimitReached,
    /// ASR-6: a model load failed.
    CouldntLoadSpeechModel {
        /// Error text.
        error: String,
        /// A downloadable model's files are on disk.
        files_on_disk: bool,
    },
    /// DL-4: a download failed with the main window closed.
    ModelDownloadFailed {
        /// Model display name.
        model: String,
        /// Error text, or the status text when there is none.
        error: String,
    },
    /// HOT-19: hotkey registration failed.
    HotkeyRegistrationFailed(RegistrationFailureNotice),
    /// LANG-3: the stored language code is not offered.
    InvalidLanguageSetting {
        /// The rejected code.
        code: String,
    },
}

impl UserMessage {
    /// Title. `naming` is the OS's name for the Accessibility permission.
    pub fn title(&self, naming: PrivacyNaming) -> &'static str {
        match self {
            Self::ModelNotReady => "Model Not Ready",
            Self::ModelNotDownloaded => "Model Not Downloaded",
            Self::StillTranscribing => "Still Transcribing",
            Self::MicrophonePermissionRequired => "Microphone Permission Required",
            Self::AccessibilityPermissionRequired => naming.required_title(),
            Self::NoSpeechDetected => speech_gate::NO_SPEECH_TITLE,
            Self::MicrophoneUnavailable { .. } => "Microphone Unavailable",
            Self::AudioProcessingError { .. } => "Audio Processing Error",
            Self::TranscriptCopiedToClipboard => "Transcript Copied to Clipboard",
            Self::PasteFailed => "Paste Failed",
            Self::ClipboardRestoreFailed => "Clipboard Restore Failed",
            Self::CopyFailed => "Copy Failed",
            Self::Cleanup { notice, .. } => notice.title(),
            Self::TranscriptionFailed { .. } => "Transcription Failed",
            Self::RecordingLimitReached => clock::RECORDING_LIMIT_TITLE,
            Self::CouldntLoadSpeechModel { .. } => "Couldn't Load Speech Model",
            Self::ModelDownloadFailed { .. } => "Model Download Failed",
            Self::HotkeyRegistrationFailed(_) => REGISTRATION_FAILURE_TITLE,
            Self::InvalidLanguageSetting { .. } => "Invalid Language Setting",
        }
    }

    /// Message body. `tray_term` names the tray icon on this platform;
    /// `naming` is the OS's name for the Accessibility permission.
    pub fn body(&self, tray_term: &str, naming: PrivacyNaming) -> String {
        match self {
            Self::ModelNotReady => {
                "Onda is still preparing the speech model. Try again in a moment.".to_owned()
            }
            Self::ModelNotDownloaded => "Your selected speech model isn't downloaded. Open Speech settings to download it or choose another model.".to_owned(),
            Self::StillTranscribing => {
                "Onda is finishing the previous dictation. Try again in a moment.".to_owned()
            }
            Self::MicrophonePermissionRequired => {
                "Onda needs microphone access to record speech.".to_owned()
            }
            Self::AccessibilityPermissionRequired => naming.required_body().to_owned(),
            Self::NoSpeechDetected => speech_gate::NO_SPEECH_MESSAGE.to_owned(),
            Self::MicrophoneUnavailable { error } => error.clone(),
            Self::AudioProcessingError { error } => format!("Failed to process audio: {error}"),
            Self::TranscriptCopiedToClipboard => naming.copied_to_clipboard_body().to_owned(),
            Self::PasteFailed => "Could not simulate paste command. Text is in clipboard.".to_owned(),
            Self::ClipboardRestoreFailed => {
                "Could not restore your previous clipboard contents. They may have been lost."
                    .to_owned()
            }
            Self::CopyFailed => "Could not copy transcription to clipboard.".to_owned(),
            Self::Cleanup {
                notice,
                output_mode,
            } => notice.message(*output_mode),
            Self::TranscriptionFailed { error } => format!(
                "Onda kept the recording. Choose Retry Failed Dictation from the {tray_term} to try again. Error: {error}"
            ),
            Self::RecordingLimitReached => clock::RECORDING_LIMIT_MESSAGE.to_owned(),
            Self::CouldntLoadSpeechModel {
                error,
                files_on_disk: false,
            } => format!(
                "{error} Check your internet connection, then choose Retry Loading Model from the {tray_term}."
            ),
            Self::CouldntLoadSpeechModel {
                error,
                files_on_disk: true,
            } => format!(
                "{error} Choose Retry Loading Model from the {tray_term}, or Re-download Model to replace its files."
            ),
            Self::ModelDownloadFailed { model, error } => {
                download_progress::download_failed_body(model, error)
            }
            Self::HotkeyRegistrationFailed(notice) => notice.message.clone(),
            Self::InvalidLanguageSetting { code } => format!(
                "The language code '{code}' is not recognized. The language has been set to Automatic instead."
            ),
        }
    }

    /// The message's class; overlay-by-design messages count as feedback.
    pub fn class(&self) -> MessageClass {
        match self {
            Self::TranscriptionFailed { .. }
            | Self::RecordingLimitReached
            | Self::CouldntLoadSpeechModel { .. }
            | Self::ModelDownloadFailed { .. }
            | Self::HotkeyRegistrationFailed(_)
            | Self::InvalidLanguageSetting { .. } => MessageClass::Background,
            Self::ModelNotReady
            | Self::ModelNotDownloaded
            | Self::StillTranscribing
            | Self::MicrophonePermissionRequired
            | Self::AccessibilityPermissionRequired
            | Self::NoSpeechDetected
            | Self::MicrophoneUnavailable { .. }
            | Self::AudioProcessingError { .. }
            | Self::TranscriptCopiedToClipboard
            | Self::PasteFailed
            | Self::ClipboardRestoreFailed
            | Self::CopyFailed
            | Self::Cleanup { .. } => MessageClass::Feedback,
        }
    }

    /// The rate-limit bucket, for messages that have one (DICT-2, CLN-13).
    pub fn rate_limit(&self) -> Option<RateLimitedNotice> {
        match self {
            Self::ModelNotReady | Self::ModelNotDownloaded => {
                Some(RateLimitedNotice::ModelNotReady)
            }
            Self::StillTranscribing => Some(RateLimitedNotice::StillTranscribing),
            Self::Cleanup {
                notice: CleanupNotice::CouldNotRefine { .. },
                ..
            } => Some(RateLimitedNotice::CleanupFailed),
            _ => None,
        }
    }
}

/// Launch-at-login status from the OS login-item registry (LOGIN-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoginItemStatus {
    /// Registered and allowed.
    Enabled,
    /// Not registered.
    Disabled,
    /// Registered but switched off by the user in system settings.
    RequiresApproval,
    /// Unbundled dev build: nothing to register.
    Unavailable,
}

/// Raw login-item registry states (macOS `SMAppService.Status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoginItemRegistryStatus {
    /// Registered and allowed.
    Enabled,
    /// Never registered, or unregistered.
    NotRegistered,
    /// Registered but needs the user's approval.
    RequiresApproval,
    /// The OS cannot find the item; reported for a never-registered app.
    NotFound,
}

impl LoginItemStatus {
    /// Maps the registry status (LOGIN-1, LOGIN-2). "Not registered" and
    /// "not found" both mean disabled; registering works either way.
    pub fn from_registry(status: LoginItemRegistryStatus, bundled: bool) -> Self {
        if !bundled {
            return Self::Unavailable;
        }
        match status {
            LoginItemRegistryStatus::Enabled => Self::Enabled,
            LoginItemRegistryStatus::RequiresApproval => Self::RequiresApproval,
            LoginItemRegistryStatus::NotRegistered | LoginItemRegistryStatus::NotFound => {
                Self::Disabled
            }
        }
    }

    /// The toggle shows on while registered, including requires-approval.
    pub fn is_on(self) -> bool {
        match self {
            Self::Enabled | Self::RequiresApproval => true,
            Self::Disabled | Self::Unavailable => false,
        }
    }

    /// The toggle can be used.
    pub fn toggle_enabled(self) -> bool {
        self != Self::Unavailable
    }
}

/// The launch-at-login toggle label (WIN-13).
pub const LOGIN_ITEM_TOGGLE_LABEL: &str = "Open Onda at login";

/// A failed toggle attempt, kept until the next successful toggle (LOGIN-2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginItemError {
    /// The user tried to turn launch at login on.
    pub enabling: bool,
    /// Error text.
    pub description: String,
}

/// Color of the caption under the toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaptionTone {
    /// Orange.
    Warning,
    /// Secondary text color.
    Secondary,
}

/// What a caption button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoginItemButton {
    /// Open the system Login Items settings.
    OpenLoginItems,
}

impl LoginItemButton {
    /// Button title.
    pub fn title(self) -> &'static str {
        match self {
            Self::OpenLoginItems => "Open Login Items",
        }
    }
}

/// The caption under the launch-at-login toggle (LOGIN-2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginItemCaption {
    /// Caption text.
    pub text: String,
    /// Caption color.
    pub tone: CaptionTone,
    /// Trailing button.
    pub button: Option<LoginItemButton>,
}

/// The caption for a status, or the last toggle error, which replaces it
/// (LOGIN-2).
pub fn login_item_caption(
    status: LoginItemStatus,
    error: Option<&LoginItemError>,
) -> Option<LoginItemCaption> {
    if let Some(error) = error {
        let verb = if error.enabling {
            "add Onda to"
        } else {
            "remove Onda from"
        };
        return Some(LoginItemCaption {
            text: format!("Couldn't {verb} Login Items: {}", error.description),
            tone: CaptionTone::Warning,
            button: None,
        });
    }
    match status {
        LoginItemStatus::RequiresApproval => Some(LoginItemCaption {
            text: "Switched off in Login Items. Turn Onda on there to open it at login.".to_owned(),
            tone: CaptionTone::Warning,
            button: Some(LoginItemButton::OpenLoginItems),
        }),
        LoginItemStatus::Unavailable => Some(LoginItemCaption {
            text: "Available only when Onda runs from its app bundle.".to_owned(),
            tone: CaptionTone::Secondary,
            button: None,
        }),
        LoginItemStatus::Enabled | LoginItemStatus::Disabled => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::{ModifierKey, Shortcut};
    use NotificationAuthorization::{
        Authorized, Denied, Ephemeral, NotDetermined, Provisional, Unsupported,
    };

    fn ms(ms: u64) -> MonoTime {
        Duration::from_millis(ms)
    }

    #[test]
    fn msg_1_authorization_mapping() {
        struct Case {
            name: &'static str,
            status: NotificationAuthorization,
            want: bool,
        }
        let cases = [
            Case {
                name: "authorized",
                status: Authorized,
                want: true,
            },
            Case {
                name: "provisional",
                status: Provisional,
                want: true,
            },
            Case {
                name: "ephemeral",
                status: Ephemeral,
                want: true,
            },
            Case {
                name: "not determined",
                status: NotDetermined,
                want: false,
            },
            Case {
                name: "denied",
                status: Denied,
                want: false,
            },
            Case {
                name: "unsupported",
                status: Unsupported,
                want: false,
            },
        ];
        for tc in cases {
            assert_eq!(tc.status.allows_posting(), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn msg_1_routing() {
        struct Case {
            name: &'static str,
            class: MessageClass,
            status: NotificationAuthorization,
            want: Channel,
        }
        let cases = [
            Case {
                name: "critical when authorized",
                class: MessageClass::Critical,
                status: Authorized,
                want: Channel::Alert,
            },
            Case {
                name: "critical without notification support",
                class: MessageClass::Critical,
                status: Unsupported,
                want: Channel::Alert,
            },
            Case {
                name: "background authorized",
                class: MessageClass::Background,
                status: Authorized,
                want: Channel::Notification,
            },
            Case {
                name: "background provisional",
                class: MessageClass::Background,
                status: Provisional,
                want: Channel::Notification,
            },
            Case {
                name: "background ephemeral",
                class: MessageClass::Background,
                status: Ephemeral,
                want: Channel::Notification,
            },
            Case {
                name: "background not asked yet",
                class: MessageClass::Background,
                status: NotDetermined,
                want: Channel::OverlayNotice,
            },
            Case {
                name: "background denied",
                class: MessageClass::Background,
                status: Denied,
                want: Channel::OverlayNotice,
            },
            Case {
                name: "background without notification support",
                class: MessageClass::Background,
                status: Unsupported,
                want: Channel::OverlayNotice,
            },
            Case {
                name: "feedback ignores authorization",
                class: MessageClass::Feedback,
                status: Authorized,
                want: Channel::OverlayNotice,
            },
        ];
        for tc in cases {
            assert_eq!(route(tc.class, tc.status), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn msg_1_post_failure_falls_back_to_overlay() {
        struct Case {
            name: &'static str,
            channel: Channel,
            want: Option<Channel>,
        }
        let cases = [
            Case {
                name: "failed notification",
                channel: Channel::Notification,
                want: Some(Channel::OverlayNotice),
            },
            Case {
                name: "overlay has no fallback",
                channel: Channel::OverlayNotice,
                want: None,
            },
            Case {
                name: "alert has no fallback",
                channel: Channel::Alert,
                want: None,
            },
        ];
        for tc in cases {
            assert_eq!(tc.channel.on_post_failure(), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn dict_2_cln_13_rate_limit() {
        struct Case {
            name: &'static str,
            shows: Vec<(RateLimitedNotice, u64)>,
            want: Vec<bool>,
        }
        use RateLimitedNotice::{
            AudioConversionFailed, CleanupFailed, ModelNotReady, StillTranscribing,
        };
        let cases = [
            Case {
                name: "first always shows",
                shows: vec![(ModelNotReady, 0)],
                want: vec![true],
            },
            Case {
                name: "repeat within 1.5 s is dropped",
                shows: vec![(ModelNotReady, 0), (ModelNotReady, 1000)],
                want: vec![true, false],
            },
            Case {
                name: "exactly 1.5 s is still dropped",
                shows: vec![(ModelNotReady, 0), (ModelNotReady, 1500)],
                want: vec![true, false],
            },
            Case {
                name: "after 1.5 s shows again",
                shows: vec![(ModelNotReady, 0), (ModelNotReady, 1501)],
                want: vec![true, true],
            },
            Case {
                name: "kinds are independent",
                shows: vec![
                    (ModelNotReady, 0),
                    (StillTranscribing, 100),
                    (CleanupFailed, 200),
                    (AudioConversionFailed, 300),
                ],
                want: vec![true, true, true, true],
            },
            Case {
                name: "conversion failures are limited too",
                shows: vec![
                    (AudioConversionFailed, 0),
                    (AudioConversionFailed, 700),
                    (AudioConversionFailed, 1501),
                ],
                want: vec![true, false, true],
            },
            Case {
                name: "dropped notice does not restart the window",
                shows: vec![
                    (CleanupFailed, 0),
                    (CleanupFailed, 1000),
                    (CleanupFailed, 1600),
                ],
                want: vec![true, false, true],
            },
        ];
        for tc in cases {
            let mut limiter = NoticeRateLimiter::new();
            let got: Vec<bool> = tc
                .shows
                .iter()
                .map(|(kind, at)| limiter.allow(*kind, ms(*at)))
                .collect();
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn copy_catalog() {
        struct Case {
            name: &'static str,
            message: UserMessage,
            term: &'static str,
            want_title: &'static str,
            want_body: &'static str,
            want_class: MessageClass,
        }
        let cases = [
            Case {
                name: "model not ready",
                message: UserMessage::ModelNotReady,
                term: "menu bar icon",
                want_title: "Model Not Ready",
                want_body: "Onda is still preparing the speech model. Try again in a moment.",
                want_class: MessageClass::Feedback,
            },
            Case {
                name: "model not downloaded",
                message: UserMessage::ModelNotDownloaded,
                term: "menu bar icon",
                want_title: "Model Not Downloaded",
                want_body: "Your selected speech model isn't downloaded. Open Speech settings to download it or choose another model.",
                want_class: MessageClass::Feedback,
            },
            Case {
                name: "still transcribing",
                message: UserMessage::StillTranscribing,
                term: "menu bar icon",
                want_title: "Still Transcribing",
                want_body: "Onda is finishing the previous dictation. Try again in a moment.",
                want_class: MessageClass::Feedback,
            },
            Case {
                name: "microphone permission",
                message: UserMessage::MicrophonePermissionRequired,
                term: "menu bar icon",
                want_title: "Microphone Permission Required",
                want_body: "Onda needs microphone access to record speech.",
                want_class: MessageClass::Feedback,
            },
            Case {
                name: "accessibility permission",
                message: UserMessage::AccessibilityPermissionRequired,
                term: "menu bar icon",
                want_title: "Accessibility Permission Required",
                want_body: "Grant accessibility permission before dictating into the active app, or switch output to Copy to clipboard in Settings.",
                want_class: MessageClass::Feedback,
            },
            Case {
                name: "no speech",
                message: UserMessage::NoSpeechDetected,
                term: "menu bar icon",
                want_title: "No Speech Detected",
                want_body: "Could not detect any speech in the recording. Try speaking louder or closer to the microphone.",
                want_class: MessageClass::Feedback,
            },
            Case {
                name: "microphone unavailable",
                message: UserMessage::MicrophoneUnavailable {
                    error: "The selected microphone is not connected".to_owned(),
                },
                term: "menu bar icon",
                want_title: "Microphone Unavailable",
                want_body: "The selected microphone is not connected",
                want_class: MessageClass::Feedback,
            },
            Case {
                name: "audio processing",
                message: UserMessage::AudioProcessingError {
                    error: "Audio converter could not be created".to_owned(),
                },
                term: "menu bar icon",
                want_title: "Audio Processing Error",
                want_body: "Failed to process audio: Audio converter could not be created",
                want_class: MessageClass::Feedback,
            },
            Case {
                name: "copied to clipboard",
                message: UserMessage::TranscriptCopiedToClipboard,
                term: "menu bar icon",
                want_title: "Transcript Copied to Clipboard",
                want_body: "Grant Accessibility permission in Privacy & Security to enable direct typing into apps.",
                want_class: MessageClass::Feedback,
            },
            Case {
                name: "paste failed",
                message: UserMessage::PasteFailed,
                term: "menu bar icon",
                want_title: "Paste Failed",
                want_body: "Could not simulate paste command. Text is in clipboard.",
                want_class: MessageClass::Feedback,
            },
            Case {
                name: "clipboard restore failed",
                message: UserMessage::ClipboardRestoreFailed,
                term: "menu bar icon",
                want_title: "Clipboard Restore Failed",
                want_body: "Could not restore your previous clipboard contents. They may have been lost.",
                want_class: MessageClass::Feedback,
            },
            Case {
                name: "copy failed",
                message: UserMessage::CopyFailed,
                term: "menu bar icon",
                want_title: "Copy Failed",
                want_body: "Could not copy transcription to clipboard.",
                want_class: MessageClass::Feedback,
            },
            Case {
                name: "refine failed while typing",
                message: UserMessage::Cleanup {
                    notice: CleanupNotice::CouldNotRefine {
                        provider: crate::cleanup::CleanupProvider::OpenRouter,
                        error: "timed out".to_owned(),
                    },
                    output_mode: OutputMode::Type,
                },
                term: "menu bar icon",
                want_title: "Couldn't Refine Transcript",
                want_body: "OpenRouter cleanup failed (timed out). Typed the raw transcript instead.",
                want_class: MessageClass::Feedback,
            },
            Case {
                name: "refine failed in clipboard mode",
                message: UserMessage::Cleanup {
                    notice: CleanupNotice::CouldNotRefine {
                        provider: crate::cleanup::CleanupProvider::OpenRouter,
                        error: "timed out".to_owned(),
                    },
                    output_mode: OutputMode::Clipboard,
                },
                term: "menu bar icon",
                want_title: "Couldn't Refine Transcript",
                want_body: "OpenRouter cleanup failed (timed out). Copied the raw transcript to the clipboard instead.",
                want_class: MessageClass::Feedback,
            },
            Case {
                name: "used raw transcript",
                message: UserMessage::Cleanup {
                    notice: CleanupNotice::UsedRawTranscript,
                    output_mode: OutputMode::Type,
                },
                term: "menu bar icon",
                want_title: "Used Raw Transcript",
                want_body: "Post-processing looked wrong.",
                want_class: MessageClass::Feedback,
            },
            Case {
                name: "transcription failed names the tray",
                message: UserMessage::TranscriptionFailed {
                    error: "Transcription failed".to_owned(),
                },
                term: "menu bar icon",
                want_title: "Transcription Failed",
                want_body: "Onda kept the recording. Choose Retry Failed Dictation from the menu bar icon to try again. Error: Transcription failed",
                want_class: MessageClass::Background,
            },
            Case {
                name: "transcription failed on another platform",
                message: UserMessage::TranscriptionFailed {
                    error: "Transcription failed".to_owned(),
                },
                term: "tray icon",
                want_title: "Transcription Failed",
                want_body: "Onda kept the recording. Choose Retry Failed Dictation from the tray icon to try again. Error: Transcription failed",
                want_class: MessageClass::Background,
            },
            Case {
                name: "recording limit",
                message: UserMessage::RecordingLimitReached,
                term: "menu bar icon",
                want_title: "Recording Limit Reached",
                want_body: "Onda stopped recording at the 15-minute limit and is transcribing what was captured. Start a new dictation to continue.",
                want_class: MessageClass::Background,
            },
            Case {
                name: "ASR-6 load failed on downloaded files",
                message: UserMessage::CouldntLoadSpeechModel {
                    error: "Bad weights.".to_owned(),
                    files_on_disk: true,
                },
                term: "menu bar icon",
                want_title: "Couldn't Load Speech Model",
                want_body: "Bad weights. Choose Retry Loading Model from the menu bar icon, or Re-download Model to replace its files.",
                want_class: MessageClass::Background,
            },
            Case {
                name: "ASR-6 load failed without files",
                message: UserMessage::CouldntLoadSpeechModel {
                    error: "Offline.".to_owned(),
                    files_on_disk: false,
                },
                term: "menu bar icon",
                want_title: "Couldn't Load Speech Model",
                want_body: "Offline. Check your internet connection, then choose Retry Loading Model from the menu bar icon.",
                want_class: MessageClass::Background,
            },
            Case {
                name: "download failed",
                message: UserMessage::ModelDownloadFailed {
                    model: "Parakeet TDT v2".to_owned(),
                    error: "Pierce unplugged the router".to_owned(),
                },
                term: "menu bar icon",
                want_title: "Model Download Failed",
                want_body: "Failed to download Parakeet TDT v2: Pierce unplugged the router",
                want_class: MessageClass::Background,
            },
            Case {
                name: "combo hotkey failed",
                message: UserMessage::HotkeyRegistrationFailed(RegistrationFailureNotice::new(
                    &Shortcut::default(),
                    -9878,
                    PrivacyNaming::Accessibility,
                )),
                term: "menu bar icon",
                want_title: "Hotkey Registration Failed",
                want_body: "Could not register the global hotkey (⌥ Space). It may be in use by another application. OSStatus: -9878",
                want_class: MessageClass::Background,
            },
            Case {
                name: "lone modifier failed",
                message: UserMessage::HotkeyRegistrationFailed(RegistrationFailureNotice::new(
                    &Shortcut::lone(ModifierKey::AltRight),
                    -1,
                    PrivacyNaming::Accessibility,
                )),
                term: "menu bar icon",
                want_title: "Hotkey Registration Failed",
                want_body: "Could not register Right Option as a global hotkey. Lone modifier keys require Accessibility permission in System Settings. OSStatus: -1",
                want_class: MessageClass::Background,
            },
            Case {
                name: "invalid language",
                message: UserMessage::InvalidLanguageSetting {
                    code: "xx".to_owned(),
                },
                term: "menu bar icon",
                want_title: "Invalid Language Setting",
                want_body: "The language code 'xx' is not recognized. The language has been set to Automatic instead.",
                want_class: MessageClass::Background,
            },
        ];
        for tc in cases {
            let naming = PrivacyNaming::Accessibility;
            assert_eq!(tc.message.title(naming), tc.want_title, "{}", tc.name);
            assert_eq!(
                tc.message.body(tc.term, naming),
                tc.want_body,
                "{}",
                tc.name
            );
            assert_eq!(tc.message.class(), tc.want_class, "{}", tc.name);
        }
    }

    #[test]
    fn accessibility_copy_follows_privacy_naming() {
        struct Case {
            name: &'static str,
            message: UserMessage,
            naming: PrivacyNaming,
            want_title: &'static str,
            want_body: &'static str,
        }
        let cases = [
            Case {
                name: "permission required",
                message: UserMessage::AccessibilityPermissionRequired,
                naming: PrivacyNaming::Accessibility,
                want_title: "Accessibility Permission Required",
                want_body: "Grant accessibility permission before dictating into the active app, or switch output to Copy to clipboard in Settings.",
            },
            Case {
                name: "permission required on macOS 27",
                message: UserMessage::AccessibilityPermissionRequired,
                naming: PrivacyNaming::DeviceControlAndDataAccess,
                want_title: "Device Control and Data Access Required",
                want_body: "Grant Device Control and Data Access before dictating into the active app, or switch output to Copy to clipboard in Settings.",
            },
            Case {
                name: "OUT-1 copied",
                message: UserMessage::TranscriptCopiedToClipboard,
                naming: PrivacyNaming::Accessibility,
                want_title: "Transcript Copied to Clipboard",
                want_body: "Grant Accessibility permission in Privacy & Security to enable direct typing into apps.",
            },
            Case {
                name: "OUT-1 copied on macOS 27",
                message: UserMessage::TranscriptCopiedToClipboard,
                naming: PrivacyNaming::DeviceControlAndDataAccess,
                want_title: "Transcript Copied to Clipboard",
                want_body: "Grant Device Control and Data Access in Privacy & Security to enable direct typing into apps.",
            },
            Case {
                name: "HOT-19 lone modifier on macOS 27",
                message: UserMessage::HotkeyRegistrationFailed(RegistrationFailureNotice::new(
                    &Shortcut::lone(ModifierKey::AltRight),
                    -1,
                    PrivacyNaming::DeviceControlAndDataAccess,
                )),
                naming: PrivacyNaming::DeviceControlAndDataAccess,
                want_title: "Hotkey Registration Failed",
                want_body: "Could not register Right Option as a global hotkey. Lone modifier keys require Device Control and Data Access in System Settings. OSStatus: -1",
            },
        ];
        for tc in cases {
            assert_eq!(tc.message.title(tc.naming), tc.want_title, "{}", tc.name);
            let got = tc.message.body("menu bar icon", tc.naming);
            assert_eq!(got, tc.want_body, "{}", tc.name);
        }
    }

    #[test]
    fn rate_limit_buckets() {
        struct Case {
            name: &'static str,
            message: UserMessage,
            want: Option<RateLimitedNotice>,
        }
        let cases = [
            Case {
                name: "model not ready",
                message: UserMessage::ModelNotReady,
                want: Some(RateLimitedNotice::ModelNotReady),
            },
            Case {
                name: "model not downloaded shares the bucket",
                message: UserMessage::ModelNotDownloaded,
                want: Some(RateLimitedNotice::ModelNotReady),
            },
            Case {
                name: "still transcribing",
                message: UserMessage::StillTranscribing,
                want: Some(RateLimitedNotice::StillTranscribing),
            },
            Case {
                name: "cleanup failed",
                message: UserMessage::Cleanup {
                    notice: CleanupNotice::CouldNotRefine {
                        provider: crate::cleanup::CleanupProvider::OpenRouter,
                        error: "boom".to_owned(),
                    },
                    output_mode: OutputMode::Type,
                },
                want: Some(RateLimitedNotice::CleanupFailed),
            },
            Case {
                name: "used raw transcript is not limited",
                message: UserMessage::Cleanup {
                    notice: CleanupNotice::UsedRawTranscript,
                    output_mode: OutputMode::Type,
                },
                want: None,
            },
            Case {
                name: "no speech is not limited",
                message: UserMessage::NoSpeechDetected,
                want: None,
            },
        ];
        for tc in cases {
            assert_eq!(tc.message.rate_limit(), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn login_1_status_mapping() {
        struct Case {
            name: &'static str,
            status: LoginItemRegistryStatus,
            bundled: bool,
            want: LoginItemStatus,
            want_on: bool,
            want_toggle: bool,
        }
        let cases = [
            Case {
                name: "enabled",
                status: LoginItemRegistryStatus::Enabled,
                bundled: true,
                want: LoginItemStatus::Enabled,
                want_on: true,
                want_toggle: true,
            },
            Case {
                name: "not registered",
                status: LoginItemRegistryStatus::NotRegistered,
                bundled: true,
                want: LoginItemStatus::Disabled,
                want_on: false,
                want_toggle: true,
            },
            Case {
                name: "requires approval shows on",
                status: LoginItemRegistryStatus::RequiresApproval,
                bundled: true,
                want: LoginItemStatus::RequiresApproval,
                want_on: true,
                want_toggle: true,
            },
            Case {
                name: "never registered",
                status: LoginItemRegistryStatus::NotFound,
                bundled: true,
                want: LoginItemStatus::Disabled,
                want_on: false,
                want_toggle: true,
            },
            Case {
                name: "bare binary",
                status: LoginItemRegistryStatus::Enabled,
                bundled: false,
                want: LoginItemStatus::Unavailable,
                want_on: false,
                want_toggle: false,
            },
        ];
        for tc in cases {
            let got = LoginItemStatus::from_registry(tc.status, tc.bundled);
            assert_eq!(got, tc.want, "{}", tc.name);
            assert_eq!(got.is_on(), tc.want_on, "{}", tc.name);
            assert_eq!(got.toggle_enabled(), tc.want_toggle, "{}", tc.name);
        }
    }

    #[test]
    fn login_2_caption() {
        struct Case {
            name: &'static str,
            status: LoginItemStatus,
            error: Option<LoginItemError>,
            want: Option<(&'static str, CaptionTone, Option<&'static str>)>,
        }
        let cases = [
            Case {
                name: "enabled has no caption",
                status: LoginItemStatus::Enabled,
                error: None,
                want: None,
            },
            Case {
                name: "disabled has no caption",
                status: LoginItemStatus::Disabled,
                error: None,
                want: None,
            },
            Case {
                name: "requires approval",
                status: LoginItemStatus::RequiresApproval,
                error: None,
                want: Some((
                    "Switched off in Login Items. Turn Onda on there to open it at login.",
                    CaptionTone::Warning,
                    Some("Open Login Items"),
                )),
            },
            Case {
                name: "unavailable",
                status: LoginItemStatus::Unavailable,
                error: None,
                want: Some((
                    "Available only when Onda runs from its app bundle.",
                    CaptionTone::Secondary,
                    None,
                )),
            },
            Case {
                name: "add error replaces the caption",
                status: LoginItemStatus::RequiresApproval,
                error: Some(LoginItemError {
                    enabling: true,
                    description: "Operation not permitted".to_owned(),
                }),
                want: Some((
                    "Couldn't add Onda to Login Items: Operation not permitted",
                    CaptionTone::Warning,
                    None,
                )),
            },
            Case {
                name: "remove error",
                status: LoginItemStatus::Enabled,
                error: Some(LoginItemError {
                    enabling: false,
                    description: "Operation not permitted".to_owned(),
                }),
                want: Some((
                    "Couldn't remove Onda from Login Items: Operation not permitted",
                    CaptionTone::Warning,
                    None,
                )),
            },
        ];
        for tc in cases {
            let got = login_item_caption(tc.status, tc.error.as_ref());
            let got = got.as_ref().map(|c| {
                (
                    c.text.as_str(),
                    c.tone,
                    c.button.map(LoginItemButton::title),
                )
            });
            assert_eq!(got, tc.want, "{}", tc.name);
        }
    }
}
