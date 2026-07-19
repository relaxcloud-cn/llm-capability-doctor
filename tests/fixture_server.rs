#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use model_capability_doctor::protocol::Protocol;

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
                            thread::spawn(move || handle_connection(stream, protocol, requests));
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

fn handle_connection(
    mut stream: TcpStream,
    protocol: Protocol,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let Some(body) = read_request_body(&mut stream) else {
        return;
    };
    requests.lock().unwrap().push(RecordedRequest {
        body: body.clone(),
        accepted_at: Instant::now(),
    });
    let response = response_for(protocol, &body);
    let bytes = response.as_bytes();
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nX-Fixture: rust-cli\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    );
    let _ = stream.write_all(headers.as_bytes());
    let _ = stream.write_all(bytes);
    let _ = stream.flush();
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
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    while bytes.len() - header_end < content_length {
        let read = stream.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    Some(String::from_utf8_lossy(&bytes[header_end..]).into_owned())
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
