mod server;

use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use onda_domain::cleanup::{
    ApiKey, CleanupError, CleanupPrompt, CleanupProvider, CleanupRequest, REQUEST_TIMEOUT,
    SYSTEM_PROMPT, TEST_SAMPLE, build_request, default_model,
};
use serde_json::json;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use self::server::{Reply, Server};
use super::*;

const KEY: &str = "sk-or-v1-greendale-human-beings-0451";
const APP: &str = "Greendale Mail";
const CLEANED: &str = "Tell Troy and Abed the study group meets at four in the library.";

// guards a test that would otherwise hang on a regression
const GUARD: Duration = Duration::from_secs(5);

fn client(url: &str) -> CleanupClient {
    // tests talk to 127.0.0.1; keep proxy settings from intercepting
    CleanupClient::build(Some(url), reqwest::Client::builder().no_proxy()).expect("client")
}

fn cleanup_request(key: &str) -> CleanupRequest {
    let key = ApiKey::new(key).expect("test key");
    build_request(
        CleanupProvider::OpenRouter,
        Some(&key),
        default_model(),
        &CleanupPrompt::default(),
        TEST_SAMPLE,
        Some(APP),
    )
}

fn success_body() -> String {
    json!({"choices": [{"message": {"content": format!("<transcript>\n{CLEANED}\n</transcript>")}}]})
        .to_string()
}

async fn refused_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("local address");
    drop(listener);
    format!("http://{address}{}", server::PATH)
}

#[tokio::test]
async fn request_on_the_wire_cln_4_cln_5_cln_6() {
    let mut server = Server::start(Reply::Respond {
        status: 200,
        body: success_body(),
    })
    .await;
    let request = cleanup_request(KEY);

    let got = client(&server.url).send(&request).await;
    assert_eq!(got, Ok(CLEANED.to_owned()));

    let captured = server.captured().await;
    assert_eq!(
        captured.request_line,
        format!("POST {} HTTP/1.1", server::PATH)
    );
    let want_headers = [
        ("content-type", "application/json".to_owned()),
        ("authorization", format!("Bearer {KEY}")),
        ("x-title", "Onda".to_owned()),
        (
            "http-referer",
            "https://rselbach.github.io/onda/".to_owned(),
        ),
    ];
    for (name, want) in want_headers {
        assert_eq!(captured.header(name), [want.as_str()], "{name}");
    }
    let want_bytes = serde_json::to_vec(&request.body).expect("serialize body");
    assert_eq!(captured.body, want_bytes, "body bytes");
    let model = default_model();
    let want_body = json!({
        "model": model.slug,
        "messages": [
            {"role": "system", "content": SYSTEM_PROMPT},
            {"role": "user", "content": format!("Dictating into: {APP}\n\n<transcript>\n{TEST_SAMPLE}\n</transcript>")},
        ],
        "stream": false,
        "temperature": 0,
        "reasoning": {"effort": "none"},
        "provider": {"sort": "latency", "zdr": true},
    });
    let body: serde_json::Value = serde_json::from_slice(&captured.body).expect("JSON body");
    assert_eq!(body, want_body, "body");
}

#[tokio::test]
async fn responses_go_through_parse_response_cln_10_cln_11() {
    struct Case {
        name: &'static str,
        status: u16,
        body: String,
        want: Result<String, CleanupError>,
    }
    let cases = [
        Case {
            name: "200 returns the cleaned text",
            status: 200,
            body: success_body(),
            want: Ok(CLEANED.to_owned()),
        },
        Case {
            name: "other 2xx is success",
            status: 201,
            body: success_body(),
            want: Ok(CLEANED.to_owned()),
        },
        Case {
            name: "empty content is a valid result",
            status: 200,
            body: r#"{"choices":[{"message":{"content":"  "}}]}"#.to_owned(),
            want: Ok(String::new()),
        },
        Case {
            name: "non-2xx with error.message",
            status: 402,
            body: r#"{"error":{"message":"Pierce spent the credits."}}"#.to_owned(),
            want: Err(CleanupError::Http {
                status: 402,
                message: Some("Pierce spent the credits.".to_owned()),
            }),
        },
        Case {
            name: "non-2xx without error.message",
            status: 503,
            body: "<html>Greendale is closed</html>".to_owned(),
            want: Err(CleanupError::Http {
                status: 503,
                message: None,
            }),
        },
        Case {
            name: "garbage body",
            status: 200,
            body: "streets ahead".to_owned(),
            want: Err(CleanupError::Malformed),
        },
        Case {
            name: "empty choices",
            status: 200,
            body: r#"{"choices":[]}"#.to_owned(),
            want: Err(CleanupError::Empty),
        },
    ];
    for tc in cases {
        let server = Server::start(Reply::Respond {
            status: tc.status,
            body: tc.body,
        })
        .await;
        let got = client(&server.url).send(&cleanup_request(KEY)).await;
        assert_eq!(got, tc.want, "{}", tc.name);
    }
}

#[tokio::test]
async fn times_out_after_twenty_seconds_cln_7() {
    let mut server = Server::start(Reply::Hang).await;
    let client = client(&server.url);
    let started = Instant::now();
    let task = tokio::spawn(async move { client.send(&cleanup_request(KEY)).await });
    // pause only once the request is on the server: a paused clock skips
    // ahead whenever the runtime idles, even mid-connect
    server.captured().await;
    tokio::time::pause();

    let got = task.await.expect("send task");

    let elapsed = started.elapsed();
    assert!(
        (REQUEST_TIMEOUT..REQUEST_TIMEOUT + Duration::from_secs(1)).contains(&elapsed),
        "timed out after {elapsed:?}"
    );
    assert_eq!(
        got.map_err(|e| e.describe(CleanupProvider::OpenRouter)),
        Err("OpenRouter request failed: The request timed out.".to_owned())
    );
    server.closed().await;
}

#[tokio::test]
async fn dropping_the_future_aborts_the_request_cln_9() {
    let mut server = Server::start(Reply::Hang).await;
    let client = client(&server.url);
    let task = tokio::spawn(async move { client.send(&cleanup_request(KEY)).await });
    server.captured().await;

    task.abort();

    let joined = task.await.expect_err("task was aborted");
    assert!(joined.is_cancelled());
    tokio::time::timeout(GUARD, server.closed())
        .await
        .expect("connection closed");
}

#[tokio::test]
async fn cancelling_the_token_aborts_the_request_cln_9() {
    let mut server = Server::start(Reply::Hang).await;
    let client = client(&server.url);
    let cancel = CancellationToken::new();
    let task = tokio::spawn({
        let cancel = cancel.clone();
        async move {
            client
                .send_cancellable(&cleanup_request(KEY), &cancel)
                .await
        }
    });
    server.captured().await;

    cancel.cancel();

    let got = tokio::time::timeout(GUARD, task)
        .await
        .expect("cancel returns promptly")
        .expect("task finished");
    assert_eq!(got, Err(Cancelled));
    tokio::time::timeout(GUARD, server.closed())
        .await
        .expect("connection closed");
}

#[tokio::test]
async fn token_result_passes_through_when_not_cancelled() {
    let server = Server::start(Reply::Respond {
        status: 200,
        body: success_body(),
    })
    .await;
    let got = client(&server.url)
        .send_cancellable(&cleanup_request(KEY), &CancellationToken::new())
        .await;
    assert_eq!(got, Ok(Ok(CLEANED.to_owned())));
}

#[tokio::test]
async fn connection_refused_is_a_transport_error_cln_11() {
    let got = client(&refused_url().await)
        .send(&cleanup_request(KEY))
        .await;

    let Err(CleanupError::Transport(message)) = &got else {
        panic!("want a transport error, got {got:?}");
    };
    assert!(
        message.to_lowercase().contains("connection refused"),
        "message: {message}"
    );
    assert!(!message.contains("127.0.0.1"), "URL leaked: {message}");
    let shown = got
        .as_ref()
        .map_err(|error| error.describe(CleanupProvider::OpenRouter));
    assert_eq!(shown, Err(format!("OpenRouter request failed: {message}")));
}

#[tokio::test]
async fn invalid_key_bytes_fail_without_echoing_the_key_cln_8() {
    // ApiKey trims only the ends, so an inner newline reaches the header
    let got = client(&refused_url().await)
        .send(&cleanup_request("sk-or-troy\nbarnes"))
        .await;
    assert_eq!(
        got,
        Err(CleanupError::Transport(
            "Invalid Authorization header: failed to parse header value".to_owned()
        ))
    );
}

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl io::Write for LogBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("log buffer").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn key_transcript_and_response_stay_out_of_errors_and_logs_cln_8_log_2() {
    let logs = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer({
            let logs = logs.clone();
            move || logs.clone()
        })
        .finish();
    // parallel tests and spawned HTTP tasks share callsites, so they must
    // see the same subscriber when those callsites first register
    tracing::subscriber::set_global_default(subscriber).expect("install the test logger");

    let mut printed = Vec::new();
    let replies = [
        (200, success_body()),
        (
            401,
            r#"{"error":{"message":"No auth credentials found"}}"#.to_owned(),
        ),
        (200, "streets ahead".to_owned()),
    ];
    for (status, body) in replies {
        let server = Server::start(Reply::Respond { status, body }).await;
        let client = client(&server.url);
        let got = client.send(&cleanup_request(KEY)).await;
        printed.push(format!("{got:?} {client:?}"));
        if let Err(error) = got {
            printed.push(error.to_string());
        }
    }
    let refused = client(&refused_url().await)
        .send(&cleanup_request(KEY))
        .await;
    let bad_key = "sk-or-troy\nbarnes";
    let invalid = client(&refused_url().await)
        .send(&cleanup_request(bad_key))
        .await;
    for got in [refused, invalid] {
        let error = got.expect_err("request failed");
        printed.push(format!("{error} {error:?}"));
    }
    printed.push(format!("{:?}", cleanup_request(KEY)));

    let logs = String::from_utf8(logs.0.lock().expect("log buffer").clone()).expect("UTF-8 logs");
    let model = default_model().slug.as_ref();
    assert!(logs.contains(model), "logs name the model: {logs}");
    assert!(logs.contains("status=200"), "logs carry the status: {logs}");
    assert!(logs.contains("latency_ms="), "logs carry latency: {logs}");
    for secret in [KEY, "sk-or-troy", "barnes", TEST_SAMPLE, CLEANED] {
        assert!(!logs.contains(secret), "logs leaked {secret:?}: {logs}");
    }
    for text in &printed {
        assert!(!text.contains(KEY), "leaked the key: {text}");
        assert!(!text.contains("sk-or-troy"), "leaked the bad key: {text}");
    }
}

#[tokio::test]
async fn list_models_gets_and_parses_the_list() {
    let body = json!({"data": [
        {"id": "openai/gpt-4o", "name": "OpenAI: GPT-4o"},
        {"id": "anthropic/claude-haiku", "name": "Anthropic: Claude Haiku"},
    ]})
    .to_string();
    let mut server = Server::start(Reply::Respond { status: 200, body }).await;

    let client = client(&server.url);
    let listing = client.list_models(CleanupProvider::OpenRouter, None);
    let got = tokio::time::timeout(GUARD, listing)
        .await
        .expect("listing finished");
    let slugs: Vec<String> = got
        .expect("models")
        .into_iter()
        .map(|m| m.slug.into_owned())
        .collect();
    assert_eq!(slugs, ["anthropic/claude-haiku", "openai/gpt-4o"]);
    let captured = server.captured().await;
    assert_eq!(
        captured.request_line,
        format!("GET {} HTTP/1.1", server::PATH)
    );
    assert!(
        captured.header("authorization").is_empty(),
        "no key is sent"
    );
}

#[tokio::test]
async fn list_models_reports_transport_errors() {
    let got = client(&refused_url().await)
        .list_models(CleanupProvider::OpenRouter, None)
        .await;
    assert!(
        matches!(got, Err(CleanupError::Transport(_))),
        "refused connection: {got:?}"
    );
}

#[tokio::test]
async fn fireworks_listing_sends_the_key_and_follows_pages() {
    let page = |models: serde_json::Value, next: &str| {
        json!({"models": models, "nextPageToken": next}).to_string()
    };
    let chat = |name: &str, display: &str| {
        json!({"name": name, "displayName": display,
               "supportsServerless": true, "conversationConfig": {"style": "x"}})
    };
    let (url, served) = Server::start_sequence(vec![
        (
            200,
            page(
                json!([chat("accounts/fireworks/models/kimi-k3", "Kimi K3")]),
                "p2",
            ),
        ),
        (
            200,
            page(
                json!([chat(
                    "accounts/fireworks/models/glm-5p3-flash",
                    "GLM 5.3 Flash"
                )]),
                "",
            ),
        ),
    ])
    .await;
    let key = ApiKey::new("fw-greendale-human-beings").expect("key");

    let client = client(&url);
    let listing = client.list_models(CleanupProvider::Fireworks, Some(&key));
    let got = tokio::time::timeout(GUARD, listing)
        .await
        .expect("listing finished")
        .expect("models");
    let names: Vec<&str> = got.iter().map(|m| m.display_name.as_ref()).collect();
    assert_eq!(names, ["GLM 5.3 Flash", "Kimi K3"], "both pages, sorted");

    let captured = tokio::time::timeout(GUARD, served)
        .await
        .expect("server finished")
        .expect("server task");
    assert_eq!(captured.len(), 2, "one request per page");
    for request in &captured {
        assert_eq!(
            request.header("authorization"),
            ["Bearer fw-greendale-human-beings"]
        );
    }
    assert!(
        captured[0]
            .request_line
            .contains("filter=supports_serverless%3Dtrue")
            && !captured[0].request_line.contains("pageToken"),
        "{}",
        captured[0].request_line
    );
    assert!(
        captured[1].request_line.contains("&pageToken=p2 "),
        "{}",
        captured[1].request_line
    );
}

#[tokio::test]
async fn fireworks_listing_needs_a_key() {
    // nothing listens: a request would be a transport error instead
    let got = client("http://127.0.0.1:9/")
        .list_models(CleanupProvider::Fireworks, None)
        .await;
    assert_eq!(got, Err(CleanupError::MissingKey));
}

#[test]
fn futures_are_send() {
    fn assert_send<T: Send>(_: &T) {}
    let client = client("http://127.0.0.1:9/");
    let request = cleanup_request(KEY);
    let cancel = CancellationToken::new();
    assert_send(&client.send(&request));
    assert_send(&client.send_cancellable(&request, &cancel));
    assert_send(&client.list_models(CleanupProvider::OpenRouter, None));
}

fn redirect_to(status: u16, location: &str) -> Reply {
    Reply::Raw {
        response: format!(
            "HTTP/1.1 {status} Moved\r\nLocation: {location}\r\nContent-Length: 0\r\n\
             Connection: close\r\n\r\n"
        )
        .into_bytes(),
        then_hang: false,
    }
}

#[tokio::test]
async fn redirects_are_refused_not_replayed_to_another_origin() {
    for status in [301, 302, 303, 307, 308] {
        // another port is another origin
        let mut elsewhere = Server::start(Reply::Respond {
            status: 200,
            body: success_body(),
        })
        .await;
        let server = Server::start(redirect_to(status, &elsewhere.url)).await;

        let got = tokio::time::timeout(GUARD, client(&server.url).send(&cleanup_request(KEY)))
            .await
            .expect("send finished");

        assert_eq!(
            got,
            Err(CleanupError::Http {
                status,
                message: None
            }),
            "{status}"
        );
        assert!(!elsewhere.was_reached(), "{status} replayed the transcript");
    }
}

#[tokio::test]
async fn listing_redirects_are_refused() {
    let mut elsewhere = Server::start(Reply::Respond {
        status: 200,
        body: json!({"data": []}).to_string(),
    })
    .await;
    let server = Server::start(redirect_to(307, &elsewhere.url)).await;
    let key = ApiKey::new("fw-greendale-human-beings").expect("key");

    let client = client(&server.url);
    let listing = client.list_models(CleanupProvider::Fireworks, Some(&key));
    let got = tokio::time::timeout(GUARD, listing)
        .await
        .expect("listing finished");

    assert_eq!(
        got,
        Err(CleanupError::Http {
            status: 307,
            message: None
        })
    );
    assert!(!elsewhere.was_reached(), "the key went to another origin");
}

#[tokio::test]
async fn production_client_refuses_plain_http() {
    let mut server = Server::start(Reply::Respond {
        status: 200,
        body: success_body(),
    })
    .await;
    let production = CleanupClient::new().expect("client");

    let got = production
        .http
        .post(&server.url)
        .body("Troy and Abed")
        .send()
        .await;

    let error = got.expect_err("plain HTTP is refused");
    assert!(error.is_builder(), "refused before connecting: {error:?}");
    assert!(!server.was_reached(), "the request went out");
}

fn chunked(total: usize) -> Vec<u8> {
    let mut response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    let piece = vec![b'x'; 64 * 1024];
    let mut left = total;
    while left > 0 {
        let size = left.min(piece.len());
        response.extend_from_slice(format!("{size:x}\r\n").as_bytes());
        response.extend_from_slice(&piece[..size]);
        response.extend_from_slice(b"\r\n");
        left -= size;
    }
    response.extend_from_slice(b"0\r\n\r\n");
    response
}

fn with_head(head: &str, body_len: usize) -> Vec<u8> {
    let mut response = head.as_bytes().to_vec();
    response.resize(head.len() + body_len, b'x');
    response
}

#[tokio::test]
async fn response_bodies_are_capped() {
    let too_large = Err(CleanupError::Transport(
        "The response was larger than 4 MB.".to_owned(),
    ));
    let over = MAX_RESPONSE_BYTES + 1;
    let cases = [
        (
            // the body never comes: the declared length alone is refused
            "declared length over the cap",
            Reply::Raw {
                response: format!("HTTP/1.1 200 OK\r\nContent-Length: {over}\r\n\r\n").into_bytes(),
                then_hang: true,
            },
            too_large.clone(),
        ),
        (
            "chunked body over the cap",
            Reply::Raw {
                response: chunked(over),
                then_hang: false,
            },
            too_large.clone(),
        ),
        (
            "close-delimited body over the cap",
            Reply::Raw {
                response: with_head("HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n", over),
                then_hang: false,
            },
            too_large.clone(),
        ),
        (
            "a short declared length bounds the read",
            Reply::Raw {
                response: with_head("HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n", over),
                then_hang: false,
            },
            Err(CleanupError::Malformed),
        ),
        (
            "chunked body at the cap is read",
            Reply::Raw {
                response: chunked(MAX_RESPONSE_BYTES),
                then_hang: false,
            },
            Err(CleanupError::Malformed),
        ),
    ];
    for (name, reply, want) in cases {
        let server = Server::start(reply).await;
        let got = tokio::time::timeout(GUARD, client(&server.url).send(&cleanup_request(KEY)))
            .await
            .unwrap_or_else(|_| panic!("{name}: send finished"));
        assert_eq!(got, want, "{name}");
    }
}

#[tokio::test]
async fn listing_pages_are_capped() {
    let server = Server::start(Reply::Raw {
        response: chunked(MAX_MODELS_PAGE_BYTES + 1),
        then_hang: false,
    })
    .await;

    let client = client(&server.url);
    let listing = client.list_models(CleanupProvider::OpenRouter, None);
    let got = tokio::time::timeout(GUARD, listing)
        .await
        .expect("listing finished");

    assert_eq!(
        got,
        Err(CleanupError::Transport(
            "The response was larger than 16 MB.".to_owned()
        ))
    );
}

#[tokio::test]
async fn cancelling_mid_body_aborts_the_request_cln_9() {
    let mut server = Server::start(Reply::Raw {
        response: b"HTTP/1.1 200 OK\r\nContent-Length: 4096\r\n\r\n{\"choices\":".to_vec(),
        then_hang: true,
    })
    .await;
    let client = client(&server.url);
    let cancel = CancellationToken::new();
    let task = tokio::spawn({
        let cancel = cancel.clone();
        async move {
            client
                .send_cancellable(&cleanup_request(KEY), &cancel)
                .await
        }
    });
    server.captured().await;

    cancel.cancel();

    let got = tokio::time::timeout(GUARD, task)
        .await
        .expect("cancel returns promptly")
        .expect("task finished");
    assert_eq!(got, Err(Cancelled));
    tokio::time::timeout(GUARD, server.closed())
        .await
        .expect("connection closed");
}
