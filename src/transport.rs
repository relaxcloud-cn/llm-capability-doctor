use crate::config::{ModelConfig, Protocol};
use anyhow::{Result, bail};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub const REQUEST_BUDGET: Duration = Duration::from_secs(180);
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub reasoning: Option<u64>,
}
impl TokenUsage {
    /// Missing reasoning accounting is never silently interpreted as zero.
    pub fn delivered_output(&self) -> Option<u64> {
        self.output?.checked_sub(self.reasoning?)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelResponse {
    pub text: String,
    pub calls: Vec<ToolCall>,
    pub normal_end: bool,
    pub refused: bool,
    pub finish_reason: String,
    pub model: Option<String>,
    pub usage: TokenUsage,
    pub wire: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HttpAttempt {
    pub number: u8,
    pub elapsed_ms: u64,
    pub http_status: Option<u16>,
    pub headers: BTreeMap<String, String>,
    pub response_body: String,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HttpEvidence {
    pub id: String,
    pub endpoint: String,
    pub protocol: Protocol,
    pub request: Value,
    pub stream: bool,
    pub elapsed_ms: u64,
    pub attempts: Vec<HttpAttempt>,
    pub parse_error: Option<String>,
    #[serde(default)]
    pub failure: Option<RequestFailure>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestFailure {
    Cancelled,
    Timeout,
    StageBudget,
    Network,
    Http(u16),
    InvalidResponse(String),
    EvidenceConflict(String),
}
#[derive(Debug)]
pub(crate) struct EvidenceConflict(&'static str);
impl std::fmt::Display for EvidenceConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for EvidenceConflict {}
impl RequestFailure {
    pub fn title(&self) -> String {
        match self {
            Self::Cancelled => "请求已取消".into(),
            Self::Timeout => "完整请求超时".into(),
            Self::StageBudget => "检测阶段预算已耗尽".into(),
            Self::Network => "无法连接模型服务".into(),
            Self::Http(code) => format!("模型服务返回 HTTP {code}"),
            Self::InvalidResponse(message) | Self::EvidenceConflict(message) => message.clone(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct RequestOutcome {
    pub evidence: HttpEvidence,
    pub response: Option<ModelResponse>,
    pub failure: Option<RequestFailure>,
}
#[derive(Clone)]
pub struct Transport {
    config: Arc<ModelConfig>,
    client: reqwest::Client,
    evidence_dir: Arc<PathBuf>,
    cancel: CancellationToken,
}
impl Transport {
    pub fn new(
        config: ModelConfig,
        evidence_dir: &Path,
        cancel: CancellationToken,
    ) -> Result<Self> {
        std::fs::create_dir_all(evidence_dir)?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .build()?;
        Ok(Self {
            config: Arc::new(config),
            client,
            evidence_dir: Arc::new(evidence_dir.to_owned()),
            cancel,
        })
    }
    pub fn config(&self) -> &ModelConfig {
        &self.config
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
    pub async fn send(
        &self,
        body: Value,
        stream: bool,
        stage_deadline: Instant,
    ) -> Result<RequestOutcome> {
        self.send_with_attempts(body, stream, stage_deadline, 3)
            .await
    }
    /// Capacity probes retain the first rejection; retrying would hide concurrency limits.
    pub async fn send_once(&self, body: Value, stage_deadline: Instant) -> Result<RequestOutcome> {
        self.send_with_attempts(body, false, stage_deadline, 1)
            .await
    }
    async fn send_with_attempts(
        &self,
        body: Value,
        stream: bool,
        stage_deadline: Instant,
        attempts: u8,
    ) -> Result<RequestOutcome> {
        let started = Instant::now();
        let deadline = stage_deadline.min(started + REQUEST_BUDGET);
        let id = format!("http-{}", Uuid::new_v4().simple());
        let safe_body = self.redact_json(body.clone());
        let mut evidence = HttpEvidence {
            id,
            endpoint: self.config.public_target().endpoint,
            protocol: self.config.protocol,
            request: safe_body,
            stream,
            elapsed_ms: 0,
            attempts: vec![],
            parse_error: None,
            failure: None,
        };
        let mut response = None;
        let mut failure = None;
        for number in 1..=attempts {
            if self.cancel.is_cancelled() {
                failure = Some(RequestFailure::Cancelled);
                break;
            }
            if Instant::now() >= deadline {
                failure = Some(RequestFailure::StageBudget);
                break;
            }
            let attempt_started = Instant::now();
            let request = self
                .client
                .post(self.config.endpoint.clone())
                .bearer_auth(self.config.api_key())
                .json(&body);
            let operation = async {
                let http = request.send().await.map_err(|_| RequestFailure::Network)?;
                let status = http.status().as_u16();
                let mut headers = BTreeMap::new();
                for name in [
                    "content-type",
                    "retry-after",
                    "x-request-id",
                    "request-id",
                    "date",
                ] {
                    if let Some(value) = http.headers().get(name).and_then(|v| v.to_str().ok()) {
                        headers.insert(name.into(), self.config.redact(value));
                    }
                }
                let mut bytes = Vec::new();
                let mut chunks = http.bytes_stream();
                while let Some(chunk) = chunks.next().await {
                    let chunk = chunk.map_err(|_| RequestFailure::Network)?;
                    if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
                        return Err(RequestFailure::InvalidResponse(
                            "响应超过检测器的安全大小限制".into(),
                        ));
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Ok((status, headers, bytes))
            };
            let outcome = tokio::select! {
                _=self.cancel.cancelled()=>Err(RequestFailure::Cancelled),
                value=tokio::time::timeout(deadline.saturating_duration_since(Instant::now()),operation)=>value.unwrap_or_else(|_|Err(if stage_deadline<started+REQUEST_BUDGET {RequestFailure::StageBudget}else{RequestFailure::Timeout})),
            };
            let mut retry_after = None;
            let mut attempt = HttpAttempt {
                number,
                elapsed_ms: attempt_started.elapsed().as_millis() as u64,
                http_status: None,
                headers: BTreeMap::new(),
                response_body: String::new(),
                error: None,
            };
            let retry = match outcome {
                Err(error) => {
                    attempt.error = Some(error.title());
                    let retry = matches!(error, RequestFailure::Network);
                    failure = Some(error);
                    retry
                }
                Ok((status, headers, bytes)) => {
                    attempt.http_status = Some(status);
                    attempt.headers = headers;
                    retry_after = attempt
                        .headers
                        .get("retry-after")
                        .and_then(|v| v.parse::<u64>().ok())
                        .map(Duration::from_secs);
                    attempt.response_body = self.config.redact(&String::from_utf8_lossy(&bytes));
                    if !(200..300).contains(&status) {
                        failure = Some(RequestFailure::Http(status));
                        status == 429 || status >= 500
                    } else {
                        let parsed = if stream {
                            parse_stream(self.config.protocol, &bytes)
                        } else {
                            parse_json(self.config.protocol, &bytes)
                        };
                        match parsed {
                            Ok(mut value) => {
                                value.text = self.config.redact(&value.text);
                                value.model = value.model.map(|v| self.config.redact(&v));
                                value.wire = self.redact_json(value.wire);
                                for call in &mut value.calls {
                                    call.arguments = self.redact_json(call.arguments.clone());
                                }
                                response = Some(value);
                                failure = None;
                            }
                            Err(error) => {
                                let text = self.config.redact(&error.to_string());
                                evidence.parse_error = Some(text.clone());
                                failure = Some(if error.is::<EvidenceConflict>() {
                                    RequestFailure::EvidenceConflict(text)
                                } else {
                                    RequestFailure::InvalidResponse(text)
                                });
                            }
                        };
                        false
                    }
                }
            };
            evidence.attempts.push(attempt);
            if !retry || number == attempts {
                break;
            }
            let delay = retry_after
                .unwrap_or_else(|| Duration::from_millis(400 * 2u64.pow((number - 1) as u32)));
            if delay >= deadline.saturating_duration_since(Instant::now()) {
                failure = Some(if stage_deadline < started + REQUEST_BUDGET {
                    RequestFailure::StageBudget
                } else {
                    RequestFailure::Timeout
                });
                break;
            }
            tokio::select! { _=self.cancel.cancelled()=>{failure=Some(RequestFailure::Cancelled);break;},_=tokio::time::sleep(delay)=>{} }
        }
        evidence.elapsed_ms = started.elapsed().as_millis() as u64;
        evidence.failure = failure.clone();
        // Every attempt is preserved, including temporary failures and cancellation.
        let path = self.evidence_dir.join(format!("{}.json", evidence.id));
        std::fs::write(path, serde_json::to_vec_pretty(&evidence)?)?;
        Ok(RequestOutcome {
            evidence,
            response,
            failure,
        })
    }
    fn redact_json(&self, value: Value) -> Value {
        match value {
            Value::String(text) => Value::String(self.config.redact(&text)),
            Value::Array(values) => {
                Value::Array(values.into_iter().map(|v| self.redact_json(v)).collect())
            }
            Value::Object(values) => Value::Object(
                values
                    .into_iter()
                    .map(|(key, value)| {
                        let lower = key.to_ascii_lowercase();
                        let value = if matches!(
                            lower.as_str(),
                            "authorization" | "api_key" | "apikey" | "api-key"
                        ) {
                            Value::String("[REDACTED]".into())
                        } else {
                            self.redact_json(value)
                        };
                        (key, value)
                    })
                    .collect(),
            ),
            value => value,
        }
    }
}

pub fn messages_body(
    config: &ModelConfig,
    messages: Value,
    output_limit: u64,
    stream: bool,
) -> Value {
    match config.protocol {
        Protocol::ChatCompletions => {
            json!({"model":config.model,"messages":messages,"max_tokens":output_limit,"stream":stream})
        }
        Protocol::Responses => {
            json!({"model":config.model,"input":messages,"max_output_tokens":output_limit,"stream":stream,"store":false})
        }
    }
}
pub fn add_schema(body: &mut Value, protocol: Protocol, schema: Value) {
    match protocol {
        Protocol::ChatCompletions => {
            body["response_format"] = json!({"type":"json_schema","json_schema":{"name":"records","strict":true,"schema":schema}})
        }
        Protocol::Responses => {
            body["text"] = json!({"format":{"type":"json_schema","name":"records","strict":true,"schema":schema}})
        }
    }
}
pub fn add_tools(body: &mut Value, protocol: Protocol, tools: &[Value]) {
    body["tools"] = Value::Array(
        tools
            .iter()
            .map(|tool| match protocol {
                Protocol::ChatCompletions => json!({"type":"function","function":tool}),
                Protocol::Responses => {
                    let mut tool = tool.clone();
                    tool["type"] = json!("function");
                    tool
                }
            })
            .collect(),
    );
}
pub fn parse_json(protocol: Protocol, bytes: &[u8]) -> Result<ModelResponse> {
    let wire: Value =
        serde_json::from_slice(bytes).map_err(|_| anyhow::anyhow!("模型接口没有返回有效 JSON"))?;
    normalize(protocol, wire)
}
pub fn normalize(protocol: Protocol, wire: Value) -> Result<ModelResponse> {
    let mut text = String::new();
    let mut calls = vec![];
    let mut refused = false;
    let (finish_reason, normal_end, usage) = match protocol {
        Protocol::ChatCompletions => {
            let choices = wire["choices"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("响应缺少 choices"))?;
            if choices.len() != 1 {
                bail!("响应需要包含一次调用对应的唯一结果");
            }
            let message = &choices[0]["message"];
            if let Some(content) = message["content"].as_str() {
                text = content.to_owned();
            }
            refused = message["refusal"].as_str().is_some_and(|v| !v.is_empty());
            for call in message["tool_calls"].as_array().into_iter().flatten() {
                if call["type"] != "function" {
                    bail!("工具调用类型不符合协议");
                }
                calls.push(parse_call(
                    &call["id"],
                    &call["function"]["name"],
                    &call["function"]["arguments"],
                )?);
            }
            let finish = choices[0]["finish_reason"]
                .as_str()
                .unwrap_or("missing")
                .to_owned();
            let normal = matches!(finish.as_str(), "stop" | "tool_calls") && !refused;
            (
                finish,
                normal,
                TokenUsage {
                    input: wire["usage"]["prompt_tokens"].as_u64(),
                    output: wire["usage"]["completion_tokens"].as_u64(),
                    reasoning: wire["usage"]["completion_tokens_details"]["reasoning_tokens"]
                        .as_u64(),
                },
            )
        }
        Protocol::Responses => {
            let output = wire["output"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("响应缺少 output"))?;
            for item in output {
                match item["type"].as_str() {
                    Some("message") => {
                        for content in item["content"].as_array().into_iter().flatten() {
                            match content["type"].as_str() {
                                Some("output_text") => text.push_str(
                                    content["text"]
                                        .as_str()
                                        .ok_or_else(|| anyhow::anyhow!("文本内容类型错误"))?,
                                ),
                                Some("refusal") => refused = true,
                                _ => {}
                            }
                        }
                    }
                    Some("function_call") => calls.push(parse_call(
                        &item["call_id"],
                        &item["name"],
                        &item["arguments"],
                    )?),
                    _ => {}
                }
            }
            let finish = wire["status"].as_str().unwrap_or("missing").to_owned();
            let normal = finish == "completed" && !refused && wire["error"].is_null();
            (
                finish,
                normal,
                TokenUsage {
                    input: wire["usage"]["input_tokens"].as_u64(),
                    output: wire["usage"]["output_tokens"].as_u64(),
                    reasoning: wire["usage"]["output_tokens_details"]["reasoning_tokens"].as_u64(),
                },
            )
        }
    };
    let mut ids = std::collections::BTreeSet::new();
    if calls.iter().any(|call| !ids.insert(call.id.clone())) {
        bail!("工具调用 ID 重复");
    }
    let model = wire["model"].as_str().map(str::to_owned);
    Ok(ModelResponse {
        text,
        calls,
        normal_end,
        refused,
        finish_reason,
        model,
        usage,
        wire,
    })
}
fn parse_call(id: &Value, name: &Value, args: &Value) -> Result<ToolCall> {
    let id = id
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("工具调用缺少 ID"))?;
    let name = name
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("工具调用缺少名称"))?;
    let raw = args
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("工具参数不符合协议"))?;
    let arguments: Value =
        serde_json::from_str(raw).map_err(|_| anyhow::anyhow!("工具参数不是有效 JSON"))?;
    if !arguments.is_object() {
        bail!("工具参数需要是 JSON 对象");
    }
    Ok(ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    })
}

/// Delimit events as bytes before UTF-8 decoding; Chinese code points may span network chunks.
#[derive(Default)]
pub struct SseDecoder {
    pending: Vec<u8>,
    lines: Vec<String>,
    frames: Vec<String>,
}
impl SseDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<()> {
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let bytes: Vec<_> = self.pending.drain(..=end).collect();
            let line = std::str::from_utf8(&bytes[..bytes.len() - 1])
                .map_err(|_| anyhow::anyhow!("流式响应包含无效 UTF-8"))?
                .trim_end_matches('\r');
            if line.is_empty() {
                if !self.lines.is_empty() {
                    self.frames.push(self.lines.join("\n"));
                    self.lines.clear();
                }
            } else if let Some(data) = line.strip_prefix("data:") {
                self.lines
                    .push(data.strip_prefix(' ').unwrap_or(data).into());
            }
        }
        Ok(())
    }
    pub fn finish(self) -> Result<Vec<String>> {
        if !self.pending.is_empty() || !self.lines.is_empty() {
            bail!("流式响应在完整事件结束前断开");
        }
        Ok(self.frames)
    }
}
pub fn parse_stream(protocol: Protocol, bytes: &[u8]) -> Result<ModelResponse> {
    let mut decoder = SseDecoder::default();
    decoder.push(bytes)?;
    let frames = decoder.finish()?;
    match protocol {
        Protocol::ChatCompletions => {
            let mut message = json!({"role":"assistant","content":""});
            let mut text = String::new();
            let mut reasoning = String::new();
            let mut finish = Value::Null;
            let mut model = Value::Null;
            let mut usage = Value::Null;
            let mut done = false;
            let mut tools: BTreeMap<u64, (String, String, String)> = BTreeMap::new();
            for frame in frames {
                if frame == "[DONE]" {
                    done = true;
                    continue;
                }
                if done {
                    bail!("流式结束标记之后仍收到结果");
                }
                let chunk: Value = serde_json::from_str(&frame)
                    .map_err(|_| anyhow::anyhow!("流式响应事件不是有效 JSON"))?;
                if !chunk["error"].is_null() {
                    bail!("流式响应返回服务错误");
                }
                if !chunk["model"].is_null() {
                    model = chunk["model"].clone();
                }
                if !chunk["usage"].is_null() {
                    usage = chunk["usage"].clone();
                }
                for choice in chunk["choices"].as_array().into_iter().flatten() {
                    if choice["index"].as_u64() != Some(0) {
                        bail!("流式响应包含未请求的结果序号");
                    }
                    let delta = &choice["delta"];
                    if let Some(value) = delta["content"].as_str() {
                        text.push_str(value);
                    }
                    if let Some(value) = delta["reasoning_content"].as_str() {
                        reasoning.push_str(value);
                    }
                    if let Some(value) = delta["refusal"].as_str() {
                        message["refusal"] = json!(value);
                    }
                    for tool in delta["tool_calls"].as_array().into_iter().flatten() {
                        let index = tool["index"]
                            .as_u64()
                            .ok_or_else(|| anyhow::anyhow!("流式工具参数缺少序号"))?;
                        let entry = tools.entry(index).or_default();
                        if let Some(value) = tool["id"].as_str().filter(|v| !v.is_empty()) {
                            if !entry.0.is_empty() && entry.0 != value {
                                return Err(EvidenceConflict(
                                    "同一流式工具调用出现冲突 ID，关联未确认",
                                )
                                .into());
                            }
                            entry.0 = value.to_owned();
                        }
                        if let Some(value) =
                            tool["function"]["name"].as_str().filter(|v| !v.is_empty())
                        {
                            if !entry.1.is_empty() && entry.1 != value {
                                return Err(EvidenceConflict(
                                    "同一流式工具调用出现冲突名称，关联未确认",
                                )
                                .into());
                            }
                            entry.1 = value.to_owned();
                        }
                        if let Some(value) = tool["function"]["arguments"].as_str() {
                            entry.2.push_str(value);
                        }
                    }
                    if !choice["finish_reason"].is_null() {
                        finish = choice["finish_reason"].clone();
                    }
                }
            }
            if !done {
                bail!("流式响应缺少完整结束标记");
            }
            message["content"] = json!(text);
            if !reasoning.is_empty() {
                message["reasoning_content"] = json!(reasoning);
            }
            if !tools.is_empty() {
                message["tool_calls"]=Value::Array(tools.into_values().map(|(id,name,args)|json!({"id":id,"type":"function","function":{"name":name,"arguments":args}})).collect());
            }
            normalize(
                protocol,
                json!({"model":model,"usage":usage,"choices":[{"message":message,"finish_reason":finish}]}),
            )
        }
        Protocol::Responses => {
            let mut text = String::new();
            let mut terminal = None;
            let mut argument_deltas: BTreeMap<String, String> = BTreeMap::new();
            for frame in frames {
                if frame == "[DONE]" {
                    continue;
                }
                let event: Value = serde_json::from_str(&frame)
                    .map_err(|_| anyhow::anyhow!("流式响应事件不是有效 JSON"))?;
                match event["type"].as_str() {
                    Some("response.output_text.delta") => text.push_str(
                        event["delta"]
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("流式文本类型错误"))?,
                    ),
                    Some("response.function_call_arguments.delta") => {
                        let id = event["item_id"]
                            .as_str()
                            .ok_or_else(|| anyhow::anyhow!("流式工具参数缺少关联 ID"))?;
                        argument_deltas.entry(id.into()).or_default().push_str(
                            event["delta"]
                                .as_str()
                                .ok_or_else(|| anyhow::anyhow!("流式工具参数类型错误"))?,
                        );
                    }
                    Some("response.completed" | "response.incomplete" | "response.failed") => {
                        if terminal.is_some() {
                            bail!("重复的流式结束事件");
                        }
                        terminal = Some(event["response"].clone());
                    }
                    Some("error") => bail!("流式响应返回服务错误"),
                    _ => {}
                }
            }
            let wire = terminal.ok_or_else(|| anyhow::anyhow!("流式响应中途断开，缺少最终事件"))?;
            let normalized = normalize(protocol, wire.clone())?;
            if normalized.text != text {
                bail!("流式文本与最终响应不一致，内容缺失");
            }
            for item in wire["output"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|v| v["type"] == "function_call")
            {
                let id = item["id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("工具项缺少关联 ID"))?;
                let expected = item["arguments"].as_str().unwrap_or("");
                if argument_deltas.get(id).map(String::as_str) != Some(expected) {
                    bail!("流式工具参数与最终响应不一致");
                }
            }
            Ok(normalized)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf8_characters_and_crlf_can_split_at_every_byte() {
        let source = "data: {\"text\":\"中文\"}\r\n\r\ndata: [DONE]\n\n";
        let mut decoder = SseDecoder::default();
        for byte in source.as_bytes() {
            decoder.push(&[*byte]).unwrap();
        }
        assert_eq!(
            decoder.finish().unwrap(),
            vec!["{\"text\":\"中文\"}", "[DONE]"]
        );
    }
    #[test]
    fn tool_argument_fragments_are_assembled_and_invalid_or_missing_endings_rejected() {
        let chunks = [
            json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call-1","function":{"name":"lookup","arguments":"{\"id\":"}}]},"finish_reason":null}]}),
            json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"中文\"}"}}]},"finish_reason":"tool_calls"}]}),
        ];
        let stream = chunks
            .iter()
            .map(|v| format!("data: {v}\n\n"))
            .collect::<String>();
        assert!(parse_stream(Protocol::ChatCompletions, stream.as_bytes()).is_err());
        let complete = stream + "data: [DONE]\n\n";
        let result = parse_stream(Protocol::ChatCompletions, complete.as_bytes()).unwrap();
        assert!(result.normal_end);
        assert_eq!(result.calls[0].arguments["id"], "中文");
        let malformed = complete.replace("\\\"中文\\\"}", "oops");
        assert!(parse_stream(Protocol::ChatCompletions, malformed.as_bytes()).is_err());
    }
    #[test]
    fn length_refusal_and_failed_responses_are_not_normal_completion() {
        let c = json!({"choices":[{"message":{"content":"partial"},"finish_reason":"length"}]});
        assert!(!normalize(Protocol::ChatCompletions, c).unwrap().normal_end);
        let r = json!({"status":"completed","output":[{"type":"message","content":[{"type":"refusal","refusal":"no"}]}]});
        assert!(!normalize(Protocol::Responses, r).unwrap().normal_end);
        let failed = json!({"status":"failed","error":{"code":"server_error"},"output":[]});
        assert!(!normalize(Protocol::Responses, failed).unwrap().normal_end);
    }
    #[test]
    fn responses_stream_requires_full_final_text_and_tool_arguments() {
        let wire = json!({"model":"m","status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"你好"}]}]});
        let frames = [
            json!({"type":"response.output_text.delta","delta":"你好"}),
            json!({"type":"response.completed","response":wire}),
        ];
        let stream = frames
            .iter()
            .map(|v| format!("data: {v}\n\n"))
            .collect::<String>();
        assert!(
            parse_stream(Protocol::Responses, stream.as_bytes())
                .unwrap()
                .normal_end
        );
        assert!(
            parse_stream(
                Protocol::Responses,
                stream
                    .replace("\"delta\":\"你好\"", "\"delta\":\"你\"")
                    .as_bytes()
            )
            .is_err()
        );
    }
    #[test]
    fn output_tokens_cannot_include_reasoning_or_invent_missing_accounting() {
        assert_eq!(
            TokenUsage {
                input: Some(20),
                output: Some(100),
                reasoning: Some(30)
            }
            .delivered_output(),
            Some(70)
        );
        assert_eq!(
            TokenUsage {
                input: Some(20),
                output: Some(100),
                reasoning: None
            }
            .delivered_output(),
            None
        );
    }
}
