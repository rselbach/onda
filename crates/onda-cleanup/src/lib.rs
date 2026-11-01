//! OpenRouter HTTP client for transcript cleanup (05, "Cleanup").
//!
//! A thin transport: [`onda_domain::cleanup::build_request`] describes the
//! request and [`onda_domain::cleanup::parse_response`] reads the reply. This
//! crate moves the bytes, caps each request at 20 s (CLN-7), keeps the API key
//! out of errors and logs (CLN-8), and keeps cancellation apart from errors
//! (CLN-9).

use onda_domain::cleanup::{
    CleanupError, CleanupRequest, ENDPOINT, REQUEST_TIMEOUT, parse_response,
};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

#[cfg(test)]
mod tests;

/// Transport message when a request runs past [`REQUEST_TIMEOUT`].
const TIMED_OUT: &str = "The request timed out.";

/// Sends cleanup requests to OpenRouter.
///
/// Cheap to clone; clones share one connection pool.
#[derive(Debug, Clone)]
pub struct CleanupClient {
    http: reqwest::Client,
    endpoint: String,
}

/// The request was cancelled before it finished (CLN-9).
///
/// Not an error and not an attempt: the caller drops the session instead of
/// feeding a result to the outcome flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

impl CleanupClient {
    /// Creates a client for [`ENDPOINT`]. Touches no network.
    pub fn new() -> Result<Self, CleanupError> {
        Self::build(ENDPOINT, reqwest::Client::builder())
    }

    fn build(endpoint: &str, builder: reqwest::ClientBuilder) -> Result<Self, CleanupError> {
        let http = builder
            .user_agent(concat!("Onda/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| {
                CleanupError::Transport(format!(
                    "Could not create the HTTP client: {}",
                    root_cause(error)
                ))
            })?;
        Ok(Self {
            http,
            endpoint: endpoint.to_owned(),
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
        let exchanged = tokio::time::timeout(REQUEST_TIMEOUT, self.exchange(request, body))
            .await
            .unwrap_or_else(|_elapsed| Err(CleanupError::Transport(TIMED_OUT.to_owned())));
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let (status, body) = match exchanged {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(model, latency_ms, %error, "OpenRouter cleanup request failed");
                return Err(error);
            }
        };
        let result = parse_response(status, &body);
        match &result {
            Ok(_) => tracing::info!(model, status, latency_ms, "OpenRouter cleanup finished"),
            Err(error) => {
                tracing::warn!(model, status, latency_ms, %error, "OpenRouter cleanup failed");
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
            tracing::debug!("OpenRouter cleanup request cancelled");
            return Err(Cancelled);
        };
        Ok(result)
    }

    async fn exchange(
        &self,
        request: &CleanupRequest,
        body: Vec<u8>,
    ) -> Result<(u16, Vec<u8>), CleanupError> {
        let response = self
            .http
            .post(&self.endpoint)
            .headers(header_map(request)?)
            .body(body)
            .send()
            .await
            .map_err(transport_error)?;
        let status = response.status().as_u16();
        let body = response.bytes().await.map_err(transport_error)?;
        Ok((status, body.to_vec()))
    }
}

// errors name the header, never its value
fn header_map(request: &CleanupRequest) -> Result<HeaderMap, CleanupError> {
    let mut headers = HeaderMap::with_capacity(request.headers.len());
    for (name, value) in &request.headers {
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
