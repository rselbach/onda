//! Ports the shell implements.
//!
//! Every method may block. The core never calls a port from the session
//! actor itself: calls run on blocking threads and report back, so a slow
//! shell can delay a step but never deadlock the state machine (ARCH-6).

use std::path::Path;
use std::sync::Arc;

use onda_domain::cleanup::OnDeviceStatus;
use onda_domain::hotkey::{GlobeKeyAction, ModifierSet, Shortcut};
use onda_domain::messages::{LoginItemRegistryStatus, NotificationAuthorization, UserMessage};
use onda_domain::models::SystemSpeechInfo;
use onda_domain::types::MicrophonePermission;
use tokio::sync::watch;

use crate::views::UpdaterState;

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
    /// The delivery was cancelled before anything was output.
    Cancelled,
}

impl InsertOutcome {
    /// The feedback notice for this outcome, if any.
    pub fn notice(self) -> Option<UserMessage> {
        match self {
            Self::Inserted | Self::Cancelled => None,
            Self::PasteFailed => Some(UserMessage::PasteFailed),
            Self::ClipboardRestoreFailed => Some(UserMessage::ClipboardRestoreFailed),
            Self::CopyFailed => Some(UserMessage::CopyFailed),
        }
    }
}

/// How a copy to the clipboard ended (OUT-1, OUT-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyOutcome {
    /// The clipboard holds the text.
    Copied,
    /// Copying failed (OUT-6).
    Failed,
    /// The delivery was cancelled before the clipboard was touched.
    Cancelled,
}

/// One delivery's commit point (DICT-19, DICT-20).
///
/// Output starts only after a successful [`Delivery::commit`]. A cancel
/// that arrives first drops the text; a cancel that arrives later changes
/// nothing, because posted keystrokes and clipboard writes cannot be taken
/// back. Clones share the state.
#[derive(Debug, Clone)]
pub struct Delivery(Arc<watch::Sender<DeliveryState>>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeliveryState {
    Pending,
    Committed,
    Cancelled,
}

impl Delivery {
    /// A delivery that has neither started output nor been cancelled.
    pub fn new() -> Self {
        Self(Arc::new(watch::Sender::new(DeliveryState::Pending)))
    }

    /// Marks output as started, right before its first visible effect.
    /// False when the delivery was cancelled first; the caller must then
    /// output nothing. Calling it again after success returns true.
    pub fn commit(&self) -> bool {
        let mut committed = false;
        self.0.send_if_modified(|state| match state {
            DeliveryState::Pending => {
                *state = DeliveryState::Committed;
                committed = true;
                true
            }
            DeliveryState::Committed => {
                committed = true;
                false
            }
            DeliveryState::Cancelled => false,
        });
        committed
    }

    /// Cancels the delivery unless output already started.
    pub fn cancel(&self) {
        self.0.send_if_modified(|state| {
            let pending = *state == DeliveryState::Pending;
            if pending {
                *state = DeliveryState::Cancelled;
            }
            pending
        });
    }

    /// Whether the delivery was cancelled before output started.
    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow() == DeliveryState::Cancelled
    }

    /// Resolves once the delivery is cancelled, so a wait before the commit
    /// (a consent prompt, a lock) can end early. Never resolves after a
    /// commit.
    pub async fn cancelled(&self) {
        let mut state = self.0.subscribe();
        // the sender lives in `self`, so the channel cannot close
        let _ = state
            .wait_for(|state| *state == DeliveryState::Cancelled)
            .await;
    }
}

impl Default for Delivery {
    fn default() -> Self {
        Self::new()
    }
}

/// Text delivery (OUT-1..OUT-7).
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
    ///
    /// `delivery` arrives uncommitted. Call [`Delivery::commit`] right
    /// before the first effect another app could see (a keystroke, an
    /// accessibility write, a clipboard write) and return
    /// [`InsertOutcome::Cancelled`] with no output when it fails. Waits
    /// before that point should end early on [`Delivery::cancelled`].
    fn insert(&self, text: &str, target: Option<&Target>, delivery: &Delivery) -> InsertOutcome;

    /// Replaces the clipboard with `text`, with no paste and no restore
    /// (OUT-1, OUT-7).
    ///
    /// Commits `delivery` like [`TextOutput::insert`]: after any wait (a
    /// lock another output holds) and right before the clipboard write,
    /// returning [`CopyOutcome::Cancelled`] untouched when that fails.
    fn copy(&self, text: &str, delivery: &Delivery) -> CopyOutcome;
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

/// Now-playing control. The core bounds `now_playing` to
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

    /// Mutes the default audio output when another app is playing through
    /// an output and the output is not muted already. Returns the muted
    /// device for `unmute_output`, or None when nothing was muted.
    fn mute_audible_output(&self) -> Option<u32>;

    /// Unmutes a device `mute_audible_output` muted. Returns false on
    /// failure.
    fn unmute_output(&self, device: u32) -> bool;
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

/// The updater (UPD-1..UPD-8). The core only displays its state and
/// forwards the user's choices; it keeps the restart from interrupting a
/// dictation (UPD-8).
pub trait Updater: Send + Sync {
    /// The current state.
    fn state(&self) -> UpdaterState;

    /// Starts a user-initiated check (UPD-1).
    fn check_now(&self);

    /// Turns automatic checks on or off.
    fn set_automatic_checks(&self, enabled: bool);

    /// Installs the staged update and relaunches (UPD-6). Blocks until Onda
    /// is quitting to relaunch (`true`) or the restart did not happen
    /// (`false`: nothing was ready, a newer update is downloading instead,
    /// or the install failed). The core calls it off the actor.
    fn restart_to_update(&self) -> bool;

    /// Opens the download of an update this copy cannot install (UPD-7).
    fn open_download(&self);
}

/// File manager services.
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

/// Apple Intelligence's on-device language model, supplied by the shell.
///
/// `respond` blocks the calling thread until the model answers; the core
/// calls it only from blocking-pool threads, never the session actor.
pub trait OnDeviceModel: Send + Sync {
    /// Whether the model can run now. Cheap; read fresh each time.
    fn status(&self) -> OnDeviceStatus;

    /// Answers `prompt` in a new session with `instructions`, at
    /// temperature 0. An error is the OS's description, never the
    /// transcript.
    fn respond(&self, instructions: &str, prompt: &str) -> Result<String, String>;
}

/// The platform speech recognizer, when the OS has one (SYS-1..SYS-7).
/// The core decides which locale to prepare and when; the shell only runs
/// the OS calls.
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn whichever_of_commit_and_cancel_comes_first_wins() {
        let delivery = Delivery::new();
        delivery.cancel();
        assert!(!delivery.commit(), "a cancelled delivery cannot commit");
        assert!(delivery.is_cancelled());

        let delivery = Delivery::new();
        assert!(delivery.commit());
        delivery.cancel();
        assert!(!delivery.is_cancelled(), "a commit cannot be taken back");
        assert!(delivery.commit(), "committing twice is fine");
    }

    #[tokio::test(start_paused = true)]
    async fn a_wait_before_the_commit_ends_on_cancel_only() {
        let delivery = Delivery::new();
        let waiting = tokio::spawn({
            let delivery = delivery.clone();
            async move { delivery.cancelled().await }
        });
        tokio::task::yield_now().await;
        delivery.cancel();
        assert!(waiting.await.is_ok());
        // already cancelled: resolves at once
        delivery.cancelled().await;

        let delivery = Delivery::new();
        assert!(delivery.commit());
        let waited = tokio::time::timeout(Duration::from_secs(120), delivery.cancelled()).await;
        assert!(
            waited.is_err(),
            "a committed delivery never reports a cancel"
        );
    }
}
