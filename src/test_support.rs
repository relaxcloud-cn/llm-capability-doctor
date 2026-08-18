use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
use url::Url;

pub enum ServerAction {
    Write(Vec<u8>),
    Delay(Duration),
    Checkpoint(Arc<Notify>),
    Close,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordedRequest {
    pub head: String,
    pub body: Vec<u8>,
}

pub struct RawTcpServer {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    accepted: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl RawTcpServer {
    pub async fn spawn(scripts: Vec<Vec<ServerAction>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind raw TCP test server");
        let address = listener.local_addr().expect("raw TCP server address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let accepted = Arc::new(AtomicUsize::new(0));
        let server_requests = Arc::clone(&requests);
        let server_accepted = Arc::clone(&accepted);
        let task = tokio::spawn(async move {
            for script in scripts {
                let (mut socket, _) = listener.accept().await.expect("accept test request");
                server_accepted.fetch_add(1, Ordering::SeqCst);
                let request = read_request(&mut socket).await.expect("read test request");
                server_requests.lock().await.push(request);

                for action in script {
                    match action {
                        ServerAction::Write(bytes) => {
                            if socket.write_all(&bytes).await.is_err() {
                                break;
                            }
                        }
                        ServerAction::Delay(duration) => tokio::time::sleep(duration).await,
                        ServerAction::Checkpoint(checkpoint) => checkpoint.notify_one(),
                        ServerAction::Close => {
                            let _ = socket.shutdown().await;
                            break;
                        }
                    }
                }
            }
        });

        Self {
            address,
            requests,
            accepted,
            task,
        }
    }

    pub fn url(&self) -> Url {
        format!("http://{}/v1/chat/completions", self.address)
            .parse()
            .expect("raw TCP fixture URL")
    }

    pub fn accepted_count(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }

    pub async fn recorded_requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().await.clone()
    }
}

impl Drop for RawTcpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn read_request(socket: &mut TcpStream) -> std::io::Result<RecordedRequest> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let head_end = loop {
        let read = socket.read(&mut buffer).await?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "request ended before headers",
            ));
        }
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(index) = find_header_end(&bytes) {
            break index;
        }
    };

    let head = String::from_utf8_lossy(&bytes[..head_end]).into_owned();
    let content_length = content_length(head.as_bytes());
    let body_start = head_end + 4;
    while bytes.len() < body_start + content_length {
        let read = socket.read(&mut buffer).await?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "request ended before body",
            ));
        }
        bytes.extend_from_slice(&buffer[..read]);
    }

    Ok(RecordedRequest {
        head,
        body: bytes[body_start..body_start + content_length].to_vec(),
    })
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length(head: &[u8]) -> usize {
    String::from_utf8_lossy(head)
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0)
}
