use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub const TRANSPORT_VERSION: &str = "chat-completions-transport/v1";
const MAX_ATTEMPTS: u8 = 3;

#[derive(Debug, Clone)]
pub struct ChatCompletionsRequest {
    pub module_id: String,
    pub prompt: String,
    /// 预置多轮消息（能力 C04 用）；为 None 时用 prompt 构造单条 user 消息。
    pub messages: Option<Vec<Value>>,
    /// 随请求发送的工具定义（能力 C03 用）。
    pub tools: Option<Vec<Value>>,
    pub max_tokens: u32,
    pub stream: bool,
}

#[derive(Debug, Clone)]
pub struct AgentTurnResponse {
    pub response: ChatCompletionsResponse,
    pub message: Option<Value>,
    pub tool_calls: Vec<Value>,
    pub text: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ChatCompletionsResponse {
    pub status: Option<u16>,
    pub body: String,
    pub parsed: Option<Value>,
    pub elapsed_ms: u128,
    pub error: Option<String>,
    pub attempts: u8,
    pub retry_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamEvent {
    pub at_ms: u64,
    pub raw: String,
    pub content_delta: Option<String>,
    pub reasoning_delta: Option<String>,
    /// delta.tool_calls 原始数组（流式工具调用增量，供 S07 归组拼接）。
    pub tool_calls_delta: Option<Value>,
    pub finish_reason: Option<String>,
    pub done: bool,
}

#[derive(Debug, Clone)]
pub struct StreamResponse {
    pub response: ChatCompletionsResponse,
    pub events: Vec<StreamEvent>,
    pub content: String,
    pub terminated: bool,
    pub parse_errors: Vec<String>,
}

type ParsedStream = (String, Vec<StreamEvent>, String, bool, Vec<String>);

#[derive(Clone)]
pub struct ChatCompletionsTransport {
    client: Client,
    endpoint: String,
    model: String,
    api_key: String,
    runtime: std::sync::Arc<tokio::runtime::Runtime>,
}

impl std::fmt::Debug for ChatCompletionsTransport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChatCompletionsTransport")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("api_key", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl ChatCompletionsTransport {
    pub fn new(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
        timeout: Duration,
    ) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| format!("创建 HTTP 运行时失败：{error}"))?;
        let client = Client::builder()
            .timeout(timeout)
            .user_agent(format!("llm-capability-doctor/{TRANSPORT_VERSION}"))
            .build()
            .map_err(|error| format!("创建 HTTP 客户端失败：{error}"))?;
        Ok(Self {
            client,
            endpoint: endpoint.into(),
            model: model.into(),
            api_key: api_key.into(),
            runtime: std::sync::Arc::new(runtime),
        })
    }

    pub fn send(&self, request: ChatCompletionsRequest) -> ChatCompletionsResponse {
        let messages = request
            .messages
            .clone()
            .unwrap_or_else(|| vec![json!({"role": "user", "content": request.prompt})]);
        let mut payload = json!({
            "model": self.model,
            "messages": messages,
            "temperature": 0,
            "max_tokens": request.max_tokens,
            "stream": request.stream,
        });
        if let Some(tools) = request.tools.clone() {
            payload["tools"] = Value::Array(tools);
            payload["tool_choice"] = Value::String("auto".into());
        }
        let started = Instant::now();
        let mut retry_reasons = Vec::new();
        for attempt in 1..=MAX_ATTEMPTS {
            let result = self.runtime.block_on(async {
                self.client
                    .post(&self.endpoint)
                    .bearer_auth(&self.api_key)
                    .json(&payload)
                    .send()
                    .await
            });
            let response = match result {
                Ok(response) => {
                    let status = response.status().as_u16();
                    let body_result = self.runtime.block_on(async { response.text().await });
                    match body_result {
                        Ok(body) => ChatCompletionsResponse {
                            status: Some(status),
                            body: truncate_body(body.clone()),
                            parsed: serde_json::from_str::<Value>(&body).ok(),
                            elapsed_ms: started.elapsed().as_millis(),
                            error: None,
                            attempts: attempt,
                            retry_reasons: retry_reasons.clone(),
                        },
                        Err(error) => ChatCompletionsResponse {
                            status: Some(status),
                            body: String::new(),
                            parsed: None,
                            elapsed_ms: started.elapsed().as_millis(),
                            error: Some(format!("读取 HTTP 响应失败：{error}")),
                            attempts: attempt,
                            retry_reasons: retry_reasons.clone(),
                        },
                    }
                }
                Err(error) => ChatCompletionsResponse {
                    status: None,
                    body: String::new(),
                    parsed: None,
                    elapsed_ms: started.elapsed().as_millis(),
                    error: Some(format!("发送 HTTP 请求失败：{error}")),
                    attempts: attempt,
                    retry_reasons: retry_reasons.clone(),
                },
            };
            let retryable = response.error.is_some()
                || response
                    .status
                    .is_some_and(|status| status == 429 || status >= 500);
            if !retryable || attempt == MAX_ATTEMPTS {
                return response;
            }
            retry_reasons.push(format!(
                "第 {attempt} 次请求返回 {:?}，按传输规则重试",
                response.status
            ));
        }
        unreachable!("MAX_ATTEMPTS is positive")
    }

    pub fn send_agent_turn(&self, messages: Vec<Value>, tools: Vec<Value>) -> AgentTurnResponse {
        let payload = json!({
            "model": self.model,
            "messages": messages,
            "tools": tools,
            "tool_choice": "auto",
            "temperature": 0,
            "max_tokens": 512,
            "stream": false,
        });
        let response = self.send_payload(payload, "agent");
        let message = response
            .parsed
            .as_ref()
            .and_then(|value| value.get("choices"))
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"))
            .cloned();
        let tool_calls = message
            .as_ref()
            .and_then(|value| value.get("tool_calls"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let text = message
            .as_ref()
            .and_then(|value| value.get("content"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        AgentTurnResponse {
            response,
            message,
            tool_calls,
            text,
        }
    }

    pub fn send_stream(&self, request: ChatCompletionsRequest) -> StreamResponse {
        let payload = json!({
            "model": self.model,
            "messages": [{"role": "user", "content": request.prompt}],
            "temperature": 0,
            "max_tokens": request.max_tokens,
            "stream": true,
        });
        self.send_stream_payload(payload)
    }

    /// 发送自定义流式 payload（供 S07 带 tools/tool_choice 的探测使用）。
    pub(crate) fn send_stream_payload(&self, payload: Value) -> StreamResponse {
        let started = Instant::now();
        let mut retry_reasons = Vec::new();
        for attempt in 1..=MAX_ATTEMPTS {
            let result = self.runtime.block_on(async {
                self.client
                    .post(&self.endpoint)
                    .bearer_auth(&self.api_key)
                    .json(&payload)
                    .send()
                    .await
            });
            let response = match result {
                Ok(response) => {
                    let status = response.status().as_u16();
                    match self.read_stream(response, started) {
                        Ok((body, events, content, terminated, parse_errors)) => StreamResponse {
                            response: ChatCompletionsResponse {
                                status: Some(status),
                                body: truncate_body(body),
                                parsed: stream_summary(&content, &events),
                                elapsed_ms: started.elapsed().as_millis(),
                                error: None,
                                attempts: attempt,
                                retry_reasons: retry_reasons.clone(),
                            },
                            events,
                            content,
                            terminated,
                            parse_errors,
                        },
                        Err(error) => StreamResponse {
                            response: ChatCompletionsResponse {
                                status: Some(status),
                                body: String::new(),
                                parsed: None,
                                elapsed_ms: started.elapsed().as_millis(),
                                error: Some(error),
                                attempts: attempt,
                                retry_reasons: retry_reasons.clone(),
                            },
                            events: Vec::new(),
                            content: String::new(),
                            terminated: false,
                            parse_errors: Vec::new(),
                        },
                    }
                }
                Err(error) => StreamResponse {
                    response: ChatCompletionsResponse {
                        status: None,
                        body: String::new(),
                        parsed: None,
                        elapsed_ms: started.elapsed().as_millis(),
                        error: Some(format!("发送 HTTP 请求失败：{error}")),
                        attempts: attempt,
                        retry_reasons: retry_reasons.clone(),
                    },
                    events: Vec::new(),
                    content: String::new(),
                    terminated: false,
                    parse_errors: Vec::new(),
                },
            };
            let retryable = response.response.error.is_some()
                || response
                    .response
                    .status
                    .is_some_and(|status| status == 429 || status >= 500);
            if !retryable || attempt == MAX_ATTEMPTS {
                return response;
            }
            retry_reasons.push(format!(
                "第 {attempt} 次流式请求返回 {:?}，按传输规则重试",
                response.response.status
            ));
        }
        unreachable!("MAX_ATTEMPTS is positive")
    }

    fn read_stream(
        &self,
        response: reqwest::Response,
        started: Instant,
    ) -> Result<ParsedStream, String> {
        self.runtime.block_on(async move {
            let mut body = Vec::new();
            let mut events = Vec::new();
            let mut content = String::new();
            let mut terminated = false;
            let mut parse_errors = Vec::new();
            let mut line_buffer = String::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|error| format!("读取流式响应失败：{error}"))?;
                line_buffer.push_str(&String::from_utf8_lossy(&chunk));
                body.extend_from_slice(&chunk);
                while let Some(newline) = line_buffer.find('\n') {
                    let line = line_buffer[..newline].trim_end_matches('\r').to_owned();
                    line_buffer.drain(..=newline);
                    parse_stream_line(
                        &line,
                        started,
                        &mut events,
                        &mut content,
                        &mut terminated,
                        &mut parse_errors,
                    );
                }
            }
            if !line_buffer.trim().is_empty() {
                parse_stream_line(
                    line_buffer.trim(),
                    started,
                    &mut events,
                    &mut content,
                    &mut terminated,
                    &mut parse_errors,
                );
            }
            Ok((
                String::from_utf8_lossy(&body).into_owned(),
                events,
                content,
                terminated,
                parse_errors,
            ))
        })
    }

    /// 发送自定义 Chat Completions payload（供 S04 工具调用等需要
    /// tools/tool_choice 等额外字段的探测使用）。
    pub(crate) fn send_payload(&self, payload: Value, module_id: &str) -> ChatCompletionsResponse {
        let started = Instant::now();
        let mut retry_reasons = Vec::new();
        for attempt in 1..=MAX_ATTEMPTS {
            let result = self.runtime.block_on(async {
                self.client
                    .post(&self.endpoint)
                    .bearer_auth(&self.api_key)
                    .json(&payload)
                    .send()
                    .await
            });
            let response = match result {
                Ok(response) => {
                    let status = response.status().as_u16();
                    let body_result = self.runtime.block_on(async { response.text().await });
                    match body_result {
                        Ok(body) => ChatCompletionsResponse {
                            status: Some(status),
                            body: truncate_body(body.clone()),
                            parsed: serde_json::from_str::<Value>(&body).ok(),
                            elapsed_ms: started.elapsed().as_millis(),
                            error: None,
                            attempts: attempt,
                            retry_reasons: retry_reasons.clone(),
                        },
                        Err(error) => ChatCompletionsResponse {
                            status: Some(status),
                            body: String::new(),
                            parsed: None,
                            elapsed_ms: started.elapsed().as_millis(),
                            error: Some(format!("读取 HTTP 响应失败：{error}")),
                            attempts: attempt,
                            retry_reasons: retry_reasons.clone(),
                        },
                    }
                }
                Err(error) => ChatCompletionsResponse {
                    status: None,
                    body: String::new(),
                    parsed: None,
                    elapsed_ms: started.elapsed().as_millis(),
                    error: Some(format!("发送 HTTP 请求失败：{error}")),
                    attempts: attempt,
                    retry_reasons: retry_reasons.clone(),
                },
            };
            let retryable = response.error.is_some()
                || response
                    .status
                    .is_some_and(|status| status == 429 || status >= 500);
            if !retryable || attempt == MAX_ATTEMPTS {
                return response;
            }
            retry_reasons.push(format!(
                "第 {attempt} 次 {module_id} 请求返回 {:?}，按传输规则重试",
                response.status
            ));
        }
        unreachable!("MAX_ATTEMPTS is positive")
    }

    pub fn model_name(&self) -> &str {
        &self.model
    }

    /// 自定义 payload 请求的证据记录：保留完整请求体而非拆解字段。
    pub(crate) fn raw_evidence_payload(
        &self,
        request_payload: &Value,
        module_id: &str,
        response: &ChatCompletionsResponse,
    ) -> Value {
        json!({
            "transport_version": TRANSPORT_VERSION,
            "module": module_id,
            "request": {
                "endpoint": redact_endpoint(&self.endpoint),
                "payload": request_payload,
            },
            "response": {
                "status": response.status,
                "body": response.parsed.clone().unwrap_or_else(|| Value::String(response.body.clone())),
                "elapsed_ms": response.elapsed_ms,
                "error": response.error,
                "attempts": response.attempts,
                "retry_reasons": response.retry_reasons,
            },
        })
    }

    pub fn evidence_payload(
        &self,
        request: &ChatCompletionsRequest,
        response: &ChatCompletionsResponse,
    ) -> Value {
        json!({
            "transport_version": TRANSPORT_VERSION,
            "module": request.module_id,
            "request": {
                "endpoint": redact_endpoint(&self.endpoint),
                "model": self.model,
                "prompt": request.prompt,
                "messages": request.messages,
                "tools": request.tools,
                "max_tokens": request.max_tokens,
                "stream": request.stream,
            },
            "response": {
                "status": response.status,
                "body": response.parsed.clone().unwrap_or_else(|| Value::String(response.body.clone())),
                "elapsed_ms": response.elapsed_ms,
                "error": response.error,
                "attempts": response.attempts,
                "retry_reasons": response.retry_reasons,
            },
        })
    }

    /// 自定义 payload 流式请求的证据记录。
    pub(crate) fn raw_stream_evidence_payload(
        &self,
        request_payload: &Value,
        module_id: &str,
        stream: &StreamResponse,
    ) -> Value {
        let mut payload = self.raw_evidence_payload(request_payload, module_id, &stream.response);
        if let Some(response) = payload.get_mut("response") {
            response["stream"] = stream_evidence(stream);
        }
        payload
    }

    pub fn stream_evidence_payload(
        &self,
        request: &ChatCompletionsRequest,
        stream: &StreamResponse,
    ) -> Value {
        let mut payload = self.evidence_payload(request, &stream.response);
        if let Some(response) = payload.get_mut("response") {
            response["stream"] = stream_evidence(stream);
        }
        payload
    }
}

fn stream_evidence(stream: &StreamResponse) -> Value {
    json!({
        "terminated": stream.terminated,
        "content": stream.content,
        "events": stream.events.iter().map(|event| json!({
            "at_ms": event.at_ms,
            "raw": event.raw,
            "content_delta": event.content_delta,
            "reasoning_delta": event.reasoning_delta,
            "tool_calls_delta": event.tool_calls_delta,
            "finish_reason": event.finish_reason,
            "done": event.done,
        })).collect::<Vec<_>>(),
        "parse_errors": stream.parse_errors,
    })
}

fn stream_summary(content: &str, events: &[StreamEvent]) -> Option<Value> {
    (!events.is_empty()).then(|| {
        json!({
            "choices": [{
                "message": {"role": "assistant", "content": content},
                "finish_reason": events.iter().rev().find_map(|event| event.finish_reason.clone()),
            }]
        })
    })
}

fn parse_stream_line(
    line: &str,
    started: Instant,
    events: &mut Vec<StreamEvent>,
    content: &mut String,
    terminated: &mut bool,
    parse_errors: &mut Vec<String>,
) {
    let line = line.trim();
    if line.is_empty() || line.starts_with(':') || line.starts_with("event:") {
        return;
    }
    let Some(data) = line
        .strip_prefix("data:")
        .or_else(|| line.strip_prefix("data :"))
    else {
        return;
    };
    let data = data.trim();
    if data == "[DONE]" {
        *terminated = true;
        events.push(StreamEvent {
            at_ms: started.elapsed().as_millis() as u64,
            raw: data.into(),
            content_delta: None,
            reasoning_delta: None,
            tool_calls_delta: None,
            finish_reason: None,
            done: true,
        });
        return;
    }
    let parsed = match serde_json::from_str::<Value>(data) {
        Ok(value) => value,
        Err(error) => {
            parse_errors.push(format!("流式事件 JSON 解析失败：{error}"));
            events.push(StreamEvent {
                at_ms: started.elapsed().as_millis() as u64,
                raw: data.into(),
                content_delta: None,
                reasoning_delta: None,
                tool_calls_delta: None,
                finish_reason: None,
                done: false,
            });
            return;
        }
    };
    let choice = parsed
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first());
    let delta = choice.and_then(|value| value.get("delta"));
    let message = choice.and_then(|value| value.get("message"));
    let content_delta = delta
        .and_then(|value| value.get("content"))
        .and_then(Value::as_str)
        .or_else(|| {
            message
                .and_then(|value| value.get("content"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            choice
                .and_then(|value| value.get("text"))
                .and_then(Value::as_str)
        })
        .map(str::to_owned);
    let reasoning_delta = delta
        .and_then(|value| {
            value
                .get("reasoning_content")
                .or_else(|| value.get("reasoning"))
        })
        .and_then(Value::as_str)
        .or_else(|| {
            message
                .and_then(|value| {
                    value
                        .get("reasoning_content")
                        .or_else(|| value.get("reasoning"))
                })
                .and_then(Value::as_str)
        })
        .map(str::to_owned);
    if let Some(value) = &content_delta {
        content.push_str(value);
    }
    let tool_calls_delta = delta
        .and_then(|value| value.get("tool_calls"))
        .or_else(|| message.and_then(|value| value.get("tool_calls")))
        .cloned();
    let finish_reason = choice
        .and_then(|value| value.get("finish_reason"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    if finish_reason.is_some() {
        *terminated = true;
    }
    events.push(StreamEvent {
        at_ms: started.elapsed().as_millis() as u64,
        raw: data.into(),
        content_delta,
        reasoning_delta,
        tool_calls_delta,
        finish_reason,
        done: false,
    });
}

fn truncate_body(body: String) -> String {
    const MAX_BODY_BYTES: usize = 256 * 1024;
    if body.len() <= MAX_BODY_BYTES {
        return body;
    }
    let mut truncated = body;
    truncated.truncate(MAX_BODY_BYTES);
    truncated.push_str("...[truncated]");
    truncated
}

fn redact_endpoint(endpoint: &str) -> String {
    match url::Url::parse(endpoint) {
        Ok(mut parsed) => {
            parsed.set_username("").ok();
            parsed.set_password(None).ok();
            parsed.set_query(None);
            parsed.to_string()
        }
        Err(_) => "[invalid-endpoint]".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn mock_server(
        status: u16,
        body: &'static str,
        connections: usize,
        delay: Duration,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            for _ in 0..connections {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 4096];
                let _ = stream.read(&mut request);
                thread::sleep(delay);
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        (format!("http://{address}/v1/chat/completions"), handle)
    }

    #[test]
    fn sends_bearer_request_and_captures_parsed_response_without_key() {
        let (endpoint, server) = mock_server(
            200,
            r#"{"choices":[{"message":{"content":"ok"}}]}"#,
            1,
            Duration::ZERO,
        );
        let transport = ChatCompletionsTransport::new(
            endpoint,
            "model-a",
            "secret-value",
            Duration::from_secs(5),
        )
        .unwrap();
        let request = ChatCompletionsRequest {
            module_id: "capability".into(),
            messages: None,
            tools: None,
            prompt: "hello".into(),
            max_tokens: 16,
            stream: false,
        };
        let response = transport.send(request.clone());
        assert_eq!(response.status, Some(200));
        assert!(response.parsed.is_some());
        let evidence = transport.evidence_payload(&request, &response).to_string();
        assert!(!evidence.contains("secret-value"));
        assert!(evidence.contains("choices"));
        server.join().unwrap();
    }

    #[test]
    fn parses_stream_events_when_sse_lines_are_split_across_network_chunks() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n")
                .unwrap();
            stream
                .write_all(b"data: {\"choices\":[{\"delta\":{\"content\":\"hel")
                .unwrap();
            stream.flush().unwrap();
            thread::sleep(Duration::from_millis(5));
            stream
                .write_all(b"lo\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\" world\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n")
                .unwrap();
            stream.flush().unwrap();
        });
        let transport = ChatCompletionsTransport::new(
            format!("http://{address}/v1/chat/completions"),
            "model-a",
            "secret-value",
            Duration::from_secs(5),
        )
        .unwrap();
        let stream = transport.send_stream(ChatCompletionsRequest {
            module_id: "performance".into(),
            messages: None,
            tools: None,
            prompt: "stream".into(),
            max_tokens: 16,
            stream: true,
        });
        assert_eq!(stream.response.status, Some(200));
        assert_eq!(stream.content, "hello world");
        assert!(stream.terminated);
        assert!(stream.events.len() >= 3);
        assert!(stream.response.parsed.is_some());
        server.join().unwrap();
    }

    #[test]
    fn agent_turn_sends_tools_and_parses_tool_calls() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 16 * 1024];
            let size = stream.read(&mut request).unwrap_or(0);
            let captured = String::from_utf8_lossy(&request[..size]).into_owned();
            let body = r#"{"choices":[{"message":{"role":"assistant","tool_calls":[{"id":"call-1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"runs/input.txt\"}"}}]}}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            captured
        });
        let transport = ChatCompletionsTransport::new(
            format!("http://{address}/v1/chat/completions"),
            "model-a",
            "secret-value",
            Duration::from_secs(5),
        )
        .unwrap();
        let turn = transport.send_agent_turn(
            vec![json!({"role": "user", "content": "read"})],
            vec![json!({"type": "function", "function": {"name": "read_file"}})],
        );
        let request = server.join().unwrap();
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0]["id"], "call-1");
        assert!(request.contains("\"tools\""));
        assert!(request.contains("\"tool_choice\":\"auto\""));
    }

    #[test]
    fn preserves_http_errors_without_turning_them_into_model_failures() {
        let (endpoint, server) = mock_server(
            429,
            r#"{"error":{"message":"slow down"}}"#,
            3,
            Duration::ZERO,
        );
        let transport = ChatCompletionsTransport::new(
            endpoint,
            "model-a",
            "secret-value",
            Duration::from_secs(5),
        )
        .unwrap();
        let response = transport.send(ChatCompletionsRequest {
            module_id: "performance".into(),
            messages: None,
            tools: None,
            prompt: "hello".into(),
            max_tokens: 16,
            stream: false,
        });
        assert_eq!(response.status, Some(429));
        assert_eq!(response.attempts, 3);
        assert_eq!(response.retry_reasons.len(), 2);
        assert_eq!(response.parsed.unwrap()["error"]["message"], "slow down");
        server.join().unwrap();
    }

    #[test]
    fn records_timeout_as_transport_error_after_retries() {
        let (endpoint, server) = mock_server(
            200,
            r#"{"choices":[{"message":{"content":"late"}}]}"#,
            1,
            Duration::from_millis(250),
        );
        let transport = ChatCompletionsTransport::new(
            endpoint,
            "model-a",
            "secret-value",
            Duration::from_millis(40),
        )
        .unwrap();
        let response = transport.send(ChatCompletionsRequest {
            module_id: "specification".into(),
            messages: None,
            tools: None,
            prompt: "hello".into(),
            max_tokens: 16,
            stream: false,
        });
        assert_eq!(response.status, None);
        assert!(response.error.is_some());
        assert_eq!(response.attempts, 3);
        server.join().unwrap();
    }
}
