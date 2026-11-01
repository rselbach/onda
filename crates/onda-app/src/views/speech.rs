//! The Speech page (WIN-18..WIN-20, 04 "Compatibility note").

use onda_domain::languages::{AUTO_CODE, AUTO_NAME, Language, LanguageSetting};
use onda_domain::model_selection::has_delete_control;
use onda_domain::models::{Catalog, ModelId};
use onda_domain::non_dictation_ui::DownloadTracker;
use onda_domain::routing::{
    ModelAvailability, NoteActionKind, compatibility_note, percent, recommended_model, supports,
};

/// One "Language" picker entry (WIN-18).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageOption {
    /// Value for `select_language`.
    pub code: String,
    /// Display name.
    pub name: String,
    /// The "Automatic" entry.
    pub automatic: bool,
    /// A divider follows this entry.
    pub divider_after: bool,
}

/// The button under an "unsupported" compatibility note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteActionView {
    /// The recommended model it acts on.
    pub model_id: String,
    /// `Use` calls `select_model`, `DownloadAndUse` calls
    /// `download_and_use`, `Downloading` is disabled.
    pub kind: NoteActionKind,
    /// Title.
    pub title: String,
    /// Clickable.
    pub enabled: bool,
}

/// The note under the language picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilityView {
    /// Caption text.
    pub text: String,
    /// Orange warning.
    pub warning: bool,
    /// Fix button, for an unsupported pair with a recommendation.
    pub action: Option<NoteActionView>,
}

/// A model row's trailing control (WIN-19).
#[derive(Debug, Clone, PartialEq)]
pub enum ModelRowControl {
    /// Progress bar, percentage, and "Cancel" (`cancel_download`).
    Downloading {
        /// Fraction in `[0, 1]`.
        progress: f64,
        /// "42%".
        percent: String,
        /// "Cancel".
        cancel_title: String,
    },
    /// Accent check mark: downloaded and selected.
    Selected,
    /// "Select" (`select_model`): downloaded, not selected.
    Select {
        /// "Select".
        title: String,
    },
    /// "Download & Use" (`download_and_use`): not downloaded.
    DownloadAndUse {
        /// "Download & Use".
        title: String,
    },
}

/// One catalog model (WIN-19).
#[derive(Debug, Clone, PartialEq)]
pub struct ModelRow {
    /// Catalog id.
    pub id: String,
    /// Name, semibold when selected.
    pub name: String,
    /// The selected model.
    pub selected: bool,
    /// Size hint.
    pub size_hint: String,
    /// "Recommended" pill for the language's recommended model.
    pub recommended: Option<String>,
    /// Description.
    pub description: String,
    /// Orange "Doesn't support <Language>".
    pub unsupported: Option<String>,
    /// Trailing control.
    pub control: ModelRowControl,
    /// Trash button tooltip ("Delete model") when the model can be deleted
    /// (`request_delete_model`).
    pub delete_tooltip: Option<String>,
}

/// An alert on the Speech page; they queue and show one at a time (WIN-20).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeechAlert {
    /// Pass to `confirm_speech_alert` or `dismiss_speech_alert`.
    pub id: u64,
    /// Title.
    pub title: String,
    /// Message.
    pub message: String,
    /// Destructive confirm button ("Delete"), for a confirmation.
    pub confirm: Option<String>,
    /// "OK", or "Cancel" next to a confirm button.
    pub dismiss: String,
}

/// The Speech page view model.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeechPage {
    /// "Language".
    pub language_label: String,
    /// "Automatic", then every language; entries mark the divider.
    pub languages: Vec<LanguageOption>,
    /// The selected language code.
    pub selected_language: String,
    /// The compatibility note.
    pub compatibility: Option<CompatibilityView>,
    /// One row per catalog model.
    pub models: Vec<ModelRow>,
    /// The alert to show now, if any.
    pub alert: Option<SpeechAlert>,
}

/// What the Speech page is built from.
#[derive(Debug, Clone, Copy)]
pub struct SpeechInputs<'a> {
    /// The catalog.
    pub catalog: &'a Catalog,
    /// The selected language.
    pub language: LanguageSetting,
    /// The selected model.
    pub selected_model: ModelId,
    /// The OS language code.
    pub os_language: Option<&'a str>,
    /// Downloaded models, built-in engines included.
    pub downloaded: &'a [ModelId],
    /// Running downloads.
    pub downloads: &'a DownloadTracker,
    /// The head of the alert queue.
    pub alert: Option<&'a SpeechAlert>,
}

/// Builds the Speech page.
pub fn speech_page(inputs: SpeechInputs<'_>) -> SpeechPage {
    let availability = |model: ModelId| availability(&inputs, model);
    let recommended = recommended_model(inputs.catalog, inputs.language);
    let mut languages = vec![LanguageOption {
        code: AUTO_CODE.to_owned(),
        name: AUTO_NAME.to_owned(),
        automatic: true,
        divider_after: true,
    }];
    languages.extend(Language::all().map(|language| LanguageOption {
        code: language.code().to_owned(),
        name: language.name().to_owned(),
        automatic: false,
        divider_after: false,
    }));
    let compatibility = compatibility_note(
        inputs.catalog,
        inputs.language,
        inputs.selected_model,
        inputs.os_language,
        availability,
    )
    .map(|note| CompatibilityView {
        text: note.text,
        warning: note.is_warning,
        action: note.action.map(|action| NoteActionView {
            model_id: action.model.as_str().to_owned(),
            kind: action.kind,
            title: action.label,
            enabled: action.enabled,
        }),
    });
    let models = inputs
        .catalog
        .models()
        .into_iter()
        .map(|info| {
            let selected = info.id == inputs.selected_model;
            let control = match availability(info.id) {
                ModelAvailability::Downloading { progress } => ModelRowControl::Downloading {
                    progress,
                    percent: format!("{}%", percent(progress)),
                    cancel_title: "Cancel".to_owned(),
                },
                ModelAvailability::Downloaded if selected => ModelRowControl::Selected,
                ModelAvailability::Downloaded => ModelRowControl::Select {
                    title: "Select".to_owned(),
                },
                ModelAvailability::NotDownloaded => ModelRowControl::DownloadAndUse {
                    title: "Download & Use".to_owned(),
                },
            };
            let deletable =
                inputs.downloaded.contains(&info.id) && has_delete_control(inputs.catalog, info.id);
            ModelRow {
                id: info.id.as_str().to_owned(),
                name: info.name.to_owned(),
                selected,
                size_hint: info.size_hint.to_owned(),
                recommended: (recommended == Some(info.id)).then(|| "Recommended".to_owned()),
                description: info.description.to_owned(),
                unsupported: (!supports(inputs.catalog, inputs.language, info.id))
                    .then(|| format!("Doesn't support {}", inputs.language.display_name())),
                control,
                delete_tooltip: deletable.then(|| "Delete model".to_owned()),
            }
        })
        .collect();
    SpeechPage {
        language_label: "Language".to_owned(),
        languages,
        selected_language: inputs.language.code().to_owned(),
        compatibility,
        models,
        alert: inputs.alert.cloned(),
    }
}

fn availability(inputs: &SpeechInputs<'_>, model: ModelId) -> ModelAvailability {
    if let Some(download) = inputs.downloads.get(model.as_str()) {
        return ModelAvailability::Downloading {
            progress: download.progress,
        };
    }
    if inputs.downloaded.contains(&model) {
        return ModelAvailability::Downloaded;
    }
    ModelAvailability::NotDownloaded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win_18_automatic_comes_first_with_the_only_divider() {
        let catalog = Catalog::new(None);
        let downloads = DownloadTracker::new();
        let page = speech_page(SpeechInputs {
            catalog: &catalog,
            language: LanguageSetting::Auto,
            selected_model: ModelId::ParakeetTdtV2,
            os_language: None,
            downloaded: &[],
            downloads: &downloads,
            alert: None,
        });
        let marked: Vec<(&str, bool, bool)> = page
            .languages
            .iter()
            .filter(|option| option.automatic || option.divider_after)
            .map(|option| (option.code.as_str(), option.automatic, option.divider_after))
            .collect();
        assert_eq!(marked, vec![(AUTO_CODE, true, true)]);
        assert_eq!(page.languages[0].name, AUTO_NAME);
        assert_eq!(page.languages.len(), Language::all().count() + 1);
    }
}
