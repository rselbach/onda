//! Shell ports as UniFFI foreign traits, adapted to `onda-app`'s port
//! traits. The adapters only convert types; every decision
//! stays in the core.

use std::fmt;
use std::path::Path;
use std::sync::Arc;

use onda_app::ports::{
    Cue, FieldText, Files, HotkeySource, InsertOutcome, LoginItem, MediaControl, Notifier,
    NowPlaying, Permissions, Power, PrivacyPane, SoundPlayer, Target, TextOutput,
};
use onda_domain::hotkey::{GlobeKeyAction, ModifierSet, Shortcut};
use onda_domain::messages::{LoginItemRegistryStatus, NotificationAuthorization};
use onda_domain::privacy_naming::PrivacyNaming;
use onda_domain::types::MicrophonePermission;
use onda_history::HistoryHooks;

use crate::ui::{HotkeyModifiers, HotkeyShortcut};

/// A shell call that failed, with a description for the log.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum ShellError {
    /// The OS call failed.
    Failed {
        /// What went wrong.
        message: String,
    },
}

impl fmt::Display for ShellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for ShellError {}

impl From<uniffi::UnexpectedUniFFICallbackError> for ShellError {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Failed {
            message: error.reason,
        }
    }
}

/// The app that had keyboard focus when a session started (DICT-3).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TargetApp {
    /// OS process id.
    pub process_id: u32,
    /// Display name.
    pub name: Option<String>,
    /// Bundle id, for the overlay icon (OUT-12).
    pub app_id: Option<String>,
}

impl From<TargetApp> for Target {
    fn from(app: TargetApp) -> Self {
        Self {
            process_id: app.process_id,
            name: app.name,
            app_id: app.app_id,
        }
    }
}

impl From<&Target> for TargetApp {
    fn from(target: &Target) -> Self {
        Self {
            process_id: target.process_id,
            name: target.name.clone(),
            app_id: target.app_id.clone(),
        }
    }
}

/// The focused text field's value and caret (OUT-2).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FieldContents {
    /// The field's whole value.
    pub text: String,
    /// Selection start in UTF-16 code units.
    pub caret_utf16: u64,
}

/// How an insertion through the OUT-3 strategies ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum InsertResult {
    /// The text reached the target.
    Inserted,
    /// The paste key event could not be created (OUT-4).
    PasteFailed,
    /// The previous clipboard could not be restored (OUT-5).
    ClipboardRestoreFailed,
    /// Copying to the clipboard failed (OUT-6).
    CopyFailed,
    /// The delivery was cancelled before anything was output.
    Cancelled,
}

/// One delivery's commit point (DICT-19). Output starts only after
/// [`Delivery::commit`] returns true; a cancel that came first means the
/// text must not be output at all.
#[derive(Default, uniffi::Object)]
pub struct Delivery(onda_app::ports::Delivery);

#[uniffi::export]
impl Delivery {
    /// A delivery that has neither started output nor been cancelled.
    #[uniffi::constructor]
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks output as started; call it right before the first effect
    /// another app could see. False when the delivery was cancelled first:
    /// output nothing and return [`InsertResult::Cancelled`].
    pub fn commit(&self) -> bool {
        self.0.commit()
    }

    /// Cancels the delivery unless output already started.
    pub fn cancel(&self) {
        self.0.cancel();
    }
}

/// Microphone permission (PERM-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MicrophoneStatus {
    /// Allowed.
    Granted,
    /// Never asked.
    NotDetermined,
    /// Refused.
    Denied,
    /// Forbidden by policy.
    Restricted,
}

/// A cue sound (SND-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SoundCue {
    /// `dictation_start`.
    Start,
    /// `dictation_stop`.
    Stop,
}

/// Notification authorization (MSG-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum NotificationStatus {
    /// Allowed.
    Authorized,
    /// Quiet delivery.
    Provisional,
    /// Allowed for a limited time.
    Ephemeral,
    /// Never asked.
    NotDetermined,
    /// Refused.
    Denied,
    /// No notification support (unbundled build).
    Unsupported,
}

/// Text delivery (OUT-1..OUT-7). Every method may block.
#[uniffi::export(with_foreign)]
pub trait TextOutputPort: Send + Sync {
    /// The app owning the focused element, or none without Accessibility.
    fn focused_target(&self) -> Option<TargetApp>;

    /// The focused field of `target` (or whatever is focused), giving up
    /// after 250 ms; none when the field does not expose its text.
    fn field_text(&self, target: Option<TargetApp>) -> Option<FieldContents>;

    /// Types `text` into `target` with the OUT-3 strategies, calling
    /// `delivery.commit()` right before the first keystroke, accessibility
    /// write, or clipboard write.
    fn insert(
        &self,
        text: String,
        target: Option<TargetApp>,
        delivery: Arc<Delivery>,
    ) -> InsertResult;

    /// Replaces the clipboard with `text`. False when copying failed. The
    /// core commits the delivery before calling it.
    fn copy(&self, text: String) -> bool;
}

/// A privacy settings pane (PERM-3).
#[uniffi::remote(Enum)]
pub enum PrivacyPane {
    /// Microphone access.
    Microphone,
    /// Accessibility.
    Accessibility,
}

/// What the OS calls the Accessibility permission; the shell reports it at
/// start.
#[uniffi::remote(Enum)]
pub enum PrivacyNaming {
    /// "Accessibility" (macOS 26).
    Accessibility,
    /// "Device Control and Data Access" (macOS 27 and later).
    DeviceControlAndDataAccess,
}

/// The login-item registry's raw status (macOS `SMAppService.Status`).
#[uniffi::remote(Enum)]
pub enum LoginItemRegistryStatus {
    /// Registered and allowed.
    Enabled,
    /// Never registered, or unregistered.
    NotRegistered,
    /// Registered but switched off in system settings.
    RequiresApproval,
    /// Not found; reported for a never-registered app.
    NotFound,
}

/// Privacy permissions (PERM-1..PERM-4). Every read is fresh.
#[uniffi::export(with_foreign)]
pub trait PermissionsPort: Send + Sync {
    /// Microphone permission.
    fn microphone(&self) -> MicrophoneStatus;

    /// Shows the microphone prompt and blocks until answered.
    fn request_microphone(&self) -> bool;

    /// Whether Accessibility is granted.
    fn accessibility_trusted(&self) -> bool;

    /// Shows the Accessibility prompt.
    fn prompt_accessibility(&self);

    /// Opens the privacy pane for `pane`, falling back to the general
    /// privacy pane (PERM-3).
    fn open_settings(&self, pane: PrivacyPane);
}

/// What a now-playing query reported: raw facts only. The core decides
/// whether they mean playing (MEDIA-1).
#[uniffi::remote(Enum)]
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

/// Now-playing control (MEDIA-1, MEDIA-5). Every method blocks; the core
/// calls them one at a time, owns the 500 ms query timeout, and decides
/// whether media is playing and when to pause and resume. A helper process
/// behind any method needs its own hard timeout so a stuck helper cannot
/// hang later commands.
#[uniffi::export(with_foreign)]
pub trait MediaControlPort: Send + Sync {
    /// What the now-playing query reports, without interpreting it.
    fn now_playing(&self) -> NowPlaying;

    /// Sends pause. False when the command failed.
    fn pause(&self) -> bool;

    /// Sends play. False when the command failed.
    fn play(&self) -> bool;

    /// Posts the system play/pause key (a toggle). False on failure.
    fn play_pause_key(&self) -> bool;
}

/// Cue playback (SND-2).
#[uniffi::export(with_foreign)]
pub trait SoundPort: Send + Sync {
    /// Plays `cue`.
    fn play(&self, cue: SoundCue);
}

/// Idle-sleep prevention (POW-1).
#[uniffi::export(with_foreign)]
pub trait PowerPort: Send + Sync {
    /// Prevents idle system sleep.
    fn begin_prevent_idle_sleep(&self, reason: String);

    /// Ends the prevention.
    fn end_prevent_idle_sleep(&self);
}

/// How registering the dictation shortcut went (HOT-19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RegistrationResult {
    /// Registered.
    Registered,
    /// The OS refused.
    Failed {
        /// The OS status code.
        status: i32,
    },
}

/// The dictation shortcut and the session-only Escape (HOT-13, HOT-14,
/// HOT-34). Raw events go to [`OndaCore::on_hotkey_event`] in the order they
/// happened.
///
/// [`OndaCore::on_hotkey_event`]: crate::OndaCore::on_hotkey_event
#[uniffi::export(with_foreign)]
pub trait HotkeyPort: Send + Sync {
    /// Registers `shortcut`, replacing whatever was registered: a combo
    /// through the global-hotkey API, a lone modifier through an event tap
    /// that needs only Accessibility, checks it first, and returns every
    /// event unchanged (HOT-14, HOT-15). Blocks until done.
    fn register_shortcut(&self, shortcut: HotkeyShortcut) -> RegistrationResult;

    /// Releases the dictation shortcut and tears the event tap down.
    fn unregister_shortcut(&self);

    /// Re-enables the event tap after the OS disabled it (HOT-17).
    fn reenable_stream(&self);

    /// Claims Escape once per modifier set, plain Escape included.
    /// Idempotent.
    fn register_cancel(&self, modifiers: Vec<HotkeyModifiers>) -> Result<(), ShellError>;

    /// Releases every Escape registration.
    fn unregister_cancel(&self);

    /// macOS's "Press 🌐 key to" setting (`AppleFnUsageType` in
    /// `com.apple.HIToolbox`), none when unset (HOT-35).
    fn globe_key_usage(&self) -> Option<i64>;

    /// Opens Keyboard settings, where "Press 🌐 key to" lives (HOT-35).
    fn open_keyboard_settings(&self);
}

/// System notifications (MSG-1, MSG-2).
#[uniffi::export(with_foreign)]
pub trait NotifierPort: Send + Sync {
    /// Authorization, read fresh.
    fn authorization(&self) -> NotificationStatus;

    /// Posts a distinct banner with sound.
    fn post(&self, title: String, body: String) -> Result<(), ShellError>;

    /// Asks for alert and sound permission, no badge (MSG-2). The core
    /// decides when (ONB-12).
    fn request_authorization(&self);
}

/// File manager services.
#[uniffi::export(with_foreign)]
pub trait FilesPort: Send + Sync {
    /// Marks `path` as excluded from backups (HIS-6).
    fn exclude_from_backup(&self, path: String) -> Result<(), ShellError>;

    /// Reveals `path` in the file manager (HIS-24).
    fn reveal_file(&self, path: String) -> Result<(), ShellError>;

    /// Opens `path` with its default app (HIS-24).
    fn open_file(&self, path: String) -> Result<(), ShellError>;
}

/// The login-item registry (LOGIN-1..LOGIN-3). Every read is fresh.
#[uniffi::export(with_foreign)]
pub trait LoginItemPort: Send + Sync {
    /// The registry's status, or none when Onda does not run from its app
    /// bundle.
    fn status(&self) -> Option<LoginItemRegistryStatus>;

    /// Registers or unregisters Onda; the error text shows in the caption.
    fn set_enabled(&self, enabled: bool) -> Result<(), ShellError>;

    /// Opens the system Login Items settings.
    fn open_settings(&self);
}

/// What the core's updater needs from the native shell (UPD-6, UPD-7). The
/// core checks, downloads, verifies, and installs.
#[uniffi::export(with_foreign)]
pub trait UpdaterPort: Send + Sync {
    /// Opens `url` in the default browser.
    fn open_url(&self, url: String);

    /// Quits normally. The update is already in place, and a relauncher
    /// opens it once this process exits.
    fn terminate(&self);
}

/// The running app bundle, for the updater (UPD-4).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct UpdateHost {
    /// Absolute path of the running `.app`.
    pub bundle_path: String,
    /// `CFBundleVersion`.
    pub build: String,
    /// The running macOS, e.g. "26.1".
    pub os_version: String,
    /// An `ONDA_DATA_DIR` verification run: no background checks, and a
    /// relaunch keeps the run's data folder.
    pub isolated: bool,
    /// Sparkle's saved automatic-check choice, used until Onda saves its
    /// own.
    pub legacy_automatic_checks: Option<bool>,
}

/// The shell's ports behind `onda-app`'s traits.
pub(crate) struct Shell {
    pub(crate) text: Arc<dyn TextOutputPort>,
    pub(crate) permissions: Arc<dyn PermissionsPort>,
    pub(crate) sounds: Arc<dyn SoundPort>,
    pub(crate) power: Arc<dyn PowerPort>,
    pub(crate) hotkeys: Arc<dyn HotkeyPort>,
    pub(crate) notifier: Arc<dyn NotifierPort>,
    pub(crate) login_item: Arc<dyn LoginItemPort>,
    pub(crate) files: Arc<dyn FilesPort>,
}

impl TextOutput for Shell {
    fn focused_target(&self) -> Option<Target> {
        self.text.focused_target().map(Target::from)
    }

    fn field_text(&self, target: Option<&Target>) -> Option<FieldText> {
        self.text
            .field_text(target.map(TargetApp::from))
            .map(|field| FieldText {
                text: field.text,
                caret_utf16: usize::try_from(field.caret_utf16).unwrap_or(usize::MAX),
            })
    }

    fn insert(
        &self,
        text: &str,
        target: Option<&Target>,
        delivery: &onda_app::ports::Delivery,
    ) -> InsertOutcome {
        match self.text.insert(
            text.to_owned(),
            target.map(TargetApp::from),
            Arc::new(Delivery(delivery.clone())),
        ) {
            InsertResult::Inserted => InsertOutcome::Inserted,
            InsertResult::PasteFailed => InsertOutcome::PasteFailed,
            InsertResult::ClipboardRestoreFailed => InsertOutcome::ClipboardRestoreFailed,
            InsertResult::CopyFailed => InsertOutcome::CopyFailed,
            InsertResult::Cancelled => InsertOutcome::Cancelled,
        }
    }

    fn copy(&self, text: &str) -> bool {
        self.text.copy(text.to_owned())
    }
}

impl Permissions for Shell {
    fn microphone(&self) -> MicrophonePermission {
        match self.permissions.microphone() {
            MicrophoneStatus::Granted => MicrophonePermission::Granted,
            MicrophoneStatus::NotDetermined => MicrophonePermission::NotDetermined,
            MicrophoneStatus::Denied => MicrophonePermission::Denied,
            MicrophoneStatus::Restricted => MicrophonePermission::Restricted,
        }
    }

    fn request_microphone(&self) -> bool {
        self.permissions.request_microphone()
    }

    fn accessibility_trusted(&self) -> bool {
        self.permissions.accessibility_trusted()
    }

    fn prompt_accessibility(&self) {
        self.permissions.prompt_accessibility();
    }

    fn open_settings(&self, pane: PrivacyPane) {
        self.permissions.open_settings(pane);
    }
}

impl SoundPlayer for Shell {
    fn play(&self, cue: Cue) {
        self.sounds.play(match cue {
            Cue::Start => SoundCue::Start,
            Cue::Stop => SoundCue::Stop,
        });
    }
}

impl Power for Shell {
    fn begin_prevent_idle_sleep(&self, reason: &str) {
        self.power.begin_prevent_idle_sleep(reason.to_owned());
    }

    fn end_prevent_idle_sleep(&self) {
        self.power.end_prevent_idle_sleep();
    }
}

impl HotkeySource for Shell {
    fn register(&self, shortcut: &Shortcut) -> Result<(), i32> {
        match self
            .hotkeys
            .register_shortcut(HotkeyShortcut::from(shortcut))
        {
            RegistrationResult::Registered => Ok(()),
            RegistrationResult::Failed { status } => Err(status),
        }
    }

    fn unregister(&self) {
        self.hotkeys.unregister_shortcut();
    }

    fn reenable_stream(&self) {
        self.hotkeys.reenable_stream();
    }

    fn register_cancel(&self, modifiers: &[ModifierSet]) -> Result<(), String> {
        let modifiers = modifiers
            .iter()
            .copied()
            .map(HotkeyModifiers::from)
            .collect();
        self.hotkeys
            .register_cancel(modifiers)
            .map_err(|error| error.to_string())
    }

    fn unregister_cancel(&self) {
        self.hotkeys.unregister_cancel();
    }

    fn globe_key_action(&self) -> GlobeKeyAction {
        GlobeKeyAction::from_apple_fn_usage_type(self.hotkeys.globe_key_usage())
    }

    fn open_keyboard_settings(&self) {
        self.hotkeys.open_keyboard_settings();
    }
}

impl Notifier for Shell {
    fn authorization(&self) -> NotificationAuthorization {
        match self.notifier.authorization() {
            NotificationStatus::Authorized => NotificationAuthorization::Authorized,
            NotificationStatus::Provisional => NotificationAuthorization::Provisional,
            NotificationStatus::Ephemeral => NotificationAuthorization::Ephemeral,
            NotificationStatus::NotDetermined => NotificationAuthorization::NotDetermined,
            NotificationStatus::Denied => NotificationAuthorization::Denied,
            NotificationStatus::Unsupported => NotificationAuthorization::Unsupported,
        }
    }

    fn post(&self, title: &str, body: &str) -> Result<(), String> {
        self.notifier
            .post(title.to_owned(), body.to_owned())
            .map_err(|error| error.to_string())
    }

    fn request_authorization(&self) {
        self.notifier.request_authorization();
    }
}

impl LoginItem for Shell {
    fn status(&self) -> Option<LoginItemRegistryStatus> {
        self.login_item.status()
    }

    fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        self.login_item
            .set_enabled(enabled)
            .map_err(|error| error.to_string())
    }

    fn open_settings(&self) {
        self.login_item.open_settings();
    }
}

/// The shell's media port behind `onda-app`'s [`MediaControl`].
pub(crate) struct ShellMedia(pub(crate) Arc<dyn MediaControlPort>);

impl MediaControl for ShellMedia {
    fn now_playing(&self) -> NowPlaying {
        self.0.now_playing()
    }

    fn pause(&self) -> bool {
        self.0.pause()
    }

    fn play(&self) -> bool {
        self.0.play()
    }

    fn play_pause_key(&self) -> bool {
        self.0.play_pause_key()
    }
}

impl Files for Shell {
    fn reveal(&self, path: &Path) -> Result<(), String> {
        self.files
            .reveal_file(path.to_string_lossy().into_owned())
            .map_err(|error| error.to_string())
    }

    fn open(&self, path: &Path) -> Result<(), String> {
        self.files
            .open_file(path.to_string_lossy().into_owned())
            .map_err(|error| error.to_string())
    }
}

/// History hooks over the shell's `Files` port. The core emits
/// `HistoryChanged` itself after each save, so the change hook only logs.
pub(crate) struct HistoryFiles(pub(crate) Arc<dyn FilesPort>);

impl HistoryHooks for HistoryFiles {
    fn history_changed(&self) {
        tracing::debug!("history changed");
    }

    fn exclude_from_backup(&self, directory: &Path) -> std::io::Result<()> {
        exclude_from_backup(self.0.as_ref(), directory)
    }
}

/// Excludes `directory` from backups through `files`.
pub(crate) fn exclude_from_backup(files: &dyn FilesPort, directory: &Path) -> std::io::Result<()> {
    files
        .exclude_from_backup(directory.to_string_lossy().into_owned())
        .map_err(std::io::Error::other)
}
