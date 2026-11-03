//! HTTP client for transcript cleanup and model listing, for every
//! cleanup provider.
//!
//! A thin transport: [`onda_domain::cleanup::build_request`] describes the
//! request and [`onda_domain::cleanup::parse_response`] reads the reply. This
//! crate moves the bytes, caps each request at 20 s (CLN-7), keeps the API key
//! out of errors and logs (CLN-8), and keeps cancellation apart from errors
//! (CLN-9).
//!
//! Requests go only to the provider's fixed HTTPS endpoints: redirects are
//! never followed, so a 3xx cannot replay the transcript, app name, or key
//! to another origin, and response bodies are read up to a size cap.

use onda_domain::cleanup::{
    ApiKey, CleanupError, CleanupModel, CleanupProvider, CleanupRequest, MAX_MODEL_PAGES,
    ModelsRequest, REQUEST_TIMEOUT, finish_models, models_request, parse_models_page,
    parse_response,
};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

#[cfg(test)]
mod tests;

/// Transport message when a request runs past [`REQUEST_TIMEOUT`].
const TIMED_OUT: &str = "The request timed out.";

/// Most bytes read from a cleanup response; a reply holds one transcript.
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// Most bytes read from one page of a model listing. OpenRouter's whole
/// list was about 0.8 MB in November 2026.
const MAX_MODELS_PAGE_BYTES: usize = 16 * 1024 * 1024;

/// Sends cleanup requests and lists models.
///
/// Cheap to clone; clones share one connection pool.
#[derive(Debug, Clone)]
pub struct CleanupClient {
    http: reqwest::Client,
    // tests point every request at a local server
    endpoint: Option<String>,
}

/// The request was cancelled before it finished (CLN-9).
///
/// Not an error and not an attempt: the caller drops the session instead of
/// feeding a result to the outcome flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

impl CleanupClient {
    /// Creates a client that posts each request to its own endpoint over
    /// HTTPS, never following redirects. Touches no network.
    pub fn new() -> Result<Self, CleanupError> {
        Self::build(None, reqwest::Client::builder())
    }

    // only tests pass an endpoint, and only they may use plain HTTP
    fn build(
        endpoint: Option<&str>,
        builder: reqwest::ClientBuilder,
    ) -> Result<Self, CleanupError> {
        let http = builder
            .user_agent(concat!("Onda/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .https_only(endpoint.is_none())
            .build()
            .map_err(|error| {
                CleanupError::Transport(format!(
                    "Could not create the HTTP client: {}",
                    root_cause(error)
                ))
            })?;
        Ok(Self {
            http,
            endpoint: endpoint.map(str::to_owned),
        })
    }

    /// Sends `request` and returns the trimmed cleaned text or why there is
    /// none (CLN-4, CLN-7, CLN-10, CLN-11).
    ///
    /// The whole exchange, from connecting to reading the last body byte, is
    /// capped at [`REQUEST_TIMEOUT`]. Dropping the future aborts the request
    /// and closes its connection.
    pub async fn send(&self, request: &CleanupRequest) -> Result<String, CleanupError> {
        let started = Instant::now();
        let body = match serde_json::to_vec(&request.body) {
            Ok(body) => body,
            Err(error) => return Err(CleanupError::Transport(error.to_string())),
        };
        let model = model_slug(&body);
        let provider = request.provider.id();
        let exchanged = tokio::time::timeout(REQUEST_TIMEOUT, self.exchange(request, body))
            .await
            .unwrap_or_else(|_elapsed| Err(CleanupError::Transport(TIMED_OUT.to_owned())));
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let (status, body) = match exchanged {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(provider, model, latency_ms, %error, "cleanup request failed");
                return Err(error);
            }
        };
        let result = parse_response(status, &body);
        match &result {
            Ok(_) => tracing::info!(provider, model, status, latency_ms, "cleanup finished"),
            Err(error) => {
                tracing::warn!(provider, model, status, latency_ms, %error, "cleanup failed");
            }
        }
        result
    }

    /// Like [`send`](Self::send), but cancelling `cancel` aborts the request
    /// and returns [`Cancelled`] instead of a result (CLN-9).
    pub async fn send_cancellable(
        &self,
        request: &CleanupRequest,
        cancel: &CancellationToken,
    ) -> Result<Result<String, CleanupError>, Cancelled> {
        let Some(result) = cancel.run_until_cancelled(self.send(request)).await else {
            tracing::debug!("cleanup request cancelled");
            return Err(Cancelled);
        };
        Ok(result)
    }

    /// Fetches every page of `provider`'s models, each page capped at
    /// [`REQUEST_TIMEOUT`], at most [`MAX_MODEL_PAGES`] pages. `key` is
    /// sent only to providers that list with it.
    pub async fn list_models(
        &self,
        provider: CleanupProvider,
        key: Option<&ApiKey>,
    ) -> Result<Vec<CleanupModel>, CleanupError> {
        let result = self.list_pages(provider, key).await;
        let provider_id = provider.id();
        match &result {
            Ok(models) => {
                tracing::info!(
                    provider = provider_id,
                    count = models.len(),
                    "listed cleanup models"
                );
            }
            Err(error) => {
                tracing::warn!(provider = provider_id, %error, "could not list cleanup models");
            }
        }
        result
    }

    async fn list_pages(
        &self,
        provider: CleanupProvider,
        key: Option<&ApiKey>,
    ) -> Result<Vec<CleanupModel>, CleanupError> {
        let mut models = Vec::new();
        let mut token: Option<String> = None;
        for _ in 0..MAX_MODEL_PAGES {
            let Some(request) = models_request(provider, key, token.as_deref())? else {
                break;
            };
            let (status, body) = tokio::time::timeout(REQUEST_TIMEOUT, self.fetch(&request))
                .await
                .unwrap_or_else(|_elapsed| Err(CleanupError::Transport(TIMED_OUT.to_owned())))?;
            let page = parse_models_page(provider, status, &body)?;
            models.extend(page.models);
            token = page.next_page_token;
            if token.is_none() {
                break;
            }
        }
        Ok(finish_models(models))
    }

    async fn fetch(&self, request: &ModelsRequest) -> Result<(u16, Vec<u8>), CleanupError> {
        // tests swap the origin and path but keep the page's query
        let url = match &self.endpoint {
            Some(endpoint) => match request.url.split_once('?') {
                Some((_, query)) => format!("{endpoint}?{query}"),
                None => endpoint.clone(),
            },
            None => request.url.clone(),
        };
        let response = self
            .http
            .get(url)
            .headers(header_map(&request.headers)?)
            .send()
            .await
            .map_err(transport_error)?;
        let status = response.status().as_u16();
        Ok((status, read_body(response, MAX_MODELS_PAGE_BYTES).await?))
    }

    async fn exchange(
        &self,
        request: &CleanupRequest,
        body: Vec<u8>,
    ) -> Result<(u16, Vec<u8>), CleanupError> {
        let Some(url) = self
            .endpoint
            .as_deref()
            .or(request.provider.chat_endpoint())
        else {
            return Err(CleanupError::Transport(
                "This provider is not reached over HTTP.".to_owned(),
            ));
        };
        let response = self
            .http
            .post(url)
            .headers(header_map(&request.headers)?)
            .body(body)
            .send()
            .await
            .map_err(transport_error)?;
        let status = response.status().as_u16();
        Ok((status, read_body(response, MAX_RESPONSE_BYTES).await?))
    }
}

// streams the body so a declared, chunked, or close-delimited length past
// `cap` fails before more than `cap` bytes are held
async fn read_body(mut response: reqwest::Response, cap: usize) -> Result<Vec<u8>, CleanupError> {
    let too_large = || {
        CleanupError::Transport(format!(
            "The response was larger than {} MB.",
            cap / (1024 * 1024)
        ))
    };
    let declared = response.content_length().unwrap_or(0);
    if declared > cap as u64 {
        return Err(too_large());
    }
    let mut body = Vec::with_capacity(declared as usize);
    while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
        if chunk.len() > cap - body.len() {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

// errors name the header, never its value
fn header_map(pairs: &[(&'static str, String)]) -> Result<HeaderMap, CleanupError> {
    let mut headers = HeaderMap::with_capacity(pairs.len());
    for (name, value) in pairs {
        let invalid = |error: &dyn std::error::Error| {
            CleanupError::Transport(format!("Invalid {name} header: {error}"))
        };
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|e| invalid(&e))?;
        let mut header_value = HeaderValue::from_str(value).map_err(|e| invalid(&e))?;
        header_value.set_sensitive(header_name == AUTHORIZATION);
        headers.insert(header_name, header_value);
    }
    Ok(headers)
}

fn transport_error(error: reqwest::Error) -> CleanupError {
    CleanupError::Transport(root_cause(error))
}

// the innermost cause reads best in a notice ("Connection refused (os error
// 61)"); reqwest's own text adds the URL and little else
fn root_cause(error: reqwest::Error) -> String {
    let error = error.without_url();
    let mut cause: &dyn std::error::Error = &error;
    while let Some(source) = cause.source() {
        cause = source;
    }
    cause.to_string()
}

#[derive(Deserialize)]
struct ModelField {
    model: String,
}

// RequestBody is opaque, so read the slug back from the serialized body
fn model_slug(body: &[u8]) -> String {
    serde_json::from_slice::<ModelField>(body).map_or_else(|_| String::new(), |field| field.model)
}
