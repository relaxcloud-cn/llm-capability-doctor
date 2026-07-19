use std::fmt;

use serde_json::{Value, json};

pub const ANTHROPIC_MAX_TOKENS: u64 = 2048;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Protocol {
    OpenAiChat,
    OpenAiResponses,
    AnthropicMessages,
    GeminiGenerateContent,
    OllamaChat,
    Unknown,
}

impl fmt::Display for Protocol {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::OpenAiChat => "openai_chat",
            Self::OpenAiResponses => "openai_responses",
            Self::AnthropicMessages => "anthropic_messages",
            Self::GeminiGenerateContent => "gemini_generate_content",
            Self::OllamaChat => "ollama_chat",
            Self::Unknown => "unknown",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AuthMode {
    Bearer,
    ApiKey,
    XApiKey,
    XGoogApiKey,
    None,
}

impl fmt::Display for AuthMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Bearer => "bearer",
            Self::ApiKey => "api_key",
            Self::XApiKey => "x_api_key",
            Self::XGoogApiKey => "x_goog_api_key",
            Self::None => "none",
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RequestSpec {
    pub body: Value,
    pub stream: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProbeCandidate {
    pub protocol: Protocol,
    pub auth: AuthMode,
}

pub const PROBE_CANDIDATES: [ProbeCandidate; 7] = [
    ProbeCandidate {
        protocol: Protocol::OpenAiChat,
        auth: AuthMode::Bearer,
    },
    ProbeCandidate {
        protocol: Protocol::OpenAiResponses,
        auth: AuthMode::Bearer,
    },
    ProbeCandidate {
        protocol: Protocol::AnthropicMessages,
        auth: AuthMode::XApiKey,
    },
    ProbeCandidate {
        protocol: Protocol::GeminiGenerateContent,
        auth: AuthMode::XGoogApiKey,
    },
    ProbeCandidate {
        protocol: Protocol::OllamaChat,
        auth: AuthMode::Bearer,
    },
    ProbeCandidate {
        protocol: Protocol::OpenAiChat,
        auth: AuthMode::ApiKey,
    },
    ProbeCandidate {
        protocol: Protocol::OpenAiResponses,
        auth: AuthMode::ApiKey,
    },
];

pub fn basic_request(protocol: Protocol, model: &str, prompt: &str, stream: bool) -> RequestSpec {
    let body = match protocol {
        Protocol::OpenAiResponses => {
            json!({"model": model, "input": prompt, "stream": stream})
        }
        Protocol::AnthropicMessages => json!({
            "model": model,
            "max_tokens": ANTHROPIC_MAX_TOKENS,
            "messages": [{"role": "user", "content": prompt}],
            "stream": stream
        }),
        Protocol::GeminiGenerateContent => json!({
            "contents": [{"role": "user", "parts": [{"text": prompt}]}],
            "generationConfig": {"maxOutputTokens": 64}
        }),
        Protocol::OpenAiChat | Protocol::OllamaChat | Protocol::Unknown => json!({
            "model": model,
            "messages": [{"role": "user", "content": prompt}],
            "stream": stream
        }),
    };
    RequestSpec { body, stream }
}

pub fn multi_turn_request(protocol: Protocol, model: &str) -> RequestSpec {
    const FIRST: &str = "Current state is OLD_STATE.";
    const ACKNOWLEDGED: &str = "Acknowledged OLD_STATE.";
    const CORRECTION: &str = "Correction: current state is NEW_STATE. Reply only NEW_STATE.";

    let body = match protocol {
        Protocol::OpenAiResponses => json!({
            "model": model,
            "input": [
                {"role": "user", "content": FIRST},
                {"role": "assistant", "content": ACKNOWLEDGED},
                {"role": "user", "content": CORRECTION}
            ],
            "stream": false
        }),
        Protocol::AnthropicMessages => json!({
            "model": model,
            "max_tokens": ANTHROPIC_MAX_TOKENS,
            "messages": [
                {"role": "user", "content": FIRST},
                {"role": "assistant", "content": ACKNOWLEDGED},
                {"role": "user", "content": CORRECTION}
            ],
            "stream": false
        }),
        Protocol::GeminiGenerateContent => json!({
            "contents": [
                {"role": "user", "parts": [{"text": FIRST}]},
                {"role": "model", "parts": [{"text": ACKNOWLEDGED}]},
                {"role": "user", "parts": [{"text": CORRECTION}]}
            ],
            "generationConfig": {"maxOutputTokens": 64}
        }),
        Protocol::OpenAiChat | Protocol::OllamaChat | Protocol::Unknown => json!({
            "model": model,
            "messages": [
                {"role": "user", "content": FIRST},
                {"role": "assistant", "content": ACKNOWLEDGED},
                {"role": "user", "content": CORRECTION}
            ],
            "stream": false
        }),
    };
    RequestSpec {
        body,
        stream: false,
    }
}

pub fn thinking_request(
    protocol: Protocol,
    model: &str,
    prompt: &str,
    effort: &str,
    stream: bool,
) -> RequestSpec {
    let mut request = basic_request(protocol, model, prompt, stream);
    let object = request
        .body
        .as_object_mut()
        .expect("protocol request bodies are JSON objects");
    match protocol {
        Protocol::OpenAiResponses => {
            object.insert(
                "reasoning".into(),
                json!({"effort": effort, "summary": "auto"}),
            );
        }
        Protocol::OpenAiChat | Protocol::OllamaChat => {
            object.insert("reasoning_effort".into(), Value::String(effort.to_owned()));
        }
        Protocol::AnthropicMessages => {
            object.insert(
                "thinking".into(),
                json!({"type": "enabled", "budget_tokens": 1024}),
            );
        }
        Protocol::GeminiGenerateContent | Protocol::Unknown => {}
    }
    request
}

pub fn matches_response(protocol: Protocol, body: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    match protocol {
        Protocol::OpenAiChat => {
            has_key(&value, "choices") && (has_key(&value, "message") || has_key(&value, "delta"))
        }
        Protocol::OpenAiResponses => {
            (value.get("object").and_then(Value::as_str) == Some("response")
                || has_key(&value, "output"))
                && (has_key(&value, "output_text")
                    || has_string(&value, "output_text")
                    || has_string_prefix(&value, "response."))
        }
        Protocol::AnthropicMessages => {
            value.get("type").and_then(Value::as_str) == Some("message")
                && (has_key(&value, "stop_reason") || has_key(&value, "content"))
        }
        Protocol::GeminiGenerateContent => {
            has_key(&value, "candidates") && has_key(&value, "parts")
        }
        Protocol::OllamaChat => has_key(&value, "message") && has_key(&value, "done"),
        Protocol::Unknown => false,
    }
}

fn has_key(value: &Value, wanted: &str) -> bool {
    match value {
        Value::Object(object) => {
            object.contains_key(wanted) || object.values().any(|child| has_key(child, wanted))
        }
        Value::Array(array) => array.iter().any(|child| has_key(child, wanted)),
        _ => false,
    }
}

fn has_string(value: &Value, wanted: &str) -> bool {
    match value {
        Value::String(text) => text == wanted,
        Value::Object(object) => object.values().any(|child| has_string(child, wanted)),
        Value::Array(array) => array.iter().any(|child| has_string(child, wanted)),
        _ => false,
    }
}

fn has_string_prefix(value: &Value, prefix: &str) -> bool {
    match value {
        Value::String(text) => text.starts_with(prefix),
        Value::Object(object) => object
            .values()
            .any(|child| has_string_prefix(child, prefix)),
        Value::Array(array) => array.iter().any(|child| has_string_prefix(child, prefix)),
        _ => false,
    }
}
