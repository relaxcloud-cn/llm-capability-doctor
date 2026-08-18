use serde_json::{Map, Value, json};
use thiserror::Error;

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
            "parallel_tool_calls": parallel
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
        Protocol::OpenAiChat | Protocol::OllamaChat | Protocol::Unknown => json!({
            "model": model,
            "messages": [{"role": "user", "content": prompt}],
            "tools": tools,
            "tool_choice": "auto",
            "parallel_tool_calls": parallel,
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
            weather_parameters(id == "043"),
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

fn weather_parameters(required_fields: bool) -> Value {
    if required_fields {
        json!({
            "type": "object",
            "properties": {
                "city": {"type": "string"},
                "unit": {"type": "string", "enum": ["C", "F"]},
                "days": {"type": "integer"}
            },
            "required": ["city", "unit", "days"],
            "additionalProperties": false
        })
    } else {
        json!({
            "type": "object",
            "properties": {"city": {"type": "string"}},
            "required": ["city"],
            "additionalProperties": false
        })
    }
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
        Protocol::OpenAiChat | Protocol::OpenAiResponses | Protocol::OllamaChat | Protocol::Unknown
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
        Protocol::OpenAiChat | Protocol::OllamaChat | Protocol::Unknown => json!({
            "type": "function",
            "function": {
                "name": name,
                "description": description,
                "parameters": parameters,
                "strict": true
            }
        }),
    }
}
