//! DeepSeek's chat requests and authenticated model listing.
//!
//! API contract: <https://api-docs.deepseek.com/api/create-chat-completion/>.

use serde::Serialize;
use serde_json::Value;

use super::catalog::CleanupModel;
use super::openrouter::{ChatMessage, CleanupError};

pub(super) const ENDPOINT: &str = "https://api.deepseek.com/chat/completions";
pub(super) const MODELS_ENDPOINT: &str = "https://api.deepseek.com/models";

/// Curated DeepSeek models, with Flash as the default.
pub static DEEPSEEK_MODELS: [CleanupModel; 2] = [
    CleanupModel::curated("deepseek-flash", "DeepSeek Flash"),
    CleanupModel::curated("deepseek-v4-pro", "DeepSeek V4 Pro"),
];

pub(super) fn resolve_model(stored: Option<&str>) -> CleanupModel {
    let Some(slug) = stored.map(str::trim).filter(|slug| !slug.is_empty()) else {
        return DEEPSEEK_MODELS[0].clone();
    };
    DEEPSEEK_MODELS
        .iter()
        .find(|model| model.slug == slug)
        .cloned()
        .unwrap_or_else(|| CleanupModel::custom(slug))
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(super) struct Body {
    pub(super) model: String,
    pub(super) messages: [ChatMessage; 2],
    pub(super) stream: bool,
    pub(super) temperature: u8,
    pub(super) thinking: Thinking,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(super) enum Thinking {
    Disabled,
}

pub(super) fn parse_models(json: &Value) -> Result<Vec<CleanupModel>, CleanupError> {
    let entries = json
        .get("data")
        .and_then(Value::as_array)
        .ok_or(CleanupError::Malformed)?;
    Ok(entries
        .iter()
        .filter_map(|entry| {
            let id = entry.get("id").and_then(Value::as_str)?.trim();
            if id.is_empty() {
                return None;
            }
            let model = match entry
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
            {
                Some(name) => CleanupModel::listed(id, name),
                None => resolve_model(Some(id)),
            };
            Some(model)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn saved_models_resolve_without_losing_custom_ids() {
        for stored in [None, Some("  "), Some(" deepseek-flash ")] {
            assert_eq!(resolve_model(stored), DEEPSEEK_MODELS[0]);
        }
        assert_eq!(resolve_model(Some("deepseek-v4-pro")), DEEPSEEK_MODELS[1]);
        assert_eq!(
            resolve_model(Some(" greendale-model ")).slug,
            "greendale-model"
        );
    }

    #[test]
    fn listing_uses_names_when_available_and_skips_missing_ids() {
        let models = parse_models(&json!({"data": [
            {"id": "deepseek-flash"},
            {"id": "greendale-model", "name": "Greendale Model"},
            {"id": "troy-model", "name": "  "},
            {"id": " "},
            {"name": "No ID"}
        ]}))
        .expect("models");
        let names: Vec<_> = models
            .iter()
            .map(|model| model.display_name.as_ref())
            .collect();
        assert_eq!(names, ["DeepSeek Flash", "Greendale Model", "troy-model"]);
        assert_eq!(parse_models(&json!({})), Err(CleanupError::Malformed));
        assert_eq!(
            parse_models(&json!({"data": {}})),
            Err(CleanupError::Malformed)
        );
    }
}
