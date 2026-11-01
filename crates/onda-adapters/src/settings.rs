//! Settings through `onda-settings`.

use std::sync::Arc;

use onda_app::seams::{SettingChange, Settings as SessionSettings, SettingsSource};
use onda_domain::hotkey::Shortcut;
use onda_domain::language_menu::recent_languages;
use onda_domain::languages::{LanguageSetting, default_language};
use onda_domain::messages::UserMessage;
use onda_domain::migration::{REMOVED_MODEL_IDS, SelectionResolution, resolve_selection};
use onda_domain::models::{Catalog, ModelId};
use onda_domain::onboarding::OnboardingFlags;
use onda_settings::Settings;

/// [`SettingsSource`] over the settings file. Computed defaults follow the
/// OS language and the catalog (SET-2a).
pub struct SettingsAdapter {
    settings: Arc<Settings>,
    catalog: Catalog,
    os_language: Option<String>,
}

impl SettingsAdapter {
    /// Reads `settings`; `os_language` is the OS locale's language code
    /// (LANG-2) and `catalog` reflects whether the shell has a system engine.
    pub fn new(settings: Arc<Settings>, catalog: Catalog, os_language: Option<String>) -> Self {
        Self {
            settings,
            catalog,
            os_language,
        }
    }
}

impl SettingsSource for SettingsAdapter {
    fn snapshot(&self) -> SessionSettings {
        let os_language = self.os_language.as_deref();
        let settings = &self.settings;
        let language = settings.selected_language(os_language);
        let stored_model = settings.selected_model(&self.catalog, os_language);
        let selected_model = resolved_model(
            &self.catalog,
            &stored_model,
            REMOVED_MODEL_IDS,
            default_language(os_language),
        );
        SessionSettings {
            activation_mode: settings.activation_mode(),
            output_mode: settings.output_mode(),
            language,
            recent_languages: recent_languages(language, &settings.recent_languages()),
            selected_model,
            cleanup_enabled: settings.post_processing_enabled(),
            cleanup_model: settings.openrouter_model(),
            replacement_rules: settings.replacement_rules(),
            history_enabled: settings.history_enabled(),
            keep_audio: settings.history_keeps_audio(),
            pause_media: settings.pause_media_during_recording(),
            sounds: settings.sound_feedback_enabled(),
            hotkey: settings.hotkey(),
            first_run_pending: settings.first_run_pending(),
            onboarding: OnboardingFlags {
                onboarding_completed: settings.onboarding_completed(),
                did_show_first_run_setup: settings.did_show_first_run_setup(),
            },
            input_device_uid: settings.input_device_uid(),
            history_retention: settings.history_retention(),
            stored_model,
            stored_recent_languages: settings.recent_languages(),
            last_notice_key: settings.last_model_migration_notice_key(),
            declined_notice_key: settings.declined_model_migration_notice_key(),
            os_language: self.os_language.clone(),
        }
    }

    fn set_selected_model(&self, model: ModelId) {
        if let Err(error) = self.settings.set_selected_model(model) {
            tracing::error!(%error, model = model.as_str(), "could not save the selected model");
        }
    }

    fn set_hotkey(&self, shortcut: &Shortcut) -> Result<(), String> {
        self.settings
            .set_hotkey(shortcut)
            .map_err(|error| error.to_string())
    }

    fn apply(&self, change: SettingChange) -> Result<(), String> {
        let settings = &self.settings;
        let result = match change {
            SettingChange::SelectedModel(model) => settings.set_selected_model(model),
            SettingChange::Language(language) => settings.set_selected_language(language),
            SettingChange::RecentLanguages(languages) => settings.set_recent_languages(&languages),
            SettingChange::InputDevice(uid) => settings.set_input_device_uid(&uid),
            SettingChange::OutputMode(mode) => settings.set_output_mode(mode),
            SettingChange::ActivationMode(mode) => settings.set_activation_mode(mode),
            SettingChange::PauseMedia(on) => settings.set_pause_media_during_recording(on),
            SettingChange::Sounds(on) => settings.set_sound_feedback_enabled(on),
            SettingChange::HistoryEnabled(on) => settings.set_history_enabled(on),
            SettingChange::HistoryKeepsAudio(on) => settings.set_history_keeps_audio(on),
            SettingChange::HistoryRetention(retention) => settings.set_history_retention(retention),
            SettingChange::ReplacementRules(rules) => settings.set_replacement_rules(&rules),
            SettingChange::CleanupEnabled(on) => settings.set_post_processing_enabled(on),
            SettingChange::CleanupModel(model) => settings.set_openrouter_model(model),
            SettingChange::Onboarding(flags) => settings
                .set_onboarding_flags(flags.onboarding_completed, flags.did_show_first_run_setup),
            SettingChange::LastNoticeKey(key) => settings.set_last_model_migration_notice_key(&key),
            SettingChange::DeclinedNoticeKey(key) => {
                settings.set_declined_model_migration_notice_key(&key)
            }
        };
        result.map_err(|error| error.to_string())
    }

    fn reset_invalid_language(&self) -> Option<UserMessage> {
        match self.settings.reset_invalid_language() {
            Ok(warning) => warning,
            Err(error) => {
                tracing::error!(%error, "could not reset the language setting");
                None
            }
        }
    }
}

// ASR-4, DL-12: an unknown or removed id resolves to its mapped
// replacement in `removed`, else the recommended model, through the same
// resolution the launch flow switches to or offers
fn resolved_model(
    catalog: &Catalog,
    stored: &str,
    removed: &[(&str, ModelId)],
    default_language: LanguageSetting,
) -> ModelId {
    // installed models only decide whether to ask first, not the model
    match resolve_selection(catalog, stored, removed, default_language, &[]) {
        SelectionResolution::Current(model) => model,
        SelectionResolution::SwitchSilently(migration)
        | SelectionResolution::NeedsDownload(migration) => migration.to,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onda_domain::routing::default_model;

    const TEST_REMOVED: &[(&str, ModelId)] = &[
        ("parakeet-tdt-v3", ModelId::ParakeetUltra),
        ("chang-speech", ModelId::SystemSpeech),
    ];

    #[test]
    fn snapshot_model_follows_the_removed_id_map_dl_12() {
        let catalog = Catalog::new(None);
        let english = default_language(Some("en"));
        struct Case {
            name: &'static str,
            stored: &'static str,
            want: ModelId,
        }
        let cases = [
            Case {
                name: "a current id is kept",
                stored: "parakeet-unified",
                want: ModelId::ParakeetUnified,
            },
            Case {
                name: "a removed id maps to its replacement",
                stored: "parakeet-tdt-v3",
                want: ModelId::ParakeetUltra,
            },
            Case {
                name: "a replacement missing from this catalog falls back",
                stored: "chang-speech",
                want: default_model(&catalog, english),
            },
            Case {
                name: "an unknown id falls back (ASR-4)",
                stored: "greendale-asr",
                want: default_model(&catalog, english),
            },
        ];
        for tc in cases {
            assert_eq!(
                resolved_model(&catalog, tc.stored, TEST_REMOVED, english),
                tc.want,
                "{}",
                tc.name
            );
        }
    }
}
