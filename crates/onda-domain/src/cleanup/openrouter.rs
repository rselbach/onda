//! OpenRouter chat-completions request and response shapes (CLN-4..CLN-11).
//!
//! The HTTP client lives outside the domain; it sends what
//! [`build_request`] describes and hands `(status, body)` to
//! [`parse_response`].

use std::collections::HashSet;
use std::fmt;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;

use super::catalog::CleanupModel;
use super::prompt::SYSTEM_PROMPT;

/// Chat-completions endpoint (CLN-4).
pub const ENDPOINT: &str = "https://openrouter.ai/api/v1/chat/completions";

/// The public model list; needs no key.
pub const MODELS_ENDPOINT: &str = "https://openrouter.ai/api/v1/models";

/// Total and idle timeout for one cleanup request (CLN-7).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Longest `error.message` kept from a non-2xx body, in characters (CLN-11).
const MAX_ERROR_MESSAGE_CHARS: usize = 200;

const TRANSCRIPT_OPEN: &str = "<transcript>";
const TRANSCRIPT_CLOSE: &str = "</transcript>";

/// A non-blank OpenRouter API key, trimmed.
///
/// `Debug` never prints the key (CLN-8).
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    /// Trims `raw`; a blank value is [`CleanupError::MissingKey`].
    pub fn new(raw: &str) -> Result<Self, CleanupError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(CleanupError::MissingKey);
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// The key itself, for the `Authorization` header only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(<redacted>)")
    }
}

/// Why a cleanup request produced no content (CLN-11).
///
/// `Display` gives the exact user-facing message; none of them contain the
/// API key (CLN-8). Cancellation is not an error (CLN-9) and has no variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupError {
    /// No key is stored, or it is blank.
    MissingKey,
    /// A non-2xx status, with `error.message` from the body when present,
    /// truncated to 200 characters.
    Http {
        /// HTTP status code.
        status: u16,
        /// OpenRouter's error message, if the body had one.
        message: Option<String>,
    },
    /// The body was not JSON, had the wrong shape, or lacked
    /// `choices[0].message.content`.
    Malformed,
    /// `choices` was missing or empty.
    Empty,
    /// The transport failed; carries the transport's description.
    Transport(String),
}

impl fmt::Display for CleanupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingKey => f.write_str("OpenRouter API key is not set."),
            Self::Http {
                status,
                message: Some(message),
            } => write!(f, "OpenRouter returned HTTP {status}: {message}"),
            Self::Http {
                status,
                message: None,
            } => write!(f, "OpenRouter returned HTTP {status}."),
            Self::Malformed => f.write_str("OpenRouter response could not be parsed."),
            Self::Empty => f.write_str("OpenRouter returned an empty response."),
            Self::Transport(error) => write!(f, "OpenRouter request failed: {error}"),
        }
    }
}

impl std::error::Error for CleanupError {}

/// One cleanup request, ready for an HTTP client: `POST` to `endpoint`
/// with [`REQUEST_TIMEOUT`], these headers, and `body` serialized as JSON.
///
/// `Debug` redacts the `Authorization` header (CLN-8).
#[derive(Clone, PartialEq)]
pub struct CleanupRequest {
    /// Where to post the request.
    pub endpoint: &'static str,
    /// Header name/value pairs (CLN-4).
    pub headers: Vec<(&'static str, String)>,
    /// Body; serialize it with `serde_json` (CLN-5).
    pub body: RequestBody,
}

/// The chat-completions body (CLN-5). Opaque; only serialized.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RequestBody {
    model: String,
    messages: [ChatMessage; 2],
    stream: bool,
    temperature: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<Reasoning>,
    provider: ProviderPreferences,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct ChatMessage {
    role: &'static str,
    content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct Reasoning {
    effort: &'static str,
}

// sort by latency because someone is waiting; zdr so dictation only reaches
// providers that keep nothing
#[derive(Debug, Clone, PartialEq, Serialize)]
struct ProviderPreferences {
    sort: &'static str,
    zdr: bool,
}

impl fmt::Debug for CleanupRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(name, value)| match *name {
                "Authorization" => (*name, "<redacted>"),
                _ => (*name, value.as_str()),
            })
            .collect();
        f.debug_struct("CleanupRequest")
            .field("endpoint", &self.endpoint)
            .field("headers", &headers)
            .field("body", &self.body)
            .finish()
    }
}

/// Builds the request that cleans `transcript` (untrimmed) with `model`,
/// naming the target app when known (CLN-4..CLN-6).
pub fn build_request(
    key: &ApiKey,
    model: &CleanupModel,
    transcript: &str,
    app_name: Option<&str>,
) -> CleanupRequest {
    let headers = vec![
        ("Content-Type", "application/json".to_owned()),
        ("Authorization", format!("Bearer {}", key.expose())),
        ("X-Title", "Onda".to_owned()),
        (
            "HTTP-Referer",
            "https://rselbach.github.io/onda/".to_owned(),
        ),
    ];
    let body = RequestBody {
        model: model.slug.clone().into_owned(),
        messages: [
            ChatMessage {
                role: "system",
                content: SYSTEM_PROMPT.to_owned(),
            },
            ChatMessage {
                role: "user",
                content: user_message(transcript, app_name),
            },
        ],
        stream: false,
        temperature: 0,
        reasoning: model.reasoning_effort.map(|effort| Reasoning {
            effort: effort.as_str(),
        }),
        provider: ProviderPreferences {
            sort: "latency",
            zdr: true,
        },
    };
    CleanupRequest {
        endpoint: ENDPOINT,
        headers,
        body,
    }
}

/// The user message: the transcript fenced in `<transcript>` tags, preceded
/// by `Dictating into: <app>` when the app is known and named (CLN-6).
pub fn user_message(transcript: &str, app_name: Option<&str>) -> String {
    let fenced = format!("{TRANSCRIPT_OPEN}\n{transcript}\n{TRANSCRIPT_CLOSE}");
    match app_name {
        Some(app) if !app.is_empty() => format!("Dictating into: {app}\n\n{fenced}"),
        _ => fenced,
    }
}

/// Turns an HTTP response into trimmed cleaned text or a [`CleanupError`]
/// (CLN-10, CLN-11).
///
/// Empty content is a valid result (CLN-12), not an error.
pub fn parse_response(status: u16, body: &[u8]) -> Result<String, CleanupError> {
    if !(200..300).contains(&status) {
        return Err(CleanupError::Http {
            status,
            message: error_message(body),
        });
    }
    let json: Value = serde_json::from_slice(body).map_err(|_| CleanupError::Malformed)?;
    let Some(object) = json.as_object() else {
        return Err(CleanupError::Malformed);
    };
    // a `choices` that is not a list of objects counts as missing, as in
    // Jabber
    let choices = object
        .get("choices")
        .and_then(Value::as_array)
        .filter(|choices| choices.iter().all(Value::is_object));
    let Some(first) = choices.and_then(|choices| choices.first()) else {
        return Err(CleanupError::Empty);
    };
    let content = first
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .ok_or(CleanupError::Malformed)?;
    Ok(unwrap_transcript_tags(content))
}

/// Turns a [`MODELS_ENDPOINT`] response into the models that write text,
/// sorted by name. Entries without an id are skipped; a missing name falls
/// back to the id.
pub fn parse_models_response(status: u16, body: &[u8]) -> Result<Vec<CleanupModel>, CleanupError> {
    if !(200..300).contains(&status) {
        return Err(CleanupError::Http {
            status,
            message: error_message(body),
        });
    }
    let json: Value = serde_json::from_slice(body).map_err(|_| CleanupError::Malformed)?;
    let entries = json
        .get("data")
        .and_then(Value::as_array)
        .ok_or(CleanupError::Malformed)?;
    let mut seen = HashSet::new();
    let mut models: Vec<CleanupModel> = entries
        .iter()
        .filter(|entry| writes_text(entry) && serves_chat(entry))
        .filter_map(|entry| {
            let id = entry.get("id").and_then(Value::as_str)?.trim();
            if id.is_empty() || !seen.insert(id) {
                return None;
            }
            let name = entry
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or(id);
            Some(CleanupModel::listed(id, name))
        })
        .collect();
    models.sort_by_cached_key(|model| model.display_name.to_lowercase());
    Ok(models)
}

// an entry that does not say what it outputs is assumed to write text
fn writes_text(entry: &Value) -> bool {
    entry
        .get("architecture")
        .and_then(|architecture| architecture.get("output_modalities"))
        .and_then(Value::as_array)
        .is_none_or(|modalities| modalities.iter().any(|m| m.as_str() == Some("text")))
}

// batch-only entries list no endpoints; one that does not say is assumed
// to serve chat completions
fn serves_chat(entry: &Value) -> bool {
    entry
        .get("endpoints")
        .and_then(Value::as_array)
        .is_none_or(|endpoints| {
            endpoints
                .iter()
                .any(|e| e.as_str() == Some("/api/v1/chat/completions"))
        })
}

// CLN-10: drop <transcript> tags a model echoed, and always trim
fn unwrap_transcript_tags(content: &str) -> String {
    let trimmed = content.trim();
    trimmed
        .strip_prefix(TRANSCRIPT_OPEN)
        .and_then(|rest| rest.strip_suffix(TRANSCRIPT_CLOSE))
        .map_or(trimmed, str::trim)
        .to_owned()
}

fn error_message(body: &[u8]) -> Option<String> {
    let json: Value = serde_json::from_slice(body).ok()?;
    let message = json.get("error")?.get("message")?.as_str()?;
    Some(
        message
            .graphemes(true)
            .take(MAX_ERROR_MESSAGE_CHARS)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::cleanup::catalog::{CLEANUP_MODELS, default_model};

    const KEY: &str = "sk-test-greendale-abc";

    fn key() -> ApiKey {
        ApiKey::new(KEY).expect("test key")
    }

    #[test]
    fn api_key_trims_and_rejects_blank_cln_2() {
        struct Case {
            name: &'static str,
            raw: &'static str,
            want: Result<&'static str, CleanupError>,
        }
        let cases = [
            Case {
                name: "empty",
                raw: "",
                want: Err(CleanupError::MissingKey),
            },
            Case {
                name: "whitespace",
                raw: "   \n  ",
                want: Err(CleanupError::MissingKey),
            },
            Case {
                name: "plain key",
                raw: KEY,
                want: Ok(KEY),
            },
            Case {
                name: "padded key is trimmed",
                raw: "  sk-troy-barnes\n",
                want: Ok("sk-troy-barnes"),
            },
        ];
        for tc in cases {
            let got = ApiKey::new(tc.raw);
            assert_eq!(
                got.as_ref().map(ApiKey::expose),
                tc.want.as_ref().map(|k| *k),
                "{}",
                tc.name
            );
        }
    }

    #[test]
    fn debug_output_never_leaks_the_key_cln_8() {
        let request = build_request(&key(), default_model(), "hello", None);
        let printed = format!("{:?} {:?}", key(), request);
        assert!(
            !printed.contains(KEY),
            "debug output leaked the key: {printed}"
        );
    }

    #[test]
    fn build_request_shape_cln_4_cln_5() {
        let model = &CLEANUP_MODELS[1];
        let request = build_request(&key(), model, "señor chang", None);
        assert_eq!(request.endpoint, ENDPOINT, "endpoint");
        let want_headers = vec![
            ("Content-Type", "application/json".to_owned()),
            ("Authorization", format!("Bearer {KEY}")),
            ("X-Title", "Onda".to_owned()),
            (
                "HTTP-Referer",
                "https://rselbach.github.io/onda/".to_owned(),
            ),
        ];
        assert_eq!(request.headers, want_headers, "headers");
        let want_body = json!({
            "model": "~anthropic/claude-haiku-latest",
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": "<transcript>\nseñor chang\n</transcript>"},
            ],
            "stream": false,
            "temperature": 0,
            "reasoning": {"effort": "none"},
            "provider": {"sort": "latency", "zdr": true},
        });
        let body = serde_json::to_value(&request.body).expect("serialize body");
        assert_eq!(body, want_body, "body");
        assert_eq!(REQUEST_TIMEOUT, Duration::from_secs(20), "CLN-7 timeout");
    }

    #[test]
    fn models_outside_the_catalog_send_no_reasoning_setting() {
        // a model with mandatory reasoning rejects effort "none"
        let model = CleanupModel::custom("openai/gpt-6.1-terra-pro");
        let request = build_request(&key(), &model, "pop pop", None);
        let body = serde_json::to_value(&request.body).expect("serialize body");
        assert_eq!(body.get("reasoning"), None, "{body}");
        assert_eq!(body["model"], "openai/gpt-6.1-terra-pro");
    }

    #[test]
    fn user_message_cln_6() {
        struct Case {
            name: &'static str,
            transcript: &'static str,
            app: Option<&'static str>,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "with app",
                transcript: "hi troy",
                app: Some("Mail"),
                want: "Dictating into: Mail\n\n<transcript>\nhi troy\n</transcript>",
            },
            Case {
                name: "without app",
                transcript: "hi troy",
                app: None,
                want: "<transcript>\nhi troy\n</transcript>",
            },
            Case {
                name: "empty app name",
                transcript: "hi troy",
                app: Some(""),
                want: "<transcript>\nhi troy\n</transcript>",
            },
            Case {
                name: "transcript is not trimmed",
                transcript: " um hello ",
                app: None,
                want: "<transcript>\n um hello \n</transcript>",
            },
        ];
        for tc in cases {
            assert_eq!(user_message(tc.transcript, tc.app), tc.want, "{}", tc.name);
        }
    }

    #[test]
    fn parse_response_cln_10_cln_11() {
        let long = "a".repeat(600);
        let long_body = format!(r#"{{"error":{{"message":"{long}"}}}}"#);
        struct Case<'a> {
            name: &'static str,
            status: u16,
            body: &'a str,
            want: Result<&'a str, CleanupError>,
        }
        let cases = [
            Case {
                name: "content is returned",
                status: 200,
                body: r#"{"choices":[{"message":{"content":"Hello."}}]}"#,
                want: Ok("Hello."),
            },
            Case {
                name: "content is trimmed",
                status: 200,
                body: r#"{"choices":[{"message":{"content":"  Hello.\n"}}]}"#,
                want: Ok("Hello."),
            },
            Case {
                name: "empty content is valid",
                status: 200,
                body: r#"{"choices":[{"message":{"content":""}}]}"#,
                want: Ok(""),
            },
            Case {
                name: "whitespace content is valid and empty",
                status: 200,
                body: r#"{"choices":[{"message":{"content":"  \n "}}]}"#,
                want: Ok(""),
            },
            Case {
                name: "echoed tags are dropped",
                status: 200,
                body: r#"{"choices":[{"message":{"content":"<transcript>\nHi Troy.\n</transcript>"}}]}"#,
                want: Ok("Hi Troy."),
            },
            Case {
                name: "echoed tags with surrounding whitespace are dropped",
                status: 200,
                body: r#"{"choices":[{"message":{"content":"\n <transcript>Hi Troy.</transcript> "}}]}"#,
                want: Ok("Hi Troy."),
            },
            Case {
                name: "a tag mentioned inside stays",
                status: 200,
                body: r#"{"choices":[{"message":{"content":"Type <transcript> here."}}]}"#,
                want: Ok("Type <transcript> here."),
            },
            Case {
                name: "only the first choice is used",
                status: 200,
                body: r#"{"choices":[{"message":{"content":"one"}},{"message":{"content":"two"}}]}"#,
                want: Ok("one"),
            },
            Case {
                name: "2xx other than 200 is success",
                status: 201,
                body: r#"{"choices":[{"message":{"content":"ok"}}]}"#,
                want: Ok("ok"),
            },
            Case {
                name: "non-2xx with a flat error has no message",
                status: 429,
                body: r#"{"error":"rate limited"}"#,
                want: Err(CleanupError::Http {
                    status: 429,
                    message: None,
                }),
            },
            Case {
                name: "non-2xx with a non-JSON body has no message",
                status: 502,
                body: "<html>bad gateway</html>",
                want: Err(CleanupError::Http {
                    status: 502,
                    message: None,
                }),
            },
            Case {
                name: "non-2xx carries the API message",
                status: 402,
                body: r#"{"error":{"message":"No more credits, Troy."}}"#,
                want: Err(CleanupError::Http {
                    status: 402,
                    message: Some("No more credits, Troy.".to_owned()),
                }),
            },
            Case {
                name: "non-2xx message is truncated to 200 characters",
                status: 500,
                body: &long_body,
                want: Err(CleanupError::Http {
                    status: 500,
                    message: Some("a".repeat(200)),
                }),
            },
            Case {
                name: "3xx is not success",
                status: 302,
                body: "",
                want: Err(CleanupError::Http {
                    status: 302,
                    message: None,
                }),
            },
            Case {
                name: "garbage is malformed",
                status: 200,
                body: "not json at all",
                want: Err(CleanupError::Malformed),
            },
            Case {
                name: "a JSON array is malformed",
                status: 200,
                body: "[]",
                want: Err(CleanupError::Malformed),
            },
            Case {
                name: "missing content is malformed",
                status: 200,
                body: r#"{"choices":[{"message":{}}]}"#,
                want: Err(CleanupError::Malformed),
            },
            Case {
                name: "null content is malformed",
                status: 200,
                body: r#"{"choices":[{"message":{"content":null}}]}"#,
                want: Err(CleanupError::Malformed),
            },
            Case {
                name: "missing message is malformed",
                status: 200,
                body: r#"{"choices":[{}]}"#,
                want: Err(CleanupError::Malformed),
            },
            Case {
                name: "empty choices is empty",
                status: 200,
                body: r#"{"choices":[]}"#,
                want: Err(CleanupError::Empty),
            },
            Case {
                name: "missing choices is empty",
                status: 200,
                body: r#"{"id":"gen-greendale"}"#,
                want: Err(CleanupError::Empty),
            },
        ];
        for tc in cases {
            let got = parse_response(tc.status, tc.body.as_bytes());
            assert_eq!(got, tc.want.map(str::to_owned), "{}", tc.name);
        }
    }

    #[test]
    fn parse_models_response_keeps_text_models_sorted_by_name() {
        let body = json!({"data": [
            {"id": "openai/gpt-4o", "name": "OpenAI: GPT-4o",
             "architecture": {"output_modalities": ["text"]}},
            {"id": "acme/painter", "name": "Acme: Painter",
             "architecture": {"output_modalities": ["image"]}},
            {"id": "anthropic/claude-haiku", "name": "Anthropic: Claude Haiku"},
            {"id": "greendale/unnamed"},
            {"id": "openai/gpt-4o", "name": "Duplicate"},
            {"id": "openai/gpt-4o:batch", "name": "OpenAI: GPT-4o (batch)", "endpoints": []},
            {"id": "acme/responses-only", "name": "Acme: Responses",
             "endpoints": ["/api/v1/responses"]},
            {"id": "x-ai/grok", "name": "xAI: Grok",
             "endpoints": ["/api/v1/chat/completions", "/api/v1/responses"]},
            {"name": "No id"},
            {"id": "  "},
        ]})
        .to_string();
        let got = parse_models_response(200, body.as_bytes()).expect("models");
        let got: Vec<(&str, &str)> = got
            .iter()
            .map(|m| (m.slug.as_ref(), m.display_name.as_ref()))
            .collect();
        let want = vec![
            ("anthropic/claude-haiku", "Anthropic: Claude Haiku"),
            ("greendale/unnamed", "greendale/unnamed"),
            ("openai/gpt-4o", "OpenAI: GPT-4o"),
            ("x-ai/grok", "xAI: Grok"),
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn parse_models_response_errors() {
        struct Case {
            name: &'static str,
            status: u16,
            body: &'static str,
            want: CleanupError,
        }
        let cases = [
            Case {
                name: "non-2xx carries the API message",
                status: 503,
                body: r#"{"error":{"message":"Down for paintball."}}"#,
                want: CleanupError::Http {
                    status: 503,
                    message: Some("Down for paintball.".to_owned()),
                },
            },
            Case {
                name: "garbage is malformed",
                status: 200,
                body: "not json",
                want: CleanupError::Malformed,
            },
            Case {
                name: "missing data is malformed",
                status: 200,
                body: r#"{"models":[]}"#,
                want: CleanupError::Malformed,
            },
        ];
        for tc in cases {
            let got = parse_models_response(tc.status, tc.body.as_bytes());
            assert_eq!(got, Err(tc.want), "{}", tc.name);
        }
    }

    #[test]
    fn error_messages_cln_11() {
        struct Case {
            name: &'static str,
            error: CleanupError,
            want: &'static str,
        }
        let cases = [
            Case {
                name: "no key",
                error: CleanupError::MissingKey,
                want: "OpenRouter API key is not set.",
            },
            Case {
                name: "http with message",
                error: CleanupError::Http {
                    status: 429,
                    message: Some("Slow down, Abed.".to_owned()),
                },
                want: "OpenRouter returned HTTP 429: Slow down, Abed.",
            },
            Case {
                name: "http without message",
                error: CleanupError::Http {
                    status: 429,
                    message: None,
                },
                want: "OpenRouter returned HTTP 429.",
            },
            Case {
                name: "malformed",
                error: CleanupError::Malformed,
                want: "OpenRouter response could not be parsed.",
            },
            Case {
                name: "empty",
                error: CleanupError::Empty,
                want: "OpenRouter returned an empty response.",
            },
            Case {
                name: "transport",
                error: CleanupError::Transport("connection reset".to_owned()),
                want: "OpenRouter request failed: connection reset",
            },
        ];
        for tc in cases {
            assert_eq!(tc.error.to_string(), tc.want, "{}", tc.name);
        }
    }
}
