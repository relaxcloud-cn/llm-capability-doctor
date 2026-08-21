use serde_json::Value;

use super::evidence_reader::ParsedRequest;
use crate::evidence::StreamTermination;
use crate::protocol::Protocol;
use crate::protocol::stream::parse_stream;

pub struct RequestProjection {
    pub visible_text: Option<String>,
    pub protocol_valid: bool,
    pub usage_valid: bool,
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
            usage_valid: false,
        };
    };
    if request.metadata.get("stream").map(String::as_str) == Some("1") {
        let parsed = parse_stream(protocol, request.response_body.as_bytes()).await;
        return RequestProjection {
            visible_text: parsed
                .assistant_turn
                .as_ref()
                .map(|turn| turn.final_text.clone()),
            protocol_valid: parsed.stream_termination == StreamTermination::Completed
                && parsed.contract_errors.is_empty(),
            usage_valid: false,
        };
    }

    project_non_stream(protocol, request.response_body.as_bytes())
}

fn project_non_stream(protocol: Protocol, body: &[u8]) -> RequestProjection {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return RequestProjection {
            visible_text: None,
            protocol_valid: false,
            usage_valid: false,
        };
    };
    let (visible_text, protocol_valid) = match protocol {
        Protocol::OpenAiChat => {
            let message = value.pointer("/choices/0/message");
            (
                message
                    .and_then(|message| message.get("content"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                message.and_then(Value::as_object).is_some(),
            )
        }
        Protocol::OpenAiResponses => {
            let text = collect_typed_text(value.get("output"), "message", "content");
            (
                text,
                value.get("id").and_then(Value::as_str).is_some()
                    && value.get("output").and_then(Value::as_array).is_some(),
            )
        }
        Protocol::AnthropicMessages => (
            collect_text_blocks(value.get("content"), Some("text")),
            value.get("type").and_then(Value::as_str) == Some("message")
                && value.get("content").and_then(Value::as_array).is_some(),
        ),
        Protocol::GeminiGenerateContent => {
            let content = value.pointer("/candidates/0/content");
            (
                collect_text_blocks(content.and_then(|value| value.get("parts")), None),
                content.and_then(Value::as_object).is_some(),
            )
        }
        Protocol::OllamaChat => (
            value
                .pointer("/message/content")
                .and_then(Value::as_str)
                .map(str::to_owned),
            value.get("message").and_then(Value::as_object).is_some()
                && value.get("done").and_then(Value::as_bool) == Some(true),
        ),
        Protocol::Unknown => (None, false),
    };
    RequestProjection {
        visible_text,
        protocol_valid,
        usage_valid: usage_valid(protocol, &value),
    }
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
            && value.pointer(output).and_then(Value::as_u64).is_some()
    })
}

fn parse_protocol(value: &str) -> Option<Protocol> {
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
