use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};
use thiserror::Error;
use url::Url;

use super::stream::{AssistantTurn, ProtocolHistory, ToolCall, ToolCorrelation};
use super::{ANTHROPIC_MAX_TOKENS, Protocol, RequestSpec};

pub fn tool_prompt(id: &str) -> Option<&'static str> {
    match id {
        "040" => Some("MODEL_DOCTOR_CASE_040. Use get_weather for Beijing."),
        "041" => Some("MODEL_DOCTOR_CASE_041. Choose the correct tool to get weather for Beijing."),
        "042" => {
            Some("MODEL_DOCTOR_CASE_042. Do not use any tool. Reply only MODEL_DOCTOR_CASE_042_OK.")
        }
        "043" => Some("MODEL_DOCTOR_CASE_043. Use get_weather for Beijing, unit C, for 3 days."),
        "044" => {
            Some("MODEL_DOCTOR_CASE_044. Use inspect_target for host example.com and port 443.")
        }
        "045" => Some(
            "MODEL_DOCTOR_CASE_045. In one response call get_weather for Beijing and get_time for UTC.",
        ),
        "046" => Some(
            "MODEL_DOCTOR_CASE_046. Call get_weather for Beijing. After its result, reply only MODEL_DOCTOR_CASE_046_OK.",
        ),
        "047" => Some(
            "MODEL_DOCTOR_CASE_047. First call get_weather for Beijing. After its result, call get_time for UTC. After that result, reply only MODEL_DOCTOR_CASE_047_OK.",
        ),
        "048" => Some(
            "MODEL_DOCTOR_CASE_048. Call get_weather for Beijing. After the result, reply MODEL_DOCTOR_CASE_048_OK followed by the exact tool result.",
        ),
        "049" => Some(
            "MODEL_DOCTOR_CASE_049. Call get_weather for Beijing. If it returns a timeout error, retry get_weather exactly once. After a successful retry, reply only MODEL_DOCTOR_CASE_049_OK.",
        ),
        "050" => Some(
            "MODEL_DOCTOR_CASE_050. Choose get_weather for Beijing from the available tool catalog.",
        ),
        _ => None,
    }
}

pub fn tool_request(protocol: Protocol, model: &str, id: &str, prompt: &str) -> RequestSpec {
    let tools = tool_definitions(protocol, id);
    let parallel = id == "045";
    let stream = matches!(id, "046" | "047" | "048" | "049");
    let mut body = match protocol {
        Protocol::OpenAiResponses => json!({
            "model": model,
            "input": prompt,
            "tools": tools,
            "tool_choice": "auto",
            "parallel_tool_calls": parallel,
            "store": true
        }),
        Protocol::AnthropicMessages => json!({
            "model": model,
            "max_tokens": ANTHROPIC_MAX_TOKENS,
            "messages": [{"role": "user", "content": prompt}],
            "tools": tools
        }),
        Protocol::GeminiGenerateContent => json!({
            "contents": [{"role": "user", "parts": [{"text": prompt}]}],
            "tools": [{"functionDeclarations": tools}]
        }),
        Protocol::OpenAiChat => json!({
            "model": model,
            "messages": [{"role": "user", "content": prompt}],
            "tools": tools,
            "tool_choice": "auto",
            "parallel_tool_calls": parallel,
            "stream": false
        }),
        Protocol::OllamaChat => json!({
            "model": model,
            "messages": [{"role": "user", "content": prompt}],
            "tools": tools,
            "stream": false
        }),
        Protocol::Unknown => json!({
            "model": model,
            "messages": [{"role": "user", "content": prompt}],
            "tools": tools,
            "stream": false
        }),
    };
    if stream && protocol != Protocol::GeminiGenerateContent {
        body.as_object_mut()
            .expect("tool request bodies are JSON objects")
            .insert("stream".into(), Value::Bool(true));
    }
    RequestSpec { body, stream }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExecutedToolResult {
    pub call: ToolCall,
    pub output: String,
    pub is_error: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum ToolRequestPhase<'a> {
    Initial,
    FollowUp {
        previous: &'a AssistantTurn,
        results: &'a [ExecutedToolResult],
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolConversation {
    protocol: Protocol,
    current_body: Value,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ToolProtocolError {
    #[error("tool requests are unavailable for unknown protocol")]
    UnsupportedProtocol,
    #[error("tool request does not satisfy the official protocol contract")]
    InvalidRequest(Vec<String>),
}

impl ToolProtocolError {
    pub fn codes(&self) -> &[String] {
        match self {
            Self::UnsupportedProtocol => &[],
            Self::InvalidRequest(codes) => codes,
        }
    }
}

impl ToolConversation {
    pub fn from_initial(protocol: Protocol, body: Value) -> Result<Self, ToolProtocolError> {
        if protocol == Protocol::Unknown {
            return Err(ToolProtocolError::UnsupportedProtocol);
        }
        let errors = validate_body(protocol, &body, ToolRequestPhase::Initial);
        if !errors.is_empty() {
            return Err(ToolProtocolError::InvalidRequest(errors));
        }
        Ok(Self {
            protocol,
            current_body: body,
        })
    }

    pub fn current_request(&self) -> RequestSpec {
        RequestSpec {
            body: self.current_body.clone(),
            stream: true,
        }
    }

    pub fn append_follow_up(
        &mut self,
        turn: &AssistantTurn,
        results: &[ExecutedToolResult],
    ) -> Result<RequestSpec, ToolProtocolError> {
        if self.protocol == Protocol::Unknown {
            return Err(ToolProtocolError::UnsupportedProtocol);
        }

        let ordered = correlate_results(self.protocol, &turn.tool_calls, results)?;
        let history = history_for(self.protocol, &turn.history)?;
        let mut next = self.current_body.clone();

        match self.protocol {
            Protocol::OpenAiChat => {
                let messages = array_mut(&mut next, "messages", self.protocol)?;
                messages.push(history);
                for (call, result) in turn.tool_calls.iter().zip(ordered) {
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": required_correlation_id(self.protocol, call)?,
                        "content": text_output(result),
                    }));
                }
                next["stream"] = Value::Bool(true);
            }
            Protocol::OpenAiResponses => {
                let ProtocolHistory::OpenAiResponses { response_id } = &turn.history else {
                    unreachable!("history_for validated the variant")
                };
                next["previous_response_id"] = Value::String(response_id.clone());
                next["input"] = Value::Array(
                    turn.tool_calls
                        .iter()
                        .zip(ordered)
                        .map(|(call, result)| {
                            json!({
                                "type": "function_call_output",
                                "call_id": required_correlation_id(self.protocol, call)
                                    .expect("correlation was validated"),
                                "output": text_output(result),
                            })
                        })
                        .collect(),
                );
                next["store"] = Value::Bool(true);
                next["stream"] = Value::Bool(true);
            }
            Protocol::AnthropicMessages => {
                let messages = array_mut(&mut next, "messages", self.protocol)?;
                messages.push(json!({"role": "assistant", "content": history}));
                let content = turn
                    .tool_calls
                    .iter()
                    .zip(ordered)
                    .map(|(call, result)| {
                        let mut block = Map::from_iter([
                            ("type".into(), Value::String("tool_result".into())),
                            (
                                "tool_use_id".into(),
                                Value::String(
                                    required_correlation_id(self.protocol, call)
                                        .expect("correlation was validated"),
                                ),
                            ),
                            ("content".into(), Value::String(text_output(result).into())),
                        ]);
                        if result.is_error {
                            block.insert("is_error".into(), Value::Bool(true));
                        }
                        Value::Object(block)
                    })
                    .collect::<Vec<_>>();
                messages.push(json!({"role": "user", "content": content}));
                next["stream"] = Value::Bool(true);
            }
            Protocol::GeminiGenerateContent => {
                let contents = array_mut(&mut next, "contents", self.protocol)?;
                contents.push(history);
                let parts = turn
                    .tool_calls
                    .iter()
                    .zip(ordered)
                    .map(|(call, result)| {
                        let response = if result.is_error {
                            json!({"error": "timeout"})
                        } else {
                            json!({"result": result.output})
                        };
                        let mut function_response = Map::from_iter([
                            ("name".into(), Value::String(call.name.clone())),
                            ("response".into(), response),
                        ]);
                        if let ToolCorrelation::Optional(Some(id)) = &call.correlation {
                            function_response.insert("id".into(), Value::String(id.clone()));
                        }
                        json!({"functionResponse": Value::Object(function_response)})
                    })
                    .collect::<Vec<_>>();
                contents.push(json!({"role": "user", "parts": parts}));
                next.as_object_mut()
                    .expect("validated Gemini body is an object")
                    .remove("stream");
            }
            Protocol::OllamaChat => {
                let messages = array_mut(&mut next, "messages", self.protocol)?;
                messages.push(history);
                for (call, result) in turn.tool_calls.iter().zip(ordered) {
                    let mut message = Map::from_iter([
                        ("role".into(), Value::String("tool".into())),
                        ("tool_name".into(), Value::String(call.name.clone())),
                        ("content".into(), Value::String(text_output(result).into())),
                    ]);
                    if let ToolCorrelation::Optional(Some(id)) = &call.correlation {
                        message.insert("tool_call_id".into(), Value::String(id.clone()));
                    }
                    messages.push(Value::Object(message));
                }
                next["stream"] = Value::Bool(true);
            }
            Protocol::Unknown => return Err(ToolProtocolError::UnsupportedProtocol),
        }

        let body_errors = validate_body(
            self.protocol,
            &next,
            ToolRequestPhase::FollowUp {
                previous: turn,
                results,
            },
        );
        if !body_errors.is_empty() {
            return Err(ToolProtocolError::InvalidRequest(body_errors));
        }

        self.current_body = next;
        Ok(self.current_request())
    }
}

fn text_output(result: &ExecutedToolResult) -> &str {
    if result.is_error {
        "ERROR: timeout"
    } else {
        &result.output
    }
}

fn array_mut<'a>(
    body: &'a mut Value,
    field: &str,
    protocol: Protocol,
) -> Result<&'a mut Vec<Value>, ToolProtocolError> {
    body.get_mut(field)
        .and_then(Value::as_array_mut)
        .ok_or_else(|| {
            ToolProtocolError::InvalidRequest(vec![request_error(
                protocol,
                "history_mismatch",
                &format!("/{field}"),
            )])
        })
}

fn history_for(protocol: Protocol, history: &ProtocolHistory) -> Result<Value, ToolProtocolError> {
    let value = match (protocol, history) {
        (Protocol::OpenAiChat, ProtocolHistory::OpenAiChat(value))
        | (Protocol::GeminiGenerateContent, ProtocolHistory::Gemini(value))
        | (Protocol::OllamaChat, ProtocolHistory::Ollama(value)) => value.clone(),
        (Protocol::AnthropicMessages, ProtocolHistory::Anthropic(content)) => {
            Value::Array(content.clone())
        }
        (Protocol::OpenAiResponses, ProtocolHistory::OpenAiResponses { response_id })
            if response_id.trim().is_empty() =>
        {
            return Err(ToolProtocolError::InvalidRequest(vec![request_error(
                protocol,
                "previous_response_id",
                "/history/response_id",
            )]));
        }
        (Protocol::OpenAiResponses, ProtocolHistory::OpenAiResponses { response_id }) => {
            Value::String(response_id.clone())
        }
        _ => {
            return Err(ToolProtocolError::InvalidRequest(vec![request_error(
                protocol,
                "history_mismatch",
                "/history",
            )]));
        }
    };
    Ok(value)
}

fn required_correlation_id(
    protocol: Protocol,
    call: &ToolCall,
) -> Result<String, ToolProtocolError> {
    match &call.correlation {
        ToolCorrelation::Required(id) if !id.is_empty() => Ok(id.clone()),
        _ => Err(ToolProtocolError::InvalidRequest(vec![request_error(
            protocol,
            "missing_call_id",
            "/tool_calls",
        )])),
    }
}

fn correlate_results<'a>(
    protocol: Protocol,
    calls: &[ToolCall],
    results: &'a [ExecutedToolResult],
) -> Result<Vec<&'a ExecutedToolResult>, ToolProtocolError> {
    let mut errors = BTreeSet::new();
    if calls.len() != results.len() {
        errors.insert(request_error(protocol, "result_count_mismatch", "/results"));
    }

    let mut ordered = vec![None; calls.len()];
    let requires_ids = matches!(
        protocol,
        Protocol::OpenAiChat | Protocol::OpenAiResponses | Protocol::AnthropicMessages
    );
    let mut expected_by_id = BTreeMap::new();
    for (position, call) in calls.iter().enumerate() {
        let id = match (&call.correlation, requires_ids) {
            (ToolCorrelation::Required(id), true) if !id.is_empty() => Some(id),
            (ToolCorrelation::Optional(Some(id)), false) if !id.is_empty() => Some(id),
            (ToolCorrelation::Optional(None), false) => None,
            _ => {
                errors.insert(request_error(
                    protocol,
                    if requires_ids {
                        "missing_call_id"
                    } else {
                        "mismatched_call_id"
                    },
                    "/tool_calls",
                ));
                None
            }
        };
        if let Some(id) = id
            && expected_by_id.insert(id.clone(), position).is_some()
        {
            errors.insert(request_error(protocol, "duplicate_call_id", "/tool_calls"));
        }
    }

    let mut seen_ids = BTreeSet::new();
    for result in results {
        if result.is_error && result.output != "ERROR: timeout" {
            errors.insert(request_error(protocol, "output", "/results"));
        }
        let result_id = match &result.call.correlation {
            ToolCorrelation::Required(id) | ToolCorrelation::Optional(Some(id))
                if !id.is_empty() =>
            {
                Some(id)
            }
            ToolCorrelation::Optional(None) => None,
            _ => {
                errors.insert(request_error(protocol, "missing_call_id", "/results"));
                continue;
            }
        };
        let position = if let Some(result_id) = result_id {
            if !seen_ids.insert(result_id.clone()) {
                errors.insert(request_error(protocol, "duplicate_call_id", "/results"));
                continue;
            }
            let Some(position) = expected_by_id.get(result_id).copied() else {
                errors.insert(request_error(protocol, "stale_call_id", "/results"));
                continue;
            };
            position
        } else {
            if requires_ids {
                errors.insert(request_error(protocol, "missing_call_id", "/results"));
                continue;
            }
            let position = calls.iter().enumerate().find_map(|(position, call)| {
                (ordered[position].is_none()
                    && call.correlation == ToolCorrelation::Optional(None)
                    && call.name == result.call.name)
                    .then_some(position)
            });
            let Some(position) = position else {
                errors.insert(request_error(protocol, "name_mismatch", "/results"));
                continue;
            };
            position
        };

        let expected_call = &calls[position];
        if result.call.name != expected_call.name {
            errors.insert(request_error(protocol, "name_mismatch", "/results"));
        }
        if result.call.index != expected_call.index
            || result.call.arguments != expected_call.arguments
            || result.call.correlation != expected_call.correlation
        {
            errors.insert(request_error(protocol, "mismatched_call_id", "/results"));
        }
        if ordered[position].replace(result).is_some() {
            errors.insert(request_error(protocol, "duplicate_call_id", "/results"));
        }
    }

    if ordered.iter().any(Option::is_none) {
        errors.insert(request_error(protocol, "missing_call_id", "/results"));
    }
    if !errors.is_empty() {
        return Err(ToolProtocolError::InvalidRequest(
            errors.into_iter().collect(),
        ));
    }
    Ok(ordered
        .into_iter()
        .map(|result| result.expect("all results were correlated"))
        .collect())
}

fn request_error(protocol: Protocol, code: &str, pointer: &str) -> String {
    format!("request.{}.{}:{pointer}", protocol_name(protocol), code)
}

fn protocol_name(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::OpenAiChat => "openai_chat",
        Protocol::OpenAiResponses => "openai_responses",
        Protocol::AnthropicMessages => "anthropic",
        Protocol::GeminiGenerateContent => "gemini",
        Protocol::OllamaChat => "ollama",
        Protocol::Unknown => "unknown",
    }
}

pub fn validate_tool_request(
    protocol: Protocol,
    endpoint: &Url,
    stream: bool,
    body: &Value,
    phase: ToolRequestPhase<'_>,
) -> Vec<String> {
    if protocol == Protocol::Unknown {
        return vec![request_error(protocol, "unsupported_protocol", "/")];
    }

    let mut errors = BTreeSet::from_iter(validate_body(protocol, body, phase));
    if !stream {
        errors.insert(request_error(protocol, "stream_required", "/stream"));
    }
    if protocol == Protocol::GeminiGenerateContent {
        if !endpoint.path().ends_with(":streamGenerateContent") {
            errors.insert(request_error(protocol, "invalid_stream_action", "/url"));
        }
        let alt_values = endpoint
            .query_pairs()
            .filter_map(|(key, value)| (key == "alt").then_some(value.into_owned()))
            .collect::<Vec<_>>();
        if alt_values != ["sse"] {
            errors.insert(request_error(protocol, "invalid_alt", "/url"));
        }
    }
    errors.into_iter().collect()
}

fn validate_body(protocol: Protocol, body: &Value, phase: ToolRequestPhase<'_>) -> Vec<String> {
    let mut errors = BTreeSet::new();
    let Some(object) = body.as_object() else {
        errors.insert(request_error(protocol, "invalid_body", "/"));
        return errors.into_iter().collect();
    };

    if protocol == Protocol::GeminiGenerateContent {
        if object.contains_key("stream") {
            errors.insert(request_error(protocol, "body_stream_forbidden", "/stream"));
        }
    } else if object.get("stream") != Some(&Value::Bool(true)) {
        errors.insert(request_error(protocol, "body_stream_required", "/stream"));
    }

    validate_tool_shapes(protocol, body, &mut errors);
    validate_cross_protocol_fields(protocol, object, &mut errors);
    if protocol == Protocol::OpenAiResponses && object.get("store") != Some(&Value::Bool(true)) {
        errors.insert(request_error(protocol, "store", "/store"));
    }

    match phase {
        ToolRequestPhase::Initial => validate_initial_profile(protocol, body, &mut errors),
        ToolRequestPhase::FollowUp { previous, results } => {
            validate_follow_up(protocol, body, previous, results, &mut errors);
        }
    }
    errors.into_iter().collect()
}

fn validate_initial_profile(protocol: Protocol, body: &Value, errors: &mut BTreeSet<String>) {
    if protocol != Protocol::GeminiGenerateContent
        && body
            .get("model")
            .and_then(Value::as_str)
            .is_none_or(|model| model.trim().is_empty())
    {
        errors.insert(request_error(protocol, "invalid_body", "/model"));
    }
    if body.get("previous_response_id").is_some() {
        errors.insert(request_error(
            protocol,
            "unexpected_previous_response_id",
            "/previous_response_id",
        ));
    }

    let has_prompt = match protocol {
        Protocol::OpenAiChat | Protocol::OllamaChat | Protocol::AnthropicMessages => body
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|messages| messages.first())
            .is_some_and(|message| {
                message.get("role").and_then(Value::as_str) == Some("user")
                    && nonempty_content(message.get("content"))
            }),
        Protocol::OpenAiResponses => nonempty_content(body.get("input")),
        Protocol::GeminiGenerateContent => body
            .get("contents")
            .and_then(Value::as_array)
            .and_then(|contents| contents.first())
            .is_some_and(|content| {
                content.get("role").and_then(Value::as_str) == Some("user")
                    && content
                        .get("parts")
                        .and_then(Value::as_array)
                        .is_some_and(|parts| {
                            parts.iter().any(|part| {
                                part.get("text")
                                    .and_then(Value::as_str)
                                    .is_some_and(|text| !text.trim().is_empty())
                            })
                        })
            }),
        Protocol::Unknown => false,
    };
    if !has_prompt {
        let pointer = match protocol {
            Protocol::OpenAiChat | Protocol::OllamaChat | Protocol::AnthropicMessages => {
                "/messages"
            }
            Protocol::OpenAiResponses => "/input",
            Protocol::GeminiGenerateContent => "/contents",
            Protocol::Unknown => "/",
        };
        errors.insert(request_error(protocol, "invalid_body", pointer));
    }
}

fn nonempty_content(value: Option<&Value>) -> bool {
    match value {
        Some(Value::String(text)) => !text.trim().is_empty(),
        Some(Value::Array(values)) => !values.is_empty(),
        Some(Value::Object(values)) => !values.is_empty(),
        _ => false,
    }
}

fn validate_tool_shapes(protocol: Protocol, body: &Value, errors: &mut BTreeSet<String>) {
    let Some(tools) = body.get("tools").and_then(Value::as_array) else {
        errors.insert(request_error(protocol, "invalid_tool_shape", "/tools"));
        return;
    };
    if tools.is_empty() {
        errors.insert(request_error(protocol, "invalid_tool_shape", "/tools"));
        return;
    }

    match protocol {
        Protocol::OpenAiChat | Protocol::OllamaChat => {
            for (index, tool) in tools.iter().enumerate() {
                let base = format!("/tools/{index}");
                if tool.get("type").and_then(Value::as_str) != Some("function") {
                    errors.insert(request_error(
                        protocol,
                        "invalid_tool_shape",
                        &format!("{base}/type"),
                    ));
                }
                let Some(function) = tool.get("function").and_then(Value::as_object) else {
                    errors.insert(request_error(
                        protocol,
                        "invalid_tool_shape",
                        &format!("{base}/function"),
                    ));
                    continue;
                };
                if function
                    .get("name")
                    .and_then(Value::as_str)
                    .is_none_or(|name| name.trim().is_empty())
                    || function
                        .get("parameters")
                        .and_then(Value::as_object)
                        .is_none()
                {
                    errors.insert(request_error(
                        protocol,
                        "invalid_tool_shape",
                        &format!("{base}/function"),
                    ));
                }
                match protocol {
                    Protocol::OpenAiChat => {
                        if function.get("strict") != Some(&Value::Bool(true)) {
                            errors.insert(request_error(
                                protocol,
                                "invalid_tool_shape",
                                &format!("{base}/function/strict"),
                            ));
                        }
                        if let Some(parameters) = function.get("parameters") {
                            validate_closed_object_schemas(
                                protocol,
                                parameters,
                                &format!("{base}/function/parameters"),
                                errors,
                            );
                        }
                    }
                    Protocol::OllamaChat => {
                        if function.contains_key("strict") {
                            errors.insert(request_error(
                                protocol,
                                "invalid_tool_shape",
                                &format!("{base}/function/strict"),
                            ));
                        }
                        if let Some(parameters) = function.get("parameters") {
                            find_schema_keyword(
                                protocol,
                                parameters,
                                &format!("{base}/function/parameters"),
                                "additionalProperties",
                                errors,
                            );
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }
        Protocol::OpenAiResponses => {
            for (index, tool) in tools.iter().enumerate() {
                let base = format!("/tools/{index}");
                if tool.get("type").and_then(Value::as_str) != Some("function")
                    || tool
                        .get("name")
                        .and_then(Value::as_str)
                        .is_none_or(|name| name.trim().is_empty())
                    || tool.get("parameters").and_then(Value::as_object).is_none()
                    || tool.get("strict") != Some(&Value::Bool(true))
                    || tool.get("function").is_some()
                {
                    errors.insert(request_error(protocol, "invalid_tool_shape", &base));
                }
                if let Some(parameters) = tool.get("parameters") {
                    validate_closed_object_schemas(
                        protocol,
                        parameters,
                        &format!("{base}/parameters"),
                        errors,
                    );
                }
            }
        }
        Protocol::AnthropicMessages => {
            for (index, tool) in tools.iter().enumerate() {
                let base = format!("/tools/{index}");
                if tool
                    .get("name")
                    .and_then(Value::as_str)
                    .is_none_or(|name| name.trim().is_empty())
                    || tool
                        .get("input_schema")
                        .and_then(Value::as_object)
                        .is_none()
                    || tool.get("strict").is_some()
                    || tool.get("function").is_some()
                {
                    errors.insert(request_error(protocol, "invalid_tool_shape", &base));
                }
            }
        }
        Protocol::GeminiGenerateContent => {
            if tools.len() != 1 {
                errors.insert(request_error(protocol, "invalid_tool_shape", "/tools"));
            }
            let Some(declarations) = tools
                .first()
                .and_then(|tool| tool.get("functionDeclarations"))
                .and_then(Value::as_array)
            else {
                errors.insert(request_error(
                    protocol,
                    "invalid_tool_shape",
                    "/tools/0/functionDeclarations",
                ));
                return;
            };
            for (index, declaration) in declarations.iter().enumerate() {
                let base = format!("/tools/0/functionDeclarations/{index}");
                if declaration
                    .get("name")
                    .and_then(Value::as_str)
                    .is_none_or(|name| name.trim().is_empty())
                    || declaration
                        .get("parameters")
                        .and_then(Value::as_object)
                        .is_none()
                    || declaration.get("strict").is_some()
                    || declaration.get("input_schema").is_some()
                {
                    errors.insert(request_error(protocol, "invalid_tool_shape", &base));
                }
                if let Some(parameters) = declaration.get("parameters") {
                    find_schema_keyword(
                        protocol,
                        parameters,
                        &format!("{base}/parameters"),
                        "additionalProperties",
                        errors,
                    );
                }
            }
        }
        Protocol::Unknown => {
            errors.insert(request_error(protocol, "unsupported_protocol", "/"));
        }
    }
}

fn validate_closed_object_schemas(
    protocol: Protocol,
    schema: &Value,
    pointer: &str,
    errors: &mut BTreeSet<String>,
) {
    match schema {
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("object")
                && object.get("additionalProperties") != Some(&Value::Bool(false))
            {
                errors.insert(request_error(
                    protocol,
                    "invalid_tool_shape",
                    &format!("{pointer}/additionalProperties"),
                ));
            }
            if let Some(properties) = object.get("properties").and_then(Value::as_object) {
                for (name, child) in properties {
                    validate_closed_object_schemas(
                        protocol,
                        child,
                        &format!("{pointer}/properties/{name}"),
                        errors,
                    );
                }
            }
            if let Some(items) = object.get("items") {
                validate_closed_object_schemas(
                    protocol,
                    items,
                    &format!("{pointer}/items"),
                    errors,
                );
            }
        }
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                validate_closed_object_schemas(
                    protocol,
                    child,
                    &format!("{pointer}/{index}"),
                    errors,
                );
            }
        }
        _ => {}
    }
}

fn find_schema_keyword(
    protocol: Protocol,
    value: &Value,
    pointer: &str,
    keyword: &str,
    errors: &mut BTreeSet<String>,
) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let child_pointer = format!("{pointer}/{key}");
                if key == keyword {
                    errors.insert(request_error(
                        protocol,
                        "unsupported_schema_keyword",
                        &child_pointer,
                    ));
                }
                find_schema_keyword(protocol, child, &child_pointer, keyword, errors);
            }
        }
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                find_schema_keyword(
                    protocol,
                    child,
                    &format!("{pointer}/{index}"),
                    keyword,
                    errors,
                );
            }
        }
        _ => {}
    }
}

fn validate_cross_protocol_fields(
    protocol: Protocol,
    object: &Map<String, Value>,
    errors: &mut BTreeSet<String>,
) {
    let forbidden: &[&str] = match protocol {
        Protocol::OpenAiChat => &["input", "contents", "previous_response_id"],
        Protocol::OpenAiResponses => &["messages", "contents", "max_tokens"],
        Protocol::AnthropicMessages => &[
            "input",
            "contents",
            "tool_choice",
            "parallel_tool_calls",
            "previous_response_id",
            "store",
        ],
        Protocol::GeminiGenerateContent => &[
            "input",
            "messages",
            "tool_choice",
            "parallel_tool_calls",
            "previous_response_id",
            "store",
        ],
        Protocol::OllamaChat => &[
            "input",
            "contents",
            "tool_choice",
            "parallel_tool_calls",
            "previous_response_id",
            "store",
        ],
        Protocol::Unknown => &[],
    };
    for field in forbidden {
        if object.contains_key(*field) {
            errors.insert(request_error(
                protocol,
                "openai_only_field",
                &format!("/{field}"),
            ));
        }
    }
}

fn validate_follow_up(
    protocol: Protocol,
    body: &Value,
    previous: &AssistantTurn,
    results: &[ExecutedToolResult],
    errors: &mut BTreeSet<String>,
) {
    let history = match history_for(protocol, &previous.history) {
        Ok(history) => history,
        Err(ToolProtocolError::InvalidRequest(codes)) => {
            errors.extend(codes);
            return;
        }
        Err(ToolProtocolError::UnsupportedProtocol) => return,
    };
    validate_turn_history(protocol, previous, errors);
    let ordered = match correlate_results(protocol, &previous.tool_calls, results) {
        Ok(ordered) => ordered,
        Err(ToolProtocolError::InvalidRequest(codes)) => {
            errors.extend(codes);
            return;
        }
        Err(ToolProtocolError::UnsupportedProtocol) => return,
    };

    match protocol {
        Protocol::OpenAiChat => {
            let Some(messages) = body.get("messages").and_then(Value::as_array) else {
                errors.insert(request_error(protocol, "history_mismatch", "/messages"));
                return;
            };
            let suffix_len = 1 + ordered.len();
            let Some(start) = messages.len().checked_sub(suffix_len) else {
                errors.insert(request_error(protocol, "history_mismatch", "/messages"));
                return;
            };
            if messages.get(start) != Some(&history) {
                errors.insert(request_error(
                    protocol,
                    "history_mismatch",
                    &format!("/messages/{start}"),
                ));
            }
            for (offset, (call, result)) in previous.tool_calls.iter().zip(ordered).enumerate() {
                let position = start + 1 + offset;
                let expected = json!({
                    "role": "tool",
                    "tool_call_id": correlation_id(call),
                    "content": text_output(result),
                });
                validate_result_message(
                    protocol,
                    messages.get(position),
                    &expected,
                    &format!("/messages/{position}"),
                    errors,
                );
            }
        }
        Protocol::OpenAiResponses => {
            let expected_response_id = history.as_str().expect("response history is a string");
            if body.get("previous_response_id").and_then(Value::as_str)
                != Some(expected_response_id)
            {
                errors.insert(request_error(
                    protocol,
                    "previous_response_id",
                    "/previous_response_id",
                ));
            }
            let Some(input) = body.get("input").and_then(Value::as_array) else {
                errors.insert(request_error(protocol, "result_count_mismatch", "/input"));
                return;
            };
            if input.len() != ordered.len() {
                errors.insert(request_error(protocol, "result_count_mismatch", "/input"));
            }
            for (index, (call, result)) in previous.tool_calls.iter().zip(ordered).enumerate() {
                let expected = json!({
                    "type": "function_call_output",
                    "call_id": correlation_id(call),
                    "output": text_output(result),
                });
                validate_result_message(
                    protocol,
                    input.get(index),
                    &expected,
                    &format!("/input/{index}"),
                    errors,
                );
            }
        }
        Protocol::AnthropicMessages => {
            let Some(messages) = body.get("messages").and_then(Value::as_array) else {
                errors.insert(request_error(protocol, "history_mismatch", "/messages"));
                return;
            };
            let Some(start) = messages.len().checked_sub(2) else {
                errors.insert(request_error(protocol, "history_mismatch", "/messages"));
                return;
            };
            let expected_assistant = json!({"role": "assistant", "content": history});
            if messages.get(start) != Some(&expected_assistant) {
                errors.insert(request_error(
                    protocol,
                    "history_mismatch",
                    &format!("/messages/{start}"),
                ));
            }
            if messages
                .get(start + 1)
                .and_then(|message| message.get("role"))
                .and_then(Value::as_str)
                != Some("user")
            {
                errors.insert(request_error(
                    protocol,
                    "history_mismatch",
                    &format!("/messages/{}/role", start + 1),
                ));
            }
            let content_pointer = format!("/messages/{}/content", start + 1);
            let Some(content) = messages
                .get(start + 1)
                .and_then(|message| message.get("content"))
                .and_then(Value::as_array)
            else {
                errors.insert(request_error(
                    protocol,
                    "result_count_mismatch",
                    &content_pointer,
                ));
                return;
            };
            if content.len() != ordered.len() {
                errors.insert(request_error(
                    protocol,
                    "result_count_mismatch",
                    &content_pointer,
                ));
            }
            for (index, (call, result)) in previous.tool_calls.iter().zip(ordered).enumerate() {
                let mut expected = Map::from_iter([
                    ("type".into(), Value::String("tool_result".into())),
                    (
                        "tool_use_id".into(),
                        Value::String(correlation_id(call).unwrap_or_default().to_owned()),
                    ),
                    ("content".into(), Value::String(text_output(result).into())),
                ]);
                if result.is_error {
                    expected.insert("is_error".into(), Value::Bool(true));
                }
                validate_result_message(
                    protocol,
                    content.get(index),
                    &Value::Object(expected),
                    &format!("{content_pointer}/{index}"),
                    errors,
                );
            }
        }
        Protocol::GeminiGenerateContent => {
            let Some(contents) = body.get("contents").and_then(Value::as_array) else {
                errors.insert(request_error(protocol, "history_mismatch", "/contents"));
                return;
            };
            let Some(start) = contents.len().checked_sub(2) else {
                errors.insert(request_error(protocol, "history_mismatch", "/contents"));
                return;
            };
            if contents.get(start) != Some(&history) {
                errors.insert(request_error(
                    protocol,
                    "history_mismatch",
                    &format!("/contents/{start}"),
                ));
            }
            if contents
                .get(start + 1)
                .and_then(|content| content.get("role"))
                .and_then(Value::as_str)
                != Some("user")
            {
                errors.insert(request_error(
                    protocol,
                    "history_mismatch",
                    &format!("/contents/{}/role", start + 1),
                ));
            }
            let parts_pointer = format!("/contents/{}/parts", start + 1);
            let Some(parts) = contents
                .get(start + 1)
                .and_then(|content| content.get("parts"))
                .and_then(Value::as_array)
            else {
                errors.insert(request_error(
                    protocol,
                    "result_count_mismatch",
                    &parts_pointer,
                ));
                return;
            };
            if parts.len() != ordered.len() {
                errors.insert(request_error(
                    protocol,
                    "result_count_mismatch",
                    &parts_pointer,
                ));
            }
            for (index, (call, result)) in previous.tool_calls.iter().zip(ordered).enumerate() {
                let response = if result.is_error {
                    json!({"error": "timeout"})
                } else {
                    json!({"result": result.output})
                };
                let mut function_response = Map::from_iter([
                    ("name".into(), Value::String(call.name.clone())),
                    ("response".into(), response),
                ]);
                if let ToolCorrelation::Optional(Some(id)) = &call.correlation {
                    function_response.insert("id".into(), Value::String(id.clone()));
                }
                validate_result_message(
                    protocol,
                    parts.get(index),
                    &json!({"functionResponse": Value::Object(function_response)}),
                    &format!("{parts_pointer}/{index}"),
                    errors,
                );
            }
        }
        Protocol::OllamaChat => {
            let Some(messages) = body.get("messages").and_then(Value::as_array) else {
                errors.insert(request_error(protocol, "history_mismatch", "/messages"));
                return;
            };
            let suffix_len = 1 + ordered.len();
            let Some(start) = messages.len().checked_sub(suffix_len) else {
                errors.insert(request_error(protocol, "history_mismatch", "/messages"));
                return;
            };
            if messages.get(start) != Some(&history) {
                errors.insert(request_error(
                    protocol,
                    "history_mismatch",
                    &format!("/messages/{start}"),
                ));
            }
            for (offset, (call, result)) in previous.tool_calls.iter().zip(ordered).enumerate() {
                let mut expected = Map::from_iter([
                    ("role".into(), Value::String("tool".into())),
                    ("tool_name".into(), Value::String(call.name.clone())),
                    ("content".into(), Value::String(text_output(result).into())),
                ]);
                if let ToolCorrelation::Optional(Some(id)) = &call.correlation {
                    expected.insert("tool_call_id".into(), Value::String(id.clone()));
                }
                let position = start + 1 + offset;
                validate_result_message(
                    protocol,
                    messages.get(position),
                    &Value::Object(expected),
                    &format!("/messages/{position}"),
                    errors,
                );
            }
        }
        Protocol::Unknown => {}
    }
}

fn validate_turn_history(
    protocol: Protocol,
    previous: &AssistantTurn,
    errors: &mut BTreeSet<String>,
) {
    match (protocol, &previous.history) {
        (Protocol::OpenAiChat, ProtocolHistory::OpenAiChat(history)) => {
            if history.get("role").and_then(Value::as_str) != Some("assistant") {
                errors.insert(request_error(protocol, "history_mismatch", "/history/role"));
            }
            let Some(history_calls) = history.get("tool_calls").and_then(Value::as_array) else {
                errors.insert(request_error(
                    protocol,
                    "history_call_count_mismatch",
                    "/history/tool_calls",
                ));
                return;
            };
            if history_calls.len() != previous.tool_calls.len() {
                errors.insert(request_error(
                    protocol,
                    "history_call_count_mismatch",
                    "/history/tool_calls",
                ));
            }
            let mut seen_ids = BTreeSet::new();
            for (index, (history_call, call)) in
                history_calls.iter().zip(&previous.tool_calls).enumerate()
            {
                let pointer = format!("/history/tool_calls/{index}");
                if history_call.get("index").is_some() {
                    errors.insert(request_error(
                        protocol,
                        "stream_only_field",
                        &format!("{pointer}/index"),
                    ));
                }
                let actual_id = history_call.get("id").and_then(Value::as_str);
                if let Some(id) = actual_id
                    && !seen_ids.insert(id)
                {
                    errors.insert(request_error(
                        protocol,
                        "duplicate_call_id",
                        &format!("{pointer}/id"),
                    ));
                }
                if actual_id != correlation_id(call) {
                    errors.insert(request_error(
                        protocol,
                        "mismatched_call_id",
                        &format!("{pointer}/id"),
                    ));
                }
                if history_call.get("type").and_then(Value::as_str) != Some("function") {
                    errors.insert(request_error(
                        protocol,
                        "history_mismatch",
                        &format!("{pointer}/type"),
                    ));
                }
                if history_call
                    .pointer("/function/name")
                    .and_then(Value::as_str)
                    != Some(call.name.as_str())
                {
                    errors.insert(request_error(
                        protocol,
                        "name_mismatch",
                        &format!("{pointer}/function/name"),
                    ));
                }
                let arguments_match = history_call
                    .pointer("/function/arguments")
                    .and_then(Value::as_str)
                    .and_then(|arguments| serde_json::from_str::<Value>(arguments).ok())
                    .is_some_and(|arguments| arguments == call.arguments);
                if !arguments_match {
                    errors.insert(request_error(
                        protocol,
                        "history_arguments_mismatch",
                        &format!("{pointer}/function/arguments"),
                    ));
                }
            }
        }
        (Protocol::AnthropicMessages, ProtocolHistory::Anthropic(content)) => {
            let history_calls = content
                .iter()
                .enumerate()
                .filter(|(_, block)| block.get("type").and_then(Value::as_str) == Some("tool_use"))
                .collect::<Vec<_>>();
            if history_calls.len() != previous.tool_calls.len() {
                errors.insert(request_error(
                    protocol,
                    "history_call_count_mismatch",
                    "/history",
                ));
            }
            let mut seen_ids = BTreeSet::new();
            for ((content_index, history_call), call) in
                history_calls.into_iter().zip(&previous.tool_calls)
            {
                let pointer = format!("/history/{content_index}");
                let actual_id = history_call.get("id").and_then(Value::as_str);
                if let Some(id) = actual_id
                    && !seen_ids.insert(id)
                {
                    errors.insert(request_error(
                        protocol,
                        "duplicate_call_id",
                        &format!("{pointer}/id"),
                    ));
                }
                if actual_id != correlation_id(call) {
                    errors.insert(request_error(
                        protocol,
                        "mismatched_call_id",
                        &format!("{pointer}/id"),
                    ));
                }
                if history_call.get("name").and_then(Value::as_str) != Some(call.name.as_str()) {
                    errors.insert(request_error(
                        protocol,
                        "name_mismatch",
                        &format!("{pointer}/name"),
                    ));
                }
                if history_call.get("input") != Some(&call.arguments) {
                    errors.insert(request_error(
                        protocol,
                        "history_arguments_mismatch",
                        &format!("{pointer}/input"),
                    ));
                }
            }
        }
        (Protocol::GeminiGenerateContent, ProtocolHistory::Gemini(history)) => {
            let Some(parts) = history.get("parts").and_then(Value::as_array) else {
                errors.insert(request_error(
                    protocol,
                    "history_call_count_mismatch",
                    "/history/parts",
                ));
                return;
            };
            let history_calls = parts
                .iter()
                .enumerate()
                .filter_map(|(part_index, part)| {
                    part.get("functionCall")
                        .map(|function_call| (part_index, function_call))
                })
                .collect::<Vec<_>>();
            if history_calls.len() != previous.tool_calls.len() {
                errors.insert(request_error(
                    protocol,
                    "history_call_count_mismatch",
                    "/history/parts",
                ));
            }
            let mut seen_ids = BTreeSet::new();
            let mut matched_indexes = BTreeSet::new();
            for (part_index, history_call) in history_calls {
                let pointer = format!("/history/parts/{part_index}/functionCall");
                let Some(call) = previous
                    .tool_calls
                    .iter()
                    .find(|call| call.index == part_index)
                else {
                    errors.insert(request_error(protocol, "mismatched_call_index", &pointer));
                    continue;
                };
                if !matched_indexes.insert(call.index) {
                    errors.insert(request_error(protocol, "mismatched_call_index", &pointer));
                }
                validate_optional_history_call(
                    protocol,
                    OptionalHistoryCall {
                        id: history_call.get("id"),
                        name: history_call.get("name"),
                        arguments: history_call.get("args"),
                        pointer: &pointer,
                        name_pointer: "name",
                        arguments_pointer: "args",
                    },
                    call,
                    &mut seen_ids,
                    errors,
                );
            }
            if matched_indexes.len() != previous.tool_calls.len() {
                errors.insert(request_error(
                    protocol,
                    "history_call_count_mismatch",
                    "/history/parts",
                ));
            }
        }
        (Protocol::OllamaChat, ProtocolHistory::Ollama(history)) => {
            if history.get("role").and_then(Value::as_str) != Some("assistant") {
                errors.insert(request_error(protocol, "history_mismatch", "/history/role"));
            }
            let Some(history_calls) = history.get("tool_calls").and_then(Value::as_array) else {
                errors.insert(request_error(
                    protocol,
                    "history_call_count_mismatch",
                    "/history/tool_calls",
                ));
                return;
            };
            if history_calls.len() != previous.tool_calls.len() {
                errors.insert(request_error(
                    protocol,
                    "history_call_count_mismatch",
                    "/history/tool_calls",
                ));
            }
            let native_indexes = history_calls
                .iter()
                .map(|history_call| {
                    history_call
                        .pointer("/function/index")
                        .and_then(Value::as_u64)
                        .and_then(|index| usize::try_from(index).ok())
                })
                .collect::<Vec<_>>();
            let indexed = native_indexes.iter().all(Option::is_some);
            let unindexed = history_calls
                .iter()
                .all(|history_call| history_call.pointer("/function/index").is_none());
            if !indexed && !unindexed {
                errors.insert(request_error(
                    protocol,
                    "mismatched_call_index",
                    "/history/tool_calls",
                ));
            }

            let mut seen_ids = BTreeSet::new();
            let mut matched_indexes = BTreeSet::new();
            for (position, history_call) in history_calls.iter().enumerate() {
                let pointer = format!("/history/tool_calls/{position}");
                let native_index = if indexed {
                    native_indexes[position]
                } else if unindexed {
                    Some(position)
                } else {
                    None
                };
                let Some(native_index) = native_index else {
                    continue;
                };
                let Some(call) = previous
                    .tool_calls
                    .iter()
                    .find(|call| call.index == native_index)
                else {
                    errors.insert(request_error(
                        protocol,
                        "mismatched_call_index",
                        &format!("{pointer}/function/index"),
                    ));
                    continue;
                };
                if !matched_indexes.insert(call.index) {
                    errors.insert(request_error(
                        protocol,
                        "mismatched_call_index",
                        &format!("{pointer}/function/index"),
                    ));
                }
                let Some(function) = history_call.get("function") else {
                    errors.insert(request_error(
                        protocol,
                        "history_mismatch",
                        &format!("{pointer}/function"),
                    ));
                    continue;
                };
                validate_optional_history_call(
                    protocol,
                    OptionalHistoryCall {
                        id: history_call.get("id"),
                        name: function.get("name"),
                        arguments: function.get("arguments"),
                        pointer: &pointer,
                        name_pointer: "function/name",
                        arguments_pointer: "function/arguments",
                    },
                    call,
                    &mut seen_ids,
                    errors,
                );
            }
            if matched_indexes.len() != previous.tool_calls.len() {
                errors.insert(request_error(
                    protocol,
                    "history_call_count_mismatch",
                    "/history/tool_calls",
                ));
            }
        }
        _ => {}
    }
}

struct OptionalHistoryCall<'a> {
    id: Option<&'a Value>,
    name: Option<&'a Value>,
    arguments: Option<&'a Value>,
    pointer: &'a str,
    name_pointer: &'static str,
    arguments_pointer: &'static str,
}

fn validate_optional_history_call(
    protocol: Protocol,
    history: OptionalHistoryCall<'_>,
    call: &ToolCall,
    seen_ids: &mut BTreeSet<String>,
    errors: &mut BTreeSet<String>,
) {
    let actual_id_text = history.id.and_then(Value::as_str);
    if let Some(id) = actual_id_text
        && !seen_ids.insert(id.to_owned())
    {
        errors.insert(request_error(
            protocol,
            "duplicate_call_id",
            &format!("{}/id", history.pointer),
        ));
    }
    let id_matches = match (&call.correlation, history.id) {
        (ToolCorrelation::Optional(None), None) => true,
        (ToolCorrelation::Optional(Some(expected)), Some(Value::String(actual))) => {
            actual == expected
        }
        _ => false,
    };
    if !id_matches {
        errors.insert(request_error(
            protocol,
            "mismatched_call_id",
            &format!("{}/id", history.pointer),
        ));
    }
    if history.name.and_then(Value::as_str) != Some(call.name.as_str()) {
        errors.insert(request_error(
            protocol,
            "name_mismatch",
            &format!("{}/{}", history.pointer, history.name_pointer),
        ));
    }
    if history.arguments != Some(&call.arguments) {
        errors.insert(request_error(
            protocol,
            "history_arguments_mismatch",
            &format!("{}/{}", history.pointer, history.arguments_pointer),
        ));
    }
}

fn correlation_id(call: &ToolCall) -> Option<&str> {
    match &call.correlation {
        ToolCorrelation::Required(id) | ToolCorrelation::Optional(Some(id)) => Some(id),
        ToolCorrelation::Optional(None) => None,
    }
}

fn validate_result_message(
    protocol: Protocol,
    actual: Option<&Value>,
    expected: &Value,
    pointer: &str,
    errors: &mut BTreeSet<String>,
) {
    let Some(actual) = actual else {
        errors.insert(request_error(protocol, "result_count_mismatch", pointer));
        return;
    };
    if actual == expected {
        return;
    }

    let valid_envelope = match protocol {
        Protocol::OpenAiChat | Protocol::OllamaChat => {
            actual.get("role").and_then(Value::as_str) == Some("tool")
        }
        Protocol::OpenAiResponses => {
            actual.get("type").and_then(Value::as_str) == Some("function_call_output")
        }
        Protocol::AnthropicMessages => {
            actual.get("type").and_then(Value::as_str) == Some("tool_result")
        }
        Protocol::GeminiGenerateContent => actual
            .get("functionResponse")
            .and_then(Value::as_object)
            .is_some(),
        Protocol::Unknown => false,
    };
    if !valid_envelope {
        errors.insert(request_error(protocol, "invalid_result_shape", pointer));
    }

    let id_pointer = match protocol {
        Protocol::OpenAiChat | Protocol::OllamaChat => "tool_call_id",
        Protocol::OpenAiResponses => "call_id",
        Protocol::AnthropicMessages => "tool_use_id",
        Protocol::GeminiGenerateContent => "functionResponse/id",
        Protocol::Unknown => "id",
    };
    if actual.pointer(&format!("/{id_pointer}")) != expected.pointer(&format!("/{id_pointer}")) {
        errors.insert(request_error(
            protocol,
            "mismatched_call_id",
            &format!("{pointer}/{id_pointer}"),
        ));
    }
    let name_pointer = match protocol {
        Protocol::GeminiGenerateContent => Some("/functionResponse/name"),
        Protocol::OllamaChat => Some("/tool_name"),
        _ => None,
    };
    if let Some(name_pointer) = name_pointer
        && actual.pointer(name_pointer) != expected.pointer(name_pointer)
    {
        errors.insert(request_error(
            protocol,
            "name_mismatch",
            &format!("{pointer}{name_pointer}"),
        ));
    }

    let output_pointer = match protocol {
        Protocol::OpenAiChat | Protocol::OllamaChat | Protocol::AnthropicMessages => "/content",
        Protocol::OpenAiResponses => "/output",
        Protocol::GeminiGenerateContent => "/functionResponse/response",
        Protocol::Unknown => "/output",
    };
    if actual.pointer(output_pointer) != expected.pointer(output_pointer) {
        let code = if protocol == Protocol::GeminiGenerateContent
            || (protocol == Protocol::AnthropicMessages
                && expected.get("is_error") == Some(&Value::Bool(true)))
        {
            "error_carrier"
        } else {
            "output"
        };
        errors.insert(request_error(
            protocol,
            code,
            &format!("{pointer}{output_pointer}"),
        ));
    }
    if protocol == Protocol::AnthropicMessages && actual.get("is_error") != expected.get("is_error")
    {
        errors.insert(request_error(
            protocol,
            "error_carrier",
            &format!("{pointer}/is_error"),
        ));
    }
    if protocol != Protocol::AnthropicMessages {
        let carrier_pointer = if protocol == Protocol::GeminiGenerateContent {
            "/functionResponse/is_error"
        } else {
            "/is_error"
        };
        if actual.pointer(carrier_pointer).is_some() {
            errors.insert(request_error(
                protocol,
                "error_carrier",
                &format!("{pointer}{carrier_pointer}"),
            ));
        }
    }
}

pub fn build_follow_up(
    protocol: Protocol,
    model: &str,
    id: &str,
    initial_body: &Value,
    response: &Value,
) -> Result<RequestSpec, FollowUpError> {
    if !matches!(id, "047" | "048" | "049") {
        return Err(FollowUpError::UnsupportedCheck(id.to_owned()));
    }
    let prompt = tool_prompt(id).ok_or_else(|| FollowUpError::UnsupportedCheck(id.to_owned()))?;
    let tools = initial_body
        .get("tools")
        .cloned()
        .ok_or(FollowUpError::MissingField("tools"))?;
    let observation = observe(protocol, response)?;
    let error_result = id == "049";
    let tool_output = if error_result {
        "ERROR: timeout"
    } else {
        "WEATHER_SUNNY"
    };

    let body = match protocol {
        Protocol::OpenAiResponses => json!({
            "model": model,
            "previous_response_id": required(observation.response_id, "response id")?,
            "input": [{
                "type": "function_call_output",
                "call_id": required(observation.call_id, "call id")?,
                "output": tool_output
            }],
            "tools": tools
        }),
        Protocol::OpenAiChat => json!({
            "model": model,
            "messages": [
                {"role": "user", "content": prompt},
                required_value(observation.assistant, "assistant message")?,
                {
                    "role": "tool",
                    "tool_call_id": required(observation.call_id, "call id")?,
                    "content": tool_output
                }
            ],
            "tools": tools,
            "stream": false
        }),
        Protocol::AnthropicMessages => {
            let mut tool_result = Map::from_iter([
                ("type".into(), Value::String("tool_result".into())),
                (
                    "tool_use_id".into(),
                    Value::String(required(observation.call_id, "call id")?),
                ),
                ("content".into(), Value::String(tool_output.into())),
            ]);
            if error_result {
                tool_result.insert("is_error".into(), Value::Bool(true));
            }
            json!({
                "model": model,
                "max_tokens": ANTHROPIC_MAX_TOKENS,
                "messages": [
                    {"role": "user", "content": prompt},
                    {"role": "assistant", "content": required_value(observation.assistant, "assistant content")?},
                    {"role": "user", "content": [Value::Object(tool_result)]}
                ],
                "tools": tools
            })
        }
        Protocol::GeminiGenerateContent => {
            let response_payload = if error_result {
                json!({"error": "timeout"})
            } else {
                json!({"result": tool_output})
            };
            json!({
                "contents": [
                    {"role": "user", "parts": [{"text": prompt}]},
                    required_value(observation.assistant, "candidate content")?,
                    {"role": "user", "parts": [{"functionResponse": {
                        "id": required(observation.call_id, "call id")?,
                        "name": required(observation.tool_name, "tool name")?,
                        "response": response_payload
                    }}]}
                ],
                "tools": tools
            })
        }
        Protocol::OllamaChat => json!({
            "model": model,
            "messages": [
                {"role": "user", "content": prompt},
                required_value(observation.assistant, "assistant message")?,
                {
                    "role": "tool",
                    "tool_name": required(observation.tool_name, "tool name")?,
                    "content": tool_output
                }
            ],
            "tools": tools,
            "stream": false
        }),
        Protocol::Unknown => return Err(FollowUpError::UnsupportedProtocol),
    };

    Ok(RequestSpec {
        body,
        stream: false,
    })
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum FollowUpError {
    #[error("unsupported tool follow-up check: {0}")]
    UnsupportedCheck(String),
    #[error("tool follow-up is unavailable for unknown protocol")]
    UnsupportedProtocol,
    #[error("missing tool response field: {0}")]
    MissingField(&'static str),
}

struct ToolObservation {
    call_id: Option<String>,
    response_id: Option<String>,
    tool_name: Option<String>,
    assistant: Option<Value>,
}

fn observe(protocol: Protocol, response: &Value) -> Result<ToolObservation, FollowUpError> {
    let observation = match protocol {
        Protocol::OpenAiChat => {
            let assistant = response.pointer("/choices/0/message").cloned();
            let call = assistant
                .as_ref()
                .and_then(|value| value.pointer("/tool_calls/0"));
            ToolObservation {
                call_id: string_at(call, "/id"),
                response_id: response
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                tool_name: string_at(call, "/function/name"),
                assistant,
            }
        }
        Protocol::OpenAiResponses => {
            let call = response
                .get("output")
                .and_then(Value::as_array)
                .and_then(|items| {
                    items.iter().find(|item| {
                        item.get("type").and_then(Value::as_str) == Some("function_call")
                    })
                });
            ToolObservation {
                call_id: string_at(call, "/call_id"),
                response_id: response
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                tool_name: string_at(call, "/name"),
                assistant: call.cloned(),
            }
        }
        Protocol::AnthropicMessages => {
            let content = response.get("content").cloned();
            let call = content
                .as_ref()
                .and_then(Value::as_array)
                .and_then(|items| {
                    items
                        .iter()
                        .find(|item| item.get("type").and_then(Value::as_str) == Some("tool_use"))
                });
            ToolObservation {
                call_id: string_at(call, "/id"),
                response_id: response
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                tool_name: string_at(call, "/name"),
                assistant: content,
            }
        }
        Protocol::GeminiGenerateContent => {
            let assistant = response.pointer("/candidates/0/content").cloned();
            let call = assistant
                .as_ref()
                .and_then(|value| value.get("parts"))
                .and_then(Value::as_array)
                .and_then(|parts| parts.iter().find_map(|part| part.get("functionCall")));
            ToolObservation {
                call_id: string_at(call, "/id"),
                response_id: None,
                tool_name: string_at(call, "/name"),
                assistant,
            }
        }
        Protocol::OllamaChat => {
            let assistant = response.get("message").cloned();
            let call = assistant
                .as_ref()
                .and_then(|value| value.pointer("/tool_calls/0"));
            ToolObservation {
                call_id: string_at(call, "/id"),
                response_id: None,
                tool_name: string_at(call, "/function/name"),
                assistant,
            }
        }
        Protocol::Unknown => return Err(FollowUpError::UnsupportedProtocol),
    };
    Ok(observation)
}

fn string_at(value: Option<&Value>, pointer: &str) -> Option<String> {
    value?
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn required(value: Option<String>, field: &'static str) -> Result<String, FollowUpError> {
    value.ok_or(FollowUpError::MissingField(field))
}

fn required_value(value: Option<Value>, field: &'static str) -> Result<Value, FollowUpError> {
    value.ok_or(FollowUpError::MissingField(field))
}

fn tool_definitions(protocol: Protocol, id: &str) -> Vec<Value> {
    let base = if id == "044" {
        vec![definition(
            protocol,
            "inspect_target",
            "Inspect a network target",
            inspect_parameters(protocol),
        )]
    } else {
        vec![definition(
            protocol,
            "get_weather",
            "Get weather",
            weather_parameters(protocol, id == "043"),
        )]
    };
    let mut tools = base;
    if matches!(id, "041" | "045" | "047" | "050") {
        tools.push(definition(
            protocol,
            "get_time",
            "Get time",
            time_parameters(protocol),
        ));
    }
    if id == "050" {
        for index in 1..=8 {
            tools.push(definition(
                protocol,
                &format!("catalog_tool_{index}"),
                "Distractor tool",
                empty_parameters(protocol),
            ));
        }
    }
    tools
}

fn weather_parameters(protocol: Protocol, required_fields: bool) -> Value {
    let mut parameters = if required_fields {
        json!({
            "type": "object",
            "properties": {
                "city": {"type": "string"},
                "unit": {"type": "string", "enum": ["C", "F"]},
                "days": {"type": "integer"}
            },
            "required": ["city", "unit", "days"]
        })
    } else {
        json!({
            "type": "object",
            "properties": {"city": {"type": "string"}},
            "required": ["city"]
        })
    };
    if uses_additional_properties(protocol) {
        parameters
            .as_object_mut()
            .expect("weather schema is an object")
            .insert("additionalProperties".into(), Value::Bool(false));
    }
    parameters
}

fn inspect_parameters(protocol: Protocol) -> Value {
    let mut target = json!({
        "type": "object",
        "properties": {"host": {"type": "string"}, "port": {"type": "integer"}},
        "required": ["host", "port"]
    });
    let mut root = json!({
        "type": "object",
        "properties": {"target": target},
        "required": ["target"]
    });
    if uses_additional_properties(protocol) {
        target
            .as_object_mut()
            .expect("target schema is an object")
            .insert("additionalProperties".into(), Value::Bool(false));
        root["properties"]["target"] = target;
        root.as_object_mut()
            .expect("root schema is an object")
            .insert("additionalProperties".into(), Value::Bool(false));
    }
    root
}

fn time_parameters(protocol: Protocol) -> Value {
    let mut parameters = json!({
        "type": "object",
        "properties": {"zone": {"type": "string"}},
        "required": ["zone"]
    });
    if uses_additional_properties(protocol) {
        parameters
            .as_object_mut()
            .expect("time schema is an object")
            .insert("additionalProperties".into(), Value::Bool(false));
    }
    parameters
}

fn empty_parameters(protocol: Protocol) -> Value {
    let mut parameters = json!({"type": "object", "properties": {}});
    if uses_additional_properties(protocol) {
        parameters
            .as_object_mut()
            .expect("empty schema is an object")
            .insert("additionalProperties".into(), Value::Bool(false));
    }
    parameters
}

fn uses_additional_properties(protocol: Protocol) -> bool {
    matches!(
        protocol,
        Protocol::OpenAiChat | Protocol::OpenAiResponses | Protocol::AnthropicMessages
    )
}

fn definition(protocol: Protocol, name: &str, description: &str, parameters: Value) -> Value {
    match protocol {
        Protocol::OpenAiResponses => json!({
            "type": "function",
            "name": name,
            "description": description,
            "parameters": parameters,
            "strict": true
        }),
        Protocol::AnthropicMessages => json!({
            "name": name,
            "description": description,
            "input_schema": parameters
        }),
        Protocol::GeminiGenerateContent => json!({
            "name": name,
            "description": description,
            "parameters": parameters
        }),
        Protocol::OpenAiChat => json!({
            "type": "function",
            "function": {
                "name": name,
                "description": description,
                "parameters": parameters,
                "strict": true
            }
        }),
        Protocol::OllamaChat | Protocol::Unknown => json!({
            "type": "function",
            "function": {
                "name": name,
                "description": description,
                "parameters": parameters
            }
        }),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use url::Url;

    use crate::protocol::stream::{AssistantTurn, ProtocolHistory, ToolCall, ToolCorrelation};
    use crate::protocol::{Protocol, normalize_request_url};

    use super::{
        ExecutedToolResult, ToolConversation, ToolProtocolError, ToolRequestPhase, tool_prompt,
        tool_request, validate_tool_request,
    };

    fn endpoint(protocol: Protocol) -> Url {
        let configured = match protocol {
            Protocol::GeminiGenerateContent => {
                Url::parse("https://example.com/v1beta/models/gemini:generateContent?key=test")
            }
            _ => Url::parse("https://example.com/v1/chat"),
        }
        .expect("valid test endpoint");
        normalize_request_url(protocol, &configured, true)
    }

    fn initial(protocol: Protocol, id: &str) -> super::RequestSpec {
        tool_request(
            protocol,
            "test-model",
            id,
            tool_prompt(id).expect("known tool check"),
        )
    }

    fn call(index: usize, id: Option<&str>, name: &str) -> ToolCall {
        let correlation = match id {
            Some(id) => ToolCorrelation::Required(id.into()),
            None => ToolCorrelation::Optional(None),
        };
        ToolCall {
            index,
            correlation,
            name: name.into(),
            arguments: if name == "get_time" {
                json!({"zone": "UTC"})
            } else {
                json!({"city": "Beijing"})
            },
        }
    }

    fn optional_call(index: usize, id: Option<&str>, name: &str) -> ToolCall {
        ToolCall {
            index,
            correlation: ToolCorrelation::Optional(id.map(str::to_owned)),
            name: name.into(),
            arguments: if name == "get_time" {
                json!({"zone": "UTC"})
            } else {
                json!({"city": "Beijing"})
            },
        }
    }

    fn turn(protocol: Protocol, response_id: &str, calls: Vec<ToolCall>) -> AssistantTurn {
        let history = match protocol {
            Protocol::OpenAiChat => ProtocolHistory::OpenAiChat(json!({
                "role": "assistant",
                "content": null,
                "tool_calls": calls.iter().map(|call| json!({
                    "id": correlation_id(&call.correlation),
                    "type": "function",
                    "function": {"name": call.name, "arguments": call.arguments.to_string()}
                })).collect::<Vec<_>>()
            })),
            Protocol::OpenAiResponses => ProtocolHistory::OpenAiResponses {
                response_id: response_id.into(),
            },
            Protocol::AnthropicMessages => {
                let mut content = vec![
                    json!({"type": "thinking", "thinking": "private", "signature": "sig"}),
                    json!({"type": "text", "text": "checking"}),
                ];
                content.extend(calls.iter().map(|call| {
                    json!({
                        "type": "tool_use",
                        "id": correlation_id(&call.correlation),
                        "name": call.name,
                        "input": call.arguments
                    })
                }));
                ProtocolHistory::Anthropic(content)
            }
            Protocol::GeminiGenerateContent => ProtocolHistory::Gemini(json!({
                "role": "model",
                "parts": calls.iter().map(|call| {
                    let mut function_call = json!({
                        "name": call.name,
                        "args": call.arguments
                    });
                    if let ToolCorrelation::Optional(Some(id)) = &call.correlation {
                        function_call["id"] = Value::String(id.clone());
                    }
                    json!({
                        "thought": true,
                        "thoughtSignature": "signature",
                        "functionCall": function_call
                    })
                }).collect::<Vec<_>>()
            })),
            Protocol::OllamaChat => ProtocolHistory::Ollama(json!({
                "role": "assistant",
                "content": "",
                "tool_calls": calls.iter().map(|call| {
                    let mut value = json!({
                        "function": {"name": call.name, "arguments": call.arguments}
                    });
                    if let ToolCorrelation::Optional(Some(id)) = &call.correlation {
                        value["id"] = Value::String(id.clone());
                    }
                    value
                }).collect::<Vec<_>>()
            })),
            Protocol::Unknown => unreachable!("unknown has no tool turn"),
        };
        AssistantTurn {
            history,
            tool_calls: calls,
            final_text: String::new(),
        }
    }

    fn correlation_id(correlation: &ToolCorrelation) -> Value {
        match correlation {
            ToolCorrelation::Required(id) | ToolCorrelation::Optional(Some(id)) => {
                Value::String(id.clone())
            }
            ToolCorrelation::Optional(None) => Value::Null,
        }
    }

    fn result(call: &ToolCall, output: &str, is_error: bool) -> ExecutedToolResult {
        ExecutedToolResult {
            call: call.clone(),
            output: output.into(),
            is_error,
        }
    }

    fn assert_valid(protocol: Protocol, request: &super::RequestSpec, phase: ToolRequestPhase<'_>) {
        assert_eq!(
            validate_tool_request(
                protocol,
                &endpoint(protocol),
                request.stream,
                &request.body,
                phase,
            ),
            Vec::<String>::new()
        );
    }

    #[test]
    fn official_initial_request_matrix_has_exact_provider_shapes() {
        let chat = initial(Protocol::OpenAiChat, "046");
        assert!(chat.stream);
        assert_eq!(chat.body["stream"], true);
        assert_eq!(chat.body["store"], Value::Null);
        assert_eq!(chat.body["tools"][0]["type"], "function");
        assert_eq!(chat.body["tools"][0]["function"]["strict"], true);
        assert_valid(Protocol::OpenAiChat, &chat, ToolRequestPhase::Initial);

        let responses = initial(Protocol::OpenAiResponses, "046");
        assert_eq!(responses.body["stream"], true);
        assert_eq!(responses.body["store"], true);
        assert_eq!(responses.body["tools"][0]["name"], "get_weather");
        assert_eq!(responses.body["tools"][0]["strict"], true);
        assert_eq!(responses.body["tools"][0]["function"], Value::Null);
        assert_valid(
            Protocol::OpenAiResponses,
            &responses,
            ToolRequestPhase::Initial,
        );

        let anthropic = initial(Protocol::AnthropicMessages, "046");
        assert_eq!(anthropic.body["stream"], true);
        assert!(anthropic.body["tools"][0].get("input_schema").is_some());
        assert!(anthropic.body["tools"][0].get("strict").is_none());
        assert_valid(
            Protocol::AnthropicMessages,
            &anthropic,
            ToolRequestPhase::Initial,
        );

        let gemini = initial(Protocol::GeminiGenerateContent, "046");
        assert!(gemini.stream);
        assert!(gemini.body.get("stream").is_none());
        assert!(
            gemini.body["tools"][0]["functionDeclarations"][0]["parameters"]
                .get("additionalProperties")
                .is_none()
        );
        assert_valid(
            Protocol::GeminiGenerateContent,
            &gemini,
            ToolRequestPhase::Initial,
        );

        let ollama = initial(Protocol::OllamaChat, "046");
        assert_eq!(ollama.body["stream"], true);
        assert!(ollama.body.get("tool_choice").is_none());
        assert!(ollama.body.get("parallel_tool_calls").is_none());
        assert!(ollama.body["tools"][0]["function"].get("strict").is_none());
        assert!(
            ollama.body["tools"][0]["function"]["parameters"]
                .get("additionalProperties")
                .is_none()
        );
        assert_valid(Protocol::OllamaChat, &ollama, ToolRequestPhase::Initial);
    }

    #[test]
    fn official_single_follow_up_matrix_preserves_native_history() {
        let protocols = [
            Protocol::OpenAiChat,
            Protocol::OpenAiResponses,
            Protocol::AnthropicMessages,
            Protocol::GeminiGenerateContent,
            Protocol::OllamaChat,
        ];
        for protocol in protocols {
            let assistant_call = if matches!(
                protocol,
                Protocol::GeminiGenerateContent | Protocol::OllamaChat
            ) {
                optional_call(0, Some("call_weather"), "get_weather")
            } else {
                call(0, Some("call_weather"), "get_weather")
            };
            let assistant = turn(protocol, "resp-1", vec![assistant_call.clone()]);
            let results = vec![result(&assistant_call, "WEATHER_SUNNY", false)];
            let mut conversation =
                ToolConversation::from_initial(protocol, initial(protocol, "046").body)
                    .expect("official initial request");
            let follow = conversation
                .append_follow_up(&assistant, &results)
                .expect("official follow-up");

            assert!(follow.stream, "{protocol:?}");
            assert_eq!(follow, conversation.current_request());
            assert_valid(
                protocol,
                &follow,
                ToolRequestPhase::FollowUp {
                    previous: &assistant,
                    results: &results,
                },
            );
            match protocol {
                Protocol::OpenAiChat => {
                    assert_eq!(follow.body["messages"][1], history_value(&assistant));
                    assert_eq!(
                        follow.body["messages"][2],
                        json!({
                            "role": "tool", "tool_call_id": "call_weather", "content": "WEATHER_SUNNY"
                        })
                    );
                }
                Protocol::OpenAiResponses => {
                    assert_eq!(follow.body["previous_response_id"], "resp-1");
                    assert_eq!(
                        follow.body["input"],
                        json!([{
                            "type": "function_call_output", "call_id": "call_weather", "output": "WEATHER_SUNNY"
                        }])
                    );
                    assert_eq!(follow.body["store"], true);
                }
                Protocol::AnthropicMessages => {
                    assert_eq!(
                        follow.body["messages"][1]["content"],
                        history_value(&assistant)
                    );
                    assert_eq!(
                        follow.body["messages"][2]["content"],
                        json!([{
                            "type": "tool_result", "tool_use_id": "call_weather", "content": "WEATHER_SUNNY"
                        }])
                    );
                }
                Protocol::GeminiGenerateContent => {
                    assert_eq!(follow.body["contents"][1], history_value(&assistant));
                    assert_eq!(
                        follow.body["contents"][2]["parts"][0]["functionResponse"],
                        json!({
                            "id": "call_weather", "name": "get_weather", "response": {"result": "WEATHER_SUNNY"}
                        })
                    );
                    assert!(follow.body.get("stream").is_none());
                }
                Protocol::OllamaChat => {
                    assert_eq!(follow.body["messages"][1], history_value(&assistant));
                    assert_eq!(
                        follow.body["messages"][2],
                        json!({
                            "role": "tool", "tool_name": "get_weather", "tool_call_id": "call_weather", "content": "WEATHER_SUNNY"
                        })
                    );
                }
                Protocol::Unknown => unreachable!(),
            }
        }
    }

    #[test]
    fn multiple_results_are_correlated_as_a_bijection_and_emitted_in_call_order() {
        for protocol in [
            Protocol::OpenAiChat,
            Protocol::OpenAiResponses,
            Protocol::AnthropicMessages,
            Protocol::GeminiGenerateContent,
            Protocol::OllamaChat,
        ] {
            let calls = if matches!(
                protocol,
                Protocol::GeminiGenerateContent | Protocol::OllamaChat
            ) {
                vec![
                    optional_call(0, Some("weather-id"), "get_weather"),
                    optional_call(1, Some("time-id"), "get_time"),
                ]
            } else {
                vec![
                    call(0, Some("weather-id"), "get_weather"),
                    call(1, Some("time-id"), "get_time"),
                ]
            };
            let assistant = turn(protocol, "resp-two", calls.clone());
            let results = vec![
                result(&calls[1], "TIME_UTC_00:00", false),
                result(&calls[0], "WEATHER_SUNNY", false),
            ];
            let mut conversation =
                ToolConversation::from_initial(protocol, initial(protocol, "047").body)
                    .expect("official initial request");
            let follow = conversation
                .append_follow_up(&assistant, &results)
                .expect("out-of-order ID results are correlated");
            assert_valid(
                protocol,
                &follow,
                ToolRequestPhase::FollowUp {
                    previous: &assistant,
                    results: &results,
                },
            );
            let serialized = follow.body.to_string();
            assert!(
                serialized.find("WEATHER_SUNNY").unwrap()
                    < serialized.find("TIME_UTC_00:00").unwrap(),
                "{protocol:?} must emit in assistant call order"
            );
            if protocol == Protocol::AnthropicMessages {
                let tool_use_ids = follow.body["messages"][1]["content"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|block| block["type"] == "tool_use")
                    .map(|block| block["id"].as_str().unwrap())
                    .collect::<Vec<_>>();
                let result_ids = follow.body["messages"][2]["content"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|block| block["tool_use_id"].as_str().unwrap())
                    .collect::<Vec<_>>();
                assert_eq!(tool_use_ids, ["weather-id", "time-id"]);
                assert_eq!(result_ids, tool_use_ids);
            }
        }
    }

    #[test]
    fn correlation_failures_are_stable_and_append_is_transactional() {
        let calls = vec![
            call(0, Some("weather-id"), "get_weather"),
            call(1, Some("time-id"), "get_time"),
        ];
        let assistant = turn(Protocol::OpenAiChat, "unused", calls.clone());
        let valid = vec![
            result(&calls[0], "WEATHER_SUNNY", false),
            result(&calls[1], "TIME_UTC_00:00", false),
        ];
        let mutations = [
            (vec![valid[0].clone()], "result_count_mismatch"),
            (
                vec![valid[0].clone(), valid[0].clone()],
                "duplicate_call_id",
            ),
            (
                {
                    let mut extra = valid.clone();
                    extra.push(result(
                        &call(2, Some("extra-id"), "get_weather"),
                        "x",
                        false,
                    ));
                    extra
                },
                "result_count_mismatch",
            ),
            (
                {
                    let mut stale = valid.clone();
                    stale[0].call.correlation = ToolCorrelation::Required("stale-id".into());
                    stale
                },
                "stale_call_id",
            ),
            (
                {
                    let mut wrong_name = valid.clone();
                    wrong_name[0].call.name = "get_time".into();
                    wrong_name
                },
                "name_mismatch",
            ),
            (
                {
                    let mut wrong_call = valid.clone();
                    wrong_call[0].call.arguments = json!({"city": "Shanghai"});
                    wrong_call
                },
                "mismatched_call_id",
            ),
        ];

        for (results, expected_code) in mutations {
            let mut conversation = ToolConversation::from_initial(
                Protocol::OpenAiChat,
                initial(Protocol::OpenAiChat, "047").body,
            )
            .expect("official initial request");
            let before = conversation.current_request();
            let error = conversation
                .append_follow_up(&assistant, &results)
                .expect_err("invalid result set");
            assert!(
                error
                    .codes()
                    .iter()
                    .any(|code| code.contains(expected_code)),
                "{error:?}"
            );
            assert_eq!(conversation.current_request(), before);
        }
    }

    #[test]
    fn check_049_error_carriers_are_provider_native() {
        for protocol in [
            Protocol::OpenAiChat,
            Protocol::OpenAiResponses,
            Protocol::AnthropicMessages,
            Protocol::GeminiGenerateContent,
            Protocol::OllamaChat,
        ] {
            let assistant_call = if matches!(
                protocol,
                Protocol::GeminiGenerateContent | Protocol::OllamaChat
            ) {
                optional_call(0, None, "get_weather")
            } else {
                call(0, Some("weather-id"), "get_weather")
            };
            let assistant = turn(protocol, "resp-timeout", vec![assistant_call.clone()]);
            let results = vec![result(&assistant_call, "ERROR: timeout", true)];
            let mut conversation =
                ToolConversation::from_initial(protocol, initial(protocol, "049").body)
                    .expect("official initial request");
            let follow = conversation
                .append_follow_up(&assistant, &results)
                .expect("error result");
            let serialized = follow.body.to_string();
            match protocol {
                Protocol::AnthropicMessages => {
                    assert_eq!(follow.body["messages"][2]["content"][0]["is_error"], true);
                    assert_eq!(
                        follow.body["messages"][2]["content"][0]["content"],
                        "ERROR: timeout"
                    );
                }
                Protocol::GeminiGenerateContent => {
                    assert!(serialized.contains("\"error\":\"timeout\""));
                }
                _ => {
                    assert!(serialized.contains("ERROR: timeout"), "{protocol:?}");
                    assert!(!serialized.contains("is_error"));
                }
            }
            assert_valid(
                protocol,
                &follow,
                ToolRequestPhase::FollowUp {
                    previous: &assistant,
                    results: &results,
                },
            );
        }
    }

    #[test]
    fn noncanonical_error_output_is_rejected_transactionally() {
        let assistant_call = call(0, Some("weather-id"), "get_weather");
        let assistant = turn(Protocol::OpenAiChat, "unused", vec![assistant_call.clone()]);
        let results = vec![result(&assistant_call, "internal timeout detail", true)];
        let mut conversation = ToolConversation::from_initial(
            Protocol::OpenAiChat,
            initial(Protocol::OpenAiChat, "049").body,
        )
        .expect("official initial request");
        let before = conversation.current_request();

        let error = conversation
            .append_follow_up(&assistant, &results)
            .expect_err("only the deterministic local timeout output is allowed");
        assert!(
            error
                .codes()
                .contains(&"request.openai_chat.output:/results".into())
        );
        assert_eq!(conversation.current_request(), before);
    }

    #[test]
    fn responses_uses_only_latest_response_id_across_multiple_rounds() {
        let protocol = Protocol::OpenAiResponses;
        let first_call = call(0, Some("weather-id"), "get_weather");
        let first = turn(protocol, "resp-1", vec![first_call.clone()]);
        let first_results = vec![result(&first_call, "WEATHER_SUNNY", false)];
        let second_call = call(0, Some("time-id"), "get_time");
        let second = turn(protocol, "resp-2", vec![second_call.clone()]);
        let second_results = vec![result(&second_call, "TIME_UTC_00:00", false)];
        let mut conversation =
            ToolConversation::from_initial(protocol, initial(protocol, "047").body)
                .expect("official initial request");
        conversation
            .append_follow_up(&first, &first_results)
            .expect("first round");
        let second_request = conversation
            .append_follow_up(&second, &second_results)
            .expect("second round");

        assert_eq!(second_request.body["previous_response_id"], "resp-2");
        assert_eq!(second_request.body["input"].as_array().unwrap().len(), 1);
        assert_eq!(second_request.body["input"][0]["call_id"], "time-id");
        assert!(!second_request.body.to_string().contains("weather-id"));
        assert_eq!(second_request.body["store"], true);
    }

    #[test]
    fn message_protocol_conversations_preserve_every_prior_round() {
        for protocol in [
            Protocol::OpenAiChat,
            Protocol::AnthropicMessages,
            Protocol::GeminiGenerateContent,
            Protocol::OllamaChat,
        ] {
            let make_call = |index, id, name| {
                if matches!(
                    protocol,
                    Protocol::GeminiGenerateContent | Protocol::OllamaChat
                ) {
                    optional_call(index, Some(id), name)
                } else {
                    call(index, Some(id), name)
                }
            };
            let first_call = make_call(0, "weather-id", "get_weather");
            let first = turn(protocol, "unused-1", vec![first_call.clone()]);
            let first_results = vec![result(&first_call, "WEATHER_SUNNY", false)];
            let second_call = make_call(0, "time-id", "get_time");
            let second = turn(protocol, "unused-2", vec![second_call.clone()]);
            let second_results = vec![result(&second_call, "TIME_UTC_00:00", false)];
            let mut conversation =
                ToolConversation::from_initial(protocol, initial(protocol, "047").body).unwrap();
            let first_request = conversation
                .append_follow_up(&first, &first_results)
                .unwrap();
            let second_request = conversation
                .append_follow_up(&second, &second_results)
                .unwrap();
            let collection = if protocol == Protocol::GeminiGenerateContent {
                "contents"
            } else {
                "messages"
            };

            assert_eq!(first_request.body[collection].as_array().unwrap().len(), 3);
            assert_eq!(second_request.body[collection].as_array().unwrap().len(), 5);
            let first_history = if protocol == Protocol::AnthropicMessages {
                json!({"role": "assistant", "content": history_value(&first)})
            } else {
                history_value(&first)
            };
            assert_eq!(second_request.body[collection][1], first_history);
        }
    }

    #[test]
    fn every_protocol_rejects_incomplete_and_mismatched_result_sets_without_mutation() {
        for protocol in [
            Protocol::OpenAiChat,
            Protocol::OpenAiResponses,
            Protocol::AnthropicMessages,
            Protocol::GeminiGenerateContent,
            Protocol::OllamaChat,
        ] {
            let calls = if matches!(
                protocol,
                Protocol::GeminiGenerateContent | Protocol::OllamaChat
            ) {
                vec![
                    optional_call(0, Some("weather-id"), "get_weather"),
                    optional_call(1, Some("time-id"), "get_time"),
                ]
            } else {
                vec![
                    call(0, Some("weather-id"), "get_weather"),
                    call(1, Some("time-id"), "get_time"),
                ]
            };
            let assistant = turn(protocol, "response-id", calls.clone());
            let valid = vec![
                result(&calls[0], "WEATHER_SUNNY", false),
                result(&calls[1], "TIME_UTC_00:00", false),
            ];
            let mut wrong_name = valid.clone();
            wrong_name[0].call.name = "get_time".into();
            for invalid in [vec![valid[0].clone()], wrong_name] {
                let mut conversation =
                    ToolConversation::from_initial(protocol, initial(protocol, "047").body)
                        .unwrap();
                let before = conversation.current_request();
                assert!(conversation.append_follow_up(&assistant, &invalid).is_err());
                assert_eq!(conversation.current_request(), before, "{protocol:?}");
            }
        }
    }

    #[test]
    fn optional_ids_are_echoed_only_when_the_model_supplies_them() {
        for protocol in [Protocol::GeminiGenerateContent, Protocol::OllamaChat] {
            for id in [Some("provided-id"), None] {
                let assistant_call = optional_call(0, id, "get_weather");
                let assistant = turn(protocol, "unused", vec![assistant_call.clone()]);
                let results = vec![result(&assistant_call, "WEATHER_SUNNY", false)];
                let mut conversation =
                    ToolConversation::from_initial(protocol, initial(protocol, "046").body)
                        .expect("official initial request");
                let follow = conversation
                    .append_follow_up(&assistant, &results)
                    .expect("optional ID");
                let serialized = follow.body.to_string();
                assert_eq!(serialized.contains("provided-id"), id.is_some());
                assert_valid(
                    protocol,
                    &follow,
                    ToolRequestPhase::FollowUp {
                        previous: &assistant,
                        results: &results,
                    },
                );
            }
        }
    }

    #[test]
    fn mixed_optional_ids_are_correlated_without_inventing_the_missing_id() {
        for protocol in [Protocol::GeminiGenerateContent, Protocol::OllamaChat] {
            let calls = vec![
                optional_call(0, Some("weather-id"), "get_weather"),
                optional_call(1, None, "get_time"),
            ];
            let assistant = turn(protocol, "unused", calls.clone());
            let results = vec![
                result(&calls[1], "TIME_UTC_00:00", false),
                result(&calls[0], "WEATHER_SUNNY", false),
            ];
            let mut conversation =
                ToolConversation::from_initial(protocol, initial(protocol, "047").body)
                    .expect("official initial request");
            let follow = conversation
                .append_follow_up(&assistant, &results)
                .expect("mixed optional IDs are official");
            let serialized = follow.body.to_string();
            assert_eq!(serialized.matches("weather-id").count(), 2);
            assert!(!serialized.contains("time-id"));
            assert!(
                serialized.find("WEATHER_SUNNY").unwrap()
                    < serialized.find("TIME_UTC_00:00").unwrap()
            );
        }
    }

    #[test]
    fn validator_reports_sorted_deduplicated_protocol_errors() {
        let mut request = initial(Protocol::OllamaChat, "046");
        request.stream = false;
        request.body["stream"] = Value::Bool(false);
        request.body["tool_choice"] = Value::String("auto".into());
        request.body["parallel_tool_calls"] = Value::Bool(false);
        request.body["tools"][0]["function"]["strict"] = Value::Bool(true);
        request.body["tools"][0]["function"]["parameters"]["additionalProperties"] =
            Value::Bool(false);
        let errors = validate_tool_request(
            Protocol::OllamaChat,
            &endpoint(Protocol::OllamaChat),
            request.stream,
            &request.body,
            ToolRequestPhase::Initial,
        );
        let mut sorted = errors.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(errors, sorted);
        assert!(errors.contains(&"request.ollama.stream_required:/stream".into()));
        assert!(errors.contains(&"request.ollama.body_stream_required:/stream".into()));
        assert!(errors.contains(&"request.ollama.openai_only_field:/tool_choice".into()));
        assert!(errors.contains(&"request.ollama.openai_only_field:/parallel_tool_calls".into()));
        assert!(
            errors.contains(&"request.ollama.invalid_tool_shape:/tools/0/function/strict".into())
        );
        assert!(errors.contains(&"request.ollama.unsupported_schema_keyword:/tools/0/function/parameters/additionalProperties".into()));
    }

    #[test]
    fn openai_strict_tools_require_closed_object_schemas() {
        for protocol in [Protocol::OpenAiChat, Protocol::OpenAiResponses] {
            let mut request = initial(protocol, "046");
            let parameters = if protocol == Protocol::OpenAiChat {
                request.body.pointer_mut("/tools/0/function/parameters")
            } else {
                request.body.pointer_mut("/tools/0/parameters")
            }
            .expect("official parameters");
            parameters
                .as_object_mut()
                .unwrap()
                .remove("additionalProperties");
            let errors = validate_tool_request(
                protocol,
                &endpoint(protocol),
                true,
                &request.body,
                ToolRequestPhase::Initial,
            );
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("invalid_tool_shape")),
                "{protocol:?}: {errors:?}"
            );
        }
    }

    #[test]
    fn validator_rejects_cross_protocol_error_carrier_fields() {
        let protocol = Protocol::OpenAiChat;
        let assistant_call = call(0, Some("weather-id"), "get_weather");
        let assistant = turn(protocol, "unused", vec![assistant_call.clone()]);
        let results = vec![result(&assistant_call, "WEATHER_SUNNY", false)];
        let mut conversation =
            ToolConversation::from_initial(protocol, initial(protocol, "046").body)
                .expect("official initial request");
        let mut follow = conversation.append_follow_up(&assistant, &results).unwrap();
        follow.body["messages"][2]["is_error"] = Value::Bool(true);
        let errors = validate_tool_request(
            protocol,
            &endpoint(protocol),
            true,
            &follow.body,
            ToolRequestPhase::FollowUp {
                previous: &assistant,
                results: &results,
            },
        );
        assert!(
            errors.contains(&"request.openai_chat.error_carrier:/messages/2/is_error".into()),
            "{errors:?}"
        );
    }

    #[test]
    fn gemini_validator_requires_stream_action_and_exactly_one_alt_sse() {
        let request = initial(Protocol::GeminiGenerateContent, "046");
        let invalid =
            Url::parse("https://example.com/v1beta/models/gemini:generateContent?alt=sse&alt=sse")
                .unwrap();
        let errors = validate_tool_request(
            Protocol::GeminiGenerateContent,
            &invalid,
            true,
            &request.body,
            ToolRequestPhase::Initial,
        );
        assert!(errors.contains(&"request.gemini.invalid_stream_action:/url".into()));
        assert!(errors.contains(&"request.gemini.invalid_alt:/url".into()));
    }

    #[test]
    fn validator_rejects_anthropic_and_gemini_result_grouping_mutations() {
        for protocol in [Protocol::AnthropicMessages, Protocol::GeminiGenerateContent] {
            let assistant_call = if protocol == Protocol::GeminiGenerateContent {
                optional_call(0, None, "get_weather")
            } else {
                call(0, Some("weather-id"), "get_weather")
            };
            let assistant = turn(protocol, "unused", vec![assistant_call.clone()]);
            let results = vec![result(&assistant_call, "WEATHER_SUNNY", false)];
            let mut conversation =
                ToolConversation::from_initial(protocol, initial(protocol, "046").body)
                    .expect("official initial request");
            let mut follow = conversation.append_follow_up(&assistant, &results).unwrap();
            let collection = if protocol == Protocol::AnthropicMessages {
                "messages"
            } else {
                "contents"
            };
            follow.body[collection][2]["role"] = Value::String("assistant".into());
            let errors = validate_tool_request(
                protocol,
                &endpoint(protocol),
                true,
                &follow.body,
                ToolRequestPhase::FollowUp {
                    previous: &assistant,
                    results: &results,
                },
            );
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("history_mismatch")),
                "{protocol:?}: {errors:?}"
            );
        }
    }

    #[test]
    fn chat_validator_rejects_stream_only_index_in_follow_up_history() {
        let protocol = Protocol::OpenAiChat;
        let assistant_call = call(0, Some("weather-id"), "get_weather");
        let normal = turn(protocol, "unused", vec![assistant_call.clone()]);
        let results = vec![result(&assistant_call, "WEATHER_SUNNY", false)];
        let mut conversation =
            ToolConversation::from_initial(protocol, initial(protocol, "046").body)
                .expect("official initial request");
        let mut follow = conversation.append_follow_up(&normal, &results).unwrap();
        let mut malformed = normal;
        let ProtocolHistory::OpenAiChat(history) = &mut malformed.history else {
            unreachable!()
        };
        history["tool_calls"][0]["index"] = Value::from(0);
        follow.body["messages"][1] = history.clone();

        let errors = validate_tool_request(
            protocol,
            &endpoint(protocol),
            true,
            &follow.body,
            ToolRequestPhase::FollowUp {
                previous: &malformed,
                results: &results,
            },
        );
        assert_eq!(
            errors,
            ["request.openai_chat.stream_only_field:/history/tool_calls/0/index"]
        );
    }

    #[test]
    fn anthropic_validator_correlates_every_native_tool_use_with_the_turn() {
        let protocol = Protocol::AnthropicMessages;
        let calls = vec![
            call(0, Some("weather-id"), "get_weather"),
            call(1, Some("time-id"), "get_time"),
        ];
        let normal = turn(protocol, "unused", calls.clone());
        let results = vec![
            result(&calls[0], "WEATHER_SUNNY", false),
            result(&calls[1], "TIME_UTC_00:00", false),
        ];
        let mut conversation =
            ToolConversation::from_initial(protocol, initial(protocol, "047").body)
                .expect("official initial request");
        let normal_follow = conversation.append_follow_up(&normal, &results).unwrap();

        for (mutation, expected_code) in [
            ("missing", "history_call_count_mismatch"),
            ("duplicate", "duplicate_call_id"),
            ("wrong", "mismatched_call_id"),
        ] {
            let mut malformed = normal.clone();
            let ProtocolHistory::Anthropic(content) = &mut malformed.history else {
                unreachable!()
            };
            match mutation {
                "missing" => {
                    content.pop();
                }
                "duplicate" => content[3]["id"] = content[2]["id"].clone(),
                "wrong" => content[3]["id"] = Value::String("stale-id".into()),
                _ => unreachable!(),
            }
            let mut body = normal_follow.body.clone();
            body["messages"][1]["content"] = Value::Array(content.clone());
            let errors = validate_tool_request(
                protocol,
                &endpoint(protocol),
                true,
                &body,
                ToolRequestPhase::FollowUp {
                    previous: &malformed,
                    results: &results,
                },
            );
            assert!(
                errors.iter().any(|error| error.contains(expected_code)),
                "{mutation}: {errors:?}"
            );
        }
    }

    #[test]
    fn gemini_validator_correlates_all_native_function_calls_transactionally() {
        let protocol = Protocol::GeminiGenerateContent;
        let calls = vec![
            optional_call(0, Some("weather-id"), "get_weather"),
            optional_call(1, None, "get_time"),
        ];
        let normal = turn(protocol, "unused", calls.clone());
        let results = vec![
            result(&calls[0], "WEATHER_SUNNY", false),
            result(&calls[1], "TIME_UTC_00:00", false),
        ];
        let mutations = [
            ("missing", "history_call_count_mismatch"),
            ("id_presence", "mismatched_call_id"),
            ("id_null", "mismatched_call_id"),
            ("name", "name_mismatch"),
            ("args", "history_arguments_mismatch"),
        ];

        for (mutation, expected_code) in mutations {
            let mut malformed = normal.clone();
            let ProtocolHistory::Gemini(history) = &mut malformed.history else {
                unreachable!()
            };
            match mutation {
                "missing" => {
                    history["parts"].as_array_mut().unwrap().pop();
                }
                "id_presence" => {
                    history["parts"][1]["functionCall"]["id"] = Value::String("invented-id".into());
                }
                "id_null" => history["parts"][1]["functionCall"]["id"] = Value::Null,
                "name" => {
                    history["parts"][0]["functionCall"]["name"] = Value::String("get_time".into());
                }
                "args" => {
                    history["parts"][0]["functionCall"]["args"] = json!({"city": "Shanghai"});
                }
                _ => unreachable!(),
            }
            let mut conversation =
                ToolConversation::from_initial(protocol, initial(protocol, "047").body).unwrap();
            let before = conversation.current_request();
            let error = conversation
                .append_follow_up(&malformed, &results)
                .expect_err("malformed Gemini native history");
            assert!(
                error
                    .codes()
                    .iter()
                    .any(|error| error.contains(expected_code)),
                "{mutation}: {error:?}"
            );
            assert_eq!(conversation.current_request(), before);
            assert!(!error.codes().join(" ").contains("invented-id"));
        }
    }

    #[test]
    fn ollama_validator_uses_native_function_indexes_and_rolls_back_mutations() {
        let protocol = Protocol::OllamaChat;
        let calls = vec![
            ToolCall {
                index: 2,
                correlation: ToolCorrelation::Optional(None),
                name: "get_time".into(),
                arguments: json!({"zone": "UTC"}),
            },
            ToolCall {
                index: 9,
                correlation: ToolCorrelation::Optional(Some("weather-id".into())),
                name: "get_weather".into(),
                arguments: json!({"city": "Beijing"}),
            },
        ];
        let normal = AssistantTurn {
            history: ProtocolHistory::Ollama(json!({
                "role": "assistant",
                "content": "",
                "future_message": {"kept": true},
                "tool_calls": [
                    {
                        "id": "weather-id",
                        "future_call": "kept",
                        "function": {
                            "index": 9,
                            "name": "get_weather",
                            "arguments": {"city": "Beijing"},
                            "future_function": true
                        }
                    },
                    {
                        "function": {
                            "index": 2,
                            "name": "get_time",
                            "arguments": {"zone": "UTC"}
                        }
                    }
                ]
            })),
            tool_calls: calls.clone(),
            final_text: String::new(),
        };
        let results = vec![
            result(&calls[1], "WEATHER_SUNNY", false),
            result(&calls[0], "TIME_UTC_00:00", false),
        ];
        let mut valid_conversation =
            ToolConversation::from_initial(protocol, initial(protocol, "047").body).unwrap();
        valid_conversation
            .append_follow_up(&normal, &results)
            .expect("native index order is independent of array position");

        for (mutation, expected_code) in [
            ("missing", "history_call_count_mismatch"),
            ("id_presence", "mismatched_call_id"),
            ("id_null", "mismatched_call_id"),
            ("index", "mismatched_call_index"),
            ("name", "name_mismatch"),
            ("arguments", "history_arguments_mismatch"),
        ] {
            let mut malformed = normal.clone();
            let ProtocolHistory::Ollama(history) = &mut malformed.history else {
                unreachable!()
            };
            match mutation {
                "missing" => {
                    history["tool_calls"].as_array_mut().unwrap().pop();
                }
                "id_presence" => {
                    history["tool_calls"][1]["id"] = Value::String("invented-id".into());
                }
                "id_null" => history["tool_calls"][1]["id"] = Value::Null,
                "index" => history["tool_calls"][1]["function"]["index"] = Value::from(7),
                "name" => {
                    history["tool_calls"][0]["function"]["name"] = Value::String("get_time".into());
                }
                "arguments" => {
                    history["tool_calls"][0]["function"]["arguments"] = json!({"city": "Shanghai"});
                }
                _ => unreachable!(),
            }
            let mut conversation =
                ToolConversation::from_initial(protocol, initial(protocol, "047").body).unwrap();
            let before = conversation.current_request();
            let error = conversation
                .append_follow_up(&malformed, &results)
                .expect_err("malformed Ollama native history");
            assert!(
                error
                    .codes()
                    .iter()
                    .any(|error| error.contains(expected_code)),
                "{mutation}: {error:?}"
            );
            assert_eq!(conversation.current_request(), before);
            assert!(!error.codes().join(" ").contains("invented-id"));
        }
    }

    #[test]
    fn responses_rejects_blank_response_id_transactionally() {
        let protocol = Protocol::OpenAiResponses;
        let assistant_call = call(0, Some("weather-id"), "get_weather");
        let assistant = turn(protocol, "   ", vec![assistant_call.clone()]);
        let results = vec![result(&assistant_call, "WEATHER_SUNNY", false)];
        let mut conversation =
            ToolConversation::from_initial(protocol, initial(protocol, "046").body).unwrap();
        let before = conversation.current_request();

        let error = conversation
            .append_follow_up(&assistant, &results)
            .expect_err("blank response IDs cannot anchor a stateful follow-up");
        assert_eq!(
            error.codes(),
            ["request.openai_responses.previous_response_id:/history/response_id"]
        );
        assert_eq!(conversation.current_request(), before);
    }

    #[test]
    fn validator_rejects_incomplete_initial_request_profiles() {
        for protocol in [
            Protocol::OpenAiChat,
            Protocol::OpenAiResponses,
            Protocol::AnthropicMessages,
            Protocol::GeminiGenerateContent,
            Protocol::OllamaChat,
        ] {
            if protocol != Protocol::GeminiGenerateContent {
                let mut missing_model = initial(protocol, "046");
                missing_model.body["model"] = Value::String(String::new());
                assert!(
                    validate_tool_request(
                        protocol,
                        &endpoint(protocol),
                        true,
                        &missing_model.body,
                        ToolRequestPhase::Initial,
                    )
                    .iter()
                    .any(|error| error.contains("invalid_body:/model")),
                    "{protocol:?}"
                );
            }

            let mut missing_name = initial(protocol, "046");
            let name_pointer = match protocol {
                Protocol::OpenAiChat | Protocol::OllamaChat => "/tools/0/function/name",
                Protocol::OpenAiResponses | Protocol::AnthropicMessages => "/tools/0/name",
                Protocol::GeminiGenerateContent => "/tools/0/functionDeclarations/0/name",
                Protocol::Unknown => unreachable!(),
            };
            *missing_name
                .body
                .pointer_mut(name_pointer)
                .expect("official name field") = Value::String(String::new());
            assert!(
                validate_tool_request(
                    protocol,
                    &endpoint(protocol),
                    true,
                    &missing_name.body,
                    ToolRequestPhase::Initial,
                )
                .iter()
                .any(|error| error.contains("invalid_tool_shape")),
                "{protocol:?}"
            );

            let mut missing_prompt = initial(protocol, "046");
            match protocol {
                Protocol::OpenAiChat | Protocol::AnthropicMessages | Protocol::OllamaChat => {
                    missing_prompt.body["messages"] = json!([]);
                }
                Protocol::OpenAiResponses => missing_prompt.body["input"] = json!(""),
                Protocol::GeminiGenerateContent => missing_prompt.body["contents"] = json!([]),
                Protocol::Unknown => unreachable!(),
            }
            assert!(
                validate_tool_request(
                    protocol,
                    &endpoint(protocol),
                    true,
                    &missing_prompt.body,
                    ToolRequestPhase::Initial,
                )
                .iter()
                .any(|error| error.contains("invalid_body")),
                "{protocol:?}"
            );
        }

        let mut responses = initial(Protocol::OpenAiResponses, "046");
        responses.body["previous_response_id"] = Value::String("stale-response".into());
        assert!(
            validate_tool_request(
                Protocol::OpenAiResponses,
                &endpoint(Protocol::OpenAiResponses),
                true,
                &responses.body,
                ToolRequestPhase::Initial,
            )
            .contains(
                &"request.openai_responses.unexpected_previous_response_id:/previous_response_id"
                    .into()
            )
        );
    }

    #[test]
    fn unknown_protocol_is_rejected_without_creating_a_chat_shape() {
        assert_eq!(
            ToolConversation::from_initial(Protocol::Unknown, json!({})),
            Err(ToolProtocolError::UnsupportedProtocol)
        );
        let errors = validate_tool_request(
            Protocol::Unknown,
            &endpoint(Protocol::Unknown),
            true,
            &json!({}),
            ToolRequestPhase::Initial,
        );
        assert_eq!(errors, ["request.unknown.unsupported_protocol:/"]);
    }

    fn history_value(turn: &AssistantTurn) -> Value {
        match &turn.history {
            ProtocolHistory::OpenAiChat(value)
            | ProtocolHistory::Gemini(value)
            | ProtocolHistory::Ollama(value) => value.clone(),
            ProtocolHistory::Anthropic(content) => Value::Array(content.clone()),
            ProtocolHistory::OpenAiResponses { response_id } => Value::String(response_id.clone()),
        }
    }
}
