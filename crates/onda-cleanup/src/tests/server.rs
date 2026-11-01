//! A one-connection HTTP/1.1 server for tests. Listens on 127.0.0.1 only; no
//! test touches the network.

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// Path the server pretends to be, matching the production endpoint.
pub const PATH: &str = "/api/v1/chat/completions";

/// What the server does once it has read the request.
pub enum Reply {
    /// Answers with this status and body.
    Respond { status: u16, body: String },
    /// Never answers; reports when the client closes the connection.
    Hang,
}

/// The request as it arrived on the wire.
#[derive(Debug)]
pub struct Captured {
    pub request_line: String,
    /// Header names lowercased, in arrival order.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Captured {
    /// Every value of header `name`.
    pub fn header(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }
}

pub struct Server {
    pub url: String,
    captured: oneshot::Receiver<Captured>,
    closed: oneshot::Receiver<()>,
}

impl Server {
    pub async fn start(reply: Reply) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("local address");
        let (captured_tx, captured) = oneshot::channel();
        let (closed_tx, closed) = oneshot::channel();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            let mut stream = BufReader::new(stream);
            let request = read_request(&mut stream).await;
            captured_tx.send(request).expect("test awaits the request");
            match reply {
                Reply::Respond { status, body } => {
                    let response = format!(
                        "HTTP/1.1 {status} Greendale\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("write response");
                }
                Reply::Hang => {
                    let mut buf = [0u8; 256];
                    while let Ok(1..) = stream.read(&mut buf).await {}
                    // the test may have finished without waiting for this
                    let _ = closed_tx.send(());
                }
            }
        });
        Server {
            url: format!("http://{address}{PATH}"),
            captured,
            closed,
        }
    }

    /// Answers one connection per reply, in order, and returns every
    /// request it read once all replies are sent.
    pub async fn start_sequence(
        replies: Vec<(u16, String)>,
    ) -> (String, tokio::task::JoinHandle<Vec<Captured>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("local address");
        let served = tokio::spawn(async move {
            let mut captured = Vec::new();
            for (status, body) in replies {
                let (stream, _) = listener.accept().await.expect("accept");
                let mut stream = BufReader::new(stream);
                captured.push(read_request(&mut stream).await);
                let response = format!(
                    "HTTP/1.1 {status} Greendale\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("write response");
            }
            captured
        });
        (format!("http://{address}{PATH}"), served)
    }

    /// The request the client sent.
    pub async fn captured(&mut self) -> Captured {
        (&mut self.captured).await.expect("server read a request")
    }

    /// Resolves when the client closes a hung connection.
    pub async fn closed(&mut self) {
        (&mut self.closed).await.expect("server saw the close");
    }
}

async fn read_request(stream: &mut BufReader<tokio::net::TcpStream>) -> Captured {
    let mut request_line = String::new();
    stream
        .read_line(&mut request_line)
        .await
        .expect("request line");
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        stream.read_line(&mut line).await.expect("header line");
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':').expect("header has a colon");
        headers.push((name.to_ascii_lowercase(), value.trim().to_owned()));
    }
    let length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .map_or(0, |(_, value)| value.parse().expect("content length"));
    let mut body = vec![0; length];
    stream.read_exact(&mut body).await.expect("body");
    Captured {
        request_line: request_line.trim_end().to_owned(),
        headers,
        body,
    }
}
