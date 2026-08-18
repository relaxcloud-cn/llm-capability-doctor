use serde_json::{Value, json};

use super::{Protocol, matches_response};

fn matches(protocol: Protocol, body: Value) -> bool {
    matches_response(
        protocol,
        &serde_json::to_vec(&body).expect("test JSON serializes"),
    )
}

#[test]
fn matches_response_accepts_native_synchronous_envelopes() {
    assert!(matches(
        Protocol::OpenAiChat,
        json!({"choices": [{"message": {"content": "hello"}}]})
    ));
    assert!(matches(
        Protocol::OpenAiResponses,
        json!({
            "id": "resp_123",
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": "hello"}]
            }]
        })
    ));
    assert!(matches(
        Protocol::AnthropicMessages,
        json!({"type": "message", "content": [{"type": "text", "text": "hello"}]})
    ));
    assert!(matches(
        Protocol::GeminiGenerateContent,
        json!({"candidates": [{"content": {"parts": [{"text": "hello"}]}}]})
    ));
    assert!(matches(
        Protocol::OllamaChat,
        json!({"message": {"content": "hello"}, "done": true})
    ));
}

#[test]
fn matches_response_rejects_non_native_or_malformed_envelopes() {
    assert!(!matches(
        Protocol::OpenAiChat,
        json!({"choices": [], "metadata": {"message": {"content": "lookalike"}}})
    ));
    assert!(!matches(
        Protocol::OpenAiChat,
        json!({"choices": [{"delta": {"content": "stream chunk"}}]})
    ));
    assert!(!matches(
        Protocol::OpenAiChat,
        json!({"choices": [{"message": {"content": []}}]})
    ));

    assert!(!matches(
        Protocol::OpenAiResponses,
        json!({
            "id": "resp_123",
            "output": [{
                "type": "function_call",
                "content": [{"type": "output_text", "text": "lookalike"}]
            }]
        })
    ));
    assert!(!matches(
        Protocol::OpenAiResponses,
        json!({
            "id": "",
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": "hello"}]
            }]
        })
    ));
    assert!(!matches(
        Protocol::OpenAiResponses,
        json!({
            "id": 1,
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": "hello"}]
            }]
        })
    ));
    assert!(!matches(
        Protocol::OpenAiResponses,
        json!({
            "id": "resp_123",
            "output": [],
            "metadata": {"output_text": "hello"}
        })
    ));
    assert!(!matches(
        Protocol::OpenAiResponses,
        json!({
            "id": "resp_123",
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": ""}]
            }],
            "metadata": {"output_text": "hello"}
        })
    ));

    assert!(!matches(
        Protocol::AnthropicMessages,
        json!({"type": "message", "content": [], "metadata": {"text": "hello"}})
    ));
    assert!(!matches(
        Protocol::AnthropicMessages,
        json!({"type": "event", "content": [{"type": "text", "text": "hello"}]})
    ));
    assert!(!matches(
        Protocol::AnthropicMessages,
        json!({"type": "message", "content": [{"type": "text", "text": 1}]})
    ));

    assert!(!matches(
        Protocol::GeminiGenerateContent,
        json!({"candidates": [], "metadata": {"parts": [{"text": "hello"}]}})
    ));
    assert!(!matches(
        Protocol::GeminiGenerateContent,
        json!({
            "candidates": [
                {"content": {"parts": []}},
                {"content": {"parts": [{"text": "hello"}]}}
            ]
        })
    ));
    assert!(!matches(
        Protocol::GeminiGenerateContent,
        json!({"candidates": [{"content": {"parts": [{"text": false}]}}]} )
    ));

    assert!(!matches(
        Protocol::OllamaChat,
        json!({"message": {"content": ""}, "done": true})
    ));
    assert!(!matches(
        Protocol::OllamaChat,
        json!({"message": {"content": "hello"}, "done": "true"})
    ));
    assert!(!matches(
        Protocol::OllamaChat,
        json!({"message": {"content": "hello"}, "done": false})
    ));
    assert!(!matches(
        Protocol::OllamaChat,
        json!({"message": "hello", "done": true, "metadata": {"content": "lookalike"}})
    ));

    assert!(!matches_response(Protocol::OpenAiChat, b"not JSON"));
    assert!(!matches(
        Protocol::Unknown,
        json!({"choices": [{"message": {"content": "hello"}}]})
    ));
}
