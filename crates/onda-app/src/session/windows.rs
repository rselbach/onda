//! The windows inside the session actor: which pages are visible, the
//! commands their controls send, the status menu items that open them, and
//! the complete view models published when their inputs change (02
//! "Events", UI-2, UI-4, UI-6, UI-11, WIN-1..WIN-21).

use std::collections::{HashMap, VecDeque};

use onda_domain::cleanup::CleanupModel;
use onda_domain::hotkey::{HotkeyPreset, Shortcut};
use onda_domain::languages::LanguageSetting;
use onda_domain::messages::{LoginItemError, LoginItemRegistryStatus, LoginItemStatus};
use onda_domain::migration::{Migration, NoticeChoice};
use onda_domain::models::ModelId;
use onda_domain::setup::{PermissionFix, SetupAction, base_model_to_download};
use onda_domain::types::{ActivationMode, HistoryRetention, MicrophonePermission, OutputMode};
use uuid::Uuid;

use super::cleanup::CleanupPageState;
use super::history::HistoryState;
use super::onboarding::OnboardingState;
use super::rules_editor::ReplacementsEditor;
use super::{Actor, Msg, PermissionSnapshot, Task, blocking_send};
use crate::effects::Effect;
use crate::events::Event;
use crate::ports::PrivacyPane;
use crate::seams::{InputDeviceInfo, SettingChange};
use crate::views::{
    AboutPage, GeneralInputs, GeneralPage, GettingStartedPage, HistoryPage, HotkeyPage,
    MainWindowSection, OnboardingView, PostProcessingPage, ReplacementsPage, SpeechAlert,
    SpeechInputs, SpeechPage, UpdaterState, about_page, general_page, getting_started_page,
    hotkey_page, speech_page,
};

/// Window commands, timers, and background results for the actor.
pub(crate) enum UiMsg {
    // windows (UI-2)
    PageAppeared(MainWindowSection),
    PageDisappeared(MainWindowSection),
    MainWindowAppeared,
    MainWindowClosed,
    OpenSection(MainWindowSection),
    // status menu (UI-6..UI-11, LANG-8..LANG-10, DL-11, UPD-1)
    MenuWillOpen,
    SelectLanguageFromMenu(LanguageSetting),
    RedownloadModel,
    CheckForUpdates,
    UpdaterChanged,
    InputDevicesChanged,
    // Getting Started and General (WIN-5, WIN-8..WIN-14)
    FixPermission(PermissionFix),
    SetupAction(SetupAction),
    SelectInput(String),
    SetOutputMode(OutputMode),
    SetPauseMedia(bool),
    SetSounds(bool),
    SetLaunchAtLogin(bool),
    OpenLoginItems,
    SetAutomaticUpdates(bool),
    // Hotkey (WIN-16, WIN-17, HOT-35)
    SetActivationMode(ActivationMode),
    ResetHotkey,
    OpenKeyboardSettings,
    // Speech (WIN-18..WIN-20)
    SelectLanguage(LanguageSetting),
    CancelDownload(ModelId),
    RequestDeleteModel(ModelId),
    ConfirmSpeechAlert(u64),
    DismissSpeechAlert(u64),
    // Post-Processing (CLN-24..CLN-27)
    SetCleanupEnabled(bool),
    SetCleanupModel(&'static CleanupModel),
    ApiKeyEdited(String),
    ApiKeyCommit,
    TestCleanup,
    DismissCleanupTestError,
    // Replacements (REP-8..REP-12)
    AddRule,
    SetRuleTriggers(String, String),
    SetRuleReplacement(String, String),
    DeleteRule(String),
    // History (HIS-17..HIS-27)
    SetHistoryEnabled(bool),
    SetHistoryRetention(HistoryRetention),
    SetHistoryKeepsAudio(bool),
    SearchHistory(String),
    CopyEntry(Uuid),
    CopyOriginal(Uuid),
    PlayRecording(Uuid),
    ShowRecording(Uuid),
    DeleteEntry(Uuid),
    RequestClearHistory,
    ConfirmHistoryDialog,
    CancelHistoryDialog,
    DismissHistoryError,
    // onboarding (ONB-1..ONB-13, WIN-4)
    RunSetupAgain,
    OnboardingContinue,
    OnboardingBack,
    OnboardingClosed,
    OnboardingSelectLanguage(LanguageSetting),
    OnboardingSearch(String),
    OnboardingToggleAllLanguages,
    OnboardingUseClipboard,
    OnboardingSelectHotkey(HotkeyPreset),
    OnboardingChooseModel(ModelId),
    OnboardingDownloadModel(ModelId),
    OnboardingToggleOtherModels,
    // the update notice (DL-14)
    NoticeChoice(NoticeChoice),
    // timers
    AutoOnboarding,
    ShowNotice,
    OnboardingPoll,
    ReplacementsDebounce(u64),
    CopiedElapsed(u64),
    // background results
    Devices(Vec<InputDeviceInfo>),
    LoginStatus(Option<LoginItemRegistryStatus>),
    LoginToggled {
        enabling: bool,
        result: Result<(), String>,
        status: Option<LoginItemRegistryStatus>,
    },
    Updater(UpdaterState),
    KeyLoaded {
        generation: u64,
        result: Result<Option<String>, String>,
    },
    KeySaved {
        value: String,
        result: Result<(), String>,
    },
    TestFinished {
        generation: u64,
        result: Result<String, String>,
    },
    HistoryLoaded(Result<Vec<onda_domain::history::HistoryEntry>, String>),
    HistoryOpDone(Result<(), String>),
    ModelDeleted {
        model: ModelId,
        reselect: Option<ModelId>,
        result: Result<(), String>,
    },
}

/// What confirming a Speech page alert does.
pub(super) enum PendingAlertAction {
    Delete(ModelId),
}

/// The Speech page's alert queue (WIN-20).
#[derive(Default)]
pub(super) struct SpeechState {
    pub(super) alerts: VecDeque<SpeechAlert>,
    pub(super) actions: HashMap<u64, PendingAlertAction>,
    pub(super) next_id: u64,
}

/// General page state read from the shell's ports.
#[derive(Default)]
pub(super) struct GeneralState {
    pub(super) devices: Vec<InputDeviceInfo>,
    // `None` until read; `Some(None)` when Onda cannot register (LOGIN-1)
    pub(super) login: Option<Option<LoginItemRegistryStatus>>,
    pub(super) login_error: Option<LoginItemError>,
    pub(super) updater: UpdaterState,
}

/// The update notice (DL-14).
#[derive(Default)]
pub(super) struct NoticeState {
    pub(super) pending: Option<Migration>,
    pub(super) open: Option<Migration>,
    pub(super) timer: Option<Task>,
}

#[derive(Default)]
struct PublishedWindows {
    getting_started: Option<GettingStartedPage>,
    general: Option<GeneralPage>,
    hotkey: Option<HotkeyPage>,
    speech: Option<SpeechPage>,
    post_processing: Option<PostProcessingPage>,
    replacements: Option<ReplacementsPage>,
    history: Option<HistoryPage>,
    about: Option<AboutPage>,
    onboarding: Option<OnboardingView>,
    regular: bool,
}

impl PublishedWindows {
    fn forget(&mut self, page: MainWindowSection) {
        match page {
            MainWindowSection::GettingStarted => self.getting_started = None,
            MainWindowSection::General => self.general = None,
            MainWindowSection::Hotkey => self.hotkey = None,
            MainWindowSection::Speech => self.speech = None,
            MainWindowSection::PostProcessing => self.post_processing = None,
            MainWindowSection::Replacements => self.replacements = None,
            MainWindowSection::History => self.history = None,
            MainWindowSection::About => self.about = None,
        }
    }
}

/// Window state the session actor owns.
#[derive(Default)]
pub(super) struct Ui {
    main_window_open: bool,
    visible: Vec<MainWindowSection>,
    pub(super) general: GeneralState,
    pub(super) speech: SpeechState,
    pub(super) cleanup: CleanupPageState,
    pub(super) replacements: ReplacementsEditor,
    pub(super) history: HistoryState,
    pub(super) onboarding: Option<OnboardingState>,
    pub(super) notice: NoticeState,
    pub(super) launch_onboarding: Option<Task>,
    published: PublishedWindows,
}

impl Ui {
    pub(super) fn is_visible(&self, page: MainWindowSection) -> bool {
        self.visible.contains(&page)
    }
}

impl Actor {
    pub(super) fn on_ui(&mut self, msg: UiMsg) {
        match msg {
            UiMsg::PageAppeared(page) => self.page_appeared(page),
            UiMsg::PageDisappeared(page) => self.page_disappeared(page),
            UiMsg::MainWindowAppeared => self.ui.main_window_open = true,
            UiMsg::MainWindowClosed => {
                for page in std::mem::take(&mut self.ui.visible) {
                    self.leave_page(page);
                }
                self.ui.main_window_open = false;
            }
            UiMsg::OpenSection(page) => self.open_section(page),
            UiMsg::MenuWillOpen => {
                self.refresh_settings();
                self.refresh_permissions();
                self.refresh_downloaded();
                self.refresh_updater();
                self.record_recent_languages();
            }
            UiMsg::SelectLanguageFromMenu(language) => {
                self.refresh_settings();
                self.select_language_from_menu(language);
            }
            UiMsg::RedownloadModel => {
                self.refresh_settings();
                self.redownload_model();
            }
            UiMsg::CheckForUpdates => self.effects.send(Effect::CheckForUpdates),
            UiMsg::UpdaterChanged => self.refresh_updater(),
            UiMsg::InputDevicesChanged => {
                if self.ui.is_visible(MainWindowSection::General) {
                    self.refresh_devices();
                }
            }
            UiMsg::FixPermission(fix) => self.fix_permission(fix),
            UiMsg::SetupAction(action) => self.setup_action(action),
            UiMsg::SelectInput(uid) => {
                self.apply_setting(SettingChange::InputDevice(uid.clone()));
                let audio = self.ports.audio.clone();
                tokio::task::spawn_blocking(move || audio.select_input(&uid));
            }
            UiMsg::SetOutputMode(mode) => self.apply_setting(SettingChange::OutputMode(mode)),
            UiMsg::SetPauseMedia(on) => self.apply_setting(SettingChange::PauseMedia(on)),
            UiMsg::SetSounds(on) => self.apply_setting(SettingChange::Sounds(on)),
            UiMsg::SetLaunchAtLogin(enabling) => {
                let login = self.ports.login_item.clone();
                tokio::spawn(blocking_send(self.messages.clone(), move || {
                    let result = login.set_enabled(enabling);
                    // LOGIN-3: re-read after every toggle attempt
                    Msg::Ui(UiMsg::LoginToggled {
                        enabling,
                        result,
                        status: login.status(),
                    })
                }));
            }
            UiMsg::OpenLoginItems => self.effects.send(Effect::OpenLoginItems),
            UiMsg::SetAutomaticUpdates(on) => {
                self.effects.send(Effect::SetAutomaticUpdates(on));
                self.refresh_updater();
            }
            UiMsg::SetActivationMode(mode) => {
                self.apply_setting(SettingChange::ActivationMode(mode));
            }
            UiMsg::ResetHotkey => self.set_hotkey(&Shortcut::default()),
            UiMsg::OpenKeyboardSettings => self.effects.send(Effect::OpenKeyboardSettings),
            UiMsg::SelectLanguage(language) => {
                self.apply_setting(SettingChange::Language(language));
            }
            UiMsg::CancelDownload(model) => self.cancel_download(model),
            UiMsg::RequestDeleteModel(model) => self.request_delete_model(model),
            UiMsg::ConfirmSpeechAlert(id) => self.close_speech_alert(id, true),
            UiMsg::DismissSpeechAlert(id) => self.close_speech_alert(id, false),
            UiMsg::SetCleanupEnabled(on) => self.set_cleanup_enabled(on),
            UiMsg::SetCleanupModel(model) => self.set_cleanup_model(model),
            UiMsg::ApiKeyEdited(text) => self.api_key_edited(text),
            UiMsg::ApiKeyCommit => self.save_api_key(),
            UiMsg::TestCleanup => self.test_cleanup(),
            UiMsg::DismissCleanupTestError => self.ui.cleanup.test_error = None,
            UiMsg::AddRule => self.add_rule(),
            UiMsg::SetRuleTriggers(id, text) => self.edit_rule(&id, Some(text), None),
            UiMsg::SetRuleReplacement(id, text) => self.edit_rule(&id, None, Some(text)),
            UiMsg::DeleteRule(id) => self.delete_rule(&id),
            UiMsg::SetHistoryEnabled(on) => self.apply_setting(SettingChange::HistoryEnabled(on)),
            UiMsg::SetHistoryRetention(retention) => self.set_history_retention(retention),
            UiMsg::SetHistoryKeepsAudio(on) => {
                self.apply_setting(SettingChange::HistoryKeepsAudio(on));
            }
            UiMsg::SearchHistory(query) => self.ui.history.query = query,
            UiMsg::CopyEntry(id) => self.copy_entry(id, false),
            UiMsg::CopyOriginal(id) => self.copy_entry(id, true),
            UiMsg::PlayRecording(id) => self.open_recording(id, false),
            UiMsg::ShowRecording(id) => self.open_recording(id, true),
            UiMsg::DeleteEntry(id) => self.delete_entry(id),
            UiMsg::RequestClearHistory => self.request_clear_history(),
            UiMsg::ConfirmHistoryDialog => self.confirm_history_dialog(),
            UiMsg::CancelHistoryDialog => self.ui.history.dialog = None,
            UiMsg::DismissHistoryError => self.ui.history.error = None,
            UiMsg::RunSetupAgain => self.open_onboarding(true),
            UiMsg::OnboardingContinue => self.onboarding_continue(),
            UiMsg::OnboardingBack => self.onboarding_back(),
            UiMsg::OnboardingClosed => self.onboarding_closed(),
            UiMsg::OnboardingSelectLanguage(language) => self.onboarding_select_language(language),
            UiMsg::OnboardingSearch(query) => {
                if let Some(onboarding) = &mut self.ui.onboarding {
                    onboarding.query = query;
                }
            }
            UiMsg::OnboardingToggleAllLanguages => {
                if let Some(onboarding) = &mut self.ui.onboarding {
                    onboarding.show_all_languages = !onboarding.show_all_languages;
                }
            }
            UiMsg::OnboardingUseClipboard => self.onboarding_use_clipboard(),
            UiMsg::OnboardingSelectHotkey(preset) => self.onboarding_select_hotkey(preset),
            UiMsg::OnboardingChooseModel(model) => self.onboarding_choose_model(model),
            UiMsg::OnboardingDownloadModel(model) => self.onboarding_download_model(model),
            UiMsg::OnboardingToggleOtherModels => {
                if let Some(onboarding) = &mut self.ui.onboarding {
                    onboarding.show_other_models = !onboarding.show_other_models;
                }
            }
            UiMsg::NoticeChoice(choice) => {
                self.refresh_settings();
                self.notice_choice(choice);
            }
            UiMsg::AutoOnboarding => {
                self.ui.launch_onboarding = None;
                self.refresh_settings();
                if self.settings.first_run_pending {
                    self.open_onboarding(false);
                }
            }
            UiMsg::ShowNotice => self.show_pending_notice(),
            UiMsg::OnboardingPoll => self.onboarding_poll(),
            UiMsg::ReplacementsDebounce(generation) => {
                if generation == self.ui.replacements.generation {
                    self.flush_replacements();
                }
            }
            UiMsg::CopiedElapsed(generation) => self.copied_elapsed(generation),
            UiMsg::Devices(devices) => self.ui.general.devices = devices,
            UiMsg::LoginStatus(status) => self.ui.general.login = Some(status),
            UiMsg::LoginToggled {
                enabling,
                result,
                status,
            } => {
                self.ui.general.login = Some(status);
                self.ui.general.login_error = result.err().map(|description| {
                    tracing::error!(error = %description, enabling, "login item toggle failed");
                    LoginItemError {
                        enabling,
                        description,
                    }
                });
            }
            UiMsg::Updater(state) => self.ui.general.updater = state,
            UiMsg::KeyLoaded { generation, result } => self.key_loaded(generation, result),
            UiMsg::KeySaved { value, result } => self.key_saved(value, result),
            UiMsg::TestFinished { generation, result } => self.test_finished(generation, result),
            UiMsg::HistoryLoaded(result) => self.history_loaded(result),
            UiMsg::HistoryOpDone(result) => self.history_op_done(result),
            UiMsg::ModelDeleted {
                model,
                reselect,
                result,
            } => self.model_deleted(model, reselect, result),
        }
    }

    /// Persists one settings change and re-reads settings (SET-5).
    pub(super) fn apply_setting(&mut self, change: SettingChange) {
        if let Err(error) = self.ports.settings.apply(change) {
            tracing::error!(%error, "could not save a setting");
        }
        self.refresh_settings();
    }

    /// Opens the main window on `page` (UI-4, UI-11).
    pub(super) fn open_section(&mut self, page: MainWindowSection) {
        self.events.emit(Event::OpenMainWindow(page));
    }

    // --- page visibility (WIN-7, WIN-15, HIS-16, CLN-25, REP-12, HOT-8) ---

    fn page_appeared(&mut self, page: MainWindowSection) {
        self.ui.main_window_open = true;
        if !self.ui.visible.contains(&page) {
            self.ui.visible.push(page);
        }
        // a page that appears always gets a fresh view
        self.forget_page(page);
        self.refresh_settings();
        match page {
            MainWindowSection::GettingStarted => {
                self.refresh_permissions();
                self.refresh_downloaded();
            }
            MainWindowSection::General => {
                self.refresh_permissions();
                self.refresh_login_item();
                self.refresh_devices();
                self.refresh_updater();
            }
            MainWindowSection::Hotkey => self.hotkey_app_became_active(),
            MainWindowSection::Speech => self.refresh_downloaded(),
            MainWindowSection::PostProcessing => self.post_processing_appeared(),
            MainWindowSection::Replacements => self.reload_replacements(),
            MainWindowSection::History => self.reload_history(true),
            MainWindowSection::About => {}
        }
    }

    fn page_disappeared(&mut self, page: MainWindowSection) {
        let was_visible = self.ui.visible.contains(&page);
        self.ui.visible.retain(|p| *p != page);
        if was_visible {
            self.leave_page(page);
        }
    }

    fn forget_page(&mut self, page: MainWindowSection) {
        self.ui.published.forget(page);
        if page == MainWindowSection::History {
            self.forget_history_view();
        }
    }

    fn leave_page(&mut self, page: MainWindowSection) {
        self.forget_page(page);
        match page {
            MainWindowSection::PostProcessing => {
                self.save_api_key();
                // leaving cancels a running test silently (CLN-27)
                self.cancel_cleanup_test();
            }
            MainWindowSection::Replacements => self.flush_replacements(),
            // the recorder disappeared: end its capture (HOT-8)
            MainWindowSection::Hotkey => self.end_recorder_capture(),
            MainWindowSection::GettingStarted
            | MainWindowSection::General
            | MainWindowSection::Speech
            | MainWindowSection::History
            | MainWindowSection::About => {}
        }
    }

    pub(super) fn windows_app_became_active(&mut self) {
        if self.ui.is_visible(MainWindowSection::General) {
            self.refresh_login_item();
            self.refresh_updater();
        }
        if self.ui.is_visible(MainWindowSection::GettingStarted) {
            self.refresh_downloaded();
        }
    }

    // --- shell reads (LOGIN-3, UPD-1, WIN-9) ---

    fn refresh_login_item(&self) {
        let login = self.ports.login_item.clone();
        tokio::spawn(blocking_send(self.messages.clone(), move || {
            Msg::Ui(UiMsg::LoginStatus(login.status()))
        }));
    }

    pub(super) fn refresh_updater(&self) {
        let updater = self.ports.updater.clone();
        tokio::spawn(blocking_send(self.messages.clone(), move || {
            Msg::Ui(UiMsg::Updater(UpdaterState {
                can_check: updater.can_check(),
                automatic: updater.automatic_checks(),
                last_check: updater.last_check(),
            }))
        }));
    }

    fn refresh_devices(&self) {
        let audio = self.ports.audio.clone();
        tokio::spawn(blocking_send(self.messages.clone(), move || {
            Msg::Ui(UiMsg::Devices(audio.input_devices()))
        }));
    }

    // --- permission fixes (WIN-5, WIN-8, ONB-6, PERM-3) ---

    pub(super) fn fix_permission(&mut self, fix: PermissionFix) {
        let permissions = self.ports.permissions.clone();
        tokio::spawn(blocking_send(self.messages.clone(), move || {
            match fix {
                PermissionFix::RequestMicrophone => {
                    let granted = match permissions.microphone() {
                        MicrophonePermission::Granted => true,
                        MicrophonePermission::NotDetermined => permissions.request_microphone(),
                        MicrophonePermission::Denied | MicrophonePermission::Restricted => false,
                    };
                    // a denial can only be undone in settings (PERM-1)
                    if !granted {
                        permissions.open_settings(PrivacyPane::Microphone);
                    }
                }
                PermissionFix::OpenMicrophoneSettings => {
                    permissions.open_settings(PrivacyPane::Microphone);
                }
                PermissionFix::OpenAccessibilitySettings => {
                    // the prompt adds Onda to the list, then the pane opens
                    permissions.prompt_accessibility();
                    permissions.open_settings(PrivacyPane::Accessibility);
                }
            }
            Msg::Permissions(PermissionSnapshot {
                microphone: permissions.microphone(),
                accessibility: permissions.accessibility_trusted(),
            })
        }));
    }

    fn setup_action(&mut self, action: SetupAction) {
        match action {
            SetupAction::RequestMicrophone => self.fix_permission(PermissionFix::RequestMicrophone),
            SetupAction::OpenAccessibilitySettings => {
                self.fix_permission(PermissionFix::OpenAccessibilitySettings);
            }
            SetupAction::DownloadBaseModel => {
                let selected = self.settings.selected_model;
                let recommended =
                    onda_domain::routing::recommended_model(&self.catalog, self.settings.language)
                        .unwrap_or(selected);
                let model = base_model_to_download(
                    selected.as_str(),
                    !self.catalog.is_built_in(selected),
                    recommended.as_str(),
                );
                if let Some(model) = ModelId::parse(model) {
                    self.begin_download(model);
                }
            }
        }
    }

    // --- publishing (02 "Events") ---

    pub(super) fn publish_windows(&mut self) {
        let regular = self.ui.main_window_open
            || self.ui.onboarding.is_some()
            || self.ui.notice.open.is_some();
        if regular != self.ui.published.regular {
            self.ui.published.regular = regular;
            self.events.emit(Event::AppPresenceChanged { regular });
        }
        for page in MainWindowSection::ALL {
            if self.ui.visible.contains(&page) {
                self.publish_page(page);
            }
        }
        self.publish_onboarding();
    }

    fn publish_page(&mut self, page: MainWindowSection) {
        match page {
            MainWindowSection::GettingStarted => {
                let view = getting_started_page(
                    &self.checklist(),
                    self.settings.hotkey.keycaps(),
                    self.settings.output_mode,
                );
                if self.ui.published.getting_started.as_ref() != Some(&view) {
                    self.events
                        .emit(Event::GettingStartedChanged(Box::new(view.clone())));
                    self.ui.published.getting_started = Some(view);
                }
            }
            MainWindowSection::General => {
                let view = self.general_view();
                if self.ui.published.general.as_ref() != Some(&view) {
                    self.events
                        .emit(Event::GeneralChanged(Box::new(view.clone())));
                    self.ui.published.general = Some(view);
                }
            }
            MainWindowSection::Hotkey => {
                let view = hotkey_page(&self.settings.hotkey, self.settings.activation_mode);
                if self.ui.published.hotkey.as_ref() != Some(&view) {
                    self.events
                        .emit(Event::HotkeyPageChanged(Box::new(view.clone())));
                    self.ui.published.hotkey = Some(view);
                }
            }
            MainWindowSection::Speech => {
                let view = speech_page(SpeechInputs {
                    catalog: &self.catalog,
                    language: self.settings.language,
                    selected_model: self.settings.selected_model,
                    os_language: self.settings.os_language.as_deref(),
                    downloaded: &self.downloaded,
                    downloads: &self.downloads,
                    alert: self.ui.speech.alerts.front(),
                });
                if self.ui.published.speech.as_ref() != Some(&view) {
                    self.events
                        .emit(Event::SpeechChanged(Box::new(view.clone())));
                    self.ui.published.speech = Some(view);
                }
            }
            MainWindowSection::PostProcessing => {
                let view = self.post_processing_view();
                if self.ui.published.post_processing.as_ref() != Some(&view) {
                    self.events
                        .emit(Event::PostProcessingChanged(Box::new(view.clone())));
                    self.ui.published.post_processing = Some(view);
                }
            }
            MainWindowSection::Replacements => {
                let view = self.replacements_view();
                if self.ui.published.replacements.as_ref() != Some(&view) {
                    self.events
                        .emit(Event::ReplacementsChanged(Box::new(view.clone())));
                    self.ui.published.replacements = Some(view);
                }
            }
            MainWindowSection::History => {
                let Some(view) = self.history_view() else {
                    return;
                };
                if self.ui.published.history.as_ref() != Some(&view) {
                    self.events
                        .emit(Event::HistoryPageChanged(Box::new(view.clone())));
                    self.ui.published.history = Some(view);
                }
            }
            MainWindowSection::About => {
                let view = about_page(&self.catalog, &self.ports.app_version);
                if self.ui.published.about.as_ref() != Some(&view) {
                    self.events
                        .emit(Event::AboutChanged(Box::new(view.clone())));
                    self.ui.published.about = Some(view);
                }
            }
        }
    }

    fn general_view(&self) -> GeneralPage {
        let general = &self.ui.general;
        let login = match general.login {
            // not read yet: show it off until the registry answers
            None => LoginItemStatus::Disabled,
            Some(None) => LoginItemStatus::Unavailable,
            Some(Some(status)) => LoginItemStatus::from_registry(status, true),
        };
        general_page(GeneralInputs {
            microphone: self.permissions.microphone,
            accessibility_granted: self.permissions.accessibility,
            accessibility_needed: self.accessibility_needed(),
            devices: &general.devices,
            selected_uid: &self.settings.input_device_uid,
            output_mode: self.settings.output_mode,
            pause_media: self.settings.pause_media,
            sounds: self.settings.sounds,
            login,
            login_error: general.login_error.as_ref(),
            updater: general.updater,
            version: &self.ports.app_version,
        })
    }

    fn publish_onboarding(&mut self) {
        let Some(view) = self.onboarding_view() else {
            self.ui.published.onboarding = None;
            return;
        };
        if self.ui.published.onboarding.as_ref() != Some(&view) {
            self.events
                .emit(Event::OnboardingChanged(Box::new(view.clone())));
            self.ui.published.onboarding = Some(view);
        }
    }
}
