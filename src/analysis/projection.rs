use serde_json::Value;

use super::evidence_reader::ParsedRequest;
use crate::evidence::StreamTermination;
use crate::protocol::stream::{AssistantTurn, parse_stream};
use crate::protocol::{Protocol, matches_response};

pub struct RequestProjection {
    pub visible_text: Option<String>,
    pub protocol_valid: bool,
    pub text_response_valid: bool,
    pub usage_valid: bool,
    pub tool_calls: Vec<ProjectedToolCall>,
    pub assistant_turn: Option<AssistantTurn>,
}

pub struct ProjectedToolCall {
    pub name: String,
    pub arguments: Value,
}

pub async fn project(request: &ParsedRequest) -> RequestProjection {
    let Some(protocol) = request
        .metadata
        .get("protocol")
        .and_then(|value| parse_protocol(value))
    else {
        return RequestProjection {
            visible_text: None,
            protocol_valid: false,
            text_response_valid: false,
            usage_valid: false,
            tool_calls: Vec::new(),
            assistant_turn: None,
        };
    };
    if request.metadata.get("stream").map(String::as_str) == Some("1") {
        let parsed = parse_stream(protocol, request.response_body.as_bytes()).await;
        let (visible_text, tool_calls) = parsed.assistant_turn.as_ref().map_or_else(
            || (None, Vec::new()),
            |turn| {
                (
                    Some(turn.final_text.clone()),
                    turn.tool_calls
                        .iter()
                        .map(|call| ProjectedToolCall {
                            name: call.name.clone(),
                            arguments: call.arguments.clone(),
                        })
                        .collect(),
                )
            },
        );
        return RequestProjection {
            visible_text,
            protocol_valid: parsed.stream_termination == StreamTermination::Completed
                && parsed.contract_errors.is_empty(),
            text_response_valid: parsed.stream_termination == StreamTermination::Completed
                && parsed.contract_errors.is_empty()
                && parsed
                    .assistant_turn
                    .as_ref()
                    .is_some_and(|turn| !turn.final_text.trim().is_empty()),
            usage_valid: false,
            tool_calls,
            assistant_turn: parsed.assistant_turn,
        };
    }

    project_non_stream(protocol, request.response_body.as_bytes())
}

fn project_non_stream(protocol: Protocol, body: &[u8]) -> RequestProjection {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return RequestProjection {
            visible_text: None,
            protocol_valid: false,
            text_response_valid: false,
            usage_valid: false,
            tool_calls: Vec::new(),
            assistant_turn: None,
        };
    };
    let visible_text = match protocol {
        Protocol::OpenAiChat => {
            let message = value.pointer("/choices/0/message");
            message
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        }
        Protocol::OpenAiResponses => collect_typed_text(value.get("output"), "message", "content"),
        Protocol::AnthropicMessages => collect_text_blocks(value.get("content"), Some("text")),
        Protocol::GeminiGenerateContent => {
            let content = value.pointer("/candidates/0/content");
            collect_text_blocks(content.and_then(|value| value.get("parts")), None)
        }
        Protocol::OllamaChat => value
            .pointer("/message/content")
            .and_then(Value::as_str)
            .map(str::to_owned),
        Protocol::Unknown => None,
    };
    RequestProjection {
        visible_text,
        protocol_valid: native_envelope_valid(protocol, &value),
        text_response_valid: matches_response(protocol, body),
        usage_valid: usage_valid(protocol, &value),
        tool_calls: Vec::new(),
        assistant_turn: None,
    }
}

fn native_envelope_valid(protocol: Protocol, value: &Value) -> bool {
    match protocol {
        Protocol::OpenAiChat => value
            .pointer("/choices/0/message")
            .and_then(Value::as_object)
            .is_some_and(|message| {
                message.get("content").is_some_and(Value::is_string)
                    || message
                        .get("tool_calls")
                        .and_then(Value::as_array)
                        .is_some_and(|calls| calls.iter().any(valid_function_call))
            }),
        Protocol::OpenAiResponses => {
            value
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.trim().is_empty())
                && value
                    .get("output")
                    .and_then(Value::as_array)
                    .is_some_and(|output| {
                        !output.is_empty()
                            && output.iter().any(|item| {
                                match item.get("type").and_then(Value::as_str) {
                                    Some("message") => item
                                        .get("content")
                                        .and_then(Value::as_array)
                                        .is_some_and(|content| {
                                            content.iter().any(|block| {
                                                block.get("type").and_then(Value::as_str)
                                                    == Some("output_text")
                                                    && block
                                                        .get("text")
                                                        .is_some_and(Value::is_string)
                                            })
                                        }),
                                    Some("function_call") => valid_flat_function_call(item),
                                    _ => false,
                                }
                            })
                    })
        }
        Protocol::AnthropicMessages => {
            value.get("type").and_then(Value::as_str) == Some("message")
                && value
                    .get("content")
                    .and_then(Value::as_array)
                    .is_some_and(|content| {
                        !content.is_empty()
                            && content.iter().any(|block| {
                                match block.get("type").and_then(Value::as_str) {
                                    Some("text") => block.get("text").is_some_and(Value::is_string),
                                    Some("tool_use") => {
                                        non_empty_string(block.get("name"))
                                            && block.get("input").is_some_and(Value::is_object)
                                    }
                                    _ => false,
                                }
                            })
                    })
        }
        Protocol::GeminiGenerateContent => value
            .pointer("/candidates/0/content/parts")
            .and_then(Value::as_array)
            .is_some_and(|parts| {
                !parts.is_empty()
                    && parts.iter().any(|part| {
                        part.get("text").is_some_and(Value::is_string)
                            || part.get("functionCall").is_some_and(|call| {
                                non_empty_string(call.get("name"))
                                    && call.get("args").is_some_and(Value::is_object)
                            })
                    })
            }),
        Protocol::OllamaChat => {
            value.get("done").and_then(Value::as_bool) == Some(true)
                && value
                    .get("message")
                    .and_then(Value::as_object)
                    .is_some_and(|message| {
                        message.get("content").is_some_and(Value::is_string)
                            || message
                                .get("tool_calls")
                                .and_then(Value::as_array)
                                .is_some_and(|calls| calls.iter().any(valid_function_call))
                    })
        }
        Protocol::Unknown => false,
    }
}

fn valid_function_call(call: &Value) -> bool {
    call.get("function").is_some_and(valid_flat_function_call)
}

fn valid_flat_function_call(call: &Value) -> bool {
    non_empty_string(call.get("name"))
        && call
            .get("arguments")
            .is_some_and(|arguments| arguments.is_string() || arguments.is_object())
}

fn non_empty_string(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|text| !text.trim().is_empty())
}

fn collect_typed_text(value: Option<&Value>, item_type: &str, child: &str) -> Option<String> {
    let mut text = String::new();
    for item in value?.as_array()? {
        if item.get("type").and_then(Value::as_str) != Some(item_type) {
            continue;
        }
        if let Some(value) = collect_text_blocks(item.get(child), Some("output_text")) {
            text.push_str(&value);
        }
    }
    Some(text)
}

fn collect_text_blocks(value: Option<&Value>, required_type: Option<&str>) -> Option<String> {
    let mut text = String::new();
    for block in value?.as_array()? {
        if required_type.is_some() && block.get("type").and_then(Value::as_str) != required_type {
            continue;
        }
        if let Some(value) = block.get("text").and_then(Value::as_str) {
            text.push_str(value);
        }
    }
    Some(text)
}

fn usage_valid(protocol: Protocol, value: &Value) -> bool {
    let paths: &[(&str, &str)] = match protocol {
        Protocol::OpenAiChat => &[("/usage/prompt_tokens", "/usage/completion_tokens")],
        Protocol::OpenAiResponses => &[("/usage/input_tokens", "/usage/output_tokens")],
        Protocol::AnthropicMessages => &[("/usage/input_tokens", "/usage/output_tokens")],
        Protocol::GeminiGenerateContent => &[(
            "/usageMetadata/promptTokenCount",
            "/usageMetadata/candidatesTokenCount",
        )],
        Protocol::OllamaChat => &[("/prompt_eval_count", "/eval_count")],
        Protocol::Unknown => &[],
    };
    paths.iter().any(|(input, output)| {
        value.pointer(input).and_then(Value::as_u64).is_some()
            && value
                .pointer(output)
                .and_then(Value::as_u64)
                .is_some_and(|count| count > 0)
    })
}

pub(super) fn parse_protocol(value: &str) -> Option<Protocol> {
    match value {
        "openai_chat" => Some(Protocol::OpenAiChat),
        "openai_responses" => Some(Protocol::OpenAiResponses),
        "anthropic_messages" => Some(Protocol::AnthropicMessages),
        "gemini_generate_content" => Some(Protocol::GeminiGenerateContent),
        "ollama_chat" => Some(Protocol::OllamaChat),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[tokio::test]
    async fn reconstructs_model_visible_text_across_stream_events() {
        let mut body =
            include_str!("../protocol/fixtures/openai_chat_final.sse").replace("046_OK", "005_OK");
        body.push('\n');
        let request = request_fixture("openai_chat", true, &body);

        let projection = project(&request).await;

        assert!(projection.protocol_valid);
        assert_eq!(
            projection.visible_text.as_deref(),
            Some("MODEL_DOCTOR_CASE_005_OK")
        );
        assert!(!body.contains("MODEL_DOCTOR_CASE_005_OK"));
    }

    #[tokio::test]
    async fn validates_native_usage_fields() {
        let request = request_fixture(
            "openai_chat",
            false,
            r#"{"choices":[{"message":{"content":"ok"}}],"usage":{"prompt_tokens":2,"completion_tokens":1}}"#,
        );

        let projection = project(&request).await;

        assert!(projection.protocol_valid);
        assert!(projection.usage_valid);
        assert_eq!(projection.visible_text.as_deref(), Some("ok"));
    }

    #[tokio::test]
    async fn rejects_empty_openai_responses_envelope() {
        let request = request_fixture(
            "openai_responses",
            false,
            r#"{"id":"","output":[{"type":"message","content":[]}]}"#,
        );

        let projection = project(&request).await;

        assert!(!projection.protocol_valid);
    }

    #[tokio::test]
    async fn rejects_zero_output_token_usage() {
        let request = request_fixture(
            "openai_chat",
            false,
            r#"{"choices":[{"message":{"content":"ok"}}],"usage":{"prompt_tokens":2,"completion_tokens":0}}"#,
        );

        let projection = project(&request).await;

        assert!(!projection.usage_valid);
    }

    #[tokio::test]
    async fn accepts_zero_input_tokens_when_output_is_positive() {
        let request = request_fixture(
            "openai_chat",
            false,
            r#"{"choices":[{"message":{"content":"ok"}}],"usage":{"prompt_tokens":0,"completion_tokens":1}}"#,
        );

        let projection = project(&request).await;

        assert!(projection.usage_valid);
    }

    #[tokio::test]
    async fn accepts_native_tool_only_envelope_without_text() {
        let request = request_fixture(
            "openai_chat",
            false,
            r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"get_weather","arguments":"{\"city\":\"Beijing\"}"}}]}}]}"#,
        );

        let projection = project(&request).await;

        assert!(projection.protocol_valid);
        assert!(!projection.text_response_valid);
    }

    fn request_fixture(protocol: &str, stream: bool, body: &str) -> ParsedRequest {
        ParsedRequest {
            request_id: "request-1".into(),
            metadata: BTreeMap::from([
                ("protocol".into(), protocol.into()),
                ("stream".into(), if stream { "1" } else { "0" }.into()),
            ]),
            metrics: BTreeMap::new(),
            request_body: String::new(),
            response_headers: String::new(),
            stderr: String::new(),
            response_body: body.into(),
        }
    }
}
