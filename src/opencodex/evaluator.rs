use crate::evidence::StreamTermination;
use crate::opencodex::contract::{Adapter, Rule, rule};
use crate::protocol::stream::StreamParseResult;

pub struct RuleFailure {
    pub rule_id: &'static str,
    pub requirement: &'static str,
    pub observed_path: String,
    pub actual: String,
    pub effect: &'static str,
}

pub struct AdapterResult {
    pub adapter: Adapter,
    pub passed: bool,
    pub failures: Vec<RuleFailure>,
}

pub fn evaluate_stream(adapter: Adapter, parsed: &StreamParseResult) -> AdapterResult {
    let mut failures = Vec::new();
    for error in &parsed.contract_errors {
        push_failure(&mut failures, failure_for_contract_error(adapter, error));
    }

    let needs_terminal_failure =
        !matches!(parsed.stream_termination, StreamTermination::ProtocolError)
            || failures.is_empty();
    if needs_terminal_failure
        && let Some(rule_id) = terminal_rule(adapter, &parsed.stream_termination)
    {
        push_failure(
            &mut failures,
            failure(
                rule(rule_id),
                "stream_termination",
                parsed.stream_termination.to_string(),
            ),
        );
    }

    AdapterResult {
        adapter,
        passed: failures.is_empty(),
        failures,
    }
}

pub fn evaluate_http_failure(adapter: Adapter, status: Option<u16>, detail: &str) -> AdapterResult {
    let actual = status.map_or_else(|| detail.to_owned(), |status| status.to_string());
    AdapterResult {
        adapter,
        passed: false,
        failures: vec![failure(rule(shape_rule(adapter)), "http_status", actual)],
    }
}

fn terminal_rule(adapter: Adapter, termination: &StreamTermination) -> Option<&'static str> {
    match termination {
        StreamTermination::Completed | StreamTermination::NotApplicable => None,
        StreamTermination::MissingTerminalEvent => Some(match adapter {
            Adapter::OpenAiChat => "OCX-CHAT-STREAM-006",
            Adapter::Anthropic => "OCX-ANTH-STREAM-006",
            Adapter::Google => "OCX-GOOGLE-STREAM-006",
        }),
        StreamTermination::MalformedStream => Some(match adapter {
            Adapter::OpenAiChat => "OCX-CHAT-STREAM-001",
            Adapter::Anthropic => "OCX-ANTH-STREAM-001",
            Adapter::Google => "OCX-GOOGLE-STREAM-001",
        }),
        _ => Some(shape_rule(adapter)),
    }
}

fn failure_for_contract_error(adapter: Adapter, error: &str) -> RuleFailure {
    let rule_id = if error.contains("duplicate_tool") || error.contains("conflicting_tool") {
        match adapter {
            Adapter::OpenAiChat => "OCX-CHAT-TOOL-003",
            Adapter::Anthropic => "OCX-ANTH-TOOL-005",
            Adapter::Google => "OCX-GOOGLE-TOOL-004",
        }
    } else if error.contains("function_args") || error.contains("arguments") {
        match adapter {
            Adapter::OpenAiChat => "OCX-CHAT-TOOL-004",
            Adapter::Anthropic => "OCX-ANTH-TOOL-005",
            Adapter::Google => "OCX-GOOGLE-TOOL-004",
        }
    } else if error.contains("tool") {
        match adapter {
            Adapter::OpenAiChat => "OCX-CHAT-TOOL-004",
            Adapter::Anthropic => "OCX-ANTH-TOOL-005",
            Adapter::Google => "OCX-GOOGLE-TOOL-004",
        }
    } else if error.contains("invalid_sse") || error.contains("malformed") {
        match adapter {
            Adapter::OpenAiChat => "OCX-CHAT-STREAM-001",
            Adapter::Anthropic => "OCX-ANTH-STREAM-001",
            Adapter::Google => "OCX-GOOGLE-STREAM-001",
        }
    } else {
        shape_rule(adapter)
    };
    let path = error
        .split_once(':')
        .map_or("response", |(_, path)| path)
        .to_owned();
    failure(rule(rule_id), &path, error.to_owned())
}

fn shape_rule(adapter: Adapter) -> &'static str {
    match adapter {
        Adapter::OpenAiChat => "OCX-CHAT-SHAPE-001",
        Adapter::Anthropic => "OCX-ANTH-SHAPE-001",
        Adapter::Google => "OCX-GOOGLE-SHAPE-001",
    }
}

fn failure(rule: &'static Rule, observed_path: &str, actual: String) -> RuleFailure {
    RuleFailure {
        rule_id: rule.id,
        requirement: rule.requirement,
        observed_path: observed_path.to_owned(),
        actual,
        effect: rule.effect,
    }
}

fn push_failure(failures: &mut Vec<RuleFailure>, failure: RuleFailure) {
    if failures.iter().all(|existing| {
        existing.rule_id != failure.rule_id || existing.observed_path != failure.observed_path
    }) {
        failures.push(failure);
    }
}

#[cfg(test)]
mod tests {
    use crate::evidence::{StreamEndSignal, StreamTermination};
    use crate::opencodex::contract::Adapter;
    use crate::protocol::stream::StreamParseResult;

    use super::evaluate_stream;

    #[test]
    fn chat_missing_done_fails_the_native_terminal_rule() {
        let result = evaluate_stream(
            Adapter::OpenAiChat,
            &StreamParseResult {
                assistant_turn: None,
                stream_termination: StreamTermination::MissingTerminalEvent,
                stream_end_signal: StreamEndSignal::None,
                model_stop_reason: None,
                event_count: 1,
                contract_errors: Vec::new(),
            },
        );

        assert_eq!(result.failures[0].rule_id, "OCX-CHAT-STREAM-006");
        assert_eq!(result.failures[0].observed_path, "stream_termination");
    }

    #[test]
    fn anthropic_duplicate_tool_id_fails_the_tool_correlation_rule() {
        let result = evaluate_stream(
            Adapter::Anthropic,
            &StreamParseResult {
                assistant_turn: None,
                stream_termination: StreamTermination::ProtocolError,
                stream_end_signal: StreamEndSignal::None,
                model_stop_reason: None,
                event_count: 2,
                contract_errors: vec!["anthropic.duplicate_tool_use_id:/content_block".into()],
            },
        );

        assert!(
            result
                .failures
                .iter()
                .any(|failure| failure.rule_id == "OCX-ANTH-TOOL-005")
        );
        assert_eq!(result.failures.len(), 1);
    }

    #[test]
    fn google_invalid_function_arguments_fails_the_native_argument_rule() {
        let result = evaluate_stream(
            Adapter::Google,
            &StreamParseResult {
                assistant_turn: None,
                stream_termination: StreamTermination::ProtocolError,
                stream_end_signal: StreamEndSignal::None,
                model_stop_reason: None,
                event_count: 1,
                contract_errors: vec![
                    "gemini.invalid_function_args:/candidates/0/content/parts/0".into(),
                ],
            },
        );

        assert!(
            result
                .failures
                .iter()
                .any(|failure| failure.rule_id == "OCX-GOOGLE-TOOL-004")
        );
    }
}
