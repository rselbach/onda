//! The curated OpenRouter model catalog (05, "Model catalog").

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

/// One curated cleanup model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CleanupModel {
    /// OpenRouter slug, sent verbatim as `model`.
    pub slug: &'static str,
    /// Label shown in the "Model" picker.
    pub display_name: &'static str,
    /// Sent as `reasoning.effort` (CLN-5, CLN-20).
    pub reasoning_effort: ReasoningEffort,
}

/// The picker's models, in display order; the first is the default.
pub const CLEANUP_MODELS: [CleanupModel; 3] = [
    CleanupModel {
        slug: "~openai/gpt-luna-latest",
        display_name: "GPT Luna (latest)",
        reasoning_effort: ReasoningEffort::Off,
    },
    CleanupModel {
        slug: "~anthropic/claude-haiku-latest",
        display_name: "Claude Haiku (latest)",
        reasoning_effort: ReasoningEffort::Off,
    },
    CleanupModel {
        slug: "google/gemini-3.1-flash-lite",
        display_name: "Gemini Flash Lite",
        reasoning_effort: ReasoningEffort::Off,
    },
];

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

/// The catalog entry with exactly this slug.
pub fn model_for_slug(slug: &str) -> Option<&'static CleanupModel> {
    CLEANUP_MODELS.iter().find(|m| m.slug == slug)
}

/// The model a stored `openRouterModel` value actually uses (CLN-21).
///
/// Replaced slugs map to their successor; missing or unknown slugs resolve
/// to the default. The picker shows this model, never the raw stored value.
pub fn resolve_model(stored: Option<&str>) -> &'static CleanupModel {
    let Some(stored) = stored else {
        return default_model();
    };
    let slug = REPLACED_SLUGS
        .iter()
        .find(|(old, _)| *old == stored)
        .map_or(stored, |(_, new)| *new);
    model_for_slug(slug).unwrap_or_else(default_model)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn catalog_is_the_curated_three_in_order_cln_20() {
        let got: Vec<(&str, &str)> = CLEANUP_MODELS
            .iter()
            .map(|m| (m.slug, m.display_name))
            .collect();
        let want = vec![
            ("~openai/gpt-luna-latest", "GPT Luna (latest)"),
            ("~anthropic/claude-haiku-latest", "Claude Haiku (latest)"),
            ("google/gemini-3.1-flash-lite", "Gemini Flash Lite"),
        ];
        assert_eq!(got, want, "catalog order and names");
        let slugs: HashSet<_> = CLEANUP_MODELS.iter().map(|m| m.slug).collect();
        let names: HashSet<_> = CLEANUP_MODELS.iter().map(|m| m.display_name).collect();
        assert_eq!(slugs.len(), CLEANUP_MODELS.len(), "unique slugs");
        assert_eq!(names.len(), CLEANUP_MODELS.len(), "unique display names");
        assert_eq!(
            default_model().slug,
            "~openai/gpt-luna-latest",
            "default is first"
        );
        for model in &CLEANUP_MODELS {
            assert_eq!(
                model.reasoning_effort.as_str(),
                "none",
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
                name: "unknown slug resolves to the default",
                stored: Some("openai/gpt-4o"),
                want: "~openai/gpt-luna-latest",
            },
            Case {
                name: "empty slug resolves to the default",
                stored: Some(""),
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
    }
}
