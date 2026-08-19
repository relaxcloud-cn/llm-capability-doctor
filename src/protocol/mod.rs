use std::fmt;

use serde_json::{Value, json};
use url::Url;

pub(crate) mod stream;
pub(crate) mod tool_loop;
pub mod tools;

#[cfg(test)]
mod stream_tests;

#[cfg(test)]
#[path = "tests.rs"]
mod compatibility_tests;

#[cfg(test)]
mod tools_tests;

pub const ANTHROPIC_MAX_TOKENS: u64 = 2048;

pub fn normalize_request_url(protocol: Protocol, configured: &Url, stream: bool) -> Url {
    if protocol != Protocol::GeminiGenerateContent || !stream {
        return configured.clone();
    }

    let path = configured.path();
    let normalized_path = if let Some(prefix) = path.strip_suffix(":generateContent") {
        format!("{prefix}:streamGenerateContent")
    } else if path.ends_with(":streamGenerateContent") {
        path.to_owned()
    } else {
        return configured.clone();
    };

    let mut normalized = configured.clone();
    normalized.set_path(&normalized_path);
    normalized.set_query(Some("alt=sse"));
    normalized
}

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
            let (budget_tokens, max_tokens) = if effort == "high" {
                (4096, 6144)
            } else {
                (1024, ANTHROPIC_MAX_TOKENS)
            };
            object.insert("max_tokens".into(), Value::from(max_tokens));
            object.insert(
                "thinking".into(),
                json!({"type": "enabled", "budget_tokens": budget_tokens}),
            );
        }
        Protocol::GeminiGenerateContent => {
            let thinking_budget = if effort == "high" { 8192 } else { 1024 };
            let generation_config = object
                .get_mut("generationConfig")
                .and_then(Value::as_object_mut)
                .expect("Gemini requests always contain generationConfig");
            generation_config.insert(
                "thinkingConfig".into(),
                json!({
                    "thinkingBudget": thinking_budget,
                    "includeThoughts": true
                }),
            );
        }
        Protocol::Unknown => {}
    }
    request
}

pub fn matches_response(protocol: Protocol, body: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    match protocol {
        Protocol::OpenAiChat => value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"))
            .and_then(Value::as_object)
            .is_some_and(|message| non_empty_string(message.get("content"))),
        Protocol::OpenAiResponses => {
            non_empty_string(value.get("id"))
                && value
                    .get("output")
                    .and_then(Value::as_array)
                    .is_some_and(|output| {
                        !output.is_empty()
                            && output.iter().any(|item| {
                                item.get("type").and_then(Value::as_str) == Some("message")
                                    && item.get("content").and_then(Value::as_array).is_some_and(
                                        |content| {
                                            content.iter().any(|block| {
                                                block.get("type").and_then(Value::as_str)
                                                    == Some("output_text")
                                                    && non_empty_string(block.get("text"))
                                            })
                                        },
                                    )
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
                                block.get("type").and_then(Value::as_str) == Some("text")
                                    && non_empty_string(block.get("text"))
                            })
                    })
        }
        Protocol::GeminiGenerateContent => value
            .get("candidates")
            .and_then(Value::as_array)
            .and_then(|candidates| candidates.first())
            .and_then(|candidate| candidate.get("content"))
            .and_then(Value::as_object)
            .and_then(|content| content.get("parts"))
            .and_then(Value::as_array)
            .is_some_and(|parts| {
                !parts.is_empty() && parts.iter().any(|part| non_empty_string(part.get("text")))
            }),
        Protocol::OllamaChat => {
            value
                .get("message")
                .and_then(Value::as_object)
                .is_some_and(|message| non_empty_string(message.get("content")))
                && value.get("done").and_then(Value::as_bool) == Some(true)
        }
        Protocol::Unknown => false,
    }
}

fn non_empty_string(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::{Protocol, normalize_request_url};

    #[test]
    fn gemini_stream_url_is_normalized() {
        let cases = [
            (
                "https://example.com/v1beta/models/gemini:generateContent?key=x&trace=1",
                "https://example.com/v1beta/models/gemini:streamGenerateContent?alt=sse",
            ),
            (
                "https://example.com/v1beta/models/gemini:streamGenerateContent?alt=json&trace=1",
                "https://example.com/v1beta/models/gemini:streamGenerateContent?alt=sse",
            ),
        ];

        for (configured, expected) in cases {
            let configured = Url::parse(configured).expect("valid configured URL");
            assert_eq!(
                normalize_request_url(Protocol::GeminiGenerateContent, &configured, true).as_str(),
                expected
            );
        }
    }

    #[test]
    fn non_stream_url_is_an_exact_clone() {
        let raw =
            "https://example.com/v1beta/models/a%2Fb:generateContent?x=a+b&x=%2F&empty=#fragment";
        let configured = Url::parse(raw).expect("valid configured URL");
        let normalized = normalize_request_url(Protocol::GeminiGenerateContent, &configured, false);

        assert_eq!(normalized, configured);
        assert_eq!(normalized.as_str(), configured.as_str());
    }

    #[test]
    fn gemini_stream_url_discards_existing_query_and_preserves_fragment() {
        let raw = "https://example.com/v1beta/models/a%2Fb:generateContent?key=a+b&path=%2F&dup=1&dup=2&empty=&flag&alt=json&%61lt=proto&action=:generateContent#section";
        let expected =
            "https://example.com/v1beta/models/a%2Fb:streamGenerateContent?alt=sse#section";
        let configured = Url::parse(raw).expect("valid configured URL");

        assert_eq!(
            configured.as_str(),
            raw,
            "the URL crate retained the raw forms under test"
        );
        assert_eq!(
            normalize_request_url(Protocol::GeminiGenerateContent, &configured, true).as_str(),
            expected
        );
    }

    #[test]
    fn stream_url_normalization_is_gemini_only_and_idempotent() {
        let raw =
            "https://example.com/v1/models/gemini:streamGenerateContent?trace=&alt=sse#fragment";
        let configured = Url::parse(raw).expect("valid configured URL");
        let once = normalize_request_url(Protocol::GeminiGenerateContent, &configured, true);
        let twice = normalize_request_url(Protocol::GeminiGenerateContent, &once, true);

        assert_eq!(
            once.as_str(),
            "https://example.com/v1/models/gemini:streamGenerateContent?alt=sse#fragment"
        );
        assert_eq!(twice, once);
        assert_eq!(
            normalize_request_url(Protocol::OpenAiChat, &configured, true),
            configured
        );

        let empty_query = Url::parse("https://example.com/v1/models/gemini:generateContent?")
            .expect("valid URL with empty query");
        assert_eq!(
            normalize_request_url(Protocol::GeminiGenerateContent, &empty_query, true).as_str(),
            "https://example.com/v1/models/gemini:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn gemini_stream_url_discards_empty_query_segments() {
        let configured =
            Url::parse("https://example.com/v1/models/gemini:generateContent?x=1&&alt=json&y=2&")
                .expect("valid URL with raw empty query segments");

        assert_eq!(
            normalize_request_url(Protocol::GeminiGenerateContent, &configured, true).as_str(),
            "https://example.com/v1/models/gemini:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn gemini_stream_url_leaves_custom_paths_unchanged() {
        for raw in [
            "https://example.com/custom/generate?alt=json&key=value",
            "https://example.com/v1/models/gemini:generateContent/?alt=json",
        ] {
            let configured = Url::parse(raw).expect("valid custom URL");
            assert_eq!(
                normalize_request_url(Protocol::GeminiGenerateContent, &configured, true),
                configured
            );
        }
    }
}
