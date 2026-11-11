//! The Post-Processing page (CLN-24..CLN-27) and its model sheet.

use std::fmt;

use onda_domain::cleanup::{
    CleanupError, CleanupModel, CleanupPrompt, CleanupProvider, ModelListing, SYSTEM_PROMPT,
    TEST_FAILED_TITLE,
};

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
        CleanupProvider::AppleIntelligence => "Post-processing runs on this Mac with Apple \
                                               Intelligence; your transcript never leaves it. \
                                               Falls back to the raw transcript if the model \
                                               is unavailable or the request fails."
            .to_owned(),
        CleanupProvider::Fireworks => "Cloud post-processing sends your transcript to Fireworks AI \
                                       for processing. Fireworks says it doesn't retain prompts or \
                                       outputs from its serverless models. The API key is stored \
                                       in your system keychain, not in preferences. Falls back to \
                                       the raw transcript if the request fails."
            .to_owned(),
    }
}

/// Why the model list is missing, as shown in the sheet. A provider that
/// lists only with a key asks for one instead of reporting an error.
pub fn models_error_text(provider: CleanupProvider, error: &CleanupError) -> String {
    let name = provider.display_name();
    match error {
        CleanupError::MissingKey => format!("Enter your {name} API key to see its models."),
        _ => format!("Couldn't load {name} models: {}", error.describe(provider)),
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
    /// Value for `select_dialog_provider`; also names its bundled logo,
    /// `<id>.svg` or `<id>-light.svg` and `<id>-dark.svg`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// An SF Symbol to show when the provider has no bundled logo.
    pub system_symbol: Option<String>,
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
    /// The sidebar, in order: every provider this Mac offers.
    pub providers: Vec<CleanupProviderOption>,
    /// The provider whose pane shows.
    pub selected_provider: String,
    /// The provider's API key field; absent for a provider that uses no
    /// key.
    pub key: Option<ApiKeyFieldView>,
    /// Why the provider cannot run now, e.g. "Turn on Apple Intelligence in
    /// System Settings…"; shown above its models.
    pub status_message: Option<String>,
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
    /// "Refresh Models"; absent for a provider with one fixed model, which
    /// also hides the search field; disabled while a listing runs.
    pub refresh_models: Option<ButtonView>,
    /// A listing is running: show a spinner next to the refresh button and,
    /// with `loading_title`, a row above the list.
    pub models_loading: bool,
    /// "Loading models…".
    pub loading_title: String,
    /// Why the list is missing, under the search field: a failed listing
    /// or, for a provider that lists only with a key, a request for one.
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

/// The prompt sheet: the whole prompt as an editable draft. Nothing is
/// saved until "Save"; "Cancel" discards the draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptEditorView {
    /// "Post-processing prompt".
    pub title: String,
    /// What a custom prompt does, under the title.
    pub intro: String,
    /// "Prompt instructions", above the editor.
    pub label: String,
    /// What the draft is: "Onda default" or "Custom".
    pub badge: String,
    /// The draft as the core knows it. Replace the editor's contents only
    /// when `revision` changes (the sheet opened, or "Restore Default"), so
    /// typing is never overwritten.
    pub text: String,
    /// Bumped when the core replaces `text`.
    pub revision: u64,
    /// Under the editor: "No changes", "Unsaved changes", what to do about
    /// a blank draft, or why a save failed.
    pub status: String,
    /// `status` is a problem; show it in red.
    pub status_is_error: bool,
    /// "4,216 characters".
    pub character_count: String,
    /// "What the model receives"; discloses `help`.
    pub help_title: String,
    /// How the transcript and app name reach the model.
    pub help: String,
    /// "Restore Default": sends `restore_default_prompt`, which puts Onda's
    /// prompt in the draft. Disabled while the draft is that prompt.
    pub restore: ButtonView,
    /// "Cancel"; sends `cancel_prompt_editor`.
    pub cancel_title: String,
    /// "Save": sends `save_prompt`. Disabled while the draft is blank or is
    /// the prompt in use.
    pub save: ButtonView,
}

/// The "Prompt" section: the one prompt every provider and model gets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSummaryView {
    /// "Prompt".
    pub title: String,
    /// "Onda default" or "Custom prompt"; clicking the row sends
    /// `open_prompt_editor`.
    pub name: String,
    /// "Instructions for cleaning up your dictation".
    pub subtitle: String,
    /// The start of the prompt in use, as one line; show up to three lines.
    pub preview: String,
    /// "Used for all post-processing models."
    pub caption: String,
    /// What the last save did, e.g. "Custom prompt saved. Applies to your
    /// next dictation."
    pub status: Option<String>,
    /// The prompt sheet, while open.
    pub editor: Option<PromptEditorView>,
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
    /// The prompt in use and its sheet.
    pub prompt: PromptSummaryView,
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
    /// `cleanupPrompt`: the prompt in use.
    pub prompt: &'a CleanupPrompt,
    /// The prompt sheet's draft, while open.
    pub prompt_draft: Option<PromptDraftInputs<'a>>,
    /// What the last prompt save did.
    pub prompt_status: Option<&'a str>,
    /// A test is running.
    pub test_running: bool,
    /// The last finished test.
    pub test_result: Option<&'a CleanupTestResult>,
    /// The last test error message.
    pub test_error: Option<&'a str>,
}

/// What the prompt sheet is built from.
#[derive(Debug, Clone, Copy)]
pub struct PromptDraftInputs<'a> {
    /// The unsaved text, exactly as typed.
    pub text: &'a str,
    /// Bumped when the core replaced the text.
    pub revision: u64,
    /// Why the last "Save" failed, until the draft changes.
    pub save_error: Option<&'a str>,
}

/// What the model sheet is built from.
#[derive(Debug, Clone, Copy)]
pub struct ProvidersDialogInputs<'a> {
    /// The provider whose pane shows.
    pub provider: CleanupProvider,
    /// The providers the sidebar lists, in order.
    pub offered: &'a [CleanupProvider],
    /// Why the provider cannot run now.
    pub status_message: Option<&'a str>,
    /// The model in use, when it belongs to this provider.
    pub selected: Option<&'a CleanupModel>,
    /// The models it listed; empty until a listing finishes.
    pub listed_models: &'a [CleanupModel],
    /// A listing is running.
    pub models_loading: bool,
    /// The last listing error.
    pub models_error: Option<&'a str>,
    /// The key field, for a provider that uses a key.
    pub key: Option<&'a ApiKeyFieldView>,
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
        providers: inputs
            .offered
            .iter()
            .map(|provider| CleanupProviderOption {
                id: provider.id().to_owned(),
                name: provider.display_name().to_owned(),
                // Apple's mark ships with the OS as a symbol
                system_symbol: provider
                    .on_device()
                    .then(|| "apple.intelligence".to_owned()),
            })
            .collect(),
        selected_provider: provider.id().to_owned(),
        key: inputs.key.cloned(),
        status_message: inputs.status_message.map(str::to_owned),
        models_label: "Models".to_owned(),
        model_groups: model_groups(inputs),
        selected_model: inputs.selected.map(|model| model.slug.clone().into_owned()),
        search_placeholder: "Search or enter a model ID".to_owned(),
        use_id_caption: "Use as model ID".to_owned(),
        refresh_models: (provider.model_listing() != ModelListing::None)
            .then(|| ButtonView::new("Refresh Models", !inputs.models_loading)),
        models_loading: inputs.models_loading,
        loading_title: "Loading models…".to_owned(),
        models_error: inputs.models_error.map(str::to_owned),
        done_title: "Done".to_owned(),
    }
}

/// Most characters of the prompt the page shows as its preview.
const PROMPT_PREVIEW_CHARS: usize = 400;

/// What a prompt save did, for the line it leaves on the page.
pub fn prompt_saved_text(saved: &CleanupPrompt) -> &'static str {
    match saved.custom() {
        Some(_) => "Custom prompt saved. Applies to your next dictation.",
        None => "Default prompt restored.",
    }
}

// one line: runs of whitespace collapse, and a long prompt is cut
fn prompt_preview(prompt: &str) -> String {
    let one_line = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    match one_line.char_indices().nth(PROMPT_PREVIEW_CHARS) {
        Some((cut, _)) => format!("{}…", &one_line[..cut]),
        None => one_line,
    }
}

// "4,216 characters"
fn character_count(text: &str) -> String {
    let count = text.chars().count();
    let digits = count.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    match count {
        1 => "1 character".to_owned(),
        _ => format!("{grouped} characters"),
    }
}

fn prompt_editor(saved: &CleanupPrompt, draft: &PromptDraftInputs<'_>) -> PromptEditorView {
    let blank = draft.text.trim().is_empty();
    let changed = draft.text != saved.text();
    let is_default = draft.text == SYSTEM_PROMPT;
    let (status, status_is_error) = match draft.save_error {
        _ if blank => ("Enter a prompt or restore the default.".to_owned(), true),
        Some(error) => (error.to_owned(), true),
        None if changed => ("Unsaved changes".to_owned(), false),
        None => ("No changes".to_owned(), false),
    };
    PromptEditorView {
        title: "Post-processing prompt".to_owned(),
        intro: "Control how Onda cleans up your dictation. Your prompt replaces the default \
                instructions for every post-processing model."
            .to_owned(),
        label: "Prompt instructions".to_owned(),
        badge: if is_default { "Onda default" } else { "Custom" }.to_owned(),
        text: draft.text.to_owned(),
        revision: draft.revision,
        status,
        status_is_error,
        character_count: character_count(draft.text),
        help_title: "What the model receives".to_owned(),
        help: "Onda sends your instructions with the dictated text inside <transcript> tags. \
               It may also include a \"Dictating into:\" line naming the app. Tell the model to \
               return only the text you want to insert."
            .to_owned(),
        restore: ButtonView::new("Restore Default", !is_default),
        cancel_title: "Cancel".to_owned(),
        save: ButtonView::new("Save", !blank && changed),
    }
}

fn prompt_summary(inputs: &PostProcessingInputs<'_>) -> PromptSummaryView {
    let prompt = inputs.prompt;
    PromptSummaryView {
        title: "Prompt".to_owned(),
        name: match prompt.custom() {
            Some(_) => "Custom prompt",
            None => "Onda default",
        }
        .to_owned(),
        subtitle: "Instructions for cleaning up your dictation".to_owned(),
        preview: prompt_preview(prompt.text()),
        caption: "Used for all post-processing models.".to_owned(),
        status: inputs.prompt_status.map(str::to_owned),
        editor: inputs
            .prompt_draft
            .as_ref()
            .map(|draft| prompt_editor(prompt, draft)),
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
            prompt: prompt_summary(&inputs),
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

    static DEFAULT_PROMPT: std::sync::LazyLock<CleanupPrompt> =
        std::sync::LazyLock::new(CleanupPrompt::default);

    fn custom(text: &str) -> CleanupPrompt {
        CleanupPrompt::from_text(text).expect("a prompt with text")
    }

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
            prompt: &DEFAULT_PROMPT,
            prompt_draft: None,
            prompt_status: None,
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
            offered: &CleanupProvider::ALL,
            status_message: None,
            selected,
            listed_models: listed,
            models_loading: false,
            models_error: None,
            key: Some(key),
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
    fn the_prompt_row_names_and_previews_the_prompt_in_use() {
        let details = post_processing_page(inputs(&CLEANUP_MODELS[0], &[]))
            .details
            .expect("details");
        let prompt = details.prompt;
        assert_eq!(prompt.title, "Prompt");
        assert_eq!(prompt.name, "Onda default");
        assert_eq!(
            prompt.subtitle,
            "Instructions for cleaning up your dictation"
        );
        assert_eq!(prompt.caption, "Used for all post-processing models.");
        assert_eq!(prompt.status, None);
        assert_eq!(prompt.editor, None, "the sheet starts closed");
        assert!(
            prompt.preview.starts_with(
                "You are a voice-to-text dictation cleaner. Your role is to clean and format"
            ),
            "{}",
            prompt.preview
        );
        assert!(
            prompt
                .preview
                .contains("anything else. ## HIGHEST PRIORITY"),
            "line breaks read as spaces: {}",
            prompt.preview
        );
        assert!(prompt.preview.ends_with('…'), "a long prompt is cut");
        assert_eq!(prompt.preview.chars().count(), PROMPT_PREVIEW_CHARS + 1);

        let saved = custom("Fix punctuation only.\n\n  Keep every word Troy said.");
        let details = post_processing_page(PostProcessingInputs {
            prompt: &saved,
            prompt_status: Some(prompt_saved_text(&saved)),
            ..inputs(&CLEANUP_MODELS[0], &[])
        })
        .details
        .expect("details");
        assert_eq!(details.prompt.name, "Custom prompt");
        assert_eq!(
            details.prompt.preview,
            "Fix punctuation only. Keep every word Troy said."
        );
        assert_eq!(
            details.prompt.status.as_deref(),
            Some("Custom prompt saved. Applies to your next dictation.")
        );
        assert_eq!(
            prompt_saved_text(&CleanupPrompt::default()),
            "Default prompt restored."
        );
    }

    #[test]
    fn the_prompt_sheet_follows_its_draft() {
        struct Case {
            name: &'static str,
            saved: CleanupPrompt,
            draft: &'static str,
            save_error: Option<&'static str>,
            want_badge: &'static str,
            want_status: &'static str,
            want_error: bool,
            want_save: bool,
            want_restore: bool,
            want_count: &'static str,
        }
        let cases = [
            Case {
                name: "the default, untouched",
                saved: CleanupPrompt::default(),
                draft: SYSTEM_PROMPT,
                save_error: None,
                want_badge: "Onda default",
                want_status: "No changes",
                want_error: false,
                want_save: false,
                want_restore: false,
                want_count: "4,216 characters",
            },
            Case {
                name: "an edit to the default",
                saved: CleanupPrompt::default(),
                draft: "Fix punctuation only.",
                save_error: None,
                want_badge: "Custom",
                want_status: "Unsaved changes",
                want_error: false,
                want_save: true,
                want_restore: true,
                want_count: "21 characters",
            },
            Case {
                name: "a saved custom prompt, untouched",
                saved: custom("Fix punctuation only."),
                draft: "Fix punctuation only.",
                save_error: None,
                want_badge: "Custom",
                want_status: "No changes",
                want_error: false,
                want_save: false,
                want_restore: true,
                want_count: "21 characters",
            },
            Case {
                name: "the default restored over a custom prompt",
                saved: custom("Fix punctuation only."),
                draft: SYSTEM_PROMPT,
                save_error: None,
                want_badge: "Onda default",
                want_status: "Unsaved changes",
                want_error: false,
                want_save: true,
                want_restore: false,
                want_count: "4,216 characters",
            },
            Case {
                name: "an empty draft cannot be saved",
                saved: CleanupPrompt::default(),
                draft: "",
                save_error: None,
                want_badge: "Custom",
                want_status: "Enter a prompt or restore the default.",
                want_error: true,
                want_save: false,
                want_restore: true,
                want_count: "0 characters",
            },
            Case {
                name: "a whitespace draft cannot be saved",
                saved: custom("Fix punctuation only."),
                draft: " \n\t",
                save_error: None,
                want_badge: "Custom",
                want_status: "Enter a prompt or restore the default.",
                want_error: true,
                want_save: false,
                want_restore: true,
                want_count: "3 characters",
            },
            Case {
                name: "a failed save says why and can be retried",
                saved: CleanupPrompt::default(),
                draft: "é",
                save_error: Some("Couldn't save the prompt: the disk is full"),
                want_badge: "Custom",
                want_status: "Couldn't save the prompt: the disk is full",
                want_error: true,
                want_save: true,
                want_restore: true,
                want_count: "1 character",
            },
        ];
        for tc in cases {
            let editor = post_processing_page(PostProcessingInputs {
                prompt: &tc.saved,
                prompt_draft: Some(PromptDraftInputs {
                    text: tc.draft,
                    revision: 7,
                    save_error: tc.save_error,
                }),
                ..inputs(&CLEANUP_MODELS[0], &[])
            })
            .details
            .expect("details")
            .prompt
            .editor
            .expect(tc.name);
            assert_eq!(editor.text, tc.draft, "{}", tc.name);
            assert_eq!(editor.revision, 7, "{}", tc.name);
            assert_eq!(editor.badge, tc.want_badge, "{}", tc.name);
            assert_eq!(editor.status, tc.want_status, "{}", tc.name);
            assert_eq!(editor.status_is_error, tc.want_error, "{}", tc.name);
            assert_eq!(editor.save.enabled, tc.want_save, "{}", tc.name);
            assert_eq!(editor.restore.enabled, tc.want_restore, "{}", tc.name);
            assert_eq!(editor.character_count, tc.want_count, "{}", tc.name);
            assert_eq!(editor.title, "Post-processing prompt", "{}", tc.name);
            assert_eq!(editor.save.title, "Save", "{}", tc.name);
            assert_eq!(editor.restore.title, "Restore Default", "{}", tc.name);
            assert_eq!(editor.cancel_title, "Cancel", "{}", tc.name);
        }
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
        assert_eq!(
            dialog.key.map(|key| key.placeholder).as_deref(),
            Some("OpenRouter API key")
        );
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
    fn the_on_device_pane_has_no_key_list_or_refresh() {
        let model = CleanupProvider::AppleIntelligence.default_model();
        let dialog = providers_dialog(&ProvidersDialogInputs {
            provider: CleanupProvider::AppleIntelligence,
            offered: &CleanupProvider::ALL,
            status_message: Some("Turn on Apple Intelligence."),
            selected: Some(model),
            listed_models: &[],
            models_loading: false,
            models_error: None,
            key: None,
        });
        assert_eq!(dialog.key, None);
        assert_eq!(dialog.refresh_models, None, "nothing to list or search");
        assert_eq!(
            dialog.status_message.as_deref(),
            Some("Turn on Apple Intelligence.")
        );
        assert_eq!(
            titles_and_slugs(&dialog),
            [("Recommended", vec!["on-device"])]
        );
        assert_eq!(dialog.selected_model.as_deref(), Some("on-device"));
    }

    #[test]
    fn models_error_names_the_provider() {
        let key = key();
        let dialog = providers_dialog(&ProvidersDialogInputs {
            models_loading: true,
            models_error: Some("Couldn't load OpenRouter models: OpenRouter returned HTTP 503."),
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
    fn models_error_text_asks_for_a_missing_key() {
        assert_eq!(
            models_error_text(CleanupProvider::Fireworks, &CleanupError::MissingKey),
            "Enter your Fireworks AI API key to see its models."
        );
        assert_eq!(
            models_error_text(
                CleanupProvider::OpenRouter,
                &CleanupError::Http {
                    status: 503,
                    message: None
                }
            ),
            "Couldn't load OpenRouter models: OpenRouter returned HTTP 503."
        );
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
