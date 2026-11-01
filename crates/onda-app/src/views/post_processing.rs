//! The Post-Processing page (CLN-24..CLN-27).

use std::fmt;

use onda_domain::cleanup::{CLEANUP_MODELS, CleanupModel, TEST_FAILED_TITLE};

use super::{AlertView, ButtonView, ToggleView};

/// The caption under the model picker (CLN-26).
const CAPTION: &str = "Cloud post-processing sends your transcript to OpenRouter and the \
                       selected model provider for processing. Onda asks OpenRouter to use only \
                       providers that don't retain your transcript. The API key is stored in \
                       your system keychain, not in preferences. Falls back to the raw \
                       transcript if the request fails.";

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

/// One model picker entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupModelOption {
    /// Value for `set_cleanup_model`.
    pub slug: String,
    /// Display name.
    pub name: String,
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
    /// The API key field.
    pub key: ApiKeyFieldView,
    /// "Model".
    pub model_label: String,
    /// The catalog, in order.
    pub models: Vec<CleanupModelOption>,
    /// The model actually used (CLN-21).
    pub selected_model: String,
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
    /// The model actually used.
    pub model: &'static CleanupModel,
    /// The key field.
    pub key: &'a ApiKeyFieldView,
    /// A test is running.
    pub test_running: bool,
    /// The last finished test.
    pub test_result: Option<&'a CleanupTestResult>,
    /// The last test error message.
    pub test_error: Option<&'a str>,
}

/// Builds the page.
pub fn post_processing_page(inputs: PostProcessingInputs<'_>) -> PostProcessingPage {
    PostProcessingPage {
        enabled: ToggleView::new("Post-process transcripts", inputs.enabled, true),
        details: inputs.enabled.then(|| PostProcessingDetails {
            key: inputs.key.clone(),
            model_label: "Model".to_owned(),
            models: CLEANUP_MODELS
                .iter()
                .map(|model| CleanupModelOption {
                    slug: model.slug.to_owned(),
                    name: model.display_name.to_owned(),
                })
                .collect(),
            selected_model: inputs.model.slug.to_owned(),
            caption: CAPTION.to_owned(),
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
    use super::*;

    fn page(enabled: bool) -> PostProcessingPage {
        let key = ApiKeyFieldView {
            placeholder: "OpenRouter API key".to_owned(),
            text: String::new(),
            revision: 0,
            loading: false,
            error: None,
        };
        post_processing_page(PostProcessingInputs {
            enabled,
            model: &CLEANUP_MODELS[0],
            key: &key,
            test_running: false,
            test_result: None,
            test_error: None,
        })
    }

    #[test]
    fn cln_26_copy_and_what_shows_when_on() {
        let off = page(false);
        assert_eq!(off.enabled.label, "Post-process transcripts");
        assert!(off.details.is_none(), "nothing else shows while off");

        let details = page(true).details.expect("details when on");
        assert_eq!(details.model_label, "Model");
        assert_eq!(details.test_button.title, "Test with Sample Text");
        assert_eq!(
            details.caption,
            "Cloud post-processing sends your transcript to OpenRouter and the selected model \
             provider for processing. Onda asks OpenRouter to use only providers that don't \
             retain your transcript. The API key is stored in your system keychain, not in \
             preferences. Falls back to the raw transcript if the request fails."
        );
        let names: Vec<&str> = details.models.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "GPT Luna (latest)",
                "Claude Haiku (latest)",
                "Gemini Flash Lite"
            ]
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
