use std::collections::BTreeSet;

use serde_json::{Map, Value};

use crate::evidence::{StreamEndSignal, StreamTermination};

use super::framing::decode_sse_chunks;
use super::{AssistantTurn, ProtocolHistory, StreamParseResult, ToolCall, ToolCorrelation};

#[derive(Default)]
struct ParserState {
    role: Option<String>,
    parts: Vec<Value>,
    tool_calls: Vec<ToolCall>,
    seen_function_ids: BTreeSet<String>,
    duplicate_function_id: bool,
    final_text: String,
    finish_reason: Option<String>,
    saw_content: bool,
    protocol_error: bool,
    contract_errors: Vec<String>,
}

pub(crate) async fn parse(body: &[u8]) -> StreamParseResult {
    let decoded = match decode_sse_chunks(vec![body.to_vec()]).await {
        Ok(decoded) => decoded,
        Err(_) => return malformed_result("invalid_sse", "/"),
    };
    let event_count = decoded.events.len();
    let mut state = ParserState::default();
    let mut malformed_stream = false;

    for (event_index, event) in decoded.events.iter().enumerate() {
        let sealed_before_event = state.protocol_error || state.finish_reason.is_some();
        if sealed_before_event {
            push_error(
                &mut state.contract_errors,
                "event_after_terminal",
                &format!("/events/{event_index}"),
            );
            state.protocol_error = true;
        }

        let named_event = !matches!(event.event.as_str(), "" | "message");
        if named_event {
            push_error(
                &mut state.contract_errors,
                "named_event",
                &format!("/events/{event_index}/event"),
            );
            state.protocol_error = true;
        }
        if event.data == "[DONE]" {
            push_error(
                &mut state.contract_errors,
                "unexpected_done",
                &format!("/events/{event_index}/data"),
            );
            state.protocol_error = true;
            continue;
        }

        let value = match serde_json::from_str::<Value>(&event.data) {
            Ok(value) => value,
            Err(_) => {
                malformed_stream = true;
                push_error(
                    &mut state.contract_errors,
                    "malformed_event_json",
                    &format!("/events/{event_index}/data"),
                );
                continue;
            }
        };
        let Some(response) = value.as_object() else {
            push_error(
                &mut state.contract_errors,
                "invalid_response",
                &format!("/events/{event_index}/data"),
            );
            continue;
        };
        if response.contains_key("error") {
            push_error(
                &mut state.contract_errors,
                "api_error",
                &format!("/events/{event_index}/data/error"),
            );
            state.protocol_error = true;
            continue;
        }
        if named_event {
            continue;
        }
        if sealed_before_event {
            if state.finish_reason.is_none() {
                observe_finish_reason(&mut state, response, event_index);
            }
            continue;
        }

        process_response(&mut state, response, event_index);
    }

    if decoded.trailing_incomplete_frame {
        malformed_stream = true;
        push_error(
            &mut state.contract_errors,
            "truncated_sse",
            &format!("/events/{event_count}"),
        );
    }

    let stream_termination = if malformed_stream {
        StreamTermination::MalformedStream
    } else if state.protocol_error {
        StreamTermination::ProtocolError
    } else {
        match state.finish_reason.as_deref() {
            None => StreamTermination::MissingTerminalEvent,
            Some("STOP") => StreamTermination::Completed,
            Some(_) => StreamTermination::ModelIncomplete,
        }
    };
    let assistant_turn = build_assistant_turn(&state);
    let stream_end_signal = state
        .finish_reason
        .as_ref()
        .map_or(StreamEndSignal::None, |reason| {
            StreamEndSignal::GeminiFinishReason(reason.clone())
        });

    StreamParseResult {
        assistant_turn,
        stream_termination,
        stream_end_signal,
        model_stop_reason: state.finish_reason,
        event_count,
        contract_errors: state.contract_errors,
    }
}

fn malformed_result(code: &str, path: &str) -> StreamParseResult {
    StreamParseResult {
        assistant_turn: None,
        stream_termination: StreamTermination::MalformedStream,
        stream_end_signal: StreamEndSignal::None,
        model_stop_reason: None,
        event_count: 0,
        contract_errors: vec![contract_error(code, path)],
    }
}

fn process_response(state: &mut ParserState, response: &Map<String, Value>, event_index: usize) {
    let Some(candidates_value) = response.get("candidates") else {
        return;
    };
    let Some(candidates) = candidates_value.as_array() else {
        push_error(
            &mut state.contract_errors,
            "invalid_candidates",
            &format!("/events/{event_index}/data/candidates"),
        );
        return;
    };
    let Some(candidate_position) =
        selected_candidate_position(candidates, event_index, &mut state.contract_errors)
    else {
        return;
    };
    let Some(candidate) = candidates[candidate_position].as_object() else {
        return;
    };

    if let Some(content) = candidate.get("content") {
        process_content(state, content, event_index, candidate_position);
    }

    process_finish_reason(state, candidate, event_index, candidate_position);
}

fn observe_finish_reason(
    state: &mut ParserState,
    response: &Map<String, Value>,
    event_index: usize,
) {
    let Some(candidates) = response.get("candidates").and_then(Value::as_array) else {
        return;
    };
    let Some(candidate_position) =
        selected_candidate_position(candidates, event_index, &mut state.contract_errors)
    else {
        return;
    };
    let Some(candidate) = candidates[candidate_position].as_object() else {
        return;
    };
    process_finish_reason(state, candidate, event_index, candidate_position);
}

fn process_finish_reason(
    state: &mut ParserState,
    candidate: &Map<String, Value>,
    event_index: usize,
    candidate_position: usize,
) {
    let Some(reason) = candidate.get("finishReason") else {
        return;
    };
    let path = format!("/events/{event_index}/data/candidates/{candidate_position}/finishReason");
    match reason.as_str() {
        Some(reason) if !reason.is_empty() => state.finish_reason = Some(reason.to_owned()),
        _ => push_error(&mut state.contract_errors, "invalid_finish_reason", &path),
    }
}

fn selected_candidate_position(
    candidates: &[Value],
    event_index: usize,
    errors: &mut Vec<String>,
) -> Option<usize> {
    let base = format!("/events/{event_index}/data/candidates");
    if candidates.is_empty() {
        return None;
    }

    let mut seen = BTreeSet::new();
    let mut zero_positions = Vec::new();
    let mut unindexed_positions = Vec::new();
    let mut invalid_positions = BTreeSet::new();

    for (position, candidate) in candidates.iter().enumerate() {
        let Some(candidate) = candidate.as_object() else {
            push_error(errors, "invalid_candidate", &format!("{base}/{position}"));
            invalid_positions.insert(position);
            continue;
        };
        match candidate.get("index") {
            None => unindexed_positions.push(position),
            Some(value) => match value.as_u64() {
                Some(index) => {
                    if !seen.insert(index) {
                        push_error(
                            errors,
                            "duplicate_candidate_index",
                            &format!("{base}/{position}/index"),
                        );
                    }
                    if index == 0 {
                        zero_positions.push(position);
                    }
                }
                None => {
                    push_error(
                        errors,
                        "invalid_candidate_index",
                        &format!("{base}/{position}/index"),
                    );
                    invalid_positions.insert(position);
                }
            },
        }
    }

    if candidates.len() == 1 {
        if invalid_positions.contains(&0) {
            return None;
        }
        if unindexed_positions == [0] || zero_positions == [0] {
            return Some(0);
        }
        push_error(errors, "missing_candidate_zero", &base);
        return None;
    }

    if !unindexed_positions.is_empty() {
        push_error(errors, "ambiguous_candidate", &base);
        return None;
    }
    if zero_positions.len() == 1 {
        return zero_positions.first().copied();
    }
    if zero_positions.is_empty() {
        push_error(errors, "missing_candidate_zero", &base);
    }
    None
}

fn process_content(
    state: &mut ParserState,
    content: &Value,
    event_index: usize,
    candidate_position: usize,
) {
    let base = format!("/events/{event_index}/data/candidates/{candidate_position}/content");
    let Some(content) = content.as_object() else {
        push_error(&mut state.contract_errors, "invalid_content", &base);
        return;
    };
    state.saw_content = true;

    if let Some(role) = content.get("role") {
        match role.as_str() {
            Some("model") => state.role = Some("model".into()),
            _ => push_error(
                &mut state.contract_errors,
                "invalid_content_role",
                &format!("{base}/role"),
            ),
        }
    }

    let Some(parts) = content.get("parts").and_then(Value::as_array) else {
        push_error(
            &mut state.contract_errors,
            "invalid_parts",
            &format!("{base}/parts"),
        );
        return;
    };
    for (part_position, part) in parts.iter().enumerate() {
        process_part(state, part, event_index, candidate_position, part_position);
    }
}

fn process_part(
    state: &mut ParserState,
    part: &Value,
    event_index: usize,
    candidate_position: usize,
    part_position: usize,
) {
    let base = format!(
        "/events/{event_index}/data/candidates/{candidate_position}/content/parts/{part_position}"
    );
    let Some(raw_part) = part.as_object() else {
        push_error(&mut state.contract_errors, "invalid_part", &base);
        return;
    };
    let global_part_index = state.parts.len();
    let mut native = raw_part.clone();

    let known_union_fields = [
        "text",
        "inlineData",
        "functionCall",
        "functionResponse",
        "fileData",
        "executableCode",
        "codeExecutionResult",
    ];
    let present_union_fields = known_union_fields
        .iter()
        .filter(|field| raw_part.contains_key(**field))
        .copied()
        .collect::<Vec<_>>();
    let ambiguous = present_union_fields.len() > 1;
    if ambiguous {
        push_error(&mut state.contract_errors, "ambiguous_part", &base);
    }

    let (thought, thought_is_valid) = match raw_part.get("thought") {
        None => (None, true),
        Some(Value::Bool(value)) => (Some(*value), true),
        Some(_) => {
            push_error(
                &mut state.contract_errors,
                "invalid_thought",
                &format!("{base}/thought"),
            );
            native.remove("thought");
            (None, false)
        }
    };
    if raw_part
        .get("thoughtSignature")
        .is_some_and(|value| !value.is_string())
    {
        push_error(
            &mut state.contract_errors,
            "invalid_thought_signature",
            &format!("{base}/thoughtSignature"),
        );
        native.remove("thoughtSignature");
    }

    if !ambiguous {
        match present_union_fields.as_slice() {
            ["text"] => match raw_part.get("text").and_then(Value::as_str) {
                Some(text) if thought_is_valid && thought != Some(true) => {
                    state.final_text.push_str(text);
                }
                Some(_) => {}
                None => {
                    push_error(
                        &mut state.contract_errors,
                        "invalid_text",
                        &format!("{base}/text"),
                    );
                    native.remove("text");
                }
            },
            ["functionCall"] => {
                if let Some(call) = normalized_function_call(
                    raw_part.get("functionCall"),
                    global_part_index,
                    &base,
                    &mut state.contract_errors,
                ) {
                    let duplicate_id = match &call.correlation {
                        ToolCorrelation::Optional(Some(id)) => {
                            !state.seen_function_ids.insert(id.clone())
                        }
                        ToolCorrelation::Optional(None) => false,
                        ToolCorrelation::Required(_) | ToolCorrelation::None => {
                            unreachable!("Gemini calls always use optional correlation")
                        }
                    };
                    if duplicate_id {
                        push_error(
                            &mut state.contract_errors,
                            "duplicate_function_id",
                            &format!("{base}/functionCall/id"),
                        );
                        state.duplicate_function_id = true;
                        state.tool_calls.clear();
                    } else if !state.duplicate_function_id {
                        state.tool_calls.push(call);
                    }
                } else {
                    native.remove("functionCall");
                }
            }
            [] => {}
            [_] => {}
            _ => unreachable!("ambiguous parts were handled above"),
        }
    }

    state.parts.push(Value::Object(native));
}

fn normalized_function_call(
    value: Option<&Value>,
    index: usize,
    part_base: &str,
    errors: &mut Vec<String>,
) -> Option<ToolCall> {
    let path = format!("{part_base}/functionCall");
    let Some(function_call) = value.and_then(Value::as_object) else {
        push_error(errors, "invalid_function_call", &path);
        return None;
    };

    let name = match function_call.get("name").and_then(Value::as_str) {
        Some(name) if !name.trim().is_empty() => Some(name.to_owned()),
        _ => {
            push_error(errors, "invalid_function_name", &format!("{path}/name"));
            None
        }
    };
    let arguments = match function_call.get("args").and_then(Value::as_object) {
        Some(arguments) => Some(Value::Object(arguments.clone())),
        None => {
            push_error(errors, "invalid_function_args", &format!("{path}/args"));
            None
        }
    };
    let correlation = match function_call.get("id") {
        None => Some(ToolCorrelation::Optional(None)),
        Some(Value::String(id)) if !id.trim().is_empty() => {
            Some(ToolCorrelation::Optional(Some(id.clone())))
        }
        Some(_) => {
            push_error(errors, "invalid_function_id", &format!("{path}/id"));
            None
        }
    };

    Some(ToolCall {
        index,
        correlation: correlation?,
        name: name?,
        arguments: arguments?,
    })
}

fn build_assistant_turn(state: &ParserState) -> Option<AssistantTurn> {
    if !state.saw_content {
        return None;
    }
    let mut content = Map::new();
    if let Some(role) = &state.role {
        content.insert("role".into(), Value::String(role.clone()));
    }
    content.insert("parts".into(), Value::Array(state.parts.clone()));

    Some(AssistantTurn {
        history: ProtocolHistory::Gemini(Value::Object(content)),
        tool_calls: state.tool_calls.clone(),
        final_text: state.final_text.clone(),
    })
}

fn contract_error(code: &str, path: &str) -> String {
    debug_assert!(path.starts_with('/'));
    format!("gemini.{code}:{path}")
}

fn push_error(errors: &mut Vec<String>, code: &str, path: &str) {
    let error = contract_error(code, path);
    if !errors.contains(&error) {
        errors.push(error);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::evidence::{StreamEndSignal, StreamTermination};
    use crate::protocol::stream::{AssistantTurn, ProtocolHistory, ToolCall, ToolCorrelation};

    use super::parse;

    const TOOL_STREAM: &[u8] = include_bytes!("../fixtures/gemini_tool.sse");
    const TOOL_WITHOUT_ID: &[u8] = include_bytes!("../fixtures/gemini_tool_without_id.sse");
    const FINAL_STREAM: &[u8] = include_bytes!("../fixtures/gemini_final.sse");
    const MISSING_TERMINAL: &[u8] = include_bytes!("../fixtures/gemini_missing_terminal.sse");
    const TRUNCATED: &[u8] = include_bytes!("../fixtures/gemini_truncated.sse");
    const API_ERROR: &[u8] = include_bytes!("../fixtures/gemini_error.sse");
    const INCOMPLETE: &[u8] = include_bytes!("../fixtures/gemini_incomplete.sse");

    #[tokio::test]
    async fn parses_structured_call_and_preserves_ordered_parts_and_signatures() {
        let parsed = parse(&complete_fixture(TOOL_STREAM)).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert_eq!(
            parsed.stream_end_signal,
            StreamEndSignal::GeminiFinishReason("STOP".into())
        );
        assert_eq!(parsed.model_stop_reason.as_deref(), Some("STOP"));
        assert_eq!(parsed.event_count, 3);
        assert!(parsed.contract_errors.is_empty());
        assert_eq!(
            parsed.assistant_turn,
            Some(AssistantTurn {
                history: ProtocolHistory::Gemini(json!({
                    "role": "model",
                    "parts": [
                        {"text": "private reasoning", "thought": true, "thoughtSignature": "sig-thought"},
                        {"functionCall": {"id": "call_weather_046", "name": "get_weather", "args": {"city": "Beijing"}}, "thoughtSignature": "sig-call"},
                        {"text": "", "thoughtSignature": "sig-empty"}
                    ]
                })),
                tool_calls: vec![ToolCall {
                    index: 1,
                    correlation: ToolCorrelation::Optional(Some("call_weather_046".into())),
                    name: "get_weather".into(),
                    arguments: json!({"city": "Beijing"}),
                }],
                final_text: String::new(),
            })
        );
    }

    #[tokio::test]
    async fn accepts_optional_call_id_and_optional_content_role_without_inventing_either() {
        let parsed = parse(&complete_fixture(TOOL_WITHOUT_ID)).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert!(parsed.contract_errors.is_empty());
        assert_eq!(
            parsed.assistant_turn,
            Some(AssistantTurn {
                history: ProtocolHistory::Gemini(json!({
                    "parts": [{
                        "functionCall": {"name": "get_time", "args": {"zone": "UTC"}},
                        "thoughtSignature": "sig-no-id"
                    }]
                })),
                tool_calls: vec![ToolCall {
                    index: 0,
                    correlation: ToolCorrelation::Optional(None),
                    name: "get_time".into(),
                    arguments: json!({"zone": "UTC"}),
                }],
                final_text: String::new(),
            })
        );
    }

    #[tokio::test]
    async fn duplicate_function_ids_in_one_turn_make_every_call_non_executable() {
        let duplicate_id = "PRIVATE_DUPLICATE_ID";
        let same_frame = sse(&[json!({"candidates": [{
            "index": 0,
            "content": {"parts": [
                {"functionCall": {"id": duplicate_id, "name": "get_weather", "args": {"city": "Beijing"}}},
                {"functionCall": {"id": duplicate_id, "name": "get_time", "args": {"zone": "UTC"}}}
            ]},
            "finishReason": "STOP"
        }]})]);
        let across_frames = sse(&[
            json!({"candidates": [{
                "index": 0,
                "content": {"parts": [{"functionCall": {
                    "id": duplicate_id,
                    "name": "get_weather",
                    "args": {"city": "Beijing"}
                }}]}
            }]}),
            json!({"candidates": [{
                "index": 0,
                "content": {"parts": [{"functionCall": {
                    "id": duplicate_id,
                    "name": "get_time",
                    "args": {"zone": "UTC"}
                }}]},
                "finishReason": "STOP"
            }]}),
        ]);

        for (body, expected_error) in [
            (
                same_frame,
                "gemini.duplicate_function_id:/events/0/data/candidates/0/content/parts/1/functionCall/id",
            ),
            (
                across_frames,
                "gemini.duplicate_function_id:/events/1/data/candidates/0/content/parts/0/functionCall/id",
            ),
        ] {
            let parsed = parse(body.as_bytes()).await;
            assert_eq!(parsed.stream_termination, StreamTermination::Completed);
            assert!(parsed.contract_errors.contains(&expected_error.to_owned()));
            assert!(
                parsed
                    .assistant_turn
                    .expect("native history")
                    .tool_calls
                    .is_empty()
            );
            assert!(!parsed.contract_errors.join(" ").contains(duplicate_id));
        }
    }

    #[tokio::test]
    async fn multiple_calls_without_ids_do_not_conflict() {
        let body = sse(&[json!({"candidates": [{
            "index": 0,
            "content": {"parts": [
                {"functionCall": {"name": "get_weather", "args": {"city": "Beijing"}}},
                {"functionCall": {"name": "get_time", "args": {"zone": "UTC"}}}
            ]},
            "finishReason": "STOP"
        }]})]);
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert!(parsed.contract_errors.is_empty());
        assert_eq!(
            parsed
                .assistant_turn
                .expect("assistant turn")
                .tool_calls
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn reconstructs_final_text_without_exposing_thought_text() {
        let parsed = parse(&complete_fixture(FINAL_STREAM)).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert_eq!(parsed.model_stop_reason.as_deref(), Some("STOP"));
        assert!(parsed.contract_errors.is_empty());
        let turn = parsed.assistant_turn.expect("assistant turn");
        assert_eq!(turn.final_text, "MODEL_DOCTOR_CASE_046_OK");
        assert_eq!(
            turn.history,
            ProtocolHistory::Gemini(json!({
                "role": "model",
                "parts": [
                    {"text": "hidden", "thought": true, "thoughtSignature": "sig-hidden"},
                    {"text": "MODEL_DOCTOR_"},
                    {"text": "CASE_046_OK"},
                    {"text": "", "thoughtSignature": "sig-final-empty"}
                ]
            }))
        );
    }

    #[tokio::test]
    async fn distinguishes_missing_incomplete_truncated_and_api_error() {
        let missing = parse(&complete_fixture(MISSING_TERMINAL)).await;
        assert_eq!(
            missing.stream_termination,
            StreamTermination::MissingTerminalEvent
        );
        assert_eq!(missing.stream_end_signal, StreamEndSignal::None);
        assert_eq!(missing.event_count, 1);

        let incomplete = parse(&complete_fixture(INCOMPLETE)).await;
        assert_eq!(
            incomplete.stream_termination,
            StreamTermination::ModelIncomplete
        );
        assert_eq!(
            incomplete.stream_end_signal,
            StreamEndSignal::GeminiFinishReason("MAX_TOKENS".into())
        );
        assert_eq!(incomplete.model_stop_reason.as_deref(), Some("MAX_TOKENS"));

        let truncated = parse(TRUNCATED).await;
        assert_eq!(
            truncated.stream_termination,
            StreamTermination::MalformedStream
        );
        assert_eq!(truncated.stream_end_signal, StreamEndSignal::None);
        assert_eq!(truncated.event_count, 1);
        assert_eq!(
            truncated.contract_errors,
            ["gemini.truncated_sse:/events/1"]
        );

        let error = parse(&complete_fixture(API_ERROR)).await;
        assert_eq!(error.stream_termination, StreamTermination::ProtocolError);
        assert_eq!(error.stream_end_signal, StreamEndSignal::None);
        assert_eq!(
            error.contract_errors,
            ["gemini.api_error:/events/0/data/error"]
        );
        assert!(!format!("{error:?}").contains("sensitive upstream detail"));
    }

    #[tokio::test]
    async fn accepts_prompt_feedback_only_as_a_legal_nonterminal_envelope() {
        let body = sse(&[json!({"promptFeedback": {"blockReason": "SAFETY"}})]);
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(
            parsed.stream_termination,
            StreamTermination::MissingTerminalEvent
        );
        assert_eq!(parsed.stream_end_signal, StreamEndSignal::None);
        assert_eq!(parsed.event_count, 1);
        assert!(parsed.contract_errors.is_empty());
        assert!(parsed.assistant_turn.is_none());
    }

    #[tokio::test]
    async fn rejects_non_object_or_missing_function_args_without_inventing_arguments() {
        for (function_call, expected) in [
            (
                json!({"name": "get_weather"}),
                "gemini.invalid_function_args:/events/0/data/candidates/0/content/parts/0/functionCall/args",
            ),
            (
                json!({"name": "get_weather", "args": "PRIVATE_ARG"}),
                "gemini.invalid_function_args:/events/0/data/candidates/0/content/parts/0/functionCall/args",
            ),
            (
                json!({"name": "", "args": {"city": "Beijing"}}),
                "gemini.invalid_function_name:/events/0/data/candidates/0/content/parts/0/functionCall/name",
            ),
        ] {
            let body = response_stream(
                json!({"parts": [{"functionCall": function_call}]}),
                Some("STOP"),
            );
            let parsed = parse(body.as_bytes()).await;

            assert_eq!(parsed.stream_termination, StreamTermination::Completed);
            assert!(parsed.contract_errors.contains(&expected.to_owned()));
            assert!(
                parsed
                    .assistant_turn
                    .as_ref()
                    .expect("history")
                    .tool_calls
                    .is_empty()
            );
            assert!(!format!("{parsed:?}").contains("PRIVATE_ARG"));
        }
    }

    #[tokio::test]
    async fn rejects_known_part_union_conflicts_but_ignores_extension_fields() {
        let body = response_stream(
            json!({"parts": [{
                "text": "ambiguous",
                "functionCall": {"name": "get_weather", "args": {"city": "Beijing"}},
                "futureField": true
            }]}),
            Some("STOP"),
        );
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert!(parsed.contract_errors.contains(
            &"gemini.ambiguous_part:/events/0/data/candidates/0/content/parts/0".to_owned()
        ));
        let turn = parsed.assistant_turn.expect("history");
        assert!(turn.tool_calls.is_empty());
        assert!(turn.final_text.is_empty());

        let extension = response_stream(
            json!({"parts": [{"text": "OK", "futureField": {"nested": true}}]}),
            Some("STOP"),
        );
        let parsed = parse(extension.as_bytes()).await;
        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert!(parsed.contract_errors.is_empty());
        assert_eq!(
            parsed.assistant_turn.as_ref().expect("turn").final_text,
            "OK"
        );
    }

    #[tokio::test]
    async fn selects_candidate_zero_across_frames_and_never_mixes_other_candidates() {
        let body = sse(&[
            json!({"candidates": [
                {"index": 7, "content": {"parts": [{"text": "IGNORE"}]}},
                {"index": 0, "content": {"parts": [{"text": "RIGHT"}]}}
            ]}),
            json!({"candidates": [
                {"index": 7, "finishReason": "STOP"},
                {"index": 0, "content": {"parts": [{"text": "_ANSWER"}]}}
            ]}),
            json!({"candidates": [{"index": 0, "finishReason": "STOP"}]}),
        ]);
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert!(parsed.contract_errors.is_empty());
        assert_eq!(
            parsed.assistant_turn.expect("selected turn").final_text,
            "RIGHT_ANSWER"
        );
    }

    #[tokio::test]
    async fn accepts_one_unindexed_candidate_but_rejects_ambiguous_or_invalid_selection() {
        let one = response_stream(json!({"parts": [{"text": "OK"}]}), Some("STOP"));
        let parsed = parse(one.as_bytes()).await;
        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert!(parsed.contract_errors.is_empty());

        let cases = [
            (
                json!({"candidates": [
                    {"content": {"parts": [{"text": "A"}]}},
                    {"content": {"parts": [{"text": "B"}]}, "finishReason": "STOP"}
                ]}),
                "gemini.ambiguous_candidate:/events/0/data/candidates",
            ),
            (
                json!({"candidates": [
                    {"index": 0, "content": {"parts": [{"text": "A"}]}},
                    {"index": 0, "finishReason": "STOP"}
                ]}),
                "gemini.duplicate_candidate_index:/events/0/data/candidates/1/index",
            ),
            (
                json!({"candidates": [{"index": 9, "finishReason": "STOP"}]}),
                "gemini.missing_candidate_zero:/events/0/data/candidates",
            ),
            (
                json!({"candidates": [{"index": -1, "finishReason": "STOP"}]}),
                "gemini.invalid_candidate_index:/events/0/data/candidates/0/index",
            ),
        ];

        for (response, expected) in cases {
            let parsed = parse(sse(&[response]).as_bytes()).await;
            assert_eq!(
                parsed.stream_termination,
                StreamTermination::MissingTerminalEvent
            );
            assert!(parsed.contract_errors.contains(&expected.to_owned()));
        }
    }

    #[tokio::test]
    async fn rejects_named_events_done_and_malformed_json() {
        let named = parse(
            b"event: response\ndata: {\"candidates\":[{\"index\":0,\"finishReason\":\"STOP\"}]}\n\n",
        )
        .await;
        assert_eq!(named.stream_termination, StreamTermination::ProtocolError);
        assert_eq!(named.stream_end_signal, StreamEndSignal::None);
        assert_eq!(
            named.contract_errors,
            ["gemini.named_event:/events/0/event"]
        );

        let done = parse(b"data: [DONE]\n\n").await;
        assert_eq!(done.stream_termination, StreamTermination::ProtocolError);
        assert_eq!(
            done.contract_errors,
            ["gemini.unexpected_done:/events/0/data"]
        );

        let malformed = parse(b"data: {PRIVATE\n\n").await;
        assert_eq!(
            malformed.stream_termination,
            StreamTermination::MalformedStream
        );
        assert_eq!(
            malformed.contract_errors,
            ["gemini.malformed_event_json:/events/0/data"]
        );
        assert!(!format!("{malformed:?}").contains("PRIVATE"));
    }

    #[tokio::test]
    async fn events_after_finish_cannot_hide_errors_or_malformed_frames() {
        let stop = json!({"candidates": [{"index": 0, "finishReason": "STOP"}]});
        let extra = parse(sse(&[stop.clone(), json!({"promptFeedback": {}})]).as_bytes()).await;
        assert_eq!(extra.stream_termination, StreamTermination::ProtocolError);
        assert_eq!(
            extra.stream_end_signal,
            StreamEndSignal::GeminiFinishReason("STOP".into())
        );
        assert_eq!(
            extra.contract_errors,
            ["gemini.event_after_terminal:/events/1"]
        );

        let api_error = parse(
            format!("data: {stop}\n\ndata: {{\"error\":{{\"message\":\"PRIVATE\"}}}}\n\n")
                .as_bytes(),
        )
        .await;
        assert_eq!(
            api_error.stream_termination,
            StreamTermination::ProtocolError
        );
        assert!(
            api_error
                .contract_errors
                .contains(&"gemini.event_after_terminal:/events/1".to_owned())
        );
        assert!(
            api_error
                .contract_errors
                .contains(&"gemini.api_error:/events/1/data/error".to_owned())
        );
        assert!(!format!("{api_error:?}").contains("PRIVATE"));

        let malformed = parse(format!("data: {stop}\n\ndata: {{PRIVATE\n\n").as_bytes()).await;
        assert_eq!(
            malformed.stream_termination,
            StreamTermination::MalformedStream
        );
        assert_eq!(
            malformed.stream_end_signal,
            StreamEndSignal::GeminiFinishReason("STOP".into())
        );
        assert!(
            malformed
                .contract_errors
                .contains(&"gemini.event_after_terminal:/events/1".to_owned())
        );
        assert!(
            malformed
                .contract_errors
                .contains(&"gemini.malformed_event_json:/events/1/data".to_owned())
        );
    }

    #[tokio::test]
    async fn protocol_errors_seal_content_but_later_finish_remains_observable() {
        let later = json!({"candidates": [{
            "index": 0,
            "content": {"parts": [{"functionCall": {
                "name": "get_weather",
                "args": {"city": "PRIVATE_AFTER_ERROR"}
            }}]},
            "finishReason": "STOP"
        }]});
        let prefixes = [
            "data: {\"error\":{\"message\":\"PRIVATE_ERROR\"}}\n\n",
            "event: response\ndata: {\"promptFeedback\":{}}\n\n",
            "data: [DONE]\n\n",
        ];

        for prefix in prefixes {
            let parsed = parse(format!("{prefix}data: {later}\n\n").as_bytes()).await;
            assert_eq!(parsed.stream_termination, StreamTermination::ProtocolError);
            assert_eq!(
                parsed.stream_end_signal,
                StreamEndSignal::GeminiFinishReason("STOP".into())
            );
            assert_eq!(parsed.model_stop_reason.as_deref(), Some("STOP"));
            assert!(parsed.assistant_turn.is_none());
            assert!(
                parsed
                    .contract_errors
                    .contains(&"gemini.event_after_terminal:/events/1".to_owned())
            );
            assert!(!format!("{parsed:?}").contains("PRIVATE_AFTER_ERROR"));
            assert!(!format!("{parsed:?}").contains("PRIVATE_ERROR"));
        }
    }

    #[tokio::test]
    async fn validates_content_role_and_part_modifier_types_without_leaking_values() {
        let body = response_stream(
            json!({"role": "PRIVATE_ROLE", "parts": [{
                "text": "OK",
                "thought": "PRIVATE_THOUGHT",
                "thoughtSignature": 7
            }]}),
            Some("STOP"),
        );
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        for expected in [
            "gemini.invalid_content_role:/events/0/data/candidates/0/content/role",
            "gemini.invalid_thought:/events/0/data/candidates/0/content/parts/0/thought",
            "gemini.invalid_thought_signature:/events/0/data/candidates/0/content/parts/0/thoughtSignature",
        ] {
            assert!(
                parsed.contract_errors.contains(&expected.to_owned()),
                "{expected}"
            );
        }
        assert!(
            parsed
                .assistant_turn
                .as_ref()
                .expect("history")
                .final_text
                .is_empty()
        );
        assert!(!format!("{parsed:?}").contains("PRIVATE_ROLE"));
        assert!(!format!("{parsed:?}").contains("PRIVATE_THOUGHT"));
    }

    #[tokio::test]
    async fn huge_sparse_candidate_indexes_do_not_drive_allocation_or_error_text() {
        let body = sse(&[json!({"candidates": [
            {"index": 18446744073709551615u64, "content": {"parts": [{"text": "PRIVATE"}]}},
            {"index": 0, "content": {"parts": [{"text": "OK"}]}, "finishReason": "STOP"}
        ]})]);
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert_eq!(
            parsed.assistant_turn.as_ref().expect("turn").final_text,
            "OK"
        );
        assert!(!format!("{parsed:?}").contains("18446744073709551615"));
        assert!(!format!("{parsed:?}").contains("PRIVATE"));
    }

    fn response_stream(content: serde_json::Value, finish_reason: Option<&str>) -> String {
        let mut candidate = serde_json::Map::from_iter([("content".into(), content)]);
        if let Some(reason) = finish_reason {
            candidate.insert("finishReason".into(), json!(reason));
        }
        sse(&[json!({"candidates": [candidate]})])
    }

    fn sse(responses: &[serde_json::Value]) -> String {
        responses
            .iter()
            .map(|response| format!("data: {response}\n\n"))
            .collect()
    }

    fn complete_fixture(fixture: &[u8]) -> Vec<u8> {
        let mut body = fixture.to_vec();
        body.push(b'\n');
        body
    }
}
