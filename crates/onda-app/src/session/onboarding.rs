//! The onboarding wizard inside the session actor (ONB-1..ONB-13, WIN-4):
//! opening it, its 1 s refresh, every user action, finishing, and closing
//! early.

use std::time::Duration;

use onda_domain::hotkey::HotkeyPreset;
use onda_domain::languages::LanguageSetting;
use onda_domain::models::ModelId;
use onda_domain::onboarding::{
    ModelChoice, ModelStatus, Onboarding, OnboardingDownloadUpdate, OnboardingEffect,
    OnboardingEnv, OnboardingOrigin, OnboardingStep, POLL_INTERVAL, SelectedModel,
};
use onda_domain::privacy_naming::PrivacyNaming;
use onda_domain::routing::{recommended_model, routes};
use onda_domain::types::MicrophonePermission;
use tokio::sync::mpsc;

use super::{Actor, Msg, Task, UiMsg};
use crate::effects::Effect;
use crate::events::Event;
use crate::seams::SettingChange;
use crate::views::{OnboardingInputs, OnboardingView, onboarding_view};

/// The open onboarding window's state.
pub(super) struct OnboardingState {
    pub(super) wizard: Onboarding,
    pub(super) query: String,
    pub(super) show_all_languages: bool,
    pub(super) show_other_models: bool,
    _poll: Task,
}

// owned values an `OnboardingEnv` borrows
struct EnvParts {
    microphone: MicrophonePermission,
    accessibility: bool,
    hotkey_display: String,
    hotkey_needs_accessibility: bool,
    privacy_naming: PrivacyNaming,
    selected: Option<(String, ModelStatus)>,
    recommended: ModelStatus,
}

impl EnvParts {
    fn env(&self) -> OnboardingEnv<'_> {
        OnboardingEnv {
            microphone: self.microphone,
            accessibility_granted: self.accessibility,
            hotkey_display: &self.hotkey_display,
            hotkey_needs_accessibility: self.hotkey_needs_accessibility,
            privacy_naming: self.privacy_naming,
            selected_model: self.selected.as_ref().map(|(name, status)| SelectedModel {
                name,
                status: *status,
            }),
            recommended_model: self.recommended,
        }
    }
}

fn ticker(every: Duration, messages: mpsc::UnboundedSender<Msg>) -> Task {
    Task(
        tokio::spawn(async move {
            let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
            loop {
                ticks.tick().await;
                if messages.send(Msg::Ui(UiMsg::OnboardingPoll)).is_err() {
                    return;
                }
            }
        })
        .abort_handle(),
    )
}

impl Actor {
    /// Shows onboarding, automatically (first run, setup guidance) or for
    /// "Run Setup Again", which first clears `onboardingCompleted` (WIN-4).
    pub(super) fn open_onboarding(&mut self, rerun: bool) {
        if self.ui.onboarding.is_some() {
            self.events.emit(Event::OpenOnboarding);
            return;
        }
        self.ui.launch_onboarding = None;
        self.refresh_settings();
        let flags = self.settings.onboarding;
        let origin = match rerun {
            true => OnboardingOrigin::RunSetupAgain,
            false => OnboardingOrigin::Automatic,
        };
        if rerun {
            self.apply_setting(SettingChange::Onboarding(flags.rerun()));
        }
        let wizard = Onboarding::open(
            origin,
            flags,
            Some(self.settings.language.code().to_owned()),
            self.ports.platform,
        );
        self.ui.onboarding = Some(OnboardingState {
            wizard,
            query: String::new(),
            show_all_languages: false,
            show_other_models: false,
            _poll: ticker(POLL_INTERVAL, self.messages.clone()),
        });
        self.onboarding_refresh();
        self.events.emit(Event::OpenOnboarding);
    }

    // ONB-2: every second and on every navigation
    pub(super) fn onboarding_refresh(&mut self) {
        self.refresh_settings();
        self.refresh_permissions();
        self.refresh_downloaded();
        self.read_globe_key();
    }

    pub(super) fn onboarding_poll(&mut self) {
        if self.ui.onboarding.is_some() {
            self.onboarding_refresh();
        }
    }

    // the language the wizard shows, else the stored one
    fn onboarding_language(&self) -> LanguageSetting {
        self.ui
            .onboarding
            .as_ref()
            .and_then(|state| state.wizard.language())
            .and_then(LanguageSetting::parse)
            .unwrap_or(self.settings.language)
    }

    fn model_status(&self, model: ModelId) -> ModelStatus {
        ModelStatus {
            downloaded: self.downloaded.contains(&model),
            download_progress: self.downloads.get(model.as_str()).map(|d| d.progress),
        }
    }

    fn env_parts(&self) -> EnvParts {
        let selected = self.settings.selected_model;
        let recommended = recommended_model(&self.catalog, self.onboarding_language());
        EnvParts {
            microphone: self.permissions.microphone,
            accessibility: self.permissions.accessibility,
            hotkey_display: self.settings.hotkey.display(self.ports.platform.key_names),
            hotkey_needs_accessibility: self
                .settings
                .hotkey
                .needs_accessibility(self.ports.platform),
            privacy_naming: self.ports.platform.privacy_naming(),
            selected: self
                .catalog
                .model(selected)
                .map(|info| (info.name.to_owned(), self.model_status(selected))),
            // no model transcribes the language: nothing to download
            recommended: recommended.map_or(
                ModelStatus {
                    downloaded: true,
                    download_progress: None,
                },
                |model| self.model_status(model),
            ),
        }
    }

    pub(super) fn onboarding_continue(&mut self) {
        self.refresh_settings();
        let parts = self.env_parts();
        let Some(state) = &mut self.ui.onboarding else {
            return;
        };
        let leaving = state.wizard.step();
        let effect = state.wizard.advance(&parts.env());
        let left_hotkey = leaving == OnboardingStep::Hotkey && state.wizard.step() != leaving;
        if left_hotkey {
            self.end_recorder_capture();
        }
        match effect {
            Some(OnboardingEffect::StartRecommendedDownload) => {
                if let Some(model) = recommended_model(&self.catalog, self.onboarding_language()) {
                    self.begin_download(model);
                }
            }
            Some(OnboardingEffect::ReachedReady) => {
                // load the chosen model so the try-it box works (ONB-9)
                if !self.load_in_flight() {
                    self.override_model = None;
                    self.start_load(false);
                }
            }
            Some(OnboardingEffect::Finish) => {
                self.finish_onboarding();
                return;
            }
            None => {}
        }
        self.onboarding_refresh();
    }

    // ONB-10, ONB-12
    fn finish_onboarding(&mut self) {
        self.end_recorder_capture();
        self.ui.onboarding = None;
        self.apply_setting(SettingChange::Onboarding(
            self.settings.onboarding.finished(),
        ));
        self.events.emit(Event::CloseOnboarding);
        if !self.load_in_flight() {
            self.override_model = None;
            self.start_load(false);
        }
        self.effects.send(Effect::RequestNotifications);
    }

    pub(super) fn onboarding_back(&mut self) {
        let Some(state) = &mut self.ui.onboarding else {
            return;
        };
        let leaving = state.wizard.step();
        if state.wizard.go_back() && leaving == OnboardingStep::Hotkey {
            self.end_recorder_capture();
        }
        self.onboarding_refresh();
    }

    // ONB-11, ONB-12: closed early, finished or not
    pub(super) fn onboarding_closed(&mut self) {
        let Some(state) = self.ui.onboarding.take() else {
            return;
        };
        self.end_recorder_capture();
        self.refresh_settings();
        let selected = self.settings.selected_model;
        let outcome = state.wizard.close(
            self.settings.onboarding,
            self.downloaded.contains(&selected),
            self.service.loaded_model() == Some(selected),
            self.load_in_flight(),
        );
        if outcome.flags != self.settings.onboarding {
            self.apply_setting(SettingChange::Onboarding(outcome.flags));
        }
        // never start a download the user did not agree to
        if outcome.load_model && !self.load_declined_fallback() {
            self.start_load(false);
        }
        if outcome.request_notifications {
            self.effects.send(Effect::RequestNotifications);
        }
    }

    // ONB-4: picking saves the language and its recommended model;
    // re-picking the current one changes nothing
    pub(super) fn onboarding_select_language(&mut self, language: LanguageSetting) {
        let recommended =
            recommended_model(&self.catalog, language).unwrap_or(self.settings.selected_model);
        let Some(state) = &mut self.ui.onboarding else {
            return;
        };
        let Some(pick) = state
            .wizard
            .select_language(language.code(), recommended.as_str())
        else {
            return;
        };
        self.apply_setting(SettingChange::Language(language));
        if let Some(model) = ModelId::parse(&pick.model_id) {
            self.apply_setting(SettingChange::SelectedModel(model));
        }
    }

    pub(super) fn onboarding_use_clipboard(&mut self) {
        let Some(state) = &mut self.ui.onboarding else {
            return;
        };
        let mode = state.wizard.use_clipboard_instead();
        self.apply_setting(SettingChange::OutputMode(mode));
    }

    // ONB-7: saves and re-registers at once; presets needing missing
    // Typing Access are disabled
    pub(super) fn onboarding_select_hotkey(&mut self, preset: HotkeyPreset) {
        let shortcut = preset.shortcut();
        if shortcut.needs_accessibility(self.ports.platform) && !self.permissions.accessibility {
            return;
        }
        self.set_hotkey(&shortcut);
        self.read_globe_key();
    }

    // ONB-8 "Use This Model"
    pub(super) fn onboarding_choose_model(&mut self, model: ModelId) {
        if self.downloaded.contains(&model) {
            self.apply_setting(SettingChange::SelectedModel(model));
        }
    }

    // ONB-8 "Download": selects and starts the download
    pub(super) fn onboarding_download_model(&mut self, model: ModelId) {
        self.apply_setting(SettingChange::SelectedModel(model));
        if let Some(state) = &mut self.ui.onboarding {
            state
                .wizard
                .download_update(OnboardingDownloadUpdate::Active);
        }
        self.begin_download(model);
    }

    pub(super) fn onboarding_view(&self) -> Option<OnboardingView> {
        let state = self.ui.onboarding.as_ref()?;
        let parts = self.env_parts();
        let env = parts.env();
        let language = self.onboarding_language();
        let models: Vec<ModelChoice> = routes(&self.catalog, language)
            .into_iter()
            .filter_map(|route| {
                let info = self.catalog.model(route.model)?;
                Some(ModelChoice {
                    id: info.id.as_str().to_owned(),
                    name: info.name.to_owned(),
                    description: info.description.to_owned(),
                    size_hint: info.size_hint.to_owned(),
                    recommended: route.is_recommended,
                    built_in: info.is_built_in(),
                    status: self.model_status(route.model),
                })
            })
            .collect();
        Some(onboarding_view(OnboardingInputs {
            wizard: &state.wizard,
            env: &env,
            platform: self.ports.platform,
            language_query: &state.query,
            show_all_languages: state.show_all_languages,
            show_other_models: state.show_other_models,
            activation: self.settings.activation_mode,
            shortcut: &self.settings.hotkey,
            globe: self.hotkeys.globe,
            models: &models,
            selected_model: self.settings.selected_model.as_str(),
            history_enabled: self.settings.history_enabled,
            retention: self.settings.history_retention,
            output_mode: self.settings.output_mode,
        }))
    }
}
