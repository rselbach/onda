//! The OpenRouter key and transport.

use onda_app::seams::{BoxFuture, CleanupTransport, SecretSource};
use onda_cleanup::CleanupClient;
use onda_domain::cleanup::{CleanupError, CleanupRequest};
use onda_settings::SecretStore;

/// [`SecretSource`] over any [`SecretStore`] (the Keychain in the app).
pub struct Keychain<S>(S);

impl<S: SecretStore> Keychain<S> {
    /// Wraps `store`.
    pub fn new(store: S) -> Self {
        Self(store)
    }
}

impl<S: SecretStore + Send + Sync> SecretSource for Keychain<S> {
    fn openrouter_key(&self) -> Result<Option<String>, String> {
        self.0
            .read()
            .map(|key| key.map(|key| key.expose().to_owned()))
            .map_err(|error| error.to_string())
    }

    fn save_openrouter_key(&self, value: &str) -> Result<(), String> {
        self.0.save(value).map_err(|error| error.to_string())
    }
}

/// [`CleanupTransport`] over the OpenRouter client.
pub struct OpenRouter(CleanupClient);

impl OpenRouter {
    /// Wraps `client`.
    pub fn new(client: CleanupClient) -> Self {
        Self(client)
    }
}

impl CleanupTransport for OpenRouter {
    fn send(&self, request: CleanupRequest) -> BoxFuture<'static, Result<String, CleanupError>> {
        let client = self.0.clone();
        Box::pin(async move { client.send(&request).await })
    }
}
