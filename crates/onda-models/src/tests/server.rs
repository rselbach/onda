//! A tiny HTTP/1.1 file server for tests: GET only, one request per
//! connection, `Range: bytes=N-` support, a request log, and failure
//! injection. Listens on 127.0.0.1 only; no test touches the network.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// URL path prefix the files are served under.
const PREFIX: &str = "/greendale/";

/// How the server misbehaves for one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Behavior {
    /// Answers every request with HTTP 500.
    Fail,
    /// Serves bytes of the right length but the wrong content.
    Corrupt,
    /// Sends the first `n` body bytes, then hangs.
    StallAfter(usize),
    /// Sends the body in 8 KiB chunks, 10 ms apart.
    Slow,
    /// Ignores Range headers and always sends the whole file with HTTP 200.
    IgnoreRange,
}

/// One request the server answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// File path relative to the base URL.
    pub path: String,
    /// The Range header, if any.
    pub range: Option<String>,
    /// Body bytes the response carried (or was meant to, for a stall).
    pub body_len: usize,
}

#[derive(Default)]
struct State {
    files: HashMap<String, Vec<u8>>,
    behaviors: HashMap<String, Behavior>,
    requests: Vec<Request>,
}

/// A running server; stops with the test runtime.
pub struct Server {
    /// Base URL for a manifest.
    pub base_url: String,
    state: Arc<Mutex<State>>,
}

impl Server {
    /// Starts serving `files` (relative path, content).
    pub async fn start(files: &[(&str, Vec<u8>)]) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("local address");
        let state = Arc::new(Mutex::new(State {
            files: files
                .iter()
                .map(|(path, content)| (path.to_string(), content.clone()))
                .collect(),
            ..State::default()
        }));
        let accept_state = Arc::clone(&state);
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(handle(stream, Arc::clone(&accept_state)));
            }
        });
        Server {
            base_url: format!("http://{address}{}", PREFIX.trim_end_matches('/')),
            state,
        }
    }

    /// Sets or clears the misbehavior for `path`.
    pub fn set_behavior(&self, path: &str, behavior: Option<Behavior>) {
        let mut state = self.state.lock().expect("server state");
        match behavior {
            Some(behavior) => state.behaviors.insert(path.to_string(), behavior),
            None => state.behaviors.remove(path),
        };
    }

    /// Every request answered so far, in order.
    pub fn requests(&self) -> Vec<Request> {
        self.state.lock().expect("server state").requests.clone()
    }

    /// Requests for `path`, in order.
    pub fn requests_for(&self, path: &str) -> Vec<Request> {
        self.requests()
            .into_iter()
            .filter(|r| r.path == path)
            .collect()
    }
}

async fn handle(stream: TcpStream, state: Arc<Mutex<State>>) {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    if reader.read_line(&mut line).await.is_err() {
        return;
    }
    let target = line
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_string();
    let mut range = None;
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) | Err(_) => break,
            Ok(_) if line == "\r\n" => break,
            Ok(_) => {}
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("range")
        {
            range = Some(value.trim().to_string());
        }
    }
    let path = target.strip_prefix(PREFIX).unwrap_or(&target).to_string();

    let (file, behavior) = {
        let state = state.lock().expect("server state");
        (
            state.files.get(&path).cloned(),
            state.behaviors.get(&path).copied(),
        )
    };
    let Some(mut content) = file else {
        respond(
            &mut writer,
            &state,
            &path,
            range,
            "404 Not Found",
            &[],
            &[],
            None,
        )
        .await;
        return;
    };
    if behavior == Some(Behavior::Fail) {
        respond(
            &mut writer,
            &state,
            &path,
            range,
            "500 Internal Server Error",
            &[],
            &[],
            None,
        )
        .await;
        return;
    }
    if behavior == Some(Behavior::Corrupt) {
        content.iter_mut().for_each(|byte| *byte = !*byte);
    }
    let offset = range
        .as_deref()
        .filter(|_| behavior != Some(Behavior::IgnoreRange))
        .and_then(|r| r.strip_prefix("bytes="))
        .and_then(|r| r.strip_suffix('-'))
        .and_then(|r| r.parse::<usize>().ok());
    match offset {
        None => {
            respond(
                &mut writer,
                &state,
                &path,
                range,
                "200 OK",
                &[],
                &content,
                behavior,
            )
            .await;
        }
        Some(offset) if offset >= content.len() => {
            respond(
                &mut writer,
                &state,
                &path,
                range,
                "416 Range Not Satisfiable",
                &[],
                &[],
                None,
            )
            .await;
        }
        Some(offset) => {
            let content_range = format!(
                "Content-Range: bytes {offset}-{}/{}",
                content.len() - 1,
                content.len()
            );
            respond(
                &mut writer,
                &state,
                &path,
                range,
                "206 Partial Content",
                &[content_range],
                &content[offset..],
                behavior,
            )
            .await;
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn respond(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    state: &Mutex<State>,
    path: &str,
    range: Option<String>,
    status: &str,
    headers: &[String],
    body: &[u8],
    behavior: Option<Behavior>,
) {
    state.lock().expect("server state").requests.push(Request {
        path: path.to_string(),
        range,
        body_len: body.len(),
    });
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for header in headers {
        head.push_str(header);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    if writer.write_all(head.as_bytes()).await.is_err() {
        return;
    }
    match behavior {
        Some(Behavior::StallAfter(n)) => {
            if writer.write_all(&body[..n.min(body.len())]).await.is_err() {
                return;
            }
            if writer.flush().await.is_err() {
                return;
            }
            std::future::pending::<()>().await;
        }
        Some(Behavior::Slow) => {
            for chunk in body.chunks(8 * 1024) {
                if writer.write_all(chunk).await.is_err() || writer.flush().await.is_err() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        _ => {
            // a write error means the client went away; dropping the writer
            // closes the connection either way
            writer.write_all(body).await.ok();
        }
    }
}
