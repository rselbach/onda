//! Ports the shell implements (02 "Ports").
//!
//! Every method may block. The core never calls a port from the session
//! actor itself: calls run on blocking threads and report back, so a slow
//! shell can delay a step but never deadlock the state machine (ARCH-6).

use std::path::Path;
use std::time::SystemTime;

use onda_domain::hotkey::{GlobeKeyAction, ModifierSet, Shortcut};
use onda_domain::messages::{LoginItemRegistryStatus, NotificationAuthorization, UserMessage};
use onda_domain::models::SystemSpeechInfo;
use onda_domain::types::MicrophonePermission;

/// The app that had keyboard focus when a session started (DICT-3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// OS process id; output goes to this process.
    pub process_id: u32,
    /// Display name, recorded in history and passed to cleanup (OUT-12).
    pub name: Option<String>,
    /// Platform app identifier (bundle id on macOS) for the overlay icon.
    pub app_id: Option<String>,
}

/// The focused text field's value and caret, as accessibility APIs report
/// them (OUT-2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldText {
    /// The field's whole value.
    pub text: String,
    /// Selection start in UTF-16 code units.
    pub caret_utf16: usize,
}

/// How an insertion through the OUT-3 strategy chain ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertOutcome {
    /// The text reached the target.
    Inserted,
    /// The paste key event could not be created; the text stays on the
    /// clipboard (OUT-4).
    PasteFailed,
    /// The text was pasted but the previous clipboard could not be restored
    /// (OUT-5).
    ClipboardRestoreFailed,
    /// Copying to the clipboard failed (OUT-6).
    CopyFailed,
}

impl InsertOutcome {
    /// The feedback notice for this outcome, if any.
    pub fn notice(self) -> Option<UserMessage> {
        match self {
            Self::Inserted => None,
            Self::PasteFailed => Some(UserMessage::PasteFailed),
            Self::ClipboardRestoreFailed => Some(UserMessage::ClipboardRestoreFailed),
            Self::CopyFailed => Some(UserMessage::CopyFailed),
        }
    }
}

/// Text delivery (02 "Ports", OUT-1..OUT-7).
pub trait TextOutput: Send + Sync {
    /// The app owning the focused element, or `None` when unknown (no
    /// Accessibility, or nothing focused).
    fn focused_target(&self) -> Option<Target>;

    /// The focused text field of `target` (or of whatever is focused when
    /// `None`), giving up after 250 ms; `None` when the field does not expose
    /// its text (OUT-2).
    fn field_text(&self, target: Option<&Target>) -> Option<FieldText>;

    /// Types `text` into `target` with the OUT-3 strategies, falling back to
    /// paste with clipboard restore.
    fn insert(&self, text: &str, target: Option<&Target>) -> InsertOutcome;

    /// Replaces the clipboard with `text`, with no paste and no restore
    /// (OUT-1, OUT-7). Returns false when copying failed.
    fn copy(&self, text: &str) -> bool;
}

/// Privacy permissions (PERM-1, PERM-4).
pub trait Permissions: Send + Sync {
    /// Current microphone permission, read fresh.
    fn microphone(&self) -> MicrophonePermission;

    /// Shows the system microphone prompt and blocks until the user answers.
    /// Returns whether access was granted.
    fn request_microphone(&self) -> bool;

    /// Whether Accessibility is granted, read fresh.
    fn accessibility_trusted(&self) -> bool;

    /// Shows the system Accessibility prompt.
    fn prompt_accessibility(&self);

    /// Opens the privacy settings pane for `pane`, falling back to the
    /// general privacy pane (PERM-3).
    fn open_settings(&self, pane: PrivacyPane);
}

/// A privacy settings pane (PERM-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrivacyPane {
    /// Microphone access.
    Microphone,
    /// Accessibility ("Typing Access").
    Accessibility,
}

/// What a now-playing query reported: raw facts only. The core decides
/// whether they mean playing (MEDIA-1).
#[derive(Debug, Clone, PartialEq)]
pub enum NowPlaying {
    /// The query failed; `reason` is for the log.
    Failed {
        /// What went wrong.
        reason: String,
    },
    /// No track is loaded.
    NoTrack,
    /// A track, with whatever the player reported about it.
    Track {
        /// The player's playing flag, when reported.
        is_playing: Option<bool>,
        /// The playback rate, when reported.
        playback_rate: Option<f64>,
    },
}

/// Now-playing control (08 "Media pause"). The core bounds `now_playing` to
/// 500 ms and runs every call one at a time, in order (MEDIA-1, MEDIA-5).
pub trait MediaControl: Send + Sync {
    /// What the now-playing query reports.
    fn now_playing(&self) -> NowPlaying;

    /// Sends pause. Returns false when the command failed.
    fn pause(&self) -> bool;

    /// Sends play. Returns false when the command failed.
    fn play(&self) -> bool;

    /// Sends the system play/pause key (a toggle). Returns false on failure.
    fn play_pause_key(&self) -> bool;
}

/// A cue sound (SND-1, SND-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    /// "dictation start", when the recording UI appears.
    Start,
    /// "dictation stop", when a session that showed its recording UI enters
    /// Transcribing.
    Stop,
}

/// Cue playback. A missing sound file is logged by the shell, never fatal
/// (SND-2).
pub trait SoundPlayer: Send + Sync {
    /// Plays `cue`, restarting it if it is still playing.
    fn play(&self, cue: Cue);
}

/// Idle-sleep prevention (DICT-10, POW-1).
pub trait Power: Send + Sync {
    /// Prevents idle system sleep, with `reason` for the OS.
    fn begin_prevent_idle_sleep(&self, reason: &str);

    /// Ends the prevention started by the last begin.
    fn end_prevent_idle_sleep(&self);
}

/// The dictation shortcut and the session-only Escape (HOT-13, HOT-14,
/// HOT-34). The core decides when to register, re-enable, and give up; the
/// shell only performs the OS calls and forwards raw events through
/// [`App::on_hotkey_event`](crate::App::on_hotkey_event) in the order they
/// happened (HOT-20a).
pub trait HotkeySource: Send + Sync {
    /// Registers `shortcut` as the dictation hotkey, replacing whatever was
    /// registered. A combo goes through the OS global-hotkey facility
    /// (HOT-13). A lone modifier opens a keyboard event stream that returns
    /// every event unchanged (HOT-15) and forwards each modifier-flags change
    /// and key-down (HOT-14). The error is the OS status code (HOT-19).
    fn register(&self, shortcut: &Shortcut) -> Result<(), i32>;

    /// Releases the dictation hotkey and closes the event stream, if any.
    fn unregister(&self);

    /// Re-enables the event stream after the OS disabled it (HOT-17).
    fn reenable_stream(&self);

    /// Claims Escape globally once per modifier set in `modifiers`, plain
    /// Escape included. Idempotent. An error is logged only (HOT-34).
    fn register_cancel(&self, modifiers: &[ModifierSet]) -> Result<(), String>;

    /// Releases every Escape registration.
    fn unregister_cancel(&self);

    /// What the OS does when Fn (🌐) is pressed on its own, read fresh
    /// (HOT-35).
    fn globe_key_action(&self) -> GlobeKeyAction;

    /// Opens the system keyboard settings, where "Press 🌐 key to" lives
    /// (HOT-35).
    fn open_keyboard_settings(&self);
}

/// System notifications (MSG-1, MSG-2).
pub trait Notifier: Send + Sync {
    /// Authorization, read fresh on every send.
    fn authorization(&self) -> NotificationAuthorization;

    /// Posts a banner with sound. An error falls back to the overlay notice.
    fn post(&self, title: &str, body: &str) -> Result<(), String>;

    /// Asks for alert and sound permission, no badge (MSG-2). The OS shows
    /// nothing when the user already answered. When to ask: ONB-12.
    fn request_authorization(&self);
}

/// The OS login-item registry (LOGIN-1..LOGIN-3).
pub trait LoginItem: Send + Sync {
    /// The registry's status, read fresh, or `None` when Onda does not run
    /// from its app bundle and cannot register (LOGIN-1 "unavailable").
    fn status(&self) -> Option<LoginItemRegistryStatus>;

    /// Registers (`true`) or unregisters Onda. The error text appears in
    /// the LOGIN-2 caption.
    fn set_enabled(&self, enabled: bool) -> Result<(), String>;

    /// Opens the system Login Items settings.
    fn open_settings(&self);
}

/// The shell-owned updater (UPD-1..UPD-4). The core only displays its state
/// and forwards the user's choices.
pub trait Updater: Send + Sync {
    /// The updater can check now (UPD-1).
    fn can_check(&self) -> bool;

    /// Starts a user-initiated check (UPD-1).
    fn check_now(&self);

    /// "Check for updates automatically".
    fn automatic_checks(&self) -> bool;

    /// Turns automatic checks on or off.
    fn set_automatic_checks(&self, enabled: bool);

    /// When the updater last checked, if ever.
    fn last_check(&self) -> Option<SystemTime>;
}

/// File manager services (02 "Ports": `Files`).
pub trait Files: Send + Sync {
    /// Reveals `path` in the file manager (HIS-24 "Show Recording in
    /// Finder").
    fn reveal(&self, path: &Path) -> Result<(), String>;

    /// Opens `path` with its default app (HIS-24 "Play Recording").
    fn open(&self, path: &Path) -> Result<(), String>;
}

/// Why a [`SystemSpeech`] call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemSpeechError {
    /// The OS has no recognizer for the locale (SYS-6).
    UnsupportedLocale {
        /// The OS's localized name for the locale, e.g. "Polish (Poland)".
        display_name: String,
    },
    /// The call was cancelled before it finished.
    Cancelled,
    /// Anything else. Never contains transcript text.
    Failed {
        /// What went wrong, for logs.
        detail: String,
    },
}

/// The platform speech recognizer, when the OS has one (02 "Ports":
/// `SystemSpeech`, SYS-1..SYS-7). The core decides which locale to prepare
/// and when; the shell only runs the OS calls.
///
/// `prepare` and `transcribe` block the calling core thread until the OS
/// finishes; the core never calls them from the session actor.
pub trait SystemSpeech: Send + Sync {
    /// Catalog facts: description, supported language codes, license, and
    /// attribution (MOD-4, SYS-1). Cheap and constant for the run.
    fn info(&self) -> SystemSpeechInfo;

    /// The OS locale identifier, e.g. "en_US", read fresh (SYS-2).
    fn os_locale(&self) -> String;

    /// Gets `locale` ready to transcribe, installing its OS speech assets
    /// if needed. `progress` hears the asset download's fraction in
    /// `[0, 1]`, and nothing when no download is needed.
    fn prepare(&self, locale: &str, progress: &mut dyn FnMut(f64))
    -> Result<(), SystemSpeechError>;

    /// Transcribes 16 kHz mono `samples` in `locale`, which was prepared
    /// before. Returns the recognizer's final results in order (SYS-7).
    fn transcribe(&self, samples: &[f32], locale: &str) -> Result<Vec<String>, SystemSpeechError>;
}
