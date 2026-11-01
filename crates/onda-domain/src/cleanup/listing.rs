//! Listing a provider's models: one `GET` per page.
//!
//! The HTTP client fetches what [`models_request`] describes, hands each
//! `(status, body)` to [`parse_models_page`], follows `next_page_token`, and
//! passes the collected models to [`finish_models`].

use std::collections::HashSet;
use std::fmt;

use serde_json::Value;

use super::catalog::CleanupModel;
use super::fireworks;
use super::openrouter::{self, ApiKey, CleanupError, error_message};
use super::provider::CleanupProvider;

/// The most pages one listing follows, so a provider that keeps returning
/// tokens cannot loop forever.
pub const MAX_MODEL_PAGES: usize = 50;

/// One listing page request: `GET url` with these headers.
///
/// `Debug` redacts the `Authorization` header (CLN-8).
#[derive(Clone, PartialEq, Eq)]
pub struct ModelsRequest {
    /// The page's URL, query included.
    pub url: String,
    /// Header name/value pairs.
    pub headers: Vec<(&'static str, String)>,
}

impl fmt::Debug for ModelsRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.headers.iter().map(|(name, _)| *name).collect();
        f.debug_struct("ModelsRequest")
            .field("url", &self.url)
            .field("headers", &names)
            .finish()
    }
}

/// One parsed page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelsPage {
    /// The page's usable models.
    pub models: Vec<CleanupModel>,
    /// The next page's token; `None` on the last page.
    pub next_page_token: Option<String>,
}

/// The request for one page of `provider`'s models; `None` for a provider
/// with nothing to list. A provider that needs a key to list and has none
/// is [`CleanupError::MissingKey`].
pub fn models_request(
    provider: CleanupProvider,
    key: Option<&ApiKey>,
    page_token: Option<&str>,
) -> Result<Option<ModelsRequest>, CleanupError> {
    let request = match provider {
        CleanupProvider::AppleIntelligence => return Ok(None),
        CleanupProvider::OpenRouter => ModelsRequest {
            url: openrouter::MODELS_ENDPOINT.to_owned(),
            headers: Vec::new(),
        },
        CleanupProvider::Fireworks => {
            let key = key.ok_or(CleanupError::MissingKey)?;
            ModelsRequest {
                url: fireworks::models_url(page_token),
                headers: vec![("Authorization", format!("Bearer {}", key.expose()))],
            }
        }
    };
    Ok(Some(request))
}

/// Reads one page of `provider`'s listing.
pub fn parse_models_page(
    provider: CleanupProvider,
    status: u16,
    body: &[u8],
) -> Result<ModelsPage, CleanupError> {
    if !(200..300).contains(&status) {
        return Err(CleanupError::Http {
            status,
            message: error_message(body),
        });
    }
    let json: Value = serde_json::from_slice(body).map_err(|_| CleanupError::Malformed)?;
    let (models, next_page_token) = match provider {
        CleanupProvider::OpenRouter => (openrouter::parse_models(&json)?, None),
        CleanupProvider::Fireworks => fireworks::parse_models(&json)?,
        CleanupProvider::AppleIntelligence => (Vec::new(), None),
    };
    Ok(ModelsPage {
        models,
        next_page_token,
    })
}

/// Drops repeated slugs, keeping the first, and sorts by name.
pub fn finish_models(models: Vec<CleanupModel>) -> Vec<CleanupModel> {
    let mut seen = HashSet::new();
    let mut models: Vec<CleanupModel> = models
        .into_iter()
        .filter(|model| seen.insert(model.slug.clone()))
        .collect();
    models.sort_by_cached_key(|model| model.display_name.to_lowercase());
    models
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_per_provider() {
        let openrouter = models_request(CleanupProvider::OpenRouter, None, Some("ignored"))
            .expect("request")
            .expect("lists");
        assert_eq!(openrouter.url, "https://openrouter.ai/api/v1/models");
        assert!(openrouter.headers.is_empty(), "OpenRouter's list is public");

        assert_eq!(
            models_request(CleanupProvider::Fireworks, None, None),
            Err(CleanupError::MissingKey),
            "Fireworks lists only with a key"
        );
        let key = ApiKey::new("fw-troy-barnes").expect("key");
        let fireworks = models_request(CleanupProvider::Fireworks, Some(&key), Some("p2"))
            .expect("request")
            .expect("lists");
        assert!(
            fireworks.url.ends_with("&pageToken=p2"),
            "{}",
            fireworks.url
        );
        assert_eq!(
            fireworks.headers,
            [("Authorization", "Bearer fw-troy-barnes".to_owned())]
        );
        assert!(
            !format!("{fireworks:?}").contains("troy"),
            "Debug never prints the key"
        );
        assert_eq!(
            models_request(CleanupProvider::AppleIntelligence, None, None),
            Ok(None),
            "one fixed model"
        );
    }

    #[test]
    fn parse_models_page_reports_http_errors() {
        let got = parse_models_page(
            CleanupProvider::Fireworks,
            401,
            br#"{"error":{"message":"You must provide an API key."}}"#,
        );
        assert_eq!(
            got,
            Err(CleanupError::Http {
                status: 401,
                message: Some("You must provide an API key.".to_owned()),
            })
        );
        assert_eq!(
            parse_models_page(CleanupProvider::OpenRouter, 200, b"nope"),
            Err(CleanupError::Malformed)
        );
    }

    #[test]
    fn finish_models_dedupes_and_sorts_by_name() {
        let got = finish_models(vec![
            CleanupModel::listed("b/zeta", "Zeta"),
            CleanupModel::listed("a/alpha", "alpha"),
            CleanupModel::listed("b/zeta", "Zeta again"),
        ]);
        let names: Vec<&str> = got.iter().map(|m| m.display_name.as_ref()).collect();
        assert_eq!(names, ["alpha", "Zeta"]);
    }
}
