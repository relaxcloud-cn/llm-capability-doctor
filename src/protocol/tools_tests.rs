use serde_json::{Value, json};

use super::{
    Protocol,
    stream::{AssistantTurn, ProtocolHistory, ToolCall, ToolCorrelation},
    tools::{ExecutedToolResult, ToolConversation, tool_prompt, tool_request},
};

const MODEL: &str = "test-model";

fn supported_protocols() -> [Protocol; 4] {
    [
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        Protocol::AnthropicMessages,
        Protocol::GeminiGenerateContent,
    ]
}

#[test]
fn checks_040_045_and_complete_loops_use_the_expected_stream_mode() {
    for protocol in supported_protocols() {
        for id in ["040", "045", "046", "047", "048", "049"] {
            let request = tool_request(protocol, MODEL, id, tool_prompt(id).unwrap());
            assert!(request.stream, "{protocol} check {id}");
            if protocol == Protocol::GeminiGenerateContent {
                assert!(
                    request.body.get("stream").is_none(),
                    "{protocol} check {id}"
                );
            } else {
                assert_eq!(request.body.get("stream"), Some(&Value::Bool(true)));
            }
        }
    }

    for protocol in [Protocol::OllamaChat, Protocol::Unknown] {
        for id in ["040", "045"] {
            let request = tool_request(protocol, MODEL, id, tool_prompt(id).unwrap());
            assert!(!request.stream, "{protocol} check {id}");
            assert_eq!(request.body.get("stream"), Some(&Value::Bool(false)));
        }
    }
}

#[test]
fn responses_uses_a_native_weather_namespace_for_041_and_047() {
    for id in ["041", "047"] {
        let request = tool_request(
            Protocol::OpenAiResponses,
            MODEL,
            id,
            tool_prompt(id).unwrap(),
        );
        assert_eq!(request.body["tools"][0]["type"], "namespace");
        assert_eq!(request.body["tools"][0]["name"], "doctor");
        assert_eq!(request.body["tools"][0]["tools"][0]["name"], "get_weather");
        assert_eq!(request.body["tools"][1]["name"], "get_time");
        if id == "047" {
            ToolConversation::from_initial(Protocol::OpenAiResponses, request.body)
                .expect("the complete-loop namespace request satisfies the integrated validator");
        }
    }
}

#[test]
fn chat_anthropic_and_gemini_flatten_the_weather_namespace() {
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
            let request = tool_request(protocol, MODEL, id, tool_prompt(id).unwrap());
            assert_eq!(
                request.body.pointer(pointer),
                Some(&json!("doctor__get_weather")),
                "{protocol} check {id}",
            );
        }
    }
}

#[test]
fn gemini_follow_up_replaces_an_upstream_id_with_one_local_id() {
    let id = "047";
    let initial = tool_request(
        Protocol::GeminiGenerateContent,
        MODEL,
        id,
        tool_prompt(id).unwrap(),
    );
    let mut conversation =
        ToolConversation::from_initial(Protocol::GeminiGenerateContent, initial.body).unwrap();
    let call = ToolCall {
        index: 0,
        correlation: ToolCorrelation::Optional(Some("upstream-call".into())),
        name: "doctor__get_weather".into(),
        arguments: json!({"city": "Beijing"}),
    };
    let turn = AssistantTurn {
        history: ProtocolHistory::Gemini(json!({
            "role": "model",
            "parts": [{"functionCall": {
                "id": "upstream-call",
                "name": "doctor__get_weather",
                "args": {"city": "Beijing"}
            }}]
        })),
        tool_calls: vec![call.clone()],
        final_text: String::new(),
    };
    let follow_up = conversation
        .append_follow_up(
            &turn,
            &[ExecutedToolResult {
                call,
                output: "WEATHER_SUNNY".into(),
                is_error: false,
            }],
        )
        .unwrap();

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
    assert!(
        replay_id
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
    );
}
