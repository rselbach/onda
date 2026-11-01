//! The curated OpenRouter model catalog and model slugs.

use std::borrow::Cow;

/// How much reasoning a cleanup request asks for, sent as `reasoning.effort`.
///
/// Cleanup rewrites what the user said rather than solving a problem, so
/// every catalog entry asks for the least reasoning its model accepts
/// (CLN-20).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReasoningEffort {
    /// Reasoning disabled; sent as `"none"`.
    Off,
}

impl ReasoningEffort {
    /// The value OpenRouter expects in `reasoning.effort`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "none",
        }
    }
}

/// One cleanup model: a curated entry, a model the provider listed, or a
/// slug the user typed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CleanupModel {
    /// Provider slug, sent verbatim as `model`.
    pub slug: Cow<'static, str>,
    /// Label shown in the "Model" picker.
    pub display_name: Cow<'static, str>,
    /// Sent as `reasoning.effort` (CLN-5, CLN-20). `None` sends no
    /// reasoning setting: models outside the catalog may require reasoning
    /// and reject `"none"`, so they get the provider's default.
    pub reasoning_effort: Option<ReasoningEffort>,
}

impl CleanupModel {
    pub(super) const fn curated(slug: &'static str, display_name: &'static str) -> Self {
        Self {
            slug: Cow::Borrowed(slug),
            display_name: Cow::Borrowed(display_name),
            reasoning_effort: Some(ReasoningEffort::Off),
        }
    }

    /// A model outside the curated catalog, shown as `display_name`.
    pub fn listed(slug: impl Into<String>, display_name: impl Into<String>) -> Self {
        Self {
            slug: Cow::Owned(slug.into()),
            display_name: Cow::Owned(display_name.into()),
            reasoning_effort: None,
        }
    }

    /// A model the user named by slug; its slug is its label.
    pub fn custom(slug: impl Into<String>) -> Self {
        let slug = slug.into();
        Self::listed(slug.clone(), slug)
    }
}

/// The curated OpenRouter models, in display order; the first is the
/// default.
pub static CLEANUP_MODELS: [CleanupModel; 3] = [
    CleanupModel::curated("~openai/gpt-luna-latest", "GPT Luna (latest)"),
    CleanupModel::curated("~anthropic/claude-haiku-latest", "Claude Haiku (latest)"),
    CleanupModel::curated("google/gemini-3.1-flash-lite", "Gemini Flash Lite"),
];

/// Apple Intelligence's one model.
pub static APPLE_MODELS: [CleanupModel; 1] =
    [CleanupModel::curated("on-device", "On-Device Model")];

/// Slugs an earlier release offered, mapped to their replacement, so a
/// catalog update keeps the user's vendor (CLN-21).
const REPLACED_SLUGS: [(&str, &str); 1] = [(
    "~google/gemini-flash-latest",
    "google/gemini-3.1-flash-lite",
)];

/// The default model, also the `openRouterModel` setting default.
pub fn default_model() -> &'static CleanupModel {
    &CLEANUP_MODELS[0]
}

/// The curated entry with exactly this slug.
pub fn model_for_slug(slug: &str) -> Option<&'static CleanupModel> {
    CLEANUP_MODELS.iter().find(|m| m.slug == slug)
}

/// The model a stored `openRouterModel` value actually uses (CLN-21).
///
/// Replaced slugs map to their successor and curated slugs to their entry.
/// Any other slug is a model the user picked from the provider's list or
/// typed, and is kept verbatim; a missing or blank value resolves to the
/// default.
pub fn resolve_model(stored: Option<&str>) -> CleanupModel {
    let Some(stored) = stored.map(str::trim).filter(|slug| !slug.is_empty()) else {
        return default_model().clone();
    };
    let slug = REPLACED_SLUGS
        .iter()
        .find(|(old, _)| *old == stored)
        .map_or(stored, |(_, new)| *new);
    model_for_slug(slug).map_or_else(|| CleanupModel::custom(slug), Clone::clone)
}

/// A typed custom model slug, trimmed; `None` when blank or when it holds
/// whitespace, which no provider slug does.
pub fn custom_model_slug(raw: &str) -> Option<String> {
    let slug = raw.trim();
    (!slug.is_empty() && !slug.contains(char::is_whitespace)).then(|| slug.to_owned())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn catalog_is_the_curated_three_in_order_cln_20() {
        let got: Vec<(&str, &str)> = CLEANUP_MODELS
            .iter()
            .map(|m| (m.slug.as_ref(), m.display_name.as_ref()))
            .collect();
        let want = vec![
            ("~openai/gpt-luna-latest", "GPT Luna (latest)"),
            ("~anthropic/claude-haiku-latest", "Claude Haiku (latest)"),
            ("google/gemini-3.1-flash-lite", "Gemini Flash Lite"),
        ];
        assert_eq!(got, want, "catalog order and names");
        let slugs: HashSet<_> = CLEANUP_MODELS.iter().map(|m| &m.slug).collect();
        let names: HashSet<_> = CLEANUP_MODELS.iter().map(|m| &m.display_name).collect();
        assert_eq!(slugs.len(), CLEANUP_MODELS.len(), "unique slugs");
        assert_eq!(names.len(), CLEANUP_MODELS.len(), "unique display names");
        assert_eq!(
            default_model().slug,
            "~openai/gpt-luna-latest",
            "default is first"
        );
        for model in &CLEANUP_MODELS {
            assert_eq!(
                model.reasoning_effort.map(ReasoningEffort::as_str),
                Some("none"),
                "{} should not reason",
                model.slug
            );
        }
    }

    #[test]
    fn resolve_model_cln_21() {
        struct Case {
            name: &'static str,
            stored: Option<&'static str>,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "missing resolves to the default",
                stored: None,
                want: "~openai/gpt-luna-latest",
            },
            Case {
                name: "a custom slug is kept verbatim",
                stored: Some("openai/gpt-4o"),
                want: "openai/gpt-4o",
            },
            Case {
                name: "empty slug resolves to the default",
                stored: Some(""),
                want: "~openai/gpt-luna-latest",
            },
            Case {
                name: "blank slug resolves to the default",
                stored: Some("  "),
                want: "~openai/gpt-luna-latest",
            },
            Case {
                name: "known slug is kept verbatim",
                stored: Some("~anthropic/claude-haiku-latest"),
                want: "~anthropic/claude-haiku-latest",
            },
            Case {
                name: "replaced slug keeps the vendor",
                stored: Some("~google/gemini-flash-latest"),
                want: "google/gemini-3.1-flash-lite",
            },
        ];
        for tc in cases {
            assert_eq!(resolve_model(tc.stored).slug, tc.want, "{}", tc.name);
        }
        assert_eq!(model_for_slug("openai/gpt-4o"), None, "unknown slug lookup");
        assert_eq!(
            resolve_model(Some("~anthropic/claude-haiku-latest")).display_name,
            "Claude Haiku (latest)",
            "curated slugs keep their name"
        );
        assert_eq!(
            resolve_model(Some("openai/gpt-4o")).display_name,
            "openai/gpt-4o",
            "custom slugs are their own name"
        );
        assert_eq!(
            resolve_model(Some("openai/gpt-4o")).reasoning_effort,
            None,
            "custom slugs keep the provider's reasoning default"
        );
    }

    #[test]
    fn custom_model_slug_trims_and_rejects_blank_or_spaced() {
        struct Case {
            name: &'static str,
            raw: &'static str,
            want: Option<&'static str>,
        }
        let cases = [
            Case {
                name: "plain slug",
                raw: "openai/gpt-4o",
                want: Some("openai/gpt-4o"),
            },
            Case {
                name: "padded slug is trimmed",
                raw: "  meta-llama/llama-3.3-70b-instruct\n",
                want: Some("meta-llama/llama-3.3-70b-instruct"),
            },
            Case {
                name: "blank",
                raw: "   ",
                want: None,
            },
            Case {
                name: "inner whitespace",
                raw: "greendale community college",
                want: None,
            },
        ];
        for tc in cases {
            assert_eq!(custom_model_slug(tc.raw).as_deref(), tc.want, "{}", tc.name);
        }
    }
}
