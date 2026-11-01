//! The services cleanup can run through.

use super::catalog::{self, APPLE_MODELS, CLEANUP_MODELS, CleanupModel};
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
    /// Apple Intelligence's on-device model, through the shell; offered
    /// only when the Mac supports it.
    AppleIntelligence,
}

/// Whether Apple Intelligence's on-device model can run now, as the OS
/// reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OnDeviceStatus {
    /// Ready to use.
    Available,
    /// This Mac cannot run Apple Intelligence, or the OS has no model API;
    /// the provider is not offered.
    DeviceNotEligible,
    /// Apple Intelligence is turned off in System Settings.
    NotEnabled,
    /// The model is still downloading or otherwise not ready.
    NotReady,
}

impl OnDeviceStatus {
    /// What the model sheet says when the model cannot run; `None` when it
    /// can.
    pub fn message(self) -> Option<&'static str> {
        match self {
            Self::Available => None,
            Self::DeviceNotEligible => Some("This Mac doesn't support Apple Intelligence."),
            Self::NotEnabled => {
                Some("Turn on Apple Intelligence in System Settings to use it for post-processing.")
            }
            Self::NotReady => {
                Some("Apple Intelligence is still getting ready. Try again in a few minutes.")
            }
        }
    }
}

/// Whether, and how, a provider lists its models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelListing {
    /// One fixed model; nothing to list.
    None,
    /// A public list.
    Public,
    /// A list that needs the API key, so it is read only while the model
    /// sheet shows, which has already read the key.
    WithKey,
}

impl CleanupProvider {
    /// Every provider, in picker order.
    pub const ALL: [CleanupProvider; 3] = [
        CleanupProvider::OpenRouter,
        CleanupProvider::Fireworks,
        CleanupProvider::AppleIntelligence,
    ];

    /// The stable id stored in `cleanupProvider`.
    pub const fn id(self) -> &'static str {
        match self {
            Self::OpenRouter => "openrouter",
            Self::Fireworks => "fireworks",
            Self::AppleIntelligence => "apple-intelligence",
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
            Self::AppleIntelligence => "Apple Intelligence",
        }
    }

    /// The chat endpoint cleanup requests are posted to; `None` for the
    /// on-device model.
    pub const fn chat_endpoint(self) -> Option<&'static str> {
        match self {
            Self::OpenRouter => Some(openrouter::ENDPOINT),
            Self::Fireworks => Some(fireworks::ENDPOINT),
            Self::AppleIntelligence => None,
        }
    }

    /// Whether requests need an API key from the credential store.
    pub const fn uses_key(self) -> bool {
        match self {
            Self::OpenRouter | Self::Fireworks => true,
            Self::AppleIntelligence => false,
        }
    }

    /// Whether the model runs on this Mac rather than over the network.
    pub const fn on_device(self) -> bool {
        matches!(self, Self::AppleIntelligence)
    }

    /// How the provider's models are listed.
    pub const fn model_listing(self) -> ModelListing {
        match self {
            Self::OpenRouter => ModelListing::Public,
            Self::Fireworks => ModelListing::WithKey,
            Self::AppleIntelligence => ModelListing::None,
        }
    }

    /// The curated models, in picker order; the first is the default.
    pub fn curated_models(self) -> &'static [CleanupModel] {
        match self {
            Self::OpenRouter => &CLEANUP_MODELS,
            Self::Fireworks => &FIREWORKS_MODELS,
            Self::AppleIntelligence => &APPLE_MODELS,
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
            // one model; any stored value means it
            Self::AppleIntelligence => APPLE_MODELS[0].clone(),
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
        assert_eq!(provider.model_listing(), ModelListing::Public);
        assert!(provider.uses_key());
        let fireworks = CleanupProvider::Fireworks;
        assert_eq!(fireworks.id(), "fireworks");
        assert_eq!(fireworks.display_name(), "Fireworks AI");
        assert_eq!(fireworks.default_model(), &FIREWORKS_MODELS[0]);
        assert_eq!(fireworks.model_listing(), ModelListing::WithKey);

        let apple = CleanupProvider::AppleIntelligence;
        assert_eq!(apple.id(), "apple-intelligence");
        assert_eq!(apple.chat_endpoint(), None);
        assert!(!apple.uses_key());
        assert!(apple.on_device());
        assert_eq!(apple.model_listing(), ModelListing::None);
        assert_eq!(
            apple.resolve_model(Some("anything")).slug,
            "on-device",
            "one fixed model"
        );
    }
}
