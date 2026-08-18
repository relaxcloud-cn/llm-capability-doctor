#![allow(dead_code)] // Task 9 wires the completed parser set into the collector.

use serde_json::Value;

use crate::evidence::{StreamEndSignal, StreamTermination};

pub(crate) mod framing;
pub(crate) mod openai_chat;

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
    None,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProtocolHistory {
    OpenAiChat(Value),
    OpenAiResponses { response_id: String },
    Anthropic(Vec<Value>),
    Gemini(Value),
    Ollama(Value),
}
