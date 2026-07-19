use model_capability_doctor::protocol::{
    AuthMode, PROBE_CANDIDATES, Protocol, basic_request, matches_response, multi_turn_request,
    thinking_request,
};
use serde_json::{Value, json};

#[test]
fn basic_requests_match_all_five_protocol_contracts() {
    let cases = [
        (
            Protocol::OpenAiChat,
            json!({
                "model": "fixture-model",
                "messages": [{"role": "user", "content": "hello"}],
                "stream": true
            }),
        ),
        (
            Protocol::OpenAiResponses,
            json!({"model": "fixture-model", "input": "hello", "stream": true}),
        ),
        (
            Protocol::AnthropicMessages,
            json!({
                "model": "fixture-model",
                "max_tokens": 2048,
                "messages": [{"role": "user", "content": "hello"}],
                "stream": true
            }),
        ),
        (
            Protocol::GeminiGenerateContent,
            json!({
                "contents": [{"role": "user", "parts": [{"text": "hello"}]}],
                "generationConfig": {"maxOutputTokens": 64}
            }),
        ),
        (
            Protocol::OllamaChat,
            json!({
                "model": "fixture-model",
                "messages": [{"role": "user", "content": "hello"}],
                "stream": true
            }),
        ),
    ];

    for (protocol, expected) in cases {
        let request = basic_request(protocol, "fixture-model", "hello", true);
        assert_eq!(request.body, expected, "{protocol}");
        assert!(request.stream);
    }
}

#[test]
fn unknown_protocol_uses_openai_chat_body_as_fallback() {
    let unknown = basic_request(Protocol::Unknown, "m", "hello", false);
    let chat = basic_request(Protocol::OpenAiChat, "m", "hello", false);
    assert_eq!(unknown, chat);
}

#[test]
fn probe_candidates_keep_shell_order() {
    let actual: Vec<(Protocol, AuthMode)> = PROBE_CANDIDATES
        .iter()
        .map(|candidate| (candidate.protocol, candidate.auth))
        .collect();
    assert_eq!(
        actual,
        [
            (Protocol::OpenAiChat, AuthMode::Bearer),
            (Protocol::OpenAiResponses, AuthMode::Bearer),
            (Protocol::AnthropicMessages, AuthMode::XApiKey),
            (Protocol::GeminiGenerateContent, AuthMode::XGoogApiKey),
            (Protocol::OllamaChat, AuthMode::Bearer),
            (Protocol::OpenAiChat, AuthMode::ApiKey),
            (Protocol::OpenAiResponses, AuthMode::ApiKey),
        ]
    );
}

#[test]
fn response_matching_recognizes_each_protocol_and_rejects_noise() {
    let cases = [
        (
            Protocol::OpenAiChat,
            json!({"choices": [{"message": {"role": "assistant", "content": "ok"}}]}),
        ),
        (
            Protocol::OpenAiResponses,
            json!({"object": "response", "output": [{"type": "output_text", "text": "ok"}]}),
        ),
        (
            Protocol::AnthropicMessages,
            json!({"type": "message", "content": [{"type": "text", "text": "ok"}], "stop_reason": "end_turn"}),
        ),
        (
            Protocol::GeminiGenerateContent,
            json!({"candidates": [{"content": {"parts": [{"text": "ok"}]}}]}),
        ),
        (
            Protocol::OllamaChat,
            json!({"message": {"role": "assistant", "content": "ok"}, "done": true}),
        ),
    ];

    for (protocol, body) in cases {
        let encoded = serde_json::to_vec(&body).unwrap();
        assert!(matches_response(protocol, &encoded), "{protocol}: {body}");
        assert!(!matches_response(protocol, br#"{"status":"ok"}"#));
    }
    assert!(!matches_response(Protocol::Unknown, br#"{"choices":[]}"#));
    assert!(!matches_response(Protocol::OpenAiChat, b"not json"));
}

#[test]
fn multi_turn_requests_preserve_protocol_roles() {
    let openai = multi_turn_request(Protocol::OpenAiChat, "m");
    assert_eq!(openai.body["messages"][1]["role"], "assistant");
    assert_eq!(
        openai.body["messages"][2]["content"],
        "Correction: current state is NEW_STATE. Reply only NEW_STATE."
    );

    let responses = multi_turn_request(Protocol::OpenAiResponses, "m");
    assert_eq!(responses.body["input"][1]["role"], "assistant");

    let anthropic = multi_turn_request(Protocol::AnthropicMessages, "m");
    assert_eq!(anthropic.body["max_tokens"], 2048);

    let gemini = multi_turn_request(Protocol::GeminiGenerateContent, "m");
    assert_eq!(gemini.body["contents"][1]["role"], "model");
    assert_eq!(gemini.body["generationConfig"]["maxOutputTokens"], 64);
}

#[test]
fn thinking_requests_encode_protocol_specific_controls() {
    let chat = thinking_request(Protocol::OpenAiChat, "m", "think", "high", false);
    assert_eq!(chat.body["reasoning_effort"], "high");

    let responses = thinking_request(Protocol::OpenAiResponses, "m", "think", "low", false);
    assert_eq!(
        responses.body["reasoning"],
        json!({"effort": "low", "summary": "auto"})
    );

    let anthropic = thinking_request(Protocol::AnthropicMessages, "m", "think", "high", false);
    assert_eq!(
        anthropic.body["thinking"],
        json!({"type": "enabled", "budget_tokens": 1024})
    );
    assert_eq!(anthropic.body["max_tokens"], 2048);

    let gemini = thinking_request(Protocol::GeminiGenerateContent, "m", "think", "high", true);
    assert_eq!(gemini.body.get("reasoning_effort"), None);
    assert!(gemini.stream);

    let ollama = thinking_request(Protocol::OllamaChat, "m", "think", "low", true);
    assert_eq!(ollama.body["reasoning_effort"], "low");
    assert!(ollama.stream);

    let unknown = thinking_request(Protocol::Unknown, "m", "think", "low", false);
    assert_eq!(unknown.body.get("reasoning_effort"), None);
}

#[test]
fn request_specs_serialize_to_json_without_losing_unicode() {
    let request = basic_request(Protocol::OpenAiChat, "模型", "你好", false);
    let encoded = serde_json::to_vec(&request.body).unwrap();
    let decoded: Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded["model"], "模型");
    assert_eq!(decoded["messages"][0]["content"], "你好");
}
