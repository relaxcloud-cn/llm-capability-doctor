use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransportOutcome {
    CompletedEof,
    ProtocolTerminated,
    Timeout,
    UpstreamDisconnect,
    ClientCancelled,
    TransportError,
}

impl fmt::Display for TransportOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CompletedEof => "completed_eof",
            Self::ProtocolTerminated => "protocol_terminated",
            Self::Timeout => "timeout",
            Self::UpstreamDisconnect => "upstream_disconnect",
            Self::ClientCancelled => "client_cancelled",
            Self::TransportError => "transport_error",
        })
    }
}

impl TransportOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::CompletedEof | Self::ProtocolTerminated)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StreamTermination {
    NotApplicable,
    Completed,
    MissingTerminalEvent,
    Timeout,
    UpstreamDisconnect,
    ClientCancelled,
    TransportError,
    HttpError,
    MalformedStream,
    ProtocolError,
    ModelIncomplete,
}

impl fmt::Display for StreamTermination {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotApplicable => "not_applicable",
            Self::Completed => "completed",
            Self::MissingTerminalEvent => "missing_terminal_event",
            Self::Timeout => "timeout",
            Self::UpstreamDisconnect => "upstream_disconnect",
            Self::ClientCancelled => "client_cancelled",
            Self::TransportError => "transport_error",
            Self::HttpError => "http_error",
            Self::MalformedStream => "malformed_stream",
            Self::ProtocolError => "protocol_error",
            Self::ModelIncomplete => "model_incomplete",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StreamEndSignal {
    None,
    OpenAiDone,
    OpenAiResponseCompleted,
    AnthropicMessageStop,
    GeminiFinishReason(String),
    OllamaDone,
}

impl fmt::Display for StreamEndSignal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => formatter.write_str("none"),
            Self::OpenAiDone => formatter.write_str("[DONE]"),
            Self::OpenAiResponseCompleted => formatter.write_str("response.completed"),
            Self::AnthropicMessageStop => formatter.write_str("message_stop"),
            Self::GeminiFinishReason(reason) => write!(formatter, "finishReason:{reason}"),
            Self::OllamaDone => formatter.write_str("done:true"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolContractStatus {
    NotApplicable,
    Conformant,
    NonConformant,
}

impl fmt::Display for ToolContractStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotApplicable => "not_applicable",
            Self::Conformant => "conformant",
            Self::NonConformant => "non_conformant",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolLoopOutcome {
    NotApplicable,
    Continued,
    Completed,
    InvalidTurn,
    TransportFailure,
    MaxTurnsExceeded,
}

impl fmt::Display for ToolLoopOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotApplicable => "not_applicable",
            Self::Continued => "continued",
            Self::Completed => "completed",
            Self::InvalidTurn => "invalid_turn",
            Self::TransportFailure => "transport_failure",
            Self::MaxTurnsExceeded => "max_turns_exceeded",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        StreamEndSignal, StreamTermination, ToolContractStatus, ToolLoopOutcome, TransportOutcome,
    };

    #[test]
    fn evidence_values_render_exact_wire_names() {
        let transport = [
            (TransportOutcome::CompletedEof, "completed_eof"),
            (TransportOutcome::ProtocolTerminated, "protocol_terminated"),
            (TransportOutcome::Timeout, "timeout"),
            (TransportOutcome::UpstreamDisconnect, "upstream_disconnect"),
            (TransportOutcome::ClientCancelled, "client_cancelled"),
            (TransportOutcome::TransportError, "transport_error"),
        ];
        let termination = [
            (StreamTermination::NotApplicable, "not_applicable"),
            (StreamTermination::Completed, "completed"),
            (
                StreamTermination::MissingTerminalEvent,
                "missing_terminal_event",
            ),
            (StreamTermination::Timeout, "timeout"),
            (StreamTermination::UpstreamDisconnect, "upstream_disconnect"),
            (StreamTermination::ClientCancelled, "client_cancelled"),
            (StreamTermination::TransportError, "transport_error"),
            (StreamTermination::HttpError, "http_error"),
            (StreamTermination::MalformedStream, "malformed_stream"),
            (StreamTermination::ProtocolError, "protocol_error"),
            (StreamTermination::ModelIncomplete, "model_incomplete"),
        ];
        let signals = [
            (StreamEndSignal::None, "none"),
            (StreamEndSignal::OpenAiDone, "[DONE]"),
            (
                StreamEndSignal::OpenAiResponseCompleted,
                "response.completed",
            ),
            (StreamEndSignal::AnthropicMessageStop, "message_stop"),
            (
                StreamEndSignal::GeminiFinishReason("STOP".to_owned()),
                "finishReason:STOP",
            ),
            (StreamEndSignal::OllamaDone, "done:true"),
        ];
        let contract = [
            (ToolContractStatus::NotApplicable, "not_applicable"),
            (ToolContractStatus::Conformant, "conformant"),
            (ToolContractStatus::NonConformant, "non_conformant"),
        ];
        let loop_outcomes = [
            (ToolLoopOutcome::NotApplicable, "not_applicable"),
            (ToolLoopOutcome::Continued, "continued"),
            (ToolLoopOutcome::Completed, "completed"),
            (ToolLoopOutcome::InvalidTurn, "invalid_turn"),
            (ToolLoopOutcome::TransportFailure, "transport_failure"),
            (ToolLoopOutcome::MaxTurnsExceeded, "max_turns_exceeded"),
        ];

        for (value, expected) in transport {
            assert_eq!(value.to_string(), expected);
        }
        for (value, expected) in termination {
            assert_eq!(value.to_string(), expected);
        }
        for (value, expected) in signals {
            assert_eq!(value.to_string(), expected);
        }
        for (value, expected) in contract {
            assert_eq!(value.to_string(), expected);
        }
        for (value, expected) in loop_outcomes {
            assert_eq!(value.to_string(), expected);
        }
    }
}
