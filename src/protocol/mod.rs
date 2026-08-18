use std::fmt;

use serde_json::{Value, json};
use url::Url;

pub(crate) mod stream;
pub(crate) mod tool_loop;
pub mod tools;

pub const ANTHROPIC_MAX_TOKENS: u64 = 2048;

pub fn normalize_request_url(protocol: Protocol, configured: &Url, stream: bool) -> Url {
    if protocol != Protocol::GeminiGenerateContent || !stream {
        return configured.clone();
    }

    let raw = configured.as_str();
    let fragment_start = raw.find('#').unwrap_or(raw.len());
    let before_fragment = &raw[..fragment_start];
    let fragment = &raw[fragment_start..];
    let query_start = before_fragment.find('?');
    let (base, query) = match query_start {
        Some(index) => (
            &before_fragment[..index],
            Some(&before_fragment[index + 1..]),
        ),
        None => (before_fragment, None),
    };
    let base = base.strip_suffix(":generateContent").map_or_else(
        || base.to_owned(),
        |prefix| format!("{prefix}:streamGenerateContent"),
    );

    let mut query_pairs = query
        .filter(|query| !query.is_empty())
        .into_iter()
        .flat_map(|query| query.split('&'))
        .filter(|pair| !query_key_is_alt(pair))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    query_pairs.push("alt=sse".into());

    Url::parse(&format!("{base}?{}{fragment}", query_pairs.join("&")))
        .expect("normalizing a parsed Gemini URL preserves URL validity")
}

fn query_key_is_alt(pair: &str) -> bool {
    url::form_urlencoded::parse(pair.as_bytes())
        .next()
        .is_some_and(|(key, _)| key == "alt")
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

#[cfg(test)]
mod tests {
    use url::Url;

    use super::{Protocol, normalize_request_url};

    #[test]
    fn gemini_stream_url_is_normalized() {
        let cases = [
            (
                "https://example.com/v1beta/models/gemini:generateContent?key=x&trace=1",
                "https://example.com/v1beta/models/gemini:streamGenerateContent?key=x&trace=1&alt=sse",
            ),
            (
                "https://example.com/v1beta/models/gemini:streamGenerateContent?alt=json&trace=1",
                "https://example.com/v1beta/models/gemini:streamGenerateContent?trace=1&alt=sse",
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
    fn gemini_stream_url_preserves_raw_unrelated_query_and_fragment() {
        let raw = "https://example.com/v1beta/models/a%2Fb:generateContent?key=a+b&path=%2F&dup=1&dup=2&empty=&flag&alt=json&%61lt=proto&action=:generateContent#section";
        let expected = "https://example.com/v1beta/models/a%2Fb:streamGenerateContent?key=a+b&path=%2F&dup=1&dup=2&empty=&flag&action=:generateContent&alt=sse#section";
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

        assert_eq!(once.as_str(), raw);
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
    fn gemini_stream_url_preserves_empty_segments_inside_a_nonempty_query() {
        let configured =
            Url::parse("https://example.com/v1/models/gemini:generateContent?x=1&&alt=json&y=2&")
                .expect("valid URL with raw empty query segments");

        assert_eq!(
            normalize_request_url(Protocol::GeminiGenerateContent, &configured, true).as_str(),
            "https://example.com/v1/models/gemini:streamGenerateContent?x=1&&y=2&&alt=sse"
        );
    }
}
