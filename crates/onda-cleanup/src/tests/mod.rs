mod server;

use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use onda_domain::cleanup::{
    ApiKey, CleanupError, CleanupRequest, REQUEST_TIMEOUT, SYSTEM_PROMPT, TEST_SAMPLE,
    build_request, default_model,
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
    CleanupClient::build(url, reqwest::Client::builder().no_proxy()).expect("client")
}

fn cleanup_request(key: &str) -> CleanupRequest {
    let key = ApiKey::new(key).expect("test key");
    build_request(&key, default_model(), TEST_SAMPLE, Some(APP))
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
        "reasoning": {"effort": model.reasoning_effort.as_str()},
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
        got.map_err(|e| e.to_string()),
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
    let shown = got.as_ref().map_err(ToString::to_string);
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
    let _guard = tracing::subscriber::set_default(subscriber);

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
    let model = default_model().slug;
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

#[test]
fn futures_are_send() {
    fn assert_send<T: Send>(_: &T) {}
    let client = client("http://127.0.0.1:9/");
    let request = cleanup_request(KEY);
    let cancel = CancellationToken::new();
    assert_send(&client.send(&request));
    assert_send(&client.send_cancellable(&request, &cancel));
}
