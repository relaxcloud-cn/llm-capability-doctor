#![allow(dead_code)] // Task 12 connects the completed parser dispatcher to the runner.

use serde_json::Value;

use crate::evidence::{StreamEndSignal, StreamTermination};
use crate::protocol::Protocol;

pub(crate) mod anthropic;
pub(crate) mod framing;
pub(crate) mod gemini;
pub(crate) mod inspector;
pub(crate) mod ollama;
pub(crate) mod openai_chat;
pub(crate) mod openai_responses;

pub(crate) use inspector::{StreamControl, StreamInspector, StreamState};

#[derive(Debug, PartialEq)]
pub struct StreamParseResult {
    pub assistant_turn: Option<AssistantTurn>,
    pub stream_termination: StreamTermination,
    pub stream_end_signal: StreamEndSignal,
    pub model_stop_reason: Option<String>,
    pub event_count: usize,
    pub contract_errors: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssistantTurn {
    pub history: ProtocolHistory,
    pub tool_calls: Vec<ToolCall>,
    pub final_text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub index: usize,
    pub correlation: ToolCorrelation,
    pub name: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolCorrelation {
    Required(String),
    Optional(Option<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProtocolHistory {
    OpenAiChat(Value),
    OpenAiResponses { response_id: String },
    Anthropic(Vec<Value>),
    Gemini(Value),
    Ollama(Value),
}

pub async fn parse_stream(protocol: Protocol, body: &[u8]) -> StreamParseResult {
    match protocol {
        Protocol::OpenAiChat => openai_chat::parse(body).await,
        Protocol::OpenAiResponses => openai_responses::parse(body).await,
        Protocol::AnthropicMessages => anthropic::parse(body).await,
        Protocol::GeminiGenerateContent => gemini::parse(body).await,
        Protocol::OllamaChat => ollama::parse(body).await,
        Protocol::Unknown => StreamParseResult {
            assistant_turn: None,
            stream_termination: StreamTermination::ProtocolError,
            stream_end_signal: StreamEndSignal::None,
            model_stop_reason: None,
            event_count: 0,
            contract_errors: vec!["protocol.unknown:/".into()],
        },
    }
}

#[cfg(test)]
mod tests {
    use crate::evidence::{StreamEndSignal, StreamTermination};
    use crate::protocol::Protocol;

    use super::parse_stream;

    #[tokio::test]
    async fn dispatcher_routes_all_protocols_and_rejects_unknown() {
        let sse_cases = [
            (
                Protocol::OpenAiChat,
                include_bytes!("../fixtures/openai_chat_tool.sse").as_slice(),
                StreamEndSignal::OpenAiDone,
            ),
            (
                Protocol::OpenAiResponses,
                include_bytes!("../fixtures/openai_responses_tool.sse").as_slice(),
                StreamEndSignal::OpenAiResponseCompleted,
            ),
            (
                Protocol::AnthropicMessages,
                include_bytes!("../fixtures/anthropic_tool.sse").as_slice(),
                StreamEndSignal::AnthropicMessageStop,
            ),
            (
                Protocol::GeminiGenerateContent,
                include_bytes!("../fixtures/gemini_tool.sse").as_slice(),
                StreamEndSignal::GeminiFinishReason("STOP".into()),
            ),
        ];

        for (protocol, fixture, expected_signal) in sse_cases {
            let mut body = fixture.to_vec();
            body.push(b'\n');
            let parsed = parse_stream(protocol, &body).await;
            assert_eq!(parsed.stream_termination, StreamTermination::Completed);
            assert_eq!(parsed.stream_end_signal, expected_signal);
        }

        let ollama_body = include_bytes!("../fixtures/ollama_tool.ndjson");
        let parsed = parse_stream(Protocol::OllamaChat, ollama_body).await;
        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert_eq!(parsed.stream_end_signal, StreamEndSignal::OllamaDone);

        let not_guessed = parse_stream(Protocol::OpenAiChat, ollama_body).await;
        assert_ne!(not_guessed.stream_termination, StreamTermination::Completed);

        let unknown = parse_stream(Protocol::Unknown, ollama_body).await;
        assert_eq!(unknown.stream_termination, StreamTermination::ProtocolError);
        assert_eq!(unknown.stream_end_signal, StreamEndSignal::None);
        assert_eq!(unknown.event_count, 0);
        assert!(unknown.assistant_turn.is_none());
        assert_eq!(unknown.model_stop_reason, None);
        assert_eq!(unknown.contract_errors, ["protocol.unknown:/"]);
    }
}
