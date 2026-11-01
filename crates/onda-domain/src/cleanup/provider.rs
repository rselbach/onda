//! The services cleanup can run through.

use super::catalog::{self, CLEANUP_MODELS, CleanupModel};
use super::fireworks::{self, FIREWORKS_MODELS};
use super::openrouter;

/// A cleanup provider: where requests go and which models it offers.
///
/// Adding a provider means a variant here, its request body and model
/// listing, its settings key, and its credential-store account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CleanupProvider {
    /// OpenRouter's chat-completions API.
    #[default]
    OpenRouter,
    /// Fireworks AI's serverless chat-completions API.
    Fireworks,
}

impl CleanupProvider {
    /// Every provider, in picker order.
    pub const ALL: [CleanupProvider; 2] = [CleanupProvider::OpenRouter, CleanupProvider::Fireworks];

    /// The stable id stored in `cleanupProvider`.
    pub const fn id(self) -> &'static str {
        match self {
            Self::OpenRouter => "openrouter",
            Self::Fireworks => "fireworks",
        }
    }

    /// The provider with this id.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|provider| provider.id() == id)
    }

    /// The name shown in the "Provider" picker and in copy.
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::OpenRouter => "OpenRouter",
            Self::Fireworks => "Fireworks AI",
        }
    }

    /// The chat endpoint cleanup requests are posted to.
    pub const fn chat_endpoint(self) -> &'static str {
        match self {
            Self::OpenRouter => openrouter::ENDPOINT,
            Self::Fireworks => fireworks::ENDPOINT,
        }
    }

    /// Whether listing the provider's models needs its API key. Without
    /// one, the list is read only while the model sheet shows, which has
    /// already read the key.
    pub const fn lists_models_with_key(self) -> bool {
        match self {
            Self::OpenRouter => false,
            Self::Fireworks => true,
        }
    }

    /// The curated models, in picker order; the first is the default.
    pub fn curated_models(self) -> &'static [CleanupModel] {
        match self {
            Self::OpenRouter => &CLEANUP_MODELS,
            Self::Fireworks => &FIREWORKS_MODELS,
        }
    }

    /// The model used when none is stored.
    pub fn default_model(self) -> &'static CleanupModel {
        &self.curated_models()[0]
    }

    /// The model a stored slug actually uses (CLN-21).
    pub fn resolve_model(self, stored: Option<&str>) -> CleanupModel {
        match self {
            Self::OpenRouter => catalog::resolve_model(stored),
            Self::Fireworks => fireworks::resolve_model(stored),
        }
    }

    /// Whether `slug` is one of the curated models.
    pub fn is_curated(self, slug: &str) -> bool {
        self.curated_models().iter().any(|model| model.slug == slug)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for provider in CleanupProvider::ALL {
            assert_eq!(CleanupProvider::from_id(provider.id()), Some(provider));
        }
        assert_eq!(CleanupProvider::from_id("greendale"), None);
        assert_eq!(CleanupProvider::default(), CleanupProvider::OpenRouter);
    }

    #[test]
    fn openrouter_offers_the_catalog_and_a_model_list() {
        let provider = CleanupProvider::OpenRouter;
        assert_eq!(provider.default_model(), &CLEANUP_MODELS[0]);
        assert!(provider.is_curated("~anthropic/claude-haiku-latest"));
        assert!(!provider.is_curated("openai/gpt-4o"));
        assert!(
            !provider.lists_models_with_key(),
            "OpenRouter's list is public"
        );
        let fireworks = CleanupProvider::Fireworks;
        assert_eq!(fireworks.id(), "fireworks");
        assert_eq!(fireworks.display_name(), "Fireworks AI");
        assert_eq!(fireworks.default_model(), &FIREWORKS_MODELS[0]);
        assert!(fireworks.lists_models_with_key());
    }
}
