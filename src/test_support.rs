use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
use url::Url;

use crate::protocol::Protocol;

#[derive(Clone, Debug, PartialEq)]
pub enum ScriptedAssistantTurn {
    Tool {
        response_id: String,
        call_id: Option<String>,
        name: String,
        arguments: serde_json::Value,
    },
    Final {
        response_id: String,
        text: String,
    },
}

pub fn encode_official_stream(protocol: Protocol, turn: &ScriptedAssistantTurn) -> Vec<u8> {
    match protocol {
        Protocol::OpenAiChat => encode_openai_chat(turn),
        Protocol::OpenAiResponses => encode_openai_responses(turn),
        Protocol::AnthropicMessages => encode_anthropic(turn),
        Protocol::GeminiGenerateContent => encode_gemini(turn),
        Protocol::OllamaChat => encode_ollama(turn),
        Protocol::Unknown => panic!("unknown protocol has no official stream"),
    }
}

fn encode_openai_chat(turn: &ScriptedAssistantTurn) -> Vec<u8> {
    let (response_id, delta, finish_reason) = match turn {
        ScriptedAssistantTurn::Tool {
            response_id,
            call_id,
            name,
            arguments,
        } => (
            response_id,
            serde_json::json!({
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "index": 0,
                    "id": call_id.as_deref().expect("OpenAI Chat requires call_id"),
                    "type": "function",
                    "function": {"name": name, "arguments": arguments.to_string()}
                }]
            }),
            "tool_calls",
        ),
        ScriptedAssistantTurn::Final { response_id, text } => (
            response_id,
            serde_json::json!({"role": "assistant", "content": text}),
            "stop",
        ),
    };
    let content = serde_json::json!({
        "id": response_id,
        "object": "chat.completion.chunk",
        "created": 1_787_041_140_u64,
        "model": "test-model",
        "choices": [{"index": 0, "delta": delta, "finish_reason": null}]
    });
    let terminal = serde_json::json!({
        "id": response_id,
        "object": "chat.completion.chunk",
        "created": 1_787_041_140_u64,
        "model": "test-model",
        "choices": [{"index": 0, "delta": {}, "finish_reason": finish_reason}]
    });
    format!("data: {content}\n\ndata: {terminal}\n\ndata: [DONE]\n\n").into_bytes()
}

fn encode_openai_responses(turn: &ScriptedAssistantTurn) -> Vec<u8> {
    let mut events = Vec::new();
    match turn {
        ScriptedAssistantTurn::Tool {
            response_id,
            call_id,
            name,
            arguments,
        } => {
            let call_id = call_id.as_deref().expect("Responses requires call_id");
            let item_id = format!("fc-{response_id}");
            let arguments = arguments.to_string();
            let added = serde_json::json!({
                "id": item_id,
                "type": "function_call",
                "call_id": call_id,
                "name": name,
                "arguments": "",
                "status": "in_progress"
            });
            let done = serde_json::json!({
                "id": item_id,
                "type": "function_call",
                "call_id": call_id,
                "name": name,
                "arguments": arguments,
                "status": "completed"
            });
            events.push(("response.created", serde_json::json!({
                "type": "response.created", "response": {"id": response_id, "status": "in_progress"}, "sequence_number": 0
            })));
            events.push(("response.output_item.added", serde_json::json!({
                "type": "response.output_item.added", "output_index": 0, "item": added, "sequence_number": 1
            })));
            events.push(("response.function_call_arguments.delta", serde_json::json!({
                "type": "response.function_call_arguments.delta", "item_id": item_id, "output_index": 0, "delta": arguments, "sequence_number": 2
            })));
            events.push(("response.function_call_arguments.done", serde_json::json!({
                "type": "response.function_call_arguments.done", "item_id": item_id, "output_index": 0, "name": name, "arguments": arguments, "sequence_number": 3
            })));
            events.push(("response.output_item.done", serde_json::json!({
                "type": "response.output_item.done", "output_index": 0, "item": done.clone(), "sequence_number": 4
            })));
            events.push(("response.completed", serde_json::json!({
                "type": "response.completed", "response": {"id": response_id, "status": "completed", "output": [done]}, "sequence_number": 5
            })));
        }
        ScriptedAssistantTurn::Final { response_id, text } => {
            let item_id = format!("msg-{response_id}");
            let added = serde_json::json!({
                "id": item_id, "type": "message", "role": "assistant", "content": [], "status": "in_progress"
            });
            let output = serde_json::json!({
                "id": item_id,
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": text, "annotations": []}],
                "status": "completed"
            });
            events.push(("response.created", serde_json::json!({
                "type": "response.created", "response": {"id": response_id, "status": "in_progress"}, "sequence_number": 0
            })));
            events.push(("response.output_item.added", serde_json::json!({
                "type": "response.output_item.added", "output_index": 0, "item": added, "sequence_number": 1
            })));
            events.push(("response.content_part.added", serde_json::json!({
                "type": "response.content_part.added", "item_id": item_id, "output_index": 0, "content_index": 0,
                "part": {"type": "output_text", "text": "", "annotations": []}, "sequence_number": 2
            })));
            events.push(("response.output_text.delta", serde_json::json!({
                "type": "response.output_text.delta", "item_id": item_id, "output_index": 0, "content_index": 0,
                "delta": text, "logprobs": [], "sequence_number": 3
            })));
            events.push(("response.output_text.done", serde_json::json!({
                "type": "response.output_text.done", "item_id": item_id, "output_index": 0, "content_index": 0,
                "text": text, "logprobs": [], "sequence_number": 4
            })));
            events.push(("response.content_part.done", serde_json::json!({
                "type": "response.content_part.done", "item_id": item_id, "output_index": 0, "content_index": 0,
                "part": {"type": "output_text", "text": text, "annotations": []}, "sequence_number": 5
            })));
            events.push(("response.output_item.done", serde_json::json!({
                "type": "response.output_item.done", "output_index": 0, "item": output.clone(), "sequence_number": 6
            })));
            events.push(("response.completed", serde_json::json!({
                "type": "response.completed", "response": {"id": response_id, "status": "completed", "output": [output]}, "sequence_number": 7
            })));
        }
    }
    events
        .into_iter()
        .map(|(event, data)| format!("event: {event}\ndata: {data}\n\n"))
        .collect::<String>()
        .into_bytes()
}

fn encode_anthropic(turn: &ScriptedAssistantTurn) -> Vec<u8> {
    let (response_id, block, delta, stop_reason) = match turn {
        ScriptedAssistantTurn::Tool {
            response_id,
            call_id,
            name,
            arguments,
        } => (
            response_id,
            serde_json::json!({
                "type": "tool_use", "id": call_id.as_deref().expect("Anthropic requires call_id"), "name": name, "input": {}
            }),
            serde_json::json!({"type": "input_json_delta", "partial_json": arguments.to_string()}),
            "tool_use",
        ),
        ScriptedAssistantTurn::Final { response_id, text } => (
            response_id,
            serde_json::json!({"type": "text", "text": ""}),
            serde_json::json!({"type": "text_delta", "text": text}),
            "end_turn",
        ),
    };
    let events = [
        (
            "message_start",
            serde_json::json!({
                "type": "message_start", "message": {"id": response_id, "type": "message", "role": "assistant", "model": "test-model", "content": [], "stop_reason": null, "stop_sequence": null, "usage": {"input_tokens": 1, "output_tokens": 1}}
            }),
        ),
        (
            "content_block_start",
            serde_json::json!({"type": "content_block_start", "index": 0, "content_block": block}),
        ),
        (
            "content_block_delta",
            serde_json::json!({"type": "content_block_delta", "index": 0, "delta": delta}),
        ),
        (
            "content_block_stop",
            serde_json::json!({"type": "content_block_stop", "index": 0}),
        ),
        (
            "message_delta",
            serde_json::json!({"type": "message_delta", "delta": {"stop_reason": stop_reason, "stop_sequence": null}, "usage": {"output_tokens": 2}}),
        ),
        ("message_stop", serde_json::json!({"type": "message_stop"})),
    ];
    events
        .into_iter()
        .map(|(event, data)| format!("event: {event}\ndata: {data}\n\n"))
        .collect::<String>()
        .into_bytes()
}

fn encode_gemini(turn: &ScriptedAssistantTurn) -> Vec<u8> {
    let part = match turn {
        ScriptedAssistantTurn::Tool {
            call_id,
            name,
            arguments,
            ..
        } => {
            let mut call = serde_json::Map::from_iter([
                ("name".into(), serde_json::Value::String(name.clone())),
                ("args".into(), arguments.clone()),
            ]);
            if let Some(call_id) = call_id {
                call.insert("id".into(), serde_json::Value::String(call_id.clone()));
            }
            serde_json::json!({"functionCall": call, "thoughtSignature": "sig-call"})
        }
        ScriptedAssistantTurn::Final { text, .. } => serde_json::json!({"text": text}),
    };
    let response = serde_json::json!({
        "candidates": [{"index": 0, "content": {"role": "model", "parts": [part]}, "finishReason": "STOP"}]
    });
    format!("data: {response}\n\n").into_bytes()
}

fn encode_ollama(turn: &ScriptedAssistantTurn) -> Vec<u8> {
    let message = match turn {
        ScriptedAssistantTurn::Tool {
            call_id,
            name,
            arguments,
            ..
        } => {
            let mut call = serde_json::Map::from_iter([(
                "function".into(),
                serde_json::json!({"index": 0, "name": name, "arguments": arguments}),
            )]);
            if let Some(call_id) = call_id {
                call.insert("id".into(), serde_json::Value::String(call_id.clone()));
            }
            serde_json::json!({"role": "assistant", "content": "", "tool_calls": [call]})
        }
        ScriptedAssistantTurn::Final { text, .. } => {
            serde_json::json!({"role": "assistant", "content": text})
        }
    };
    let response = serde_json::json!({
        "model": "test-model",
        "created_at": "2026-08-18T00:00:00Z",
        "message": message,
        "done": true,
        "done_reason": "stop"
    });
    format!("{response}\n").into_bytes()
}

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

    pub fn url_with_path(&self, path_and_query: &str) -> Url {
        format!("http://{}{path_and_query}", self.address)
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

#[test]
fn official_ollama_fixture_uses_native_tool_call_fields() {
    let turn = ScriptedAssistantTurn::Tool {
        response_id: "response-fixture".into(),
        call_id: Some("call-fixture".into()),
        name: "get_weather".into(),
        arguments: serde_json::json!({"city": "Shanghai"}),
    };
    let bytes = encode_official_stream(Protocol::OllamaChat, &turn);
    let record: serde_json::Value =
        serde_json::from_slice(&bytes).expect("official Ollama NDJSON record");
    let call = &record["message"]["tool_calls"][0];

    assert_eq!(call["id"], "call-fixture");
    assert_eq!(call["function"]["index"], 0);
    assert!(
        call.get("type").is_none(),
        "Ollama ToolCall has no type field"
    );
}
