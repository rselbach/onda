//! The provider keys and the cleanup transport.

use std::sync::Arc;

use onda_app::ports::OnDeviceModel;
use onda_app::seams::{BoxFuture, CleanupTransport, SecretSource};
use onda_cleanup::CleanupClient;
use onda_domain::cleanup::{
    ApiKey, CleanupError, CleanupModel, CleanupProvider, CleanupRequest, OnDeviceStatus,
    REQUEST_TIMEOUT, on_device_content,
};
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

/// [`CleanupTransport`] for every provider: HTTP providers through the
/// cleanup client, Apple Intelligence through the shell's on-device model.
pub struct CloudCleanup {
    http: CleanupClient,
    on_device: Option<Arc<dyn OnDeviceModel>>,
}

impl CloudCleanup {
    /// Wraps `http`, with the on-device model when the shell has one.
    pub fn new(http: CleanupClient, on_device: Option<Arc<dyn OnDeviceModel>>) -> Self {
        Self { http, on_device }
    }
}

impl CleanupTransport for CloudCleanup {
    fn send(&self, request: CleanupRequest) -> BoxFuture<'static, Result<String, CleanupError>> {
        let Some(prompt) = request.on_device_prompt().cloned() else {
            let client = self.http.clone();
            return Box::pin(async move { client.send(&request).await });
        };
        let model = self.on_device.clone();
        Box::pin(async move {
            let Some(model) = model else {
                return Err(CleanupError::Transport(
                    OnDeviceStatus::DeviceNotEligible
                        .message()
                        .unwrap_or_default()
                        .to_owned(),
                ));
            };
            // the model blocks its thread; a timed-out call finishes there
            // and its answer is dropped (CLN-7)
            let respond = tokio::task::spawn_blocking(move || {
                model.respond(&prompt.instructions, &prompt.prompt)
            });
            match tokio::time::timeout(REQUEST_TIMEOUT, respond).await {
                Ok(Ok(Ok(content))) => Ok(on_device_content(&content)),
                Ok(Ok(Err(error))) => Err(CleanupError::Transport(error)),
                Ok(Err(error)) => Err(CleanupError::Transport(error.to_string())),
                Err(_elapsed) => Err(CleanupError::Transport("The request timed out.".to_owned())),
            }
        })
    }

    fn on_device_status(&self) -> OnDeviceStatus {
        self.on_device
            .as_ref()
            .map_or(OnDeviceStatus::DeviceNotEligible, |model| model.status())
    }

    fn list_models(
        &self,
        provider: CleanupProvider,
        key: Option<ApiKey>,
    ) -> BoxFuture<'static, Result<Vec<CleanupModel>, CleanupError>> {
        let client = self.http.clone();
        Box::pin(async move { client.list_models(provider, key.as_ref()).await })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use onda_domain::cleanup::{APPLE_MODELS, build_request};

    use super::*;

    struct FakeModel {
        status: OnDeviceStatus,
        reply: Result<String, String>,
        prompts: Mutex<Vec<(String, String)>>,
    }

    impl OnDeviceModel for FakeModel {
        fn status(&self) -> OnDeviceStatus {
            self.status
        }

        fn respond(&self, instructions: &str, prompt: &str) -> Result<String, String> {
            self.prompts
                .lock()
                .expect("prompts")
                .push((instructions.to_owned(), prompt.to_owned()));
            self.reply.clone()
        }
    }

    fn transport(model: Option<Arc<FakeModel>>) -> CloudCleanup {
        let http = CleanupClient::new().expect("client");
        CloudCleanup::new(http, model.map(|m| m as Arc<dyn OnDeviceModel>))
    }

    fn on_device_request() -> CleanupRequest {
        build_request(
            CleanupProvider::AppleIntelligence,
            None,
            &APPLE_MODELS[0],
            "um hi abed",
            None,
        )
    }

    #[tokio::test]
    async fn on_device_requests_go_to_the_model_not_http() {
        let model = Arc::new(FakeModel {
            status: OnDeviceStatus::Available,
            reply: Ok("<transcript>\nHi Abed.\n</transcript>\n".to_owned()),
            prompts: Mutex::new(Vec::new()),
        });
        let got = transport(Some(model.clone()))
            .send(on_device_request())
            .await;
        assert_eq!(got, Ok("Hi Abed.".to_owned()), "echoed tags are dropped");
        let prompts = model.prompts.lock().expect("prompts").clone();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].1, "<transcript>\num hi abed\n</transcript>");
    }

    #[tokio::test]
    async fn on_device_failures_are_transport_errors() {
        let model = Arc::new(FakeModel {
            status: OnDeviceStatus::Available,
            reply: Err("Exceeded the model's context window.".to_owned()),
            prompts: Mutex::new(Vec::new()),
        });
        let got = transport(Some(model)).send(on_device_request()).await;
        assert_eq!(
            got,
            Err(CleanupError::Transport(
                "Exceeded the model's context window.".to_owned()
            ))
        );

        let got = transport(None).send(on_device_request()).await;
        assert!(matches!(got, Err(CleanupError::Transport(_))), "{got:?}");
    }

    #[test]
    fn status_without_a_model_is_not_eligible() {
        assert_eq!(
            transport(None).on_device_status(),
            OnDeviceStatus::DeviceNotEligible
        );
        let model = Arc::new(FakeModel {
            status: OnDeviceStatus::NotReady,
            reply: Ok(String::new()),
            prompts: Mutex::new(Vec::new()),
        });
        assert_eq!(
            transport(Some(model)).on_device_status(),
            OnDeviceStatus::NotReady
        );
    }
}
