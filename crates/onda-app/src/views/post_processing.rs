//! The Post-Processing page (CLN-24..CLN-27) and its model sheet.

use std::fmt;

use onda_domain::cleanup::{CleanupModel, CleanupProvider, TEST_FAILED_TITLE};

use super::{AlertView, ButtonView, ToggleView};

/// The caption under the model picker (CLN-26).
fn caption(provider: CleanupProvider) -> String {
    match provider {
        CleanupProvider::OpenRouter => "Cloud post-processing sends your transcript to OpenRouter \
                                        and the selected model provider for processing. Onda asks \
                                        OpenRouter to use only providers that don't retain your \
                                        transcript. The API key is stored in your system keychain, \
                                        not in preferences. Falls back to the raw transcript if the \
                                        request fails."
            .to_owned(),
    }
}

/// The API key field's placeholder (CLN-26).
pub fn key_placeholder(provider: CleanupProvider) -> String {
    format!("{} API key", provider.display_name())
}

/// The API key field (CLN-25). `Debug` never prints the key (CLN-8).
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKeyFieldView {
    /// "OpenRouter API key".
    pub placeholder: String,
    /// The field text as the core knows it. Replace the field's contents
    /// only when `revision` changes (the stored key finished loading), so
    /// typing is never overwritten.
    pub text: String,
    /// Bumped when the core replaces `text`.
    pub revision: u64,
    /// The stored key is loading.
    pub loading: bool,
    /// Inline Keychain error in red, e.g. "Keychain read failed (OSStatus
    /// -25293)." (CLN-24).
    pub error: Option<String>,
}

impl fmt::Debug for ApiKeyFieldView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiKeyFieldView")
            .field("text", &"<redacted>")
            .field("revision", &self.revision)
            .field("loading", &self.loading)
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

/// One provider entry in the sheet's sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupProviderOption {
    /// Value for `select_dialog_provider`.
    pub id: String,
    /// Display name.
    pub name: String,
}

/// One model row in the sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupModelOption {
    /// Value for `select_cleanup_model`.
    pub slug: String,
    /// Display name.
    pub name: String,
}

/// A titled run of model rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupModelGroup {
    /// "Recommended", "Custom", or "All OpenRouter Models".
    pub title: String,
    /// The rows, in display order.
    pub models: Vec<CleanupModelOption>,
}

/// The model sheet: pick a provider, set its key, and click the model to
/// use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvidersDialog {
    /// "Choose Model".
    pub title: String,
    /// The sidebar, in order.
    pub providers: Vec<CleanupProviderOption>,
    /// The provider whose pane shows.
    pub selected_provider: String,
    /// The provider's API key field.
    pub key: ApiKeyFieldView,
    /// "Models".
    pub models_label: String,
    /// The provider's models: curated, then the model in use when it is a
    /// typed ID, then the rest of the provider's list by name. Empty groups
    /// are left out.
    pub model_groups: Vec<CleanupModelGroup>,
    /// The model in use, when it belongs to this provider; clicking a row
    /// sends `select_cleanup_model`.
    pub selected_model: Option<String>,
    /// Placeholder for the one field above the list. Its text filters rows
    /// by name or slug in the shell; text that matches no slug offers a
    /// row that sends it to `select_cleanup_model`.
    pub search_placeholder: String,
    /// Caption on that row, under the typed ID.
    pub use_id_caption: String,
    /// "Refresh Models"; absent when the provider cannot list its models,
    /// disabled while a listing runs.
    pub refresh_models: Option<ButtonView>,
    /// A listing is running: show a spinner next to the refresh button and,
    /// with `loading_title`, a row above the list.
    pub models_loading: bool,
    /// "Loading models…".
    pub loading_title: String,
    /// Why the last listing failed, in red under the list.
    pub models_error: Option<String>,
    /// "Done"; sends `close_providers_dialog`.
    pub done_title: String,
}

/// A finished test (CLN-27), selectable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupTestResult {
    /// "Said: <sample>".
    pub said: String,
    /// "Result: <output>".
    pub result: String,
}

/// What shows while post-processing is on (CLN-26).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostProcessingDetails {
    /// "Model"; clicking the row sends `open_providers_dialog`.
    pub model_label: String,
    /// The provider in use, e.g. "OpenRouter".
    pub provider_name: String,
    /// The model in use (CLN-21), by name.
    pub model_name: String,
    /// The model sheet, while open.
    pub dialog: Option<ProvidersDialog>,
    /// The privacy caption.
    pub caption: String,
    /// "Test with Sample Text"; disabled while a test runs.
    pub test_button: ButtonView,
    /// Show a small spinner next to the button.
    pub test_running: bool,
    /// The last finished test.
    pub test_result: Option<CleanupTestResult>,
    /// "Test Failed" alert; dismiss with `dismiss_cleanup_test_error`.
    pub test_error: Option<AlertView>,
}

/// The Post-Processing page view model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostProcessingPage {
    /// "Post-process transcripts".
    pub enabled: ToggleView,
    /// Shown only while the toggle is on.
    pub details: Option<PostProcessingDetails>,
}

/// What the page is built from.
#[derive(Debug, Clone, Copy)]
pub struct PostProcessingInputs<'a> {
    /// `postProcessingEnabled`.
    pub enabled: bool,
    /// `cleanupProvider`.
    pub provider: CleanupProvider,
    /// The model actually used.
    pub model: &'a CleanupModel,
    /// The models the provider listed, for the model's name; empty until a
    /// listing finishes.
    pub listed_models: &'a [CleanupModel],
    /// The model sheet, while open.
    pub dialog: Option<ProvidersDialogInputs<'a>>,
    /// A test is running.
    pub test_running: bool,
    /// The last finished test.
    pub test_result: Option<&'a CleanupTestResult>,
    /// The last test error message.
    pub test_error: Option<&'a str>,
}

/// What the model sheet is built from.
#[derive(Debug, Clone, Copy)]
pub struct ProvidersDialogInputs<'a> {
    /// The provider whose pane shows.
    pub provider: CleanupProvider,
    /// The model in use, when it belongs to this provider.
    pub selected: Option<&'a CleanupModel>,
    /// The models it listed; empty until a listing finishes.
    pub listed_models: &'a [CleanupModel],
    /// A listing is running.
    pub models_loading: bool,
    /// The last listing error.
    pub models_error: Option<&'a str>,
    /// The key field.
    pub key: &'a ApiKeyFieldView,
}

fn option(model: &CleanupModel) -> CleanupModelOption {
    CleanupModelOption {
        slug: model.slug.clone().into_owned(),
        name: model.display_name.clone().into_owned(),
    }
}

// a typed ID takes the provider's name for it once listed
fn named(model: &CleanupModel, listed: &[CleanupModel]) -> CleanupModelOption {
    listed
        .iter()
        .find(|listed| listed.slug == model.slug)
        .map_or_else(|| option(model), option)
}

fn model_groups(inputs: &ProvidersDialogInputs<'_>) -> Vec<CleanupModelGroup> {
    let provider = inputs.provider;
    let listed = inputs.listed_models;
    // a typed ID the provider does not list still shows, so it stays visible
    // as the selection
    let custom = inputs
        .selected
        .filter(|model| {
            !provider.is_curated(&model.slug) && !listed.iter().any(|m| m.slug == model.slug)
        })
        .map(option)
        .into_iter()
        .collect();
    let rest = listed
        .iter()
        .filter(|model| !provider.is_curated(&model.slug))
        .map(option)
        .collect();
    let groups = [
        (
            "Recommended".to_owned(),
            provider.curated_models().iter().map(option).collect(),
        ),
        ("Custom".to_owned(), custom),
        (format!("All {} Models", provider.display_name()), rest),
    ];
    groups
        .into_iter()
        .filter(|(_, models): &(String, Vec<CleanupModelOption>)| !models.is_empty())
        .map(|(title, models)| CleanupModelGroup { title, models })
        .collect()
}

fn providers_dialog(inputs: &ProvidersDialogInputs<'_>) -> ProvidersDialog {
    let provider = inputs.provider;
    ProvidersDialog {
        title: "Choose Model".to_owned(),
        providers: CleanupProvider::ALL
            .iter()
            .map(|provider| CleanupProviderOption {
                id: provider.id().to_owned(),
                name: provider.display_name().to_owned(),
            })
            .collect(),
        selected_provider: provider.id().to_owned(),
        key: inputs.key.clone(),
        models_label: "Models".to_owned(),
        model_groups: model_groups(inputs),
        selected_model: inputs.selected.map(|model| model.slug.clone().into_owned()),
        search_placeholder: "Search or enter a model ID".to_owned(),
        use_id_caption: "Use as model ID".to_owned(),
        refresh_models: provider
            .models_endpoint()
            .map(|_| ButtonView::new("Refresh Models", !inputs.models_loading)),
        models_loading: inputs.models_loading,
        loading_title: "Loading models…".to_owned(),
        models_error: inputs
            .models_error
            .map(|error| format!("Couldn't load {} models: {error}", provider.display_name())),
        done_title: "Done".to_owned(),
    }
}

/// Builds the page.
pub fn post_processing_page(inputs: PostProcessingInputs<'_>) -> PostProcessingPage {
    let provider = inputs.provider;
    PostProcessingPage {
        enabled: ToggleView::new("Post-process transcripts", inputs.enabled, true),
        details: inputs.enabled.then(|| PostProcessingDetails {
            model_label: "Model".to_owned(),
            provider_name: provider.display_name().to_owned(),
            model_name: named(inputs.model, inputs.listed_models).name,
            dialog: inputs.dialog.as_ref().map(providers_dialog),
            caption: caption(provider),
            test_button: ButtonView::new("Test with Sample Text", !inputs.test_running),
            test_running: inputs.test_running,
            test_result: inputs.test_result.cloned(),
            test_error: inputs.test_error.map(|message| AlertView {
                title: TEST_FAILED_TITLE.to_owned(),
                message: message.to_owned(),
            }),
        }),
    }
}

#[cfg(test)]
mod tests {
    use onda_domain::cleanup::CLEANUP_MODELS;

    use super::*;

    fn key() -> ApiKeyFieldView {
        ApiKeyFieldView {
            placeholder: "OpenRouter API key".to_owned(),
            text: String::new(),
            revision: 0,
            loading: false,
            error: None,
        }
    }

    fn inputs<'a>(model: &'a CleanupModel, listed: &'a [CleanupModel]) -> PostProcessingInputs<'a> {
        PostProcessingInputs {
            enabled: true,
            provider: CleanupProvider::OpenRouter,
            model,
            listed_models: listed,
            dialog: None,
            test_running: false,
            test_result: None,
            test_error: None,
        }
    }

    fn dialog_inputs<'a>(
        selected: Option<&'a CleanupModel>,
        listed: &'a [CleanupModel],
        key: &'a ApiKeyFieldView,
    ) -> ProvidersDialogInputs<'a> {
        ProvidersDialogInputs {
            provider: CleanupProvider::OpenRouter,
            selected,
            listed_models: listed,
            models_loading: false,
            models_error: None,
            key,
        }
    }

    fn titles_and_slugs(dialog: &ProvidersDialog) -> Vec<(&str, Vec<&str>)> {
        dialog
            .model_groups
            .iter()
            .map(|group| {
                (
                    group.title.as_str(),
                    group.models.iter().map(|m| m.slug.as_str()).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn cln_26_copy_and_what_shows_when_on() {
        let off = post_processing_page(PostProcessingInputs {
            enabled: false,
            ..inputs(&CLEANUP_MODELS[0], &[])
        });
        assert_eq!(off.enabled.label, "Post-process transcripts");
        assert!(off.details.is_none(), "nothing else shows while off");

        let details = post_processing_page(inputs(&CLEANUP_MODELS[0], &[]))
            .details
            .expect("details when on");
        assert_eq!(details.model_label, "Model");
        assert_eq!(details.provider_name, "OpenRouter");
        assert_eq!(details.model_name, "GPT Luna (latest)");
        assert_eq!(details.dialog, None, "the sheet starts closed");
        assert_eq!(details.test_button.title, "Test with Sample Text");
        assert_eq!(
            details.caption,
            "Cloud post-processing sends your transcript to OpenRouter and the selected model \
             provider for processing. Onda asks OpenRouter to use only providers that don't \
             retain your transcript. The API key is stored in your system keychain, not in \
             preferences. Falls back to the raw transcript if the request fails."
        );
        assert_eq!(
            key_placeholder(CleanupProvider::OpenRouter),
            "OpenRouter API key"
        );
    }

    #[test]
    fn the_page_names_a_listed_model_and_falls_back_to_its_id() {
        let listed = vec![CleanupModel::listed("openai/gpt-4o", "OpenAI: GPT-4o")];
        let cases = [
            ("openai/gpt-4o", "OpenAI: GPT-4o"),
            ("greendale/human-being", "greendale/human-being"),
        ];
        for (slug, want) in cases {
            let model = CleanupModel::custom(slug);
            let details = post_processing_page(inputs(&model, &listed))
                .details
                .expect("details");
            assert_eq!(details.model_name, want, "{slug}");
        }
    }

    #[test]
    fn the_sheet_lists_curated_custom_then_the_rest() {
        let listed = vec![
            CleanupModel::listed("~anthropic/claude-haiku-latest", "Anthropic: Haiku"),
            CleanupModel::listed("openai/gpt-4o", "OpenAI: GPT-4o"),
            CleanupModel::listed("x-ai/grok", "xAI: Grok"),
        ];
        let typed = CleanupModel::custom("greendale/human-being");
        let key = key();
        let dialog = providers_dialog(&dialog_inputs(Some(&typed), &listed, &key));
        assert_eq!(
            titles_and_slugs(&dialog),
            [
                (
                    "Recommended",
                    vec![
                        "~openai/gpt-luna-latest",
                        "~anthropic/claude-haiku-latest",
                        "google/gemini-3.1-flash-lite"
                    ]
                ),
                ("Custom", vec!["greendale/human-being"]),
                ("All OpenRouter Models", vec!["openai/gpt-4o", "x-ai/grok"]),
            ]
        );
        assert_eq!(
            dialog.selected_model.as_deref(),
            Some("greendale/human-being")
        );
        assert_eq!(dialog.title, "Choose Model");
        assert_eq!(dialog.selected_provider, "openrouter");
        assert_eq!(dialog.key.placeholder, "OpenRouter API key");
        assert_eq!(dialog.search_placeholder, "Search or enter a model ID");
        assert_eq!(dialog.use_id_caption, "Use as model ID");
        assert_eq!(dialog.done_title, "Done");

        // a listed selection needs no Custom group
        let listed_pick = CleanupModel::custom("x-ai/grok");
        let dialog = providers_dialog(&dialog_inputs(Some(&listed_pick), &listed, &key));
        let titles: Vec<&str> = dialog
            .model_groups
            .iter()
            .map(|g| g.title.as_str())
            .collect();
        assert_eq!(titles, ["Recommended", "All OpenRouter Models"]);
    }

    #[test]
    fn models_error_names_the_provider() {
        let key = key();
        let dialog = providers_dialog(&ProvidersDialogInputs {
            models_loading: true,
            models_error: Some("OpenRouter returned HTTP 503."),
            ..dialog_inputs(None, &[], &key)
        });
        assert_eq!(
            dialog.models_error.as_deref(),
            Some("Couldn't load OpenRouter models: OpenRouter returned HTTP 503.")
        );
        assert_eq!(dialog.refresh_models.map(|b| b.enabled), Some(false));
        assert_eq!(dialog.loading_title, "Loading models…");
        assert_eq!(dialog.selected_model, None);
    }

    #[test]
    fn cln_8_debug_never_prints_the_key() {
        let field = ApiKeyFieldView {
            placeholder: "OpenRouter API key".to_owned(),
            text: "sk-or-troy-barnes".to_owned(),
            revision: 1,
            loading: false,
            error: None,
        };
        let printed = format!("{field:?}");
        assert!(!printed.contains("troy"), "{printed}");
    }
}
