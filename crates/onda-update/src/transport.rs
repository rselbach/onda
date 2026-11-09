//! HTTPS for the updater: the bounded feed request and resumable archive
//! downloads checked against the signed size and SHA-256 before anything
//! uses them (UPD-5). Redirects are followed only to the hosts
//! [`Origins`] allows, and only over HTTPS; the signed hash, not the host,
//! is what makes a download trustworthy.

use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{CONTENT_RANGE, RANGE};
use reqwest::redirect::Policy;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::feed::{Artifact, MAX_FEED_BYTES, Origins};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_REDIRECTS: usize = 5;
/// Requests per download before it gives up; each resumes the last.
const ATTEMPTS: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_millis(if cfg!(test) { 10 } else { 2000 });

/// Why a transfer failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferError {
    /// The network or server failed; trying again later may work.
    Network(String),
    /// The bytes are not the signed ones; the partial file is gone.
    Mismatch(String),
    /// The local disk failed.
    Io(String),
}

impl fmt::Display for TransferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(why) | Self::Mismatch(why) | Self::Io(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for TransferError {}

/// The updater's HTTP client.
#[derive(Clone)]
pub struct Transport {
    client: reqwest::Client,
    origins: Arc<Origins>,
}

impl Transport {
    /// A client that follows redirects only where `origins` allows.
    pub fn new(origins: Origins, user_agent: &str) -> Result<Self, String> {
        let origins = Arc::new(origins);
        let policy_origins = origins.clone();
        let policy = Policy::custom(move |attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.error("too many redirects");
            }
            if policy_origins.allows_redirect(attempt.url()) {
                return attempt.follow();
            }
            let refused = format!("refused a redirect to {}", attempt.url());
            attempt.error(refused)
        });
        let client = reqwest::Client::builder()
            .user_agent(user_agent)
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .redirect(policy)
            .https_only(origins.https_only)
            .build()
            .map_err(|error| format!("could not create the update client: {error}"))?;
        Ok(Self { client, origins })
    }

    /// Fetches the feed, refusing anything over [`MAX_FEED_BYTES`].
    pub async fn fetch_feed(&self, url: &str) -> Result<Vec<u8>, TransferError> {
        let network = |error: reqwest::Error| TransferError::Network(describe(&error));
        let mut response = self.client.get(url).send().await.map_err(network)?;
        if response.status() != StatusCode::OK {
            return Err(TransferError::Network(format!(
                "the update feed answered HTTP {}",
                response.status()
            )));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(network)? {
            if body.len() + chunk.len() > MAX_FEED_BYTES {
                return Err(TransferError::Mismatch(
                    "the update feed is too large".to_owned(),
                ));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    /// Downloads `artifact` into `dir` as `<sha256>.download`, resuming a
    /// partial file from an earlier attempt, and returns the verified file.
    /// `progress` hears the bytes on disk. A finished file that does not
    /// match is deleted, so the next attempt starts over.
    pub async fn download(
        &self,
        artifact: &Artifact,
        dir: &Path,
        progress: &(dyn Fn(u64) + Send + Sync),
    ) -> Result<PathBuf, TransferError> {
        if !self.origins.allows_artifact(&artifact.url) {
            return Err(TransferError::Mismatch(format!(
                "refused to download {}",
                artifact.url
            )));
        }
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|error| io_error("create", dir, &error))?;
        let part = dir.join(format!("{}.download", artifact.sha256));
        let mut last = None;
        for attempt in 1..=ATTEMPTS {
            if attempt > 1 {
                tokio::time::sleep(RETRY_DELAY).await;
            }
            match self.fetch_rest(artifact, &part, progress).await {
                Ok(()) => {
                    last = None;
                    break;
                }
                Err(TransferError::Network(why)) => {
                    tracing::warn!(attempt, %why, "update download interrupted");
                    last = Some(TransferError::Network(why));
                }
                Err(error) => return Err(discard(&part, error).await),
            }
        }
        if let Some(error) = last {
            return Err(error);
        }
        let checked = part.clone();
        let want = artifact.clone();
        let verdict = tokio::task::spawn_blocking(move || verify_file(&checked, &want))
            .await
            .map_err(|error| TransferError::Io(format!("could not check the update: {error}")))?;
        match verdict {
            Ok(()) => Ok(part),
            Err(error) => Err(discard(&part, error).await),
        }
    }

    /// One request for whatever `part` still lacks.
    async fn fetch_rest(
        &self,
        artifact: &Artifact,
        part: &Path,
        progress: &(dyn Fn(u64) + Send + Sync),
    ) -> Result<(), TransferError> {
        let mut offset = tokio::fs::metadata(part).await.map_or(0, |meta| meta.len());
        if offset > artifact.size {
            offset = 0;
        }
        if offset == artifact.size {
            return Ok(());
        }
        let mut request = self.client.get(&artifact.url);
        if offset > 0 {
            tracing::info!(offset, size = artifact.size, "resuming the update download");
            request = request.header(RANGE, format!("bytes={offset}-"));
        }
        let network = |error: reqwest::Error| TransferError::Network(describe(&error));
        let mut response = request.send().await.map_err(network)?;
        match response.status() {
            StatusCode::PARTIAL_CONTENT if offset > 0 => {
                let resumed = response
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|range| range.starts_with(&format!("bytes {offset}-")));
                if !resumed {
                    return Err(TransferError::Mismatch(
                        "the server resumed the update at the wrong offset".to_owned(),
                    ));
                }
            }
            StatusCode::OK => offset = 0,
            status if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS => {
                return Err(TransferError::Network(format!(
                    "the update download answered HTTP {status}"
                )));
            }
            status => {
                return Err(TransferError::Mismatch(format!(
                    "the update download answered HTTP {status}"
                )));
            }
        }
        let opened = match offset {
            0 => tokio::fs::File::create(part).await,
            _ => tokio::fs::OpenOptions::new().append(true).open(part).await,
        };
        let mut out = opened.map_err(|error| io_error("open", part, &error))?;
        let mut written = offset;
        progress(written);
        loop {
            let chunk = match response.chunk().await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(error) => {
                    // keep what landed so the next request resumes it
                    out.flush()
                        .await
                        .map_err(|error| io_error("write", part, &error))?;
                    return Err(network(error));
                }
            };
            if written + chunk.len() as u64 > artifact.size {
                return Err(TransferError::Mismatch(format!(
                    "the server sent more than the {} bytes the feed signed",
                    artifact.size
                )));
            }
            out.write_all(&chunk)
                .await
                .map_err(|error| io_error("write", part, &error))?;
            written += chunk.len() as u64;
            progress(written);
        }
        out.flush()
            .await
            .map_err(|error| io_error("write", part, &error))?;
        if written < artifact.size {
            return Err(TransferError::Network(format!(
                "the connection closed after {written} of {} bytes",
                artifact.size
            )));
        }
        Ok(())
    }
}

/// Checks `path` against the artifact's signed size and SHA-256.
pub fn verify_file(path: &Path, artifact: &Artifact) -> Result<(), TransferError> {
    let size = std::fs::metadata(path)
        .map_err(|error| io_error("read", path, &error))?
        .len();
    if size != artifact.size {
        return Err(TransferError::Mismatch(format!(
            "the update has {size} bytes; the feed signed {}",
            artifact.size
        )));
    }
    let digest = sha256_file(path).map_err(|error| io_error("read", path, &error))?;
    if digest != artifact.sha256 {
        return Err(TransferError::Mismatch(
            "the update's SHA-256 does not match the feed".to_owned(),
        ));
    }
    Ok(())
}

/// Lowercase hex SHA-256 of a file.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

async fn discard(part: &Path, error: TransferError) -> TransferError {
    if let Err(remove) = tokio::fs::remove_file(part).await
        && remove.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(error = %remove, path = %part.display(), "could not delete a bad update download");
    }
    error
}

fn io_error(action: &str, path: &Path, error: &std::io::Error) -> TransferError {
    TransferError::Io(format!("could not {action} {}: {error}", path.display()))
}

/// The error and its causes on one line; reqwest's own text omits them.
fn describe(error: &reqwest::Error) -> String {
    let mut text = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Mutex;

    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
    use tokio::net::TcpListener;

    use super::*;

    /// How the test server answers one path.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Behavior {
        Serve,
        /// The right length, the wrong bytes.
        Corrupt,
        /// Closes the connection after this many body bytes.
        CutAfter(usize),
        /// Redirects to the given path on the same server.
        Redirect(&'static str),
        Fail,
    }

    #[derive(Default)]
    struct State {
        files: Vec<(String, Vec<u8>, Behavior)>,
        requests: Vec<(String, Option<String>)>,
    }

    /// A tiny HTTP/1.1 server on 127.0.0.1: GET, `Range: bytes=N-`, one
    /// request per connection, and failure injection.
    #[derive(Clone)]
    pub(crate) struct Server {
        pub(crate) base: String,
        state: Arc<Mutex<State>>,
    }

    impl Server {
        pub(crate) async fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let state = Arc::new(Mutex::new(State::default()));
            let accept = state.clone();
            tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    tokio::spawn(answer(stream, accept.clone()));
                }
            });
            Self { base, state }
        }

        pub(crate) fn put(&self, path: &str, body: Vec<u8>, behavior: Behavior) {
            let mut state = self.state.lock().unwrap();
            state.files.retain(|(p, _, _)| p != path);
            state.files.push((path.to_owned(), body, behavior));
        }

        pub(crate) fn replace_body(&self, path: &str, body: Vec<u8>) {
            let mut state = self.state.lock().unwrap();
            if let Some(file) = state.files.iter_mut().find(|(p, _, _)| p == path) {
                file.1 = body;
            }
        }

        pub(crate) fn behave(&self, path: &str, behavior: Behavior) {
            let mut state = self.state.lock().unwrap();
            if let Some(file) = state.files.iter_mut().find(|(p, _, _)| p == path) {
                file.2 = behavior;
            }
        }

        pub(crate) fn requests(&self, path: &str) -> Vec<Option<String>> {
            let state = self.state.lock().unwrap();
            state
                .requests
                .iter()
                .filter(|(p, _)| p == path)
                .map(|(_, range)| range.clone())
                .collect()
        }

        pub(crate) fn url(&self, path: &str) -> String {
            format!("{}{path}", self.base)
        }

        pub(crate) fn origins(&self) -> Origins {
            Origins {
                artifact_prefix: format!("{}/releases/", self.base),
                redirect_hosts: vec!["127.0.0.1".to_owned()],
                https_only: false,
            }
        }
    }

    async fn answer(stream: tokio::net::TcpStream, state: Arc<Mutex<State>>) {
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        if reader.read_line(&mut line).await.is_err() {
            return;
        }
        let path = line
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_owned();
        let mut range = None;
        loop {
            line.clear();
            if reader.read_line(&mut line).await.unwrap_or(0) == 0 || line.trim().is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("range")
            {
                range = Some(value.trim().to_owned());
            }
        }
        let found = {
            let mut state = state.lock().unwrap();
            state.requests.push((path.clone(), range.clone()));
            state
                .files
                .iter()
                .find(|(p, _, _)| *p == path)
                .map(|(_, body, behavior)| (body.clone(), *behavior))
        };
        let Some((mut body, behavior)) = found else {
            let _ = writer
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
            return;
        };
        match behavior {
            Behavior::Fail => {
                let _ = writer
                    .write_all(
                        b"HTTP/1.1 500 Oops\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await;
                return;
            }
            Behavior::Redirect(to) => {
                let head = format!(
                    "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = writer.write_all(head.as_bytes()).await;
                return;
            }
            Behavior::Corrupt => body.iter_mut().for_each(|byte| *byte ^= 0xff),
            Behavior::Serve | Behavior::CutAfter(_) => {}
        }
        let total = body.len();
        let start = range
            .as_deref()
            .and_then(|r| r.strip_prefix("bytes="))
            .and_then(|r| r.strip_suffix('-'))
            .and_then(|r| r.parse::<usize>().ok())
            .filter(|start| *start < total);
        let (status, slice, extra) = match start {
            Some(start) => (
                "206 Partial Content",
                &body[start..],
                format!("Content-Range: bytes {start}-{}/{total}\r\n", total - 1),
            ),
            None => ("200 OK", &body[..], String::new()),
        };
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
            slice.len()
        );
        let _ = writer.write_all(head.as_bytes()).await;
        if let Behavior::CutAfter(n) = behavior {
            // close mid-body: the client sees the connection drop
            let _ = writer.write_all(&slice[..n.min(slice.len())]).await;
            let _ = writer.shutdown().await;
            return;
        }
        let _ = writer.write_all(slice).await;
        let _ = writer.flush().await;
        // drain so the client sees a clean close
        let mut rest = Vec::new();
        let _ = reader.read_to_end(&mut rest).await;
    }

    pub(crate) fn artifact_for(server: &Server, path: &str, body: &[u8]) -> Artifact {
        Artifact {
            os: crate::OS.to_owned(),
            arch: crate::ARCH.to_owned(),
            kind: crate::feed::KIND_APP_ZIP.to_owned(),
            minimum_os: "26.0".to_owned(),
            url: server.url(path),
            size: body.len() as u64,
            sha256: Sha256::digest(body)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        }
    }

    fn body() -> Vec<u8> {
        (0..200_000u32).map(|i| (i % 251) as u8).collect()
    }

    #[tokio::test]
    async fn a_cut_download_resumes_from_its_partial_file() {
        let server = Server::start().await;
        let dir = tempfile::tempdir().unwrap();
        let path = "/releases/v2/Onda.zip";
        // every response stops 50 000 bytes in: three attempts fall short
        server.put(path, body(), Behavior::CutAfter(50_000));
        let artifact = artifact_for(&server, path, &body());
        let transport = Transport::new(server.origins(), "Onda/test").unwrap();
        let first = transport.download(&artifact, dir.path(), &|_| {}).await;
        assert!(matches!(first, Err(TransferError::Network(_))), "{first:?}");

        server.behave(path, Behavior::Serve);
        let file = transport
            .download(&artifact, dir.path(), &|_| {})
            .await
            .unwrap();
        assert_eq!(std::fs::read(file).unwrap(), body());
        let ranges = server.requests(path);
        let want = [
            None,
            Some("bytes=50000-"),
            Some("bytes=100000-"),
            Some("bytes=150000-"),
        ];
        assert_eq!(
            ranges.iter().map(Option::as_deref).collect::<Vec<_>>(),
            want
        );
    }

    #[tokio::test]
    async fn corrupt_bytes_are_deleted_and_never_returned() {
        let server = Server::start().await;
        let dir = tempfile::tempdir().unwrap();
        let path = "/releases/v2/Onda.zip";
        server.put(path, body(), Behavior::Corrupt);
        let artifact = artifact_for(&server, path, &body());
        let transport = Transport::new(server.origins(), "Onda/test").unwrap();
        let got = transport.download(&artifact, dir.path(), &|_| {}).await;
        assert!(matches!(got, Err(TransferError::Mismatch(_))), "{got:?}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn a_body_longer_than_signed_is_refused() {
        let server = Server::start().await;
        let dir = tempfile::tempdir().unwrap();
        let path = "/releases/v2/Onda.zip";
        server.put(path, body(), Behavior::Serve);
        let mut artifact = artifact_for(&server, path, &body());
        artifact.size -= 1;
        let transport = Transport::new(server.origins(), "Onda/test").unwrap();
        let got = transport.download(&artifact, dir.path(), &|_| {}).await;
        assert!(matches!(got, Err(TransferError::Mismatch(_))), "{got:?}");
    }

    #[tokio::test]
    async fn redirects_follow_the_origins() {
        let server = Server::start().await;
        let dir = tempfile::tempdir().unwrap();
        server.put("/storage/blob", body(), Behavior::Serve);
        server.put(
            "/releases/v2/Onda.zip",
            Vec::new(),
            Behavior::Redirect("/storage/blob"),
        );
        let artifact = Artifact {
            url: server.url("/releases/v2/Onda.zip"),
            ..artifact_for(&server, "/storage/blob", &body())
        };
        let allowed = Transport::new(server.origins(), "Onda/test").unwrap();
        allowed
            .download(&artifact, dir.path(), &|_| {})
            .await
            .unwrap();

        let strict = Origins {
            redirect_hosts: vec!["github.com".to_owned()],
            ..server.origins()
        };
        let refused = Transport::new(strict, "Onda/test").unwrap();
        let other = tempfile::tempdir().unwrap();
        let got = refused.download(&artifact, other.path(), &|_| {}).await;
        assert!(matches!(got, Err(TransferError::Network(_))), "{got:?}");
    }

    #[tokio::test]
    async fn artifacts_outside_the_origins_are_never_requested() {
        let server = Server::start().await;
        let dir = tempfile::tempdir().unwrap();
        server.put("/elsewhere/Onda.zip", body(), Behavior::Serve);
        let artifact = artifact_for(&server, "/elsewhere/Onda.zip", &body());
        let transport = Transport::new(server.origins(), "Onda/test").unwrap();
        let got = transport.download(&artifact, dir.path(), &|_| {}).await;
        assert!(matches!(got, Err(TransferError::Mismatch(_))), "{got:?}");
        assert!(server.requests("/elsewhere/Onda.zip").is_empty());
    }

    #[tokio::test]
    async fn oversized_feeds_are_refused() {
        let server = Server::start().await;
        server.put(
            "/update.json",
            vec![b' '; MAX_FEED_BYTES + 1],
            Behavior::Serve,
        );
        server.put("/missing.json", Vec::new(), Behavior::Fail);
        let transport = Transport::new(server.origins(), "Onda/test").unwrap();
        let got = transport.fetch_feed(&server.url("/update.json")).await;
        assert!(matches!(got, Err(TransferError::Mismatch(_))), "{got:?}");
        let got = transport.fetch_feed(&server.url("/missing.json")).await;
        assert!(matches!(got, Err(TransferError::Network(_))), "{got:?}");
    }
}
