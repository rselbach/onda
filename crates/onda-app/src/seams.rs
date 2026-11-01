//! Core-side seams: things the core owns through portable crates (audio,
//! models, history, cleanup HTTP, secrets, settings), abstracted so the
//! session engine runs against fakes in tests and adapters in the app.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use jiff::Timestamp;
use onda_asr::SpeechEngine;
use onda_domain::cleanup::{CleanupError, CleanupModel, CleanupProvider, CleanupRequest};
use onda_domain::history::{HistoryEntry, NewEntry, RecentTranscript};
use onda_domain::hotkey::Shortcut;
use onda_domain::languages::LanguageSetting;
use onda_domain::messages::UserMessage;
use onda_domain::models::ModelId;
use onda_domain::onboarding::OnboardingFlags;
use onda_domain::replacements::ReplacementRule;
use onda_domain::types::{ActivationMode, HistoryRetention, OutputMode};
use uuid::Uuid;

/// A boxed, sendable future, so async seams stay object-safe.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The settings the session engine reads (SET-1). Values are already
/// validated and resolved: an unknown model id or a bad hotkey reads as its
/// default (SET-2, HOT-5, ASR-4).
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// `hotkeyActivationMode`.
    pub activation_mode: ActivationMode,
    /// `outputMode`.
    pub output_mode: OutputMode,
    /// `selectedLanguage`.
    pub language: LanguageSetting,
    /// Recent languages for the tray menu, most recent first (LANG-8).
    pub recent_languages: Vec<LanguageSetting>,
    /// `selectedModel`, resolved.
    pub selected_model: ModelId,
    /// `postProcessingEnabled` (CLN-1).
    pub cleanup_enabled: bool,
    /// `cleanupProvider`.
    pub cleanup_provider: CleanupProvider,
    /// The provider's model, resolved from its slug (CLN-21).
    pub cleanup_model: CleanupModel,
    /// `replacementEntries` (REP-7).
    pub replacement_rules: Vec<ReplacementRule>,
    /// Save transcripts (HIS-1).
    pub history_enabled: bool,
    /// Keep audio recordings (HIS-1).
    pub keep_audio: bool,
    /// Pause media while recording (MEDIA-1).
    pub pause_media: bool,
    /// Play cue sounds (SND-2).
    pub sounds: bool,
    /// The dictation shortcut (HOT-1).
    pub hotkey: Shortcut,
    /// First-run onboarding is pending (SET-4).
    pub first_run_pending: bool,
    /// `onboardingCompleted` and `didShowFirstRunSetup` (SET-4).
    pub onboarding: OnboardingFlags,
    /// `inputDeviceUID`; empty for the system default (AUD-2).
    pub input_device_uid: String,
    /// `historyRetention` (HIS-13).
    pub history_retention: HistoryRetention,
    /// The raw stored `selectedModel`, possibly removed or unknown, for the
    /// catalog-evolution checks (DL-12).
    pub stored_model: String,
    /// The raw stored `recentLanguages` (LANG-8).
    pub stored_recent_languages: String,
    /// `lastModelMigrationNoticeKey` (DL-14).
    pub last_notice_key: String,
    /// `declinedModelMigrationNoticeKey` (DL-14).
    pub declined_notice_key: String,
    /// The OS locale's language code (LANG-2), also the system language
    /// the compatibility note names.
    pub os_language: Option<String>,
}

/// One settings write the windows make (SET-5). Setting a value it already
/// has writes nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingChange {
    /// `selectedModel`, written as is, even when not downloaded (onboarding
    /// picks a model before downloading it, ONB-4, ONB-8).
    SelectedModel(ModelId),
    /// `selectedLanguage`.
    Language(LanguageSetting),
    /// `recentLanguages`, most recent first (LANG-8).
    RecentLanguages(Vec<LanguageSetting>),
    /// `inputDeviceUID`; empty for the system default.
    InputDevice(String),
    /// `outputMode`.
    OutputMode(OutputMode),
    /// `hotkeyActivationMode`.
    ActivationMode(ActivationMode),
    /// `pauseMediaDuringRecording`.
    PauseMedia(bool),
    /// `soundFeedbackEnabled`.
    Sounds(bool),
    /// `historyEnabled`.
    HistoryEnabled(bool),
    /// `historyKeepsAudio`.
    HistoryKeepsAudio(bool),
    /// `historyRetention`.
    HistoryRetention(HistoryRetention),
    /// `replacementEntries`, already filtered for persistence (REP-12).
    ReplacementRules(Vec<ReplacementRule>),
    /// `postProcessingEnabled`.
    CleanupEnabled(bool),
    /// `cleanupProvider`.
    CleanupProvider(CleanupProvider),
    /// The provider's model slug (`openRouterModel`).
    CleanupModel(CleanupProvider, CleanupModel),
    /// Both onboarding flags in one write (SET-4).
    Onboarding(OnboardingFlags),
    /// `lastModelMigrationNoticeKey`.
    LastNoticeKey(String),
    /// `declinedModelMigrationNoticeKey`; empty ends the declined state.
    DeclinedNoticeKey(String),
}

/// Where settings come from (`onda-settings` in the app).
pub trait SettingsSource: Send + Sync {
    /// The current settings. Cheap; called once per command and per session.
    fn snapshot(&self) -> Settings;

    /// Persists `selectedModel` after a model switch (DL-7).
    fn set_selected_model(&self, model: ModelId);

    /// Persists the dictation shortcut (HOT-12). An error is logged.
    fn set_hotkey(&self, shortcut: &Shortcut) -> Result<(), String>;

    /// Persists one change. The error is logged by the caller.
    fn apply(&self, change: SettingChange) -> Result<(), String>;

    /// LANG-3: stores `auto` over a stored language code that is not
    /// offered and returns the "Invalid Language Setting" warning to post;
    /// `None` when nothing needed resetting. A write error is logged.
    fn reset_invalid_language(&self) -> Option<UserMessage>;
}

/// Each provider's API key in the OS credential store (CLN-2, CLN-23).
/// Providers never share a key.
pub trait SecretSource: Send + Sync {
    /// `provider`'s stored key, `Ok(None)` when none is stored. Read once
    /// per dictation. An `Err` is logged and treated as no key; its text is
    /// the inline CLN-24 message ("Keychain read failed (OSStatus N).").
    fn api_key(&self, provider: CleanupProvider) -> Result<Option<String>, String>;

    /// Saves what the user typed as `provider`'s key: trimmed, and blank
    /// deletes the stored key (CLN-24). The error text is the inline CLN-24
    /// message.
    fn save_api_key(&self, provider: CleanupProvider, value: &str) -> Result<(), String>;
}

/// Sends one cleanup request (`onda-cleanup` in the app).
///
/// The transport posts `request` with its headers and JSON body, enforces
/// [`REQUEST_TIMEOUT`](onda_domain::cleanup::REQUEST_TIMEOUT), and returns
/// [`parse_response`](onda_domain::cleanup::parse_response)'s result.
/// Dropping the future cancels the request (CLN-9).
pub trait CleanupTransport: Send + Sync {
    /// Sends `request`.
    fn send(&self, request: CleanupRequest) -> BoxFuture<'static, Result<String, CleanupError>>;

    /// The models `provider` lists; empty when it cannot list them.
    fn list_models(
        &self,
        provider: CleanupProvider,
    ) -> BoxFuture<'static, Result<Vec<CleanupModel>, CleanupError>>;
}

/// Transcript history (`onda-history` in the app).
pub trait HistorySink: Send + Sync {
    /// Saves one entry (HIS-2). `audio` is the session's 16 kHz samples when
    /// "Keep audio recordings" is on; the store encodes the WAV and fills in
    /// [`NewEntry::audio_byte_count`], which arrives as `None`. Blocking. An
    /// error is logged and never blocks delivery.
    fn save(&self, entry: NewEntry, audio: Option<Arc<[f32]>>) -> Result<(), String>;

    /// Up to five newest entries with text, for the tray menu (HIS-28).
    fn recent_transcripts(&self) -> Vec<RecentTranscript>;

    /// Every entry, newest first (HIS-9). Blocking.
    fn entries(&self) -> Result<Vec<HistoryEntry>, String>;

    /// Deletes one entry (HIS-24). Blocking.
    fn delete(&self, id: Uuid) -> Result<(), String>;

    /// "Clear History" (HIS-16a). Blocking.
    fn delete_all(&self) -> Result<(), String>;

    /// Applies `retention` to the whole list at `now` (HIS-16). Blocking.
    /// `Ok(true)` when it deleted entries or removed recordings, so the
    /// app announces the change (HIS-10).
    fn apply_retention(&self, retention: HistoryRetention, now: Timestamp) -> Result<bool, String>;

    /// The recording of entry `id`; the error is the message to show, e.g.
    /// "The recording for this dictation is missing." (HIS-24). Blocking.
    fn audio_path(&self, id: Uuid) -> Result<PathBuf, String>;
}

/// Installed speech models and their engines (`onda-models` plus the
/// inference backend in the app).
pub trait ModelStore: Send + Sync {
    /// Models whose files are installed (built-in engines included). May
    /// touch the disk.
    fn downloaded(&self) -> Vec<ModelId>;

    /// A fresh, unloaded engine for `model`, or `None` when no backend runs
    /// it.
    fn engine(&self, model: ModelId) -> Option<Box<dyn SpeechEngine>>;

    /// Starts downloading `model`, or joins its running download (DL-1).
    /// Progress and the outcome come back through
    /// [`App::download_event`](crate::App::download_event), including a
    /// `Finished` for a model that is already complete.
    fn start_download(&self, model: ModelId);

    /// Cancels `model`'s download, keeping partial data (DL-3).
    fn cancel_download(&self, model: ModelId);

    /// Deletes `model`'s folder (DL-9). Blocking. The error is the text
    /// after "Failed to delete <Model>: " (WIN-20).
    fn delete(&self, model: ModelId) -> Result<(), String>;

    /// Deletes `model`'s folder and downloads it again (DL-11); reports
    /// like [`ModelStore::start_download`].
    fn redownload(&self, model: ModelId);

    /// Deletes the folders of models the catalog removed (DL-15). Blocking;
    /// failures are logged.
    fn remove_obsolete(&self);
}

/// A model download's progress or outcome, in order (DL-1..DL-4).
#[derive(Debug, Clone, PartialEq)]
pub struct DownloadUpdate {
    /// The model being downloaded.
    pub model: ModelId,
    /// What happened.
    pub kind: DownloadUpdateKind,
}

/// What happened to a download.
#[derive(Debug, Clone, PartialEq)]
pub enum DownloadUpdateKind {
    /// Started or resumed.
    Started,
    /// Fraction in `[0, 1]`, already throttled (DL-2).
    Progress(f64),
    /// Every file is on disk and verified.
    Finished,
    /// Cancelled; partial data is kept (DL-3).
    Cancelled,
    /// Failed after retries (DL-4).
    Failed {
        /// User-facing description; empty when there is none.
        error: String,
    },
}

/// An input device for the General page picker (WIN-9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDeviceInfo {
    /// Stable unique id, stored in `inputDeviceUID` (AUD-2).
    pub uid: String,
    /// Display name.
    pub name: String,
    /// The OS default input.
    pub is_default: bool,
}

/// A capture that just opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureInfo {
    /// Tags this capture's signals.
    pub capture_id: u64,
    /// Input device name, for the silent-microphone title (AUD-13).
    pub device_name: String,
}

/// The microphone, as the session engine uses it (`onda-audio`'s
/// `AudioCapture` in the app). Samples are 16 kHz mono `f32` (AUD-1).
///
/// Methods are short requests to a capture worker and may be called from
/// the session actor.
pub trait AudioSource: Send + Sync {
    /// Opens a capture. The error is the notice to show.
    fn start(&self) -> Result<CaptureInfo, UserMessage>;

    /// Records the hotkey release; returns the sample count at that moment
    /// (GATE-6).
    fn mark_release(&self) -> usize;

    /// Closes the capture and returns everything it captured.
    fn stop(&self) -> Vec<f32>;

    /// A copy of the open capture's samples so far.
    fn samples(&self) -> Vec<f32>;

    /// The open capture's sample count so far.
    fn sample_count(&self) -> usize;

    /// Connected input devices, read fresh (WIN-9). Blocking.
    fn input_devices(&self) -> Vec<InputDeviceInfo>;

    /// Records from `uid` from now on, or the system default when empty;
    /// the input rebuilds (AUD-2, SET-5).
    fn select_input(&self, uid: &str);
}

/// Something the audio source reports while a capture is open, delivered
/// through [`AudioSignalSender`](crate::AudioSignalSender).
#[derive(Debug, Clone, PartialEq)]
pub enum AudioSignal {
    /// Input level in `[0, 1]`, at most 30 per second (AUD-8).
    Level(f32),
    /// The capture cannot continue (device lost, no buffers): stop and
    /// transcribe what was captured, showing `notice` (AUD-9, AUD-10).
    Interrupted {
        /// The affected capture.
        capture_id: u64,
        /// The audio error notice.
        notice: UserMessage,
    },
    /// The input rebuilt (device switch, configuration change) and
    /// recording continues on `device_name`; the silent-microphone title
    /// follows it (AUD-13).
    Rebuilt {
        /// Name of the device now in use.
        device_name: String,
    },
    /// One buffer failed to convert; recording continues and the notice is
    /// rate limited.
    ConversionFailed {
        /// The affected capture.
        capture_id: u64,
        /// The audio error notice.
        notice: UserMessage,
    },
}
