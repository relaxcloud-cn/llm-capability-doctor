#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use model_capability_doctor::protocol::Protocol;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct RecordedRequest {
    pub body: String,
    pub accepted_at: Instant,
}

pub struct FixtureServer {
    address: SocketAddr,
    shutdown: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    thread: Option<JoinHandle<()>>,
}

impl FixtureServer {
    pub fn start(protocol: Protocol) -> Self {
        Self::start_delayed(protocol, Duration::ZERO)
    }

    pub fn start_delayed(protocol: Protocol, response_delay: Duration) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread = {
            let shutdown = shutdown.clone();
            let requests = requests.clone();
            thread::spawn(move || {
                while !shutdown.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let requests = requests.clone();
                            thread::spawn(move || {
                                handle_connection(stream, protocol, response_delay, requests)
                            });
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(1));
                        }
                        Err(_) => break,
                    }
                }
            })
        };
        Self {
            address,
            shutdown,
            requests,
            thread: Some(thread),
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}/v1/model", self.address)
    }

    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub struct TlsFixtureServer {
    address: SocketAddr,
    shutdown: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

impl TlsFixtureServer {
    pub async fn start(protocol: Protocol) -> Self {
        use rcgen::{CertifiedKey, generate_simple_self_signed};
        use tokio_rustls::rustls::ServerConfig;
        use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

        let CertifiedKey { cert, signing_key } =
            generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
        let certificate = CertificateDer::from(cert.der().to_vec());
        let private_key =
            PrivateKeyDer::from(PrivatePkcs8KeyDer::from(signing_key.serialize_der()));
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![certificate], private_key)
            .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let shutdown = CancellationToken::new();
        let task = {
            let shutdown = shutdown.clone();
            tokio::spawn(async move {
                loop {
                    let accepted = tokio::select! {
                        _ = shutdown.cancelled() => break,
                        accepted = listener.accept() => accepted,
                    };
                    let Ok((stream, _)) = accepted else {
                        break;
                    };
                    let acceptor = acceptor.clone();
                    tokio::spawn(async move {
                        let Ok(mut stream) = acceptor.accept(stream).await else {
                            return;
                        };
                        let mut request = Vec::new();
                        let mut buffer = [0_u8; 4096];
                        loop {
                            let Ok(read) = stream.read(&mut buffer).await else {
                                return;
                            };
                            if read == 0 {
                                return;
                            }
                            request.extend_from_slice(&buffer[..read]);
                            if let Some(header_position) = find_bytes(&request, b"\r\n\r\n") {
                                let header_end = header_position + 4;
                                let headers = String::from_utf8_lossy(&request[..header_end]);
                                let length = content_length(&headers);
                                while request.len() - header_end < length {
                                    let Ok(read) = stream.read(&mut buffer).await else {
                                        return;
                                    };
                                    if read == 0 {
                                        break;
                                    }
                                    request.extend_from_slice(&buffer[..read]);
                                }
                                break;
                            }
                        }
                        let response = normal_response(protocol);
                        let headers = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            response.len()
                        );
                        let _ = stream.write_all(headers.as_bytes()).await;
                        let _ = stream.write_all(response.as_bytes()).await;
                        let _ = stream.shutdown().await;
                    });
                }
            })
        };
        Self {
            address,
            shutdown,
            task,
        }
    }

    pub fn url(&self) -> String {
        format!("https://localhost:{}/v1/model", self.address.port())
    }
}

impl Drop for TlsFixtureServer {
    fn drop(&mut self) {
        self.shutdown.cancel();
        self.task.abort();
    }
}

fn handle_connection(
    mut stream: TcpStream,
    protocol: Protocol,
    response_delay: Duration,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    while let Some(body) = read_request_body(&mut stream) {
        requests.lock().unwrap().push(RecordedRequest {
            body: body.clone(),
            accepted_at: Instant::now(),
        });
        thread::sleep(response_delay);
        let response = response_for(protocol, &body);
        let bytes = response.as_bytes();
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nX-Fixture: rust-cli\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
            bytes.len()
        );
        if stream.write_all(headers.as_bytes()).is_err()
            || stream.write_all(bytes).is_err()
            || stream.flush().is_err()
        {
            break;
        }
    }
}

fn read_request_body(stream: &mut TcpStream) -> Option<String> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut buffer).ok()?;
        if read == 0 {
            return None;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(position) = find_bytes(&bytes, b"\r\n\r\n") {
            break position + 4;
        }
        if bytes.len() > 64 * 1024 {
            return None;
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let content_length = content_length(&headers);
    while bytes.len() - header_end < content_length {
        let read = stream.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    Some(String::from_utf8_lossy(&bytes[header_end..]).into_owned())
}

fn content_length(headers: &str) -> usize {
    headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0)
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn response_for(protocol: Protocol, request: &str) -> String {
    let is_tool_request =
        request.contains("\"tools\"") || request.contains("\"functionDeclarations\"");
    let is_follow_up = request.contains("function_call_output")
        || request.contains("\"role\":\"tool\"")
        || request.contains("tool_result")
        || request.contains("functionResponse");
    if is_tool_request && !is_follow_up {
        return tool_response(protocol);
    }
    normal_response(protocol)
}

fn normal_response(protocol: Protocol) -> String {
    match protocol {
        Protocol::OpenAiChat => r#"{"id":"chatcmpl-fixture","choices":[{"message":{"role":"assistant","content":"MODEL_DOCTOR_OK"}}],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4}}"#.into(),
        Protocol::OpenAiResponses => r#"{"id":"resp-fixture","object":"response","output":[{"type":"message","content":[{"type":"output_text","text":"MODEL_DOCTOR_OK"}]}],"usage":{"input_tokens":3,"output_tokens":1,"total_tokens":4}}"#.into(),
        Protocol::AnthropicMessages => r#"{"id":"msg-fixture","type":"message","content":[{"type":"text","text":"MODEL_DOCTOR_OK"}],"stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":1}}"#.into(),
        Protocol::GeminiGenerateContent => r#"{"candidates":[{"content":{"role":"model","parts":[{"text":"MODEL_DOCTOR_OK"}]}}],"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":1,"totalTokenCount":4}}"#.into(),
        Protocol::OllamaChat => r#"{"message":{"role":"assistant","content":"MODEL_DOCTOR_OK"},"done":true,"prompt_eval_count":3,"eval_count":1}"#.into(),
        Protocol::Unknown => r#"{"status":"unknown"}"#.into(),
    }
}

fn tool_response(protocol: Protocol) -> String {
    match protocol {
        Protocol::OpenAiChat => r#"{"id":"chatcmpl-tool","choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call-chat-1","type":"function","function":{"name":"get_weather","arguments":"{\"city\":\"Beijing\"}"}}]}}]}"#.into(),
        Protocol::OpenAiResponses => r#"{"id":"resp-tool","object":"response","output":[{"type":"function_call","call_id":"call-responses-1","name":"get_weather","arguments":"{\"city\":\"Beijing\"}"}]}"#.into(),
        Protocol::AnthropicMessages => r#"{"id":"msg-tool","type":"message","content":[{"type":"tool_use","id":"toolu-anthropic-1","name":"get_weather","input":{"city":"Beijing"}}],"stop_reason":"tool_use"}"#.into(),
        Protocol::GeminiGenerateContent => r#"{"candidates":[{"content":{"role":"model","parts":[{"functionCall":{"id":"call-gemini-1","name":"get_weather","args":{"city":"Beijing"}}}]}}]}"#.into(),
        Protocol::OllamaChat => r#"{"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"get_weather","arguments":{"city":"Beijing"}}}]},"done":true}"#.into(),
        Protocol::Unknown => r#"{"status":"unknown"}"#.into(),
    }
}
