//! Onda orchestration.
//!
//! The core is the app: it owns state, decides
//! with `onda-domain`, and pushes complete view models to the shell through
//! an [`EventSink`]. The shell renders them and implements the [`ports`].
//!
//! [`App`] runs one session actor (ARCH-7) on a tokio runtime the shell
//! never sees. Commands are non-blocking: each one is queued to the actor in
//! call order (ARCH-11), and the actor reports through events.

mod effects;
mod events;
mod media;
mod pipeline;
pub mod ports;
pub mod seams;
mod session;
pub mod system_speech;
mod transcription;
pub mod views;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use onda_domain::hotkey::{
    KeyCode, ModifierFlagsChange, RecorderEvent, RecorderOutcome, Shortcut, ShortcutRecorder,
};
use onda_domain::models::{Catalog, ModelId};
use tokio::runtime::{Handle, Runtime};
use tokio::sync::{mpsc, oneshot};

use events::EventBus;
pub use events::{Event, EventSink};
use onda_domain::cleanup::CleanupProvider;
use onda_domain::hotkey::HotkeyPreset;
use onda_domain::languages::LanguageSetting;
use onda_domain::migration::NoticeChoice;
use onda_domain::privacy_naming::PrivacyNaming;
use onda_domain::setup::{PermissionFix, SetupAction};
use onda_domain::types::{ActivationMode, HistoryRetention, OutputMode};
use onda_domain::updates::UpdateAction;
use ports::{
    Files, HotkeySource, LoginItem, MediaControl, Notifier, Permissions, Power, SoundPlayer,
    TextOutput, Updater,
};
use seams::{
    AudioSignal, AudioSource, CleanupTransport, DownloadUpdate, HistorySink, ModelStore,
    SecretSource, SettingsSource,
};
use session::{Actor, Msg, UiMsg};
use uuid::Uuid;
pub use views::MainWindowSection;

/// How long [`App::shutdown`] waits for background work after the session
/// ended.
const RUNTIME_SHUTDOWN: Duration = Duration::from_secs(1);

/// Everything the core calls out to. The shell supplies the ports; the app
/// wires the core-side seams to their crates.
pub struct Ports {
    /// Text delivery.
    pub text_output: Arc<dyn TextOutput>,
    /// Privacy permissions.
    pub permissions: Arc<dyn Permissions>,
    /// Now-playing control, when the platform has one (MEDIA-1).
    pub media: Option<Arc<dyn MediaControl>>,
    /// Cue sounds.
    pub sounds: Arc<dyn SoundPlayer>,
    /// Idle-sleep prevention.
    pub power: Arc<dyn Power>,
    /// The dictation shortcut and the session-only Escape.
    pub hotkeys: Arc<dyn HotkeySource>,
    /// System notifications.
    pub notifier: Arc<dyn Notifier>,
    /// The login-item registry (LOGIN-1).
    pub login_item: Arc<dyn LoginItem>,
    /// The shell-owned updater (UPD-1).
    pub updater: Arc<dyn Updater>,
    /// File manager services.
    pub files: Arc<dyn Files>,
    /// The microphone.
    pub audio: Arc<dyn AudioSource>,
    /// Installed models and engines.
    pub models: Arc<dyn ModelStore>,
    /// This run's model catalog: with the system engine when the shell
    /// provides a [`SystemSpeech`](ports::SystemSpeech) port (MOD-4), whose
    /// engine `models` then serves.
    pub catalog: Catalog,
    /// Transcript history.
    pub history: Arc<dyn HistorySink>,
    /// OpenRouter requests.
    pub cleanup: Arc<dyn CleanupTransport>,
    /// The OpenRouter key.
    pub secrets: Arc<dyn SecretSource>,
    /// Settings.
    pub settings: Arc<dyn SettingsSource>,
    /// What copy calls the tray icon on this platform: "menu bar icon" on
    /// macOS, "tray icon" elsewhere.
    pub tray_term: Arc<str>,
    /// What the OS calls the Accessibility permission, which copy names
    /// the way the running OS does.
    pub privacy_naming: PrivacyNaming,
    /// The app version the shell reports, e.g. "1.2.0" (WIN-14, WIN-21).
    pub app_version: String,
}

/// A raw dictation-hotkey event from the shell, in the order it happened
/// (HOT-20a).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// The combo shortcut went down.
    ComboDown,
    /// The combo shortcut went up.
    ComboUp,
    /// A modifier-flags change from the lone-modifier event stream
    /// (HOT-16); the core runs the gesture (HOT-21..HOT-23).
    ModifierFlags(ModifierFlagsChange),
    /// A key-down from the lone-modifier event stream (HOT-14a).
    KeyDown(KeyCode),
    /// The OS disabled the lone-modifier event stream (timeout or user
    /// input). The core re-enables it or gives up (HOT-17).
    StreamDisabled {
        /// The OS status to report if the core gives up.
        status: i32,
    },
    /// The Escape cancel hotkey fired (HOT-34).
    Cancel,
}

/// Feeds audio-source signals to a running core. Cheap to clone and send.
#[derive(Clone)]
pub struct AudioSignalSender {
    messages: mpsc::UnboundedSender<Msg>,
}

impl AudioSignalSender {
    /// Delivers `signal`. Returns false once the core is gone.
    pub fn send(&self, signal: AudioSignal) -> bool {
        self.messages.send(Msg::Audio(signal)).is_ok()
    }
}

/// A running Onda core.
pub struct App {
    messages: mpsc::UnboundedSender<Msg>,
    events: Arc<EventBus>,
    // the shell needs the swallow decision synchronously (HOT-10), so the
    // recorder lives here and the actor reads it to publish its view
    recorder: Arc<Mutex<ShortcutRecorder>>,
    runtime: Option<Runtime>,
}

impl App {
    /// Starts the core on its own runtime and publishes the initial view
    /// models.
    pub fn start(ports: Ports, sink: Arc<dyn EventSink>) -> std::io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("onda-core")
            .enable_all()
            .build()?;
        let mut app = Self::start_on(runtime.handle(), ports, sink);
        app.runtime = Some(runtime);
        Ok(app)
    }

    /// Starts the core on an existing runtime (tests, embedders that own
    /// one). Shut it down with [`App::shutdown_async`].
    pub fn start_on(runtime: &Handle, ports: Ports, sink: Arc<dyn EventSink>) -> Self {
        let events = Arc::new(EventBus::new(sink));
        let (messages, inbox) = mpsc::unbounded_channel();
        let recorder = Arc::new(Mutex::new(ShortcutRecorder::new()));
        let actor_events = events.clone();
        let actor_messages = messages.clone();
        let actor_recorder = recorder.clone();
        runtime.spawn(async move {
            Actor::new(ports, actor_events, actor_messages, actor_recorder)
                .run(inbox)
                .await;
        });
        Self {
            messages,
            events,
            recorder,
            runtime: None,
        }
    }

    /// The UI is up: refresh permissions and load the selected model unless
    /// first-run onboarding is pending (ASR-1).
    pub fn ui_ready(&self) {
        self.send(Msg::UiReady);
    }

    /// Onda became the active app: re-read permissions (UI-6, PERM-2).
    pub fn app_became_active(&self) {
        self.send(Msg::AppBecameActive);
    }

    /// Settings changed outside a session command; views re-read them.
    pub fn settings_changed(&self) {
        self.send(Msg::SettingsChanged);
    }

    /// A raw dictation-hotkey event.
    pub fn on_hotkey_event(&self, event: HotkeyEvent) {
        self.send(Msg::Hotkey(event));
    }

    /// The shell hit a hotkey failure outside a registration the core asked
    /// for (such as installing its handler); `status` is the OS status code.
    /// Held until the UI is ready (HOT-20), then notified per HOT-19.
    pub fn hotkey_registration_failed(&self, status: i32) {
        self.send(Msg::HotkeyRegistrationFailed(status));
    }

    /// Saves `shortcut`, then registers it once (HOT-12, SET-5).
    pub fn set_hotkey(&self, shortcut: Shortcut) {
        self.send(Msg::SetHotkey(shortcut));
    }

    /// The shortcut recorder started: the global hotkey is unregistered so
    /// pressing it does not dictate, and any retry episode ends (HOT-8).
    pub fn begin_hotkey_capture(&self) {
        lock(&self.recorder).start();
        self.send(Msg::CaptureBegan);
    }

    /// The record button: cancels the running capture, else starts one
    /// (HOT-8, HOT-11).
    pub fn toggle_hotkey_capture(&self) {
        let mut recorder = lock(&self.recorder);
        if recorder.stop() {
            drop(recorder);
            self.send(Msg::CaptureEnded);
            return;
        }
        recorder.start();
        drop(recorder);
        self.send(Msg::CaptureBegan);
    }

    /// Recording ended without a change: the button was clicked again,
    /// Cancel was clicked, or the recorder disappeared. Re-registers the
    /// current shortcut (HOT-8).
    pub fn end_hotkey_capture(&self) {
        if lock(&self.recorder).stop() {
            self.send(Msg::CaptureEnded);
        }
    }

    /// The shell could not start listening for keys: ends the capture at
    /// once with "Could not start shortcut recording." (HOT-11).
    pub fn hotkey_capture_failed(&self) {
        lock(&self.recorder).start_failed();
        self.send(Msg::CaptureEnded);
    }

    /// A local key event while Onda's own window is focused (HOT-9).
    /// Returns whether the shell must swallow it: every event is swallowed
    /// while recording (HOT-10).
    pub fn recorder_event(&self, event: RecorderEvent) -> bool {
        let outcome = lock(&self.recorder).handle(event);
        match outcome {
            RecorderOutcome::PassThrough => return false,
            RecorderOutcome::Wait => {}
            RecorderOutcome::Commit(shortcut) => self.send(Msg::CaptureCommitted(shortcut)),
            RecorderOutcome::Cancel => self.send(Msg::CaptureEnded),
            RecorderOutcome::Rejected(_) => self.send(Msg::RecorderChanged),
        }
        true
    }

    /// The download overlay's close button: hide it until the download or
    /// load ends (OVL-17).
    pub fn dismiss_download_overlay(&self) {
        self.send(Msg::DismissDownloadOverlay);
    }

    /// The menu's Start / Stop / Cancel Dictation item (UI-9, DICT-21).
    pub fn toggle_dictation_from_menu(&self) {
        self.send(Msg::MenuToggle);
    }

    /// Starts recording when idle. Repeated desktop commands are harmless.
    pub fn start_dictation(&self) {
        self.send(Msg::StartDictation);
    }

    /// Finishes a recording or aborts pending start checks; idle is a no-op.
    pub fn stop_dictation(&self) {
        self.send(Msg::StopDictation);
    }

    /// Cancels the session: overlay ✕, menu cancel (DICT-19).
    pub fn cancel_dictation(&self) {
        self.send(Msg::Cancel);
    }

    /// "Retry Failed Dictation" (DICT-23).
    pub fn retry_failed_dictation(&self) {
        self.send(Msg::Retry);
    }

    /// "Discard Failed Recording" (DICT-24).
    pub fn discard_failed_dictation(&self) {
        self.send(Msg::DiscardFailed);
    }

    /// "Paste Last Transcript" / "Copy Last Transcript" (DICT-26).
    pub fn deliver_last_transcript(&self) {
        self.send(Msg::DeliverLast);
    }

    /// A "Recent Transcripts" entry (DICT-27).
    pub fn deliver_transcript(&self, text: impl Into<String>) {
        self.send(Msg::DeliverText(text.into()));
    }

    /// An explicit model selection from Settings: cancels a running session
    /// and reloads (DL-7, ASR-8, DICT-29).
    pub fn select_model(&self, model: ModelId) {
        self.send(Msg::SelectModel(model));
    }

    /// "Download & Use": download, then switch when idle (DL-8, DICT-29).
    pub fn download_and_use(&self, model: ModelId) {
        self.send(Msg::DownloadAndUse(model));
    }

    /// A model download's progress or outcome, in order (DL-1..DL-4,
    /// ASR-9, DL-8).
    pub fn download_event(&self, update: DownloadUpdate) {
        self.send(Msg::Download(update));
    }

    /// "Retry Loading Model" (ASR-10).
    pub fn retry_model_load(&self) {
        self.send(Msg::RetryModelLoad);
    }

    // --- windows (UI-2, UI-4, UI-11) ---

    /// A main window page appeared: it gets a fresh view model, and the
    /// core re-reads what the page shows (WIN-7, WIN-15, HIS-16, CLN-25,
    /// REP-12).
    pub fn page_appeared(&self, page: MainWindowSection) {
        self.ui(UiMsg::PageAppeared(page));
    }

    /// A main window page disappeared: pending edits flush, a running
    /// cleanup test is cancelled, and a recorder capture ends (CLN-25,
    /// CLN-27, REP-12, HOT-8).
    pub fn page_disappeared(&self, page: MainWindowSection) {
        self.ui(UiMsg::PageDisappeared(page));
    }

    /// The main window appeared (UI-2).
    pub fn main_window_appeared(&self) {
        self.ui(UiMsg::MainWindowAppeared);
    }

    /// The main window closed: every visible page disappears (UI-2).
    pub fn main_window_closed(&self) {
        self.ui(UiMsg::MainWindowClosed);
    }

    // --- status menu (UI-6..UI-11) ---

    /// The status menu is opening: re-read permissions and the updater, and
    /// record the current language as recent (UI-6, LANG-8).
    pub fn menu_will_open(&self) {
        self.ui(UiMsg::MenuWillOpen);
    }

    /// "Fix Permissions…": the main window on General (UI-8).
    pub fn fix_permissions(&self) {
        self.ui(UiMsg::OpenSection(MainWindowSection::General));
    }

    /// "Open Onda": the main window on Getting Started (UI-11).
    pub fn open_app(&self) {
        self.ui(UiMsg::OpenSection(MainWindowSection::GettingStarted));
    }

    /// "Settings…" in the menu or the app menu: the main window on General
    /// (UI-4, UI-11).
    pub fn open_settings(&self) {
        self.ui(UiMsg::OpenSection(MainWindowSection::General));
    }

    /// "Show All History…": the main window on History (HIS-28).
    pub fn show_all_history(&self) {
        self.ui(UiMsg::OpenSection(MainWindowSection::History));
    }

    /// "Other Language…": the main window on Speech (LANG-9).
    pub fn choose_other_language(&self) {
        self.ui(UiMsg::OpenSection(MainWindowSection::Speech));
    }

    /// A Language submenu item (LANG-10).
    pub fn select_language_from_menu(&self, language: LanguageSetting) {
        self.ui(UiMsg::SelectLanguageFromMenu(language));
    }

    /// "Re-download Model" (DL-11).
    pub fn redownload_model(&self) {
        self.ui(UiMsg::RedownloadModel);
    }

    /// The update item in the menu or the update button on General: check,
    /// restart to update, or download (UPD-1, UPD-6, UPD-7).
    pub fn update_action(&self, action: UpdateAction) {
        self.ui(UiMsg::UpdateAction(action));
    }

    /// The updater's state changed; views re-read it.
    pub fn updater_changed(&self) {
        self.ui(UiMsg::UpdaterChanged);
    }

    /// Input devices came or went, or the default changed (WIN-9).
    pub fn input_devices_changed(&self) {
        self.ui(UiMsg::InputDevicesChanged);
    }

    // --- Getting Started and General ---

    /// A permission fix button (WIN-8, ONB-6).
    pub fn fix_permission(&self, fix: PermissionFix) {
        self.ui(UiMsg::FixPermission(fix));
    }

    /// A checklist button (WIN-5).
    pub fn setup_action(&self, action: SetupAction) {
        self.ui(UiMsg::SetupAction(action));
    }

    /// The "Input" picker; empty for "System Default" (WIN-9).
    pub fn select_input_device(&self, uid: impl Into<String>) {
        self.ui(UiMsg::SelectInput(uid.into()));
    }

    /// "After transcription" (WIN-10).
    pub fn set_output_mode(&self, mode: OutputMode) {
        self.ui(UiMsg::SetOutputMode(mode));
    }

    /// "Pause media while recording" (WIN-11).
    pub fn set_pause_media(&self, on: bool) {
        self.ui(UiMsg::SetPauseMedia(on));
    }

    /// "Play sounds when dictation starts and stops" (WIN-12).
    pub fn set_sounds(&self, on: bool) {
        self.ui(UiMsg::SetSounds(on));
    }

    /// "Open Onda at login" (LOGIN-1..LOGIN-3).
    pub fn set_launch_at_login(&self, on: bool) {
        self.ui(UiMsg::SetLaunchAtLogin(on));
    }

    /// "Open Login Items" (LOGIN-2).
    pub fn open_login_items(&self) {
        self.ui(UiMsg::OpenLoginItems);
    }

    /// "Check for updates automatically" (UPD-1).
    pub fn set_automatic_update_checks(&self, on: bool) {
        self.ui(UiMsg::SetAutomaticUpdates(on));
    }

    // --- Hotkey ---

    /// The "Activation" picker (WIN-17).
    pub fn set_activation_mode(&self, mode: ActivationMode) {
        self.ui(UiMsg::SetActivationMode(mode));
    }

    /// "Reset to ⌥ Space" (WIN-16).
    pub fn reset_hotkey(&self) {
        self.ui(UiMsg::ResetHotkey);
    }

    /// "Open Keyboard Settings" next to the Fn warning (HOT-35).
    pub fn open_keyboard_settings(&self) {
        self.ui(UiMsg::OpenKeyboardSettings);
    }

    // --- Speech ---

    /// The Speech page "Language" picker: saves the language only (WIN-18).
    pub fn select_language(&self, language: LanguageSetting) {
        self.ui(UiMsg::SelectLanguage(language));
    }

    /// A download's "Cancel" (DL-3).
    pub fn cancel_download(&self, model: ModelId) {
        self.ui(UiMsg::CancelDownload(model));
    }

    /// A model row's trash button: queues the confirmation (DL-9).
    pub fn request_delete_model(&self, model: ModelId) {
        self.ui(UiMsg::RequestDeleteModel(model));
    }

    /// The Speech alert's confirm button (WIN-20).
    pub fn confirm_speech_alert(&self, id: u64) {
        self.ui(UiMsg::ConfirmSpeechAlert(id));
    }

    /// The Speech alert's dismiss or cancel button (WIN-20).
    pub fn dismiss_speech_alert(&self, id: u64) {
        self.ui(UiMsg::DismissSpeechAlert(id));
    }

    // --- Post-Processing ---

    /// "Post-process transcripts" (CLN-26).
    pub fn set_cleanup_enabled(&self, on: bool) {
        self.ui(UiMsg::SetCleanupEnabled(on));
    }

    /// The "Model" row: opens the model sheet, which loads the API key
    /// (CLN-25) and the provider's model list.
    pub fn open_providers_dialog(&self) {
        self.ui(UiMsg::OpenProvidersDialog);
    }

    /// "Done" in the model sheet; saves the key field (CLN-25).
    pub fn close_providers_dialog(&self) {
        self.ui(UiMsg::CloseProvidersDialog);
    }

    /// A provider in the sheet's sidebar.
    pub fn select_dialog_provider(&self, provider: CleanupProvider) {
        self.ui(UiMsg::SelectDialogProvider(provider));
    }

    /// A model row or the use-by-ID field in the model sheet: `provider`
    /// and the model become the ones in use (CLN-26).
    pub fn select_cleanup_model(&self, provider: CleanupProvider, slug: impl Into<String>) {
        self.ui(UiMsg::SelectCleanupModel {
            provider,
            slug: slug.into(),
        });
    }

    /// "Refresh Models" in the sheet.
    pub fn refresh_cleanup_models(&self) {
        self.ui(UiMsg::RefreshCleanupModels);
    }

    /// The API key field changed; typing during the load cancels it
    /// (CLN-25).
    pub fn api_key_edited(&self, text: impl Into<String>) {
        self.ui(UiMsg::ApiKeyEdited(text.into()));
    }

    /// Return or end of editing in the API key field (CLN-25).
    pub fn api_key_commit(&self) {
        self.ui(UiMsg::ApiKeyCommit);
    }

    /// The "Prompt" row: opens the prompt sheet with the prompt in use as
    /// its draft.
    pub fn open_prompt_editor(&self) {
        self.ui(UiMsg::OpenPromptEditor);
    }

    /// The prompt sheet's text changed. Nothing is saved.
    pub fn prompt_edited(&self, text: impl Into<String>) {
        self.ui(UiMsg::PromptEdited(text.into()));
    }

    /// "Restore Default" in the prompt sheet: Onda's prompt becomes the
    /// draft, unsaved until "Save".
    pub fn restore_default_prompt(&self) {
        self.ui(UiMsg::RestoreDefaultPrompt);
    }

    /// "Cancel" or Escape in the prompt sheet: discards the draft.
    pub fn cancel_prompt_editor(&self) {
        self.ui(UiMsg::CancelPromptEditor);
    }

    /// "Save" in the prompt sheet: the draft becomes the prompt for every
    /// provider and model; Onda's own text removes the override. A blank
    /// draft is refused.
    pub fn save_prompt(&self) {
        self.ui(UiMsg::SavePrompt);
    }

    /// "Test with Sample Text" (CLN-27).
    pub fn test_cleanup(&self) {
        self.ui(UiMsg::TestCleanup);
    }

    /// Dismisses the "Test Failed" alert.
    pub fn dismiss_cleanup_test_error(&self) {
        self.ui(UiMsg::DismissCleanupTestError);
    }

    // --- Replacements ---

    /// "Add Rule" (REP-10).
    pub fn add_replacement_rule(&self) {
        self.ui(UiMsg::AddRule);
    }

    /// A rule's "Replace" field, raw (REP-9).
    pub fn set_replacement_triggers(&self, id: impl Into<String>, text: impl Into<String>) {
        self.ui(UiMsg::SetRuleTriggers(id.into(), text.into()));
    }

    /// A rule's "With" field (REP-9).
    pub fn set_replacement_text(&self, id: impl Into<String>, text: impl Into<String>) {
        self.ui(UiMsg::SetRuleReplacement(id.into(), text.into()));
    }

    /// A rule's "Delete Rule" button (REP-9).
    pub fn delete_replacement_rule(&self, id: impl Into<String>) {
        self.ui(UiMsg::DeleteRule(id.into()));
    }

    // --- History ---

    /// "Save transcripts", also "Turn On History" (HIS-18, HIS-26).
    pub fn set_history_enabled(&self, on: bool) {
        self.ui(UiMsg::SetHistoryEnabled(on));
    }

    /// "Keep for"; asks first when entries would be deleted (HIS-17).
    pub fn set_history_retention(&self, retention: HistoryRetention) {
        self.ui(UiMsg::SetHistoryRetention(retention));
    }

    /// "Keep audio recordings" (HIS-18).
    pub fn set_history_keeps_audio(&self, on: bool) {
        self.ui(UiMsg::SetHistoryKeepsAudio(on));
    }

    /// The search field (HIS-19).
    pub fn search_history(&self, query: impl Into<String>) {
        self.ui(UiMsg::SearchHistory(query.into()));
    }

    /// A row's copy button, the "Copy" item, or the Copy command (HIS-22,
    /// HIS-25).
    pub fn copy_history_entry(&self, id: Uuid) {
        self.ui(UiMsg::CopyEntry(id));
    }

    /// "Copy Original" (HIS-24).
    pub fn copy_history_original(&self, id: Uuid) {
        self.ui(UiMsg::CopyOriginal(id));
    }

    /// "Play Recording" (HIS-24).
    pub fn play_history_recording(&self, id: Uuid) {
        self.ui(UiMsg::PlayRecording(id));
    }

    /// "Show Recording in Finder" (HIS-24).
    pub fn show_history_recording(&self, id: Uuid) {
        self.ui(UiMsg::ShowRecording(id));
    }

    /// "Delete", or the Delete key on the selected row (HIS-24, HIS-25).
    pub fn delete_history_entry(&self, id: Uuid) {
        self.ui(UiMsg::DeleteEntry(id));
    }

    /// "Clear History…": asks first (HIS-20).
    pub fn request_clear_history(&self) {
        self.ui(UiMsg::RequestClearHistory);
    }

    /// The History confirmation's destructive button (HIS-17, HIS-20).
    pub fn confirm_history_dialog(&self) {
        self.ui(UiMsg::ConfirmHistoryDialog);
    }

    /// The History confirmation's "Cancel": the setting stays (HIS-17).
    pub fn cancel_history_dialog(&self) {
        self.ui(UiMsg::CancelHistoryDialog);
    }

    /// Dismisses the "History Error" alert (HIS-27).
    pub fn dismiss_history_error(&self) {
        self.ui(UiMsg::DismissHistoryError);
    }

    // --- onboarding ---

    /// "Run Setup Again" (WIN-4).
    pub fn run_setup_again(&self) {
        self.ui(UiMsg::RunSetupAgain);
    }

    /// The primary button (ONB-1).
    pub fn onboarding_continue(&self) {
        self.ui(UiMsg::OnboardingContinue);
    }

    /// "Back" (ONB-1).
    pub fn onboarding_back(&self) {
        self.ui(UiMsg::OnboardingBack);
    }

    /// The user closed the onboarding window (ONB-11).
    pub fn onboarding_closed(&self) {
        self.ui(UiMsg::OnboardingClosed);
    }

    /// A language card (ONB-4).
    pub fn onboarding_select_language(&self, language: LanguageSetting) {
        self.ui(UiMsg::OnboardingSelectLanguage(language));
    }

    /// The language search field (ONB-4).
    pub fn onboarding_search_languages(&self, query: impl Into<String>) {
        self.ui(UiMsg::OnboardingSearch(query.into()));
    }

    /// "Show All" / "Popular Only" (ONB-4).
    pub fn onboarding_toggle_all_languages(&self) {
        self.ui(UiMsg::OnboardingToggleAllLanguages);
    }

    /// "Use the clipboard instead" (ONB-6).
    pub fn onboarding_use_clipboard(&self) {
        self.ui(UiMsg::OnboardingUseClipboard);
    }

    /// A hotkey preset card (ONB-7).
    pub fn onboarding_select_hotkey(&self, preset: HotkeyPreset) {
        self.ui(UiMsg::OnboardingSelectHotkey(preset));
    }

    /// "Use This Model" (ONB-8).
    pub fn onboarding_choose_model(&self, model: ModelId) {
        self.ui(UiMsg::OnboardingChooseModel(model));
    }

    /// "Download": selects and downloads (ONB-8).
    pub fn onboarding_download_model(&self, model: ModelId) {
        self.ui(UiMsg::OnboardingDownloadModel(model));
    }

    /// "Show other models" / "Hide other models" (ONB-8).
    pub fn onboarding_toggle_other_models(&self) {
        self.ui(UiMsg::OnboardingToggleOtherModels);
    }

    // --- the update notice ---

    /// A notice button; closing the window is [`NoticeChoice::NotNow`]
    /// (DL-14).
    pub fn update_notice_choice(&self, choice: NoticeChoice) {
        self.ui(UiMsg::NoticeChoice(choice));
    }

    /// Where the audio source reports levels and interruptions.
    pub fn audio_signals(&self) -> AudioSignalSender {
        AudioSignalSender {
            messages: self.messages.clone(),
        }
    }

    /// Blocks until every event emitted so far reached the sink.
    pub fn flush_events(&self) {
        self.events.flush();
    }

    /// Cancels any session (DICT-28a), lets media resume and queued shell
    /// calls drain, delivers pending events, and stops. On a runtime this
    /// app does not own, use [`App::shutdown_async`] instead; here it only
    /// requests the stop.
    pub fn shutdown(mut self) {
        let (done, wait) = oneshot::channel();
        self.send(Msg::Shutdown(done));
        if let Some(runtime) = self.runtime.take() {
            let _ = runtime.block_on(wait);
            runtime.shutdown_timeout(RUNTIME_SHUTDOWN);
        }
        self.events.close();
    }

    /// [`App::shutdown`] for an app started with [`App::start_on`].
    pub async fn shutdown_async(&self) {
        let (done, wait) = oneshot::channel();
        self.send(Msg::Shutdown(done));
        let _ = wait.await;
        self.events.close();
    }

    fn ui(&self, msg: UiMsg) {
        self.send(Msg::Ui(msg));
    }

    fn send(&self, msg: Msg) {
        if self.messages.send(msg).is_err() {
            tracing::warn!("the core has stopped");
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
