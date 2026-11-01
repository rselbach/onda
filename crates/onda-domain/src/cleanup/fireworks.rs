//! Fireworks AI: its curated models, request body, and model listing.
//!
//! The chat API follows OpenAI's shape, so [`super::parse_response`] reads
//! its replies. Listing models needs the API key.

use serde::Serialize;
use serde_json::Value;

use super::catalog::CleanupModel;
use super::openrouter::{ChatMessage, CleanupError};

/// Chat-completions endpoint.
pub const ENDPOINT: &str = "https://api.fireworks.ai/inference/v1/chat/completions";

/// The models Fireworks itself hosts; paged, and needs the key.
pub const MODELS_ENDPOINT: &str = "https://api.fireworks.ai/v1/accounts/fireworks/models";

// the most the listing returns per page
const PAGE_SIZE: u32 = 200;

/// The curated Fireworks models, in display order; the first is the
/// default. Cleanup asks for no reasoning (CLN-20), so each must accept
/// `reasoning_effort: "none"`: GLM 5.3 Flash is thinking-only and rejects
/// it, so it is not here.
pub static FIREWORKS_MODELS: [CleanupModel; 1] = [CleanupModel::curated(
    "accounts/fireworks/models/deepseek-v4p1-flash",
    "DeepSeek V4.1 Flash",
)];

const MODEL_PREFIX: &str = "accounts/fireworks/models/";

/// The model a stored Fireworks slug uses: the curated entry, or the slug
/// itself named by its last path segment. Blank resolves to the default.
pub fn resolve_model(stored: Option<&str>) -> CleanupModel {
    let Some(stored) = stored.map(str::trim).filter(|slug| !slug.is_empty()) else {
        return FIREWORKS_MODELS[0].clone();
    };
    if let Some(model) = FIREWORKS_MODELS.iter().find(|m| m.slug == stored) {
        return model.clone();
    }
    let name = stored.strip_prefix(MODEL_PREFIX).unwrap_or(stored);
    CleanupModel::listed(stored, name)
}

/// The Fireworks chat body. Opaque; only serialized.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(super) struct Body {
    pub(super) model: String,
    pub(super) messages: [ChatMessage; 2],
    pub(super) stream: bool,
    pub(super) temperature: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) reasoning_effort: Option<&'static str>,
}

/// The listing URL for one page.
pub(super) fn models_url(page_token: Option<&str>) -> String {
    let mut url =
        format!("{MODELS_ENDPOINT}?pageSize={PAGE_SIZE}&filter=supports_serverless%3Dtrue");
    if let Some(token) = page_token {
        url.push_str("&pageToken=");
        url.push_str(&percent_encode(token));
    }
    url
}

/// One listing page: serverless chat models and the next page's token.
pub(super) fn parse_models(
    json: &Value,
) -> Result<(Vec<CleanupModel>, Option<String>), CleanupError> {
    let entries = match json.get("models") {
        // an account with nothing to list may omit the field
        None => &Vec::new(),
        Some(models) => models.as_array().ok_or(CleanupError::Malformed)?,
    };
    let models = entries
        .iter()
        // only models with a conversation config serve chat completions;
        // others need a dedicated deployment
        .filter(|entry| {
            entry.get("supportsServerless").and_then(Value::as_bool) == Some(true)
                && entry
                    .get("conversationConfig")
                    .is_some_and(Value::is_object)
        })
        .filter_map(|entry| {
            let id = entry.get("name").and_then(Value::as_str)?.trim();
            if id.is_empty() {
                return None;
            }
            let name = entry
                .get("displayName")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| id.strip_prefix(MODEL_PREFIX).unwrap_or(id));
            Some(CleanupModel::listed(id, name))
        })
        .collect();
    let next = json
        .get("nextPageToken")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(str::to_owned);
    Ok((models, next))
}

// page tokens are opaque; encode everything but unreserved characters
fn percent_encode(raw: &str) -> String {
    raw.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn resolve_model_names_custom_slugs_by_their_last_segment() {
        struct Case {
            name: &'static str,
            stored: Option<&'static str>,
            want: (&'static str, &'static str),
        }
        let cases = [
            Case {
                name: "missing resolves to the default",
                stored: None,
                want: (
                    "accounts/fireworks/models/deepseek-v4p1-flash",
                    "DeepSeek V4.1 Flash",
                ),
            },
            Case {
                name: "blank resolves to the default",
                stored: Some("  "),
                want: (
                    "accounts/fireworks/models/deepseek-v4p1-flash",
                    "DeepSeek V4.1 Flash",
                ),
            },
            Case {
                name: "curated keeps its name",
                stored: Some("accounts/fireworks/models/deepseek-v4p1-flash"),
                want: (
                    "accounts/fireworks/models/deepseek-v4p1-flash",
                    "DeepSeek V4.1 Flash",
                ),
            },
            Case {
                name: "a thinking-only model is not curated",
                stored: Some("accounts/fireworks/models/glm-5p3-flash"),
                want: ("accounts/fireworks/models/glm-5p3-flash", "glm-5p3-flash"),
            },
            Case {
                name: "a Fireworks slug is named by its model",
                stored: Some("accounts/fireworks/models/kimi-k3"),
                want: ("accounts/fireworks/models/kimi-k3", "kimi-k3"),
            },
            Case {
                name: "any other slug is its own name",
                stored: Some("accounts/greendale/deployments/human-being"),
                want: (
                    "accounts/greendale/deployments/human-being",
                    "accounts/greendale/deployments/human-being",
                ),
            },
        ];
        for tc in cases {
            let got = resolve_model(tc.stored);
            assert_eq!(
                (got.slug.as_ref(), got.display_name.as_ref()),
                tc.want,
                "{}",
                tc.name
            );
        }
        assert_eq!(
            resolve_model(Some("accounts/fireworks/models/kimi-k3")).reasoning_effort,
            None,
            "only curated models turn reasoning off"
        );
    }

    #[test]
    fn models_url_pages_serverless_models() {
        assert_eq!(
            models_url(None),
            "https://api.fireworks.ai/v1/accounts/fireworks/models?pageSize=200&filter=supports_serverless%3Dtrue"
        );
        assert_eq!(
            models_url(Some("abc+/=")),
            "https://api.fireworks.ai/v1/accounts/fireworks/models?pageSize=200&filter=supports_serverless%3Dtrue&pageToken=abc%2B%2F%3D"
        );
    }

    #[test]
    fn parse_models_keeps_serverless_chat_models() {
        let page = json!({
            "models": [
                {"name": "accounts/fireworks/models/glm-5p3-flash", "displayName": "GLM 5.3 Flash",
                 "supportsServerless": true, "conversationConfig": {"style": "glm"}},
                {"name": "accounts/fireworks/models/kimi-k3",
                 "supportsServerless": true, "conversationConfig": {"style": "kimi"}},
                {"name": "accounts/fireworks/models/qwen3p8-flash-next-fp8", "displayName": "Qwen3.8 Flash Next",
                 "supportsServerless": false, "conversationConfig": {"style": "qwen"}},
                {"name": "accounts/fireworks/models/nomic-embed", "displayName": "Nomic Embed",
                 "supportsServerless": true},
                {"displayName": "No name", "supportsServerless": true, "conversationConfig": {}},
            ],
            "nextPageToken": "page-2",
        });
        let (models, next) = parse_models(&page).expect("page");
        let got: Vec<(&str, &str)> = models
            .iter()
            .map(|m| (m.slug.as_ref(), m.display_name.as_ref()))
            .collect();
        assert_eq!(
            got,
            [
                ("accounts/fireworks/models/glm-5p3-flash", "GLM 5.3 Flash"),
                ("accounts/fireworks/models/kimi-k3", "kimi-k3"),
            ]
        );
        assert_eq!(next.as_deref(), Some("page-2"));

        let (models, next) = parse_models(&json!({"nextPageToken": ""})).expect("last page");
        assert!(models.is_empty());
        assert_eq!(next, None, "an empty token ends the listing");
        assert_eq!(
            parse_models(&json!({"models": "nope"})),
            Err(CleanupError::Malformed)
        );
    }
}
