//! The provider keys and the OpenRouter transport.

use onda_app::seams::{BoxFuture, CleanupTransport, SecretSource};
use onda_cleanup::CleanupClient;
use onda_domain::cleanup::{ApiKey, CleanupError, CleanupModel, CleanupProvider, CleanupRequest};
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
    fn api_key(&self, provider: CleanupProvider) -> Result<Option<String>, String> {
        self.0
            .read(provider)
            .map(|key| key.map(|key| key.expose().to_owned()))
            .map_err(|error| error.to_string())
    }

    fn save_api_key(&self, provider: CleanupProvider, value: &str) -> Result<(), String> {
        self.0
            .save(provider, value)
            .map_err(|error| error.to_string())
    }
}

/// [`CleanupTransport`] over the cleanup HTTP client, for every provider.
pub struct CloudCleanup(CleanupClient);

impl CloudCleanup {
    /// Wraps `client`.
    pub fn new(client: CleanupClient) -> Self {
        Self(client)
    }
}

impl CleanupTransport for CloudCleanup {
    fn send(&self, request: CleanupRequest) -> BoxFuture<'static, Result<String, CleanupError>> {
        let client = self.0.clone();
        Box::pin(async move { client.send(&request).await })
    }

    fn list_models(
        &self,
        provider: CleanupProvider,
        key: Option<ApiKey>,
    ) -> BoxFuture<'static, Result<Vec<CleanupModel>, CleanupError>> {
        let client = self.0.clone();
        Box::pin(async move { client.list_models(provider, key.as_ref()).await })
    }
}
