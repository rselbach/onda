//! Window, onboarding, notice, and status menu commands (M7, M8). Each
//! is one user action; ids cross as strings and are parsed here. An id the
//! core does not know is logged and ignored: the shell only ever sends ids
//! the core gave it.

use onda_app::App;
use onda_app::MainWindowSection;
use onda_domain::cleanup::CleanupProvider;
use onda_domain::hotkey::HotkeyPreset;
use onda_domain::languages::LanguageSetting;
use onda_domain::migration::NoticeChoice;
use onda_domain::models::ModelId;
use onda_domain::setup::{PermissionFix, SetupAction};
use onda_domain::types::{ActivationMode, HistoryRetention, OutputMode};
use uuid::Uuid;

use crate::OndaCore;

fn model(id: &str) -> Option<ModelId> {
    let parsed = ModelId::parse(id);
    if parsed.is_none() {
        tracing::warn!(id, "unknown model id from the shell");
    }
    parsed
}

fn language(code: &str) -> Option<LanguageSetting> {
    let parsed = LanguageSetting::parse(code);
    if parsed.is_none() {
        tracing::warn!(code, "unknown language code from the shell");
    }
    parsed
}

fn entry(id: &str) -> Option<Uuid> {
    let parsed = Uuid::parse_str(id).ok();
    if parsed.is_none() {
        tracing::warn!(id, "bad history entry id from the shell");
    }
    parsed
}

fn cleanup_provider(id: &str) -> Option<CleanupProvider> {
    let parsed = CleanupProvider::from_id(id);
    if parsed.is_none() {
        tracing::warn!(id, "unknown cleanup provider from the shell");
    }
    parsed
}

#[uniffi::export]
impl OndaCore {
    // --- windows (UI-2) ---

    /// A main window page appeared; it gets a fresh view model (WIN-7,
    /// WIN-15, HIS-16, CLN-25, REP-12).
    pub fn page_appeared(&self, page: MainWindowSection) {
        self.with_app(|app| app.page_appeared(page));
    }

    /// A main window page disappeared: pending edits flush, a cleanup test
    /// is cancelled, and a recorder capture ends.
    pub fn page_disappeared(&self, page: MainWindowSection) {
        self.with_app(|app| app.page_disappeared(page));
    }

    /// The main window appeared (UI-2).
    pub fn main_window_appeared(&self) {
        self.with_app(App::main_window_appeared);
    }

    /// The main window closed (UI-2, CLN-25, REP-12).
    pub fn main_window_closed(&self) {
        self.with_app(App::main_window_closed);
    }

    // --- status menu and app menu (UI-6..UI-11) ---

    /// "Fix Permissions…" (UI-8).
    pub fn fix_permissions(&self) {
        self.with_app(App::fix_permissions);
    }

    /// "Open Onda" (UI-11).
    pub fn open_app(&self) {
        self.with_app(App::open_app);
    }

    /// "Settings…" (UI-4, UI-11).
    pub fn open_settings(&self) {
        self.with_app(App::open_settings);
    }

    /// "Show All History…" (HIS-28).
    pub fn show_all_history(&self) {
        self.with_app(App::show_all_history);
    }

    /// "Other Language…" (LANG-9).
    pub fn choose_other_language(&self) {
        self.with_app(App::choose_other_language);
    }

    /// A Language submenu item, by code (LANG-10).
    pub fn select_language_from_menu(&self, code: String) {
        if let Some(language) = language(&code) {
            self.with_app(|app| app.select_language_from_menu(language));
        }
    }

    /// "Re-download Model" (DL-11).
    pub fn redownload_model(&self) {
        self.with_app(App::redownload_model);
    }

    /// "Check for Updates…" (UPD-1).
    pub fn check_for_updates(&self) {
        self.with_app(App::check_for_updates);
    }

    /// The updater's state changed; the core re-reads it.
    pub fn updater_changed(&self) {
        self.with_app(App::updater_changed);
    }

    // --- Getting Started and General ---

    /// A permission fix button (WIN-8, ONB-6).
    pub fn fix_permission(&self, fix: PermissionFix) {
        self.with_app(|app| app.fix_permission(fix));
    }

    /// A checklist button (WIN-5).
    pub fn setup_action(&self, action: SetupAction) {
        self.with_app(|app| app.setup_action(action));
    }

    /// The "Input" picker; empty for "System Default" (WIN-9).
    pub fn select_input_device(&self, uid: String) {
        self.with_app(|app| app.select_input_device(uid));
    }

    /// "After transcription" (WIN-10).
    pub fn set_output_mode(&self, mode: OutputMode) {
        self.with_app(|app| app.set_output_mode(mode));
    }

    /// "Pause media while recording" (WIN-11).
    pub fn set_pause_media(&self, on: bool) {
        self.with_app(|app| app.set_pause_media(on));
    }

    /// "Play sounds when dictation starts and stops" (WIN-12).
    pub fn set_sounds(&self, on: bool) {
        self.with_app(|app| app.set_sounds(on));
    }

    /// "Open Onda at login" (LOGIN-1..LOGIN-3).
    pub fn set_launch_at_login(&self, on: bool) {
        self.with_app(|app| app.set_launch_at_login(on));
    }

    /// "Open Login Items" (LOGIN-2).
    pub fn open_login_items(&self) {
        self.with_app(App::open_login_items);
    }

    /// "Check for updates automatically" (UPD-1).
    pub fn set_automatic_update_checks(&self, on: bool) {
        self.with_app(|app| app.set_automatic_update_checks(on));
    }

    // --- Hotkey ---

    /// The "Activation" picker (WIN-17).
    pub fn set_activation_mode(&self, mode: ActivationMode) {
        self.with_app(|app| app.set_activation_mode(mode));
    }

    /// "Reset to ⌥ Space" (WIN-16).
    pub fn reset_hotkey(&self) {
        self.with_app(App::reset_hotkey);
    }

    /// "Open Keyboard Settings" (HOT-35).
    pub fn open_keyboard_settings(&self) {
        self.with_app(App::open_keyboard_settings);
    }

    // --- Speech ---

    /// The Speech page "Language" picker (WIN-18).
    pub fn select_language(&self, code: String) {
        if let Some(language) = language(&code) {
            self.with_app(|app| app.select_language(language));
        }
    }

    /// "Select" or the compatibility note's "Use <Model>" (DL-7).
    pub fn select_model(&self, id: String) {
        if let Some(model) = model(&id) {
            self.with_app(|app| app.select_model(model));
        }
    }

    /// "Download & Use" (DL-8).
    pub fn download_and_use(&self, id: String) {
        if let Some(model) = model(&id) {
            self.with_app(|app| app.download_and_use(model));
        }
    }

    /// A download's "Cancel" (DL-3, ONB-8).
    pub fn cancel_download(&self, id: String) {
        if let Some(model) = model(&id) {
            self.with_app(|app| app.cancel_download(model));
        }
    }

    /// A model row's trash button (DL-9).
    pub fn request_delete_model(&self, id: String) {
        if let Some(model) = model(&id) {
            self.with_app(|app| app.request_delete_model(model));
        }
    }

    /// The Speech alert's confirm button (WIN-20).
    pub fn confirm_speech_alert(&self, id: u64) {
        self.with_app(|app| app.confirm_speech_alert(id));
    }

    /// The Speech alert's dismiss button (WIN-20).
    pub fn dismiss_speech_alert(&self, id: u64) {
        self.with_app(|app| app.dismiss_speech_alert(id));
    }

    // --- Post-Processing ---

    /// "Post-process transcripts" (CLN-26).
    pub fn set_cleanup_enabled(&self, on: bool) {
        self.with_app(|app| app.set_cleanup_enabled(on));
    }

    /// The "Model" row on Post-Processing.
    pub fn open_providers_dialog(&self) {
        self.with_app(App::open_providers_dialog);
    }

    /// "Done" in the model sheet, or the sheet going away.
    pub fn close_providers_dialog(&self) {
        self.with_app(App::close_providers_dialog);
    }

    /// A provider in the sheet's sidebar, by id.
    pub fn select_dialog_provider(&self, id: String) {
        if let Some(provider) = cleanup_provider(&id) {
            self.with_app(|app| app.select_dialog_provider(provider));
        }
    }

    /// A model row or the use-by-ID field in the model sheet: the provider,
    /// by id, and the model become the ones in use (CLN-26).
    pub fn select_cleanup_model(&self, provider: String, slug: String) {
        if let Some(provider) = cleanup_provider(&provider) {
            self.with_app(|app| app.select_cleanup_model(provider, slug));
        }
    }

    /// "Refresh Models" in the sheet.
    pub fn refresh_cleanup_models(&self) {
        self.with_app(App::refresh_cleanup_models);
    }

    /// The API key field changed (CLN-25).
    pub fn api_key_edited(&self, text: String) {
        self.with_app(|app| app.api_key_edited(text));
    }

    /// Return or end of editing in the API key field (CLN-25).
    pub fn api_key_commit(&self) {
        self.with_app(App::api_key_commit);
    }

    /// "Test with Sample Text" (CLN-27).
    pub fn test_cleanup(&self) {
        self.with_app(App::test_cleanup);
    }

    /// Dismisses "Test Failed".
    pub fn dismiss_cleanup_test_error(&self) {
        self.with_app(App::dismiss_cleanup_test_error);
    }

    // --- Replacements ---

    /// "Add Rule" (REP-10).
    pub fn add_replacement_rule(&self) {
        self.with_app(App::add_replacement_rule);
    }

    /// A rule's "Replace" text, raw (REP-9).
    pub fn set_replacement_triggers(&self, id: String, text: String) {
        self.with_app(|app| app.set_replacement_triggers(id, text));
    }

    /// A rule's "With" text (REP-9).
    pub fn set_replacement_text(&self, id: String, text: String) {
        self.with_app(|app| app.set_replacement_text(id, text));
    }

    /// "Delete Rule" (REP-9).
    pub fn delete_replacement_rule(&self, id: String) {
        self.with_app(|app| app.delete_replacement_rule(id));
    }

    // --- History ---

    /// "Save transcripts" or "Turn On History" (HIS-18, HIS-26).
    pub fn set_history_enabled(&self, on: bool) {
        self.with_app(|app| app.set_history_enabled(on));
    }

    /// "Keep for" (HIS-17).
    pub fn set_history_retention(&self, retention: HistoryRetention) {
        self.with_app(|app| app.set_history_retention(retention));
    }

    /// "Keep audio recordings" (HIS-18).
    pub fn set_history_keeps_audio(&self, on: bool) {
        self.with_app(|app| app.set_history_keeps_audio(on));
    }

    /// The search field (HIS-19).
    pub fn search_history(&self, query: String) {
        self.with_app(|app| app.search_history(query));
    }

    /// Copy a row's transcript (HIS-22, HIS-25).
    pub fn copy_history_entry(&self, id: String) {
        if let Some(id) = entry(&id) {
            self.with_app(|app| app.copy_history_entry(id));
        }
    }

    /// "Copy Original" (HIS-24).
    pub fn copy_history_original(&self, id: String) {
        if let Some(id) = entry(&id) {
            self.with_app(|app| app.copy_history_original(id));
        }
    }

    /// "Play Recording" (HIS-24).
    pub fn play_history_recording(&self, id: String) {
        if let Some(id) = entry(&id) {
            self.with_app(|app| app.play_history_recording(id));
        }
    }

    /// "Show Recording in Finder" (HIS-24).
    pub fn show_history_recording(&self, id: String) {
        if let Some(id) = entry(&id) {
            self.with_app(|app| app.show_history_recording(id));
        }
    }

    /// "Delete" or the Delete key (HIS-24, HIS-25).
    pub fn delete_history_entry(&self, id: String) {
        if let Some(id) = entry(&id) {
            self.with_app(|app| app.delete_history_entry(id));
        }
    }

    /// "Clear History…" (HIS-20).
    pub fn request_clear_history(&self) {
        self.with_app(App::request_clear_history);
    }

    /// The History confirmation's destructive button.
    pub fn confirm_history_dialog(&self) {
        self.with_app(App::confirm_history_dialog);
    }

    /// The History confirmation's "Cancel".
    pub fn cancel_history_dialog(&self) {
        self.with_app(App::cancel_history_dialog);
    }

    /// Dismisses "History Error" (HIS-27).
    pub fn dismiss_history_error(&self) {
        self.with_app(App::dismiss_history_error);
    }

    // --- onboarding ---

    /// "Run Setup Again" (WIN-4).
    pub fn run_setup_again(&self) {
        self.with_app(App::run_setup_again);
    }

    /// The primary button (ONB-1).
    pub fn onboarding_continue(&self) {
        self.with_app(App::onboarding_continue);
    }

    /// "Back" (ONB-1).
    pub fn onboarding_back(&self) {
        self.with_app(App::onboarding_back);
    }

    /// The user closed the onboarding window (ONB-11).
    pub fn onboarding_closed(&self) {
        self.with_app(App::onboarding_closed);
    }

    /// A language card, by code (ONB-4).
    pub fn onboarding_select_language(&self, code: String) {
        if let Some(language) = language(&code) {
            self.with_app(|app| app.onboarding_select_language(language));
        }
    }

    /// The language search field (ONB-4).
    pub fn onboarding_search_languages(&self, query: String) {
        self.with_app(|app| app.onboarding_search_languages(query));
    }

    /// "Show All" / "Popular Only" (ONB-4).
    pub fn onboarding_toggle_all_languages(&self) {
        self.with_app(App::onboarding_toggle_all_languages);
    }

    /// "Use the clipboard instead" (ONB-6).
    pub fn onboarding_use_clipboard(&self) {
        self.with_app(App::onboarding_use_clipboard);
    }

    /// A hotkey preset card (ONB-7).
    pub fn onboarding_select_hotkey(&self, preset: HotkeyPreset) {
        self.with_app(|app| app.onboarding_select_hotkey(preset));
    }

    /// "Use This Model" (ONB-8).
    pub fn onboarding_choose_model(&self, id: String) {
        if let Some(model) = model(&id) {
            self.with_app(|app| app.onboarding_choose_model(model));
        }
    }

    /// "Download" (ONB-8).
    pub fn onboarding_download_model(&self, id: String) {
        if let Some(model) = model(&id) {
            self.with_app(|app| app.onboarding_download_model(model));
        }
    }

    /// "Show other models" / "Hide other models" (ONB-8).
    pub fn onboarding_toggle_other_models(&self) {
        self.with_app(App::onboarding_toggle_other_models);
    }

    // --- the update notice ---

    /// A notice button; closing the window sends `NotNow` (DL-14).
    pub fn update_notice_choice(&self, choice: NoticeChoice) {
        self.with_app(|app| app.update_notice_choice(choice));
    }
}
