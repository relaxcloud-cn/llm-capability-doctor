use serde_json::{Value, json};

use super::{
    Protocol,
    tools::{FollowUpError, build_follow_up, tool_request},
};

const MODEL: &str = "test-model";
const PROMPT: &str = "test prompt";

fn supported_protocols() -> [Protocol; 4] {
    [
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        Protocol::AnthropicMessages,
        Protocol::GeminiGenerateContent,
    ]
}

#[test]
fn checks_040_and_045_stream_for_supported_protocols() {
    for protocol in supported_protocols() {
        for id in ["040", "045"] {
            let request = tool_request(protocol, MODEL, id, PROMPT);

            assert!(request.stream, "{protocol} check {id}");
            if protocol == Protocol::GeminiGenerateContent {
                assert!(
                    request.body.get("stream").is_none(),
                    "{protocol} check {id}"
                );
            } else {
                assert_eq!(
                    request.body.get("stream"),
                    Some(&Value::Bool(true)),
                    "{protocol} check {id}"
                );
            }
        }
    }
}

#[test]
fn check_047_and_out_of_profile_protocols_remain_non_streaming() {
    for protocol in supported_protocols() {
        let request = tool_request(protocol, MODEL, "047", PROMPT);

        assert!(!request.stream, "{protocol}");
        if protocol != Protocol::GeminiGenerateContent {
            assert_ne!(
                request.body.get("stream"),
                Some(&Value::Bool(true)),
                "{protocol}"
            );
        }
    }

    for protocol in [Protocol::OllamaChat, Protocol::Unknown] {
        for id in ["040", "045"] {
            let request = tool_request(protocol, MODEL, id, PROMPT);

            assert!(!request.stream, "{protocol} check {id}");
            assert_eq!(
                request.body.get("stream"),
                Some(&Value::Bool(false)),
                "{protocol} check {id}"
            );
        }
    }
}

#[test]
fn check_045_preserves_parallel_tool_calls_where_supported() {
    for protocol in [Protocol::OpenAiChat, Protocol::OpenAiResponses] {
        let request = tool_request(protocol, MODEL, "045", PROMPT);

        assert_eq!(
            request.body.get("parallel_tool_calls"),
            Some(&Value::Bool(true)),
            "{protocol}"
        );
    }
}

#[test]
fn responses_namespaces_weather_for_checks_041_and_047() {
    for id in ["041", "047"] {
        let request = tool_request(Protocol::OpenAiResponses, MODEL, id, PROMPT);

        assert_eq!(
            request.body["tools"],
            json!([
                {
                    "type": "namespace",
                    "name": "doctor",
                    "tools": [{
                        "type": "function",
                        "name": "get_weather",
                        "description": "Get weather",
                        "parameters": {
                            "type": "object",
                            "properties": {"city": {"type": "string"}},
                            "required": ["city"],
                            "additionalProperties": false
                        },
                        "strict": true
                    }]
                },
                {
                    "type": "function",
                    "name": "get_time",
                    "description": "Get time",
                    "parameters": {
                        "type": "object",
                        "properties": {"zone": {"type": "string"}},
                        "required": ["zone"],
                        "additionalProperties": false
                    },
                    "strict": true
                }
            ]),
            "check {id}"
        );
    }
}

#[test]
fn chat_anthropic_and_google_flatten_the_weather_namespace() {
    let cases = [
        (Protocol::OpenAiChat, "/tools/0/function/name"),
        (Protocol::AnthropicMessages, "/tools/0/name"),
        (
            Protocol::GeminiGenerateContent,
            "/tools/0/functionDeclarations/0/name",
        ),
    ];

    for (protocol, pointer) in cases {
        for id in ["041", "047"] {
            let request = tool_request(protocol, MODEL, id, PROMPT);

            assert_eq!(
                request.body.pointer(pointer),
                Some(&json!("doctor__get_weather")),
                "{protocol} check {id}"
            );
        }
    }
}

#[test]
fn namespace_is_not_applied_to_other_checks_or_out_of_profile_protocols() {
    for protocol in supported_protocols() {
        let request = tool_request(protocol, MODEL, "045", PROMPT);
        let pointer = match protocol {
            Protocol::OpenAiResponses => "/tools/0/name",
            Protocol::OpenAiChat => "/tools/0/function/name",
            Protocol::AnthropicMessages => "/tools/0/name",
            Protocol::GeminiGenerateContent => "/tools/0/functionDeclarations/0/name",
            Protocol::OllamaChat | Protocol::Unknown => unreachable!(),
        };

        assert_eq!(
            request.body.pointer(pointer),
            Some(&json!("get_weather")),
            "{protocol}"
        );
    }

    for protocol in [Protocol::OllamaChat, Protocol::Unknown] {
        for id in ["041", "047"] {
            let request = tool_request(protocol, MODEL, id, PROMPT);

            assert_eq!(
                request.body.pointer("/tools/0/function/name"),
                Some(&json!("get_weather")),
                "{protocol} check {id}"
            );
        }
    }
}

#[test]
fn chat_responses_and_anthropic_replay_non_empty_correlation_ids() {
    let cases = [
        (
            Protocol::OpenAiChat,
            json!({"choices": [{"message": {"role": "assistant", "tool_calls": [{
                "id": "chat-call", "type": "function", "function": {"name": "doctor__get_weather", "arguments": "{}"}
            }]}}]}),
            "/messages/2/tool_call_id",
            "chat-call",
        ),
        (
            Protocol::OpenAiResponses,
            json!({"id": "response-id", "output": [{
                "type": "function_call", "call_id": "responses-call", "name": "get_weather", "arguments": "{}"
            }]}),
            "/input/0/call_id",
            "responses-call",
        ),
        (
            Protocol::AnthropicMessages,
            json!({"content": [{
                "type": "tool_use", "id": "anthropic-call", "name": "doctor__get_weather", "input": {}
            }]}),
            "/messages/2/content/0/tool_use_id",
            "anthropic-call",
        ),
    ];

    for (protocol, response, pointer, expected) in cases {
        let initial = tool_request(protocol, MODEL, "047", PROMPT);
        let follow_up = build_follow_up(protocol, MODEL, "047", &initial.body, &response)
            .expect("valid correlation metadata");

        assert_eq!(
            follow_up.body.pointer(pointer),
            Some(&json!(expected)),
            "{protocol}"
        );
        assert!(!follow_up.stream, "{protocol}");
    }
}

#[test]
fn required_protocol_ids_reject_empty_strings() {
    let cases = [
        (
            Protocol::OpenAiChat,
            json!({"choices": [{"message": {"role": "assistant", "tool_calls": [{
                "id": "", "function": {"name": "doctor__get_weather", "arguments": "{}"}
            }]}}]}),
            "call id",
        ),
        (
            Protocol::OpenAiResponses,
            json!({"id": "response-id", "output": [{
                "type": "function_call", "call_id": "", "name": "get_weather", "arguments": "{}"
            }]}),
            "call id",
        ),
        (
            Protocol::OpenAiResponses,
            json!({"id": "", "output": [{
                "type": "function_call", "call_id": "responses-call", "name": "get_weather", "arguments": "{}"
            }]}),
            "response id",
        ),
        (
            Protocol::AnthropicMessages,
            json!({"content": [{
                "type": "tool_use", "id": "", "name": "doctor__get_weather", "input": {}
            }]}),
            "call id",
        ),
    ];

    for (protocol, response, field) in cases {
        let initial = tool_request(protocol, MODEL, "047", PROMPT);

        assert_eq!(
            build_follow_up(protocol, MODEL, "047", &initial.body, &response),
            Err(FollowUpError::MissingField(field)),
            "{protocol} {field}"
        );
    }
}

#[test]
fn google_follow_up_uses_a_local_id_when_upstream_id_is_supplied() {
    let response = google_response(Some("upstream-call"), "doctor__get_weather");
    let follow_up = google_follow_up("047", &response).expect("valid Google function call");

    let replay_id = follow_up
        .body
        .pointer("/contents/1/parts/0/functionCall/id");
    assert_eq!(
        replay_id,
        follow_up
            .body
            .pointer("/contents/2/parts/0/functionResponse/id")
    );
    assert_ne!(replay_id, Some(&json!("upstream-call")));
    assert_eq!(
        follow_up
            .body
            .pointer("/contents/2/parts/0/functionResponse/name"),
        Some(&json!("doctor__get_weather"))
    );
}

#[test]
fn google_follow_up_uses_a_local_id_when_upstream_id_is_absent() {
    let response = google_response(None, "doctor__get_weather");
    let follow_up = google_follow_up("048", &response).expect("Google IDs are optional");

    let replay_id = follow_up
        .body
        .pointer("/contents/1/parts/0/functionCall/id");
    assert!(
        replay_id
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty())
    );
    assert_eq!(
        replay_id,
        follow_up
            .body
            .pointer("/contents/2/parts/0/functionResponse/id")
    );
}

#[test]
fn google_follow_up_avoids_a_generated_id_collision() {
    let response = google_response(Some("call_model_doctor_049"), "get_weather");
    let follow_up = google_follow_up("049", &response).expect("valid Google function call");

    let replay_id = follow_up
        .body
        .pointer("/contents/1/parts/0/functionCall/id");
    assert_eq!(
        replay_id,
        follow_up
            .body
            .pointer("/contents/2/parts/0/functionResponse/id")
    );
    assert_ne!(replay_id, Some(&json!("call_model_doctor_049")));
}

#[test]
fn google_follow_up_rejects_an_empty_tool_name() {
    let response = google_response(None, "");
    let initial = tool_request(Protocol::GeminiGenerateContent, MODEL, "047", PROMPT);

    assert_eq!(
        build_follow_up(
            Protocol::GeminiGenerateContent,
            MODEL,
            "047",
            &initial.body,
            &response,
        ),
        Err(FollowUpError::MissingField("tool name"))
    );
}

fn google_response(id: Option<&str>, name: &str) -> Value {
    let mut function_call = json!({"name": name, "args": {"city": "Beijing"}});
    if let Some(id) = id {
        function_call["id"] = json!(id);
    }
    json!({
        "candidates": [{
            "content": {"role": "model", "parts": [{"functionCall": function_call}]}
        }]
    })
}

fn google_follow_up(id: &str, response: &Value) -> Result<super::RequestSpec, FollowUpError> {
    let initial = tool_request(Protocol::GeminiGenerateContent, MODEL, id, PROMPT);
    build_follow_up(
        Protocol::GeminiGenerateContent,
        MODEL,
        id,
        &initial.body,
        response,
    )
}
