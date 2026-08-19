use std::collections::{BTreeMap, HashSet};

use serde_json::{Map, Number, Value};

use crate::evidence::{StreamEndSignal, StreamTermination};

use super::framing::decode_sse_chunks;
use super::{AssistantTurn, ProtocolHistory, StreamParseResult, ToolCall, ToolCorrelation};

#[derive(Default)]
struct ParserState {
    id: Option<String>,
    created: Option<Number>,
    model: Option<String>,
    text: String,
    tool_calls: BTreeMap<usize, ToolAccumulator>,
    finish_reason: Option<String>,
    usage_chunk_seen: bool,
    saw_chunk: bool,
    contract_errors: Vec<String>,
}

#[derive(Default)]
struct ToolAccumulator {
    id: Option<String>,
    kind: Option<String>,
    name: Option<String>,
    arguments: String,
    arguments_seen: bool,
    invalid: bool,
}

pub(crate) async fn parse(body: &[u8]) -> StreamParseResult {
    let decoded = match decode_sse_chunks(vec![body.to_vec()]).await {
        Ok(decoded) => decoded,
        Err(_) => {
            return StreamParseResult {
                assistant_turn: None,
                stream_termination: StreamTermination::MalformedStream,
                stream_end_signal: StreamEndSignal::None,
                model_stop_reason: None,
                event_count: 0,
                contract_errors: vec![contract_error("invalid_sse", "/")],
            };
        }
    };

    let event_count = decoded.events.len();
    let mut state = ParserState::default();
    let mut done_index = None;
    let mut malformed_stream = false;
    let mut protocol_error = false;

    for (event_index, event) in decoded.events.iter().enumerate() {
        let is_message_event = matches!(event.event.as_str(), "" | "message");
        if let Some(first_done_index) = done_index {
            protocol_error = true;
            if is_message_event && event.data == "[DONE]" {
                push_error(
                    &mut state.contract_errors,
                    "duplicate_done",
                    &format!("/events/{event_index}"),
                );
            } else {
                push_error(
                    &mut state.contract_errors,
                    "early_done",
                    &format!("/events/{first_done_index}"),
                );
            }
            if event.event == "error" {
                process_error_event(
                    &mut state.contract_errors,
                    &event.data,
                    event_index,
                    &mut malformed_stream,
                );
            } else if is_message_event && event.data != "[DONE]" {
                inspect_message_after_done(
                    &mut state.contract_errors,
                    &event.data,
                    event_index,
                    &mut malformed_stream,
                );
            }
            continue;
        }

        match event.event.as_str() {
            "" | "message" if event.data == "[DONE]" => {
                done_index = Some(event_index);
                continue;
            }
            "" | "message" => {}
            "error" => {
                protocol_error = true;
                process_error_event(
                    &mut state.contract_errors,
                    &event.data,
                    event_index,
                    &mut malformed_stream,
                );
                continue;
            }
            _ => continue,
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

        if value.get("error").is_some() {
            protocol_error = true;
            push_error(&mut state.contract_errors, "api_error", "/error");
            continue;
        }

        process_chunk(&mut state, &value, event_index);
    }

    if decoded.trailing_incomplete_frame {
        malformed_stream = true;
        push_error(
            &mut state.contract_errors,
            "truncated_sse",
            &format!("/events/{event_count}"),
        );
    }

    let has_done = done_index.is_some();

    let finish_is_normal = normal_finish(&state);
    if has_done && !finish_is_normal {
        let code = match state.finish_reason.as_deref() {
            None => "missing_finish_reason",
            Some("stop" | "tool_calls") => "finish_reason_mismatch",
            Some(_) => "non_normal_finish_reason",
        };
        push_error(&mut state.contract_errors, code, "/choices/0/finish_reason");
    }

    let assistant_turn = build_assistant_turn(&mut state);
    let stream_termination = if malformed_stream {
        StreamTermination::MalformedStream
    } else if protocol_error {
        StreamTermination::ProtocolError
    } else if !has_done {
        StreamTermination::MissingTerminalEvent
    } else if !finish_is_normal {
        StreamTermination::ModelIncomplete
    } else {
        StreamTermination::Completed
    };

    StreamParseResult {
        assistant_turn,
        stream_termination,
        stream_end_signal: if has_done {
            StreamEndSignal::OpenAiDone
        } else {
            StreamEndSignal::None
        },
        model_stop_reason: state.finish_reason,
        event_count,
        contract_errors: state.contract_errors,
    }
}

fn process_error_event(
    errors: &mut Vec<String>,
    data: &str,
    event_index: usize,
    malformed_stream: &mut bool,
) {
    push_error(errors, "api_error", &format!("/events/{event_index}"));
    if data != "[DONE]" && serde_json::from_str::<Value>(data).is_err() {
        *malformed_stream = true;
        push_error(
            errors,
            "malformed_event_json",
            &format!("/events/{event_index}/data"),
        );
    }
}

fn inspect_message_after_done(
    errors: &mut Vec<String>,
    data: &str,
    event_index: usize,
    malformed_stream: &mut bool,
) {
    match serde_json::from_str::<Value>(data) {
        Ok(value) if value.get("error").is_some() => push_error(
            errors,
            "api_error",
            &format!("/events/{event_index}/data/error"),
        ),
        Ok(_) => {}
        Err(_) => {
            *malformed_stream = true;
            push_error(
                errors,
                "malformed_event_json",
                &format!("/events/{event_index}/data"),
            );
        }
    }
}

fn process_chunk(state: &mut ParserState, value: &Value, event_index: usize) {
    let Some(chunk) = value.as_object() else {
        push_error(
            &mut state.contract_errors,
            "invalid_chunk",
            &format!("/events/{event_index}/data"),
        );
        return;
    };
    state.saw_chunk = true;

    correlate_string(
        &mut state.id,
        chunk.get("id"),
        "id",
        event_index,
        &mut state.contract_errors,
    );
    validate_object(chunk, event_index, &mut state.contract_errors);
    correlate_created(
        &mut state.created,
        chunk.get("created"),
        event_index,
        &mut state.contract_errors,
    );
    correlate_string(
        &mut state.model,
        chunk.get("model"),
        "model",
        event_index,
        &mut state.contract_errors,
    );

    let choices_path = format!("/events/{event_index}/data/choices");
    let Some(choices) = chunk.get("choices").and_then(Value::as_array) else {
        push_error(&mut state.contract_errors, "invalid_choices", &choices_path);
        return;
    };
    if choices.is_empty() {
        process_usage_only_chunk(state, chunk, event_index);
        return;
    }
    if chunk.get("usage").is_some_and(|usage| !usage.is_null()) {
        push_error(
            &mut state.contract_errors,
            "unexpected_usage",
            &format!("/events/{event_index}/data/usage"),
        );
    }

    process_choices(state, choices, event_index);
}

fn process_usage_only_chunk(
    state: &mut ParserState,
    chunk: &Map<String, Value>,
    event_index: usize,
) {
    let finish_is_normal = normal_finish(state);
    let usage_is_non_empty_object = chunk
        .get("usage")
        .and_then(Value::as_object)
        .is_some_and(|usage| !usage.is_empty());

    if !finish_is_normal {
        push_error(
            &mut state.contract_errors,
            "usage_before_finish",
            &format!("/events/{event_index}/data/choices"),
        );
    }
    if !usage_is_non_empty_object {
        push_error(
            &mut state.contract_errors,
            "invalid_usage_chunk",
            &format!("/events/{event_index}/data/usage"),
        );
    }
    if state.usage_chunk_seen {
        push_error(
            &mut state.contract_errors,
            "duplicate_usage_chunk",
            &format!("/events/{event_index}/data/usage"),
        );
    }

    if finish_is_normal && usage_is_non_empty_object && !state.usage_chunk_seen {
        state.usage_chunk_seen = true;
    }
}

fn correlate_string(
    expected: &mut Option<String>,
    value: Option<&Value>,
    field: &str,
    event_index: usize,
    errors: &mut Vec<String>,
) {
    let path = format!("/events/{event_index}/data/{field}");
    let Some(value) = value.and_then(Value::as_str) else {
        push_error(errors, &format!("invalid_{field}"), &path);
        return;
    };

    match expected {
        None => *expected = Some(value.to_owned()),
        Some(expected) if expected != value => {
            push_error(errors, &format!("conflicting_{field}"), &path);
        }
        Some(_) => {}
    }
}

fn correlate_created(
    expected: &mut Option<Number>,
    value: Option<&Value>,
    event_index: usize,
    errors: &mut Vec<String>,
) {
    let path = format!("/events/{event_index}/data/created");
    let Some(number) = value.and_then(Value::as_number) else {
        push_error(errors, "invalid_created", &path);
        return;
    };
    if number.as_i64().is_none() && number.as_u64().is_none() {
        push_error(errors, "invalid_created", &path);
        return;
    }

    match expected {
        None => *expected = Some(number.clone()),
        Some(expected) if expected != number => {
            push_error(errors, "conflicting_created", &path);
        }
        Some(_) => {}
    }
}

fn validate_object(chunk: &Map<String, Value>, event_index: usize, errors: &mut Vec<String>) {
    if chunk.get("object").and_then(Value::as_str) != Some("chat.completion.chunk") {
        push_error(
            errors,
            "invalid_object",
            &format!("/events/{event_index}/data/object"),
        );
    }
}

fn process_choices(state: &mut ParserState, choices: &[Value], event_index: usize) {
    let mut seen_indexes = HashSet::new();
    let mut selected = None;

    for (position, choice) in choices.iter().enumerate() {
        let choice_path = format!("/events/{event_index}/data/choices/{position}");
        let Some(choice) = choice.as_object() else {
            push_error(&mut state.contract_errors, "invalid_choice", &choice_path);
            continue;
        };
        let Some(index) = json_index(choice.get("index")) else {
            push_error(
                &mut state.contract_errors,
                "invalid_choice_index",
                &format!("{choice_path}/index"),
            );
            continue;
        };
        if !seen_indexes.insert(index) {
            push_error(
                &mut state.contract_errors,
                "duplicate_choice_index",
                &format!("{choice_path}/index"),
            );
            continue;
        }

        let Some(delta) = choice.get("delta").and_then(Value::as_object) else {
            push_error(
                &mut state.contract_errors,
                "invalid_delta",
                &format!("{choice_path}/delta"),
            );
            continue;
        };

        if index == 0 {
            selected = Some((position, choice, delta));
        }
    }

    let Some((position, choice, delta)) = selected else {
        push_error(
            &mut state.contract_errors,
            "missing_selected_choice",
            &format!("/events/{event_index}/data/choices"),
        );
        return;
    };
    process_selected_choice(state, choice, delta, event_index, position);
}

fn process_selected_choice(
    state: &mut ParserState,
    choice: &Map<String, Value>,
    delta: &Map<String, Value>,
    event_index: usize,
    position: usize,
) {
    let delta_path = format!("/events/{event_index}/data/choices/{position}/delta");
    if state.finish_reason.is_some() {
        let mut reported_specific_error = false;
        if delta.contains_key("content") {
            reported_specific_error = true;
            push_error(
                &mut state.contract_errors,
                "post_finish_content",
                &format!("{delta_path}/content"),
            );
        }
        if delta.contains_key("tool_calls") {
            reported_specific_error = true;
            push_error(
                &mut state.contract_errors,
                "post_finish_tool_calls",
                &format!("{delta_path}/tool_calls"),
            );
        }
        if choice
            .get("finish_reason")
            .is_some_and(|value| !value.is_null())
        {
            reported_specific_error = true;
            push_error(
                &mut state.contract_errors,
                "duplicate_finish_reason",
                &format!("/events/{event_index}/data/choices/{position}/finish_reason"),
            );
        }
        if !reported_specific_error {
            push_error(
                &mut state.contract_errors,
                "post_finish_choice",
                &format!("/events/{event_index}/data/choices/{position}"),
            );
        }
        return;
    }

    if let Some(role) = delta.get("role")
        && role.as_str() != Some("assistant")
    {
        push_error(
            &mut state.contract_errors,
            "invalid_role",
            &format!("{delta_path}/role"),
        );
    }

    if let Some(content) = delta.get("content") {
        match content {
            Value::Null => {}
            Value::String(fragment) => state.text.push_str(fragment),
            _ => push_error(
                &mut state.contract_errors,
                "invalid_content",
                &format!("{delta_path}/content"),
            ),
        }
    }

    if let Some(tool_calls) = delta.get("tool_calls") {
        let Some(tool_calls) = tool_calls.as_array() else {
            push_error(
                &mut state.contract_errors,
                "invalid_tool_calls",
                &format!("{delta_path}/tool_calls"),
            );
            process_finish_reason(state, choice, event_index, position);
            return;
        };
        process_tool_deltas(state, tool_calls, event_index, position);
    }

    process_finish_reason(state, choice, event_index, position);
}

fn process_finish_reason(
    state: &mut ParserState,
    choice: &Map<String, Value>,
    event_index: usize,
    position: usize,
) {
    let Some(value) = choice.get("finish_reason") else {
        return;
    };
    if value.is_null() {
        return;
    }
    let path = format!("/events/{event_index}/data/choices/{position}/finish_reason");
    let Some(reason) = value.as_str() else {
        push_error(&mut state.contract_errors, "invalid_finish_reason", &path);
        return;
    };

    match &state.finish_reason {
        None => state.finish_reason = Some(reason.to_owned()),
        Some(existing) if existing != reason => {
            push_error(
                &mut state.contract_errors,
                "conflicting_finish_reason",
                &path,
            );
        }
        Some(_) => {}
    }
}

fn process_tool_deltas(
    state: &mut ParserState,
    tool_calls: &[Value],
    event_index: usize,
    choice_position: usize,
) {
    let mut seen_indexes = HashSet::new();
    for (position, tool_call) in tool_calls.iter().enumerate() {
        let path = format!(
            "/events/{event_index}/data/choices/{choice_position}/delta/tool_calls/{position}"
        );
        let Some(tool_call) = tool_call.as_object() else {
            push_error(&mut state.contract_errors, "invalid_tool_call", &path);
            continue;
        };
        let Some(index) = json_index(tool_call.get("index")) else {
            push_error(
                &mut state.contract_errors,
                "invalid_tool_index",
                &format!("{path}/index"),
            );
            continue;
        };
        if !seen_indexes.insert(index) {
            push_error(
                &mut state.contract_errors,
                "duplicate_tool_index",
                &format!("{path}/index"),
            );
            continue;
        }

        let accumulator = state.tool_calls.entry(index).or_default();
        merge_optional_string(
            &mut accumulator.id,
            tool_call.get("id"),
            "tool_id",
            &format!("{path}/id"),
            &mut accumulator.invalid,
            &mut state.contract_errors,
        );
        merge_optional_string(
            &mut accumulator.kind,
            tool_call.get("type"),
            "tool_type",
            &format!("{path}/type"),
            &mut accumulator.invalid,
            &mut state.contract_errors,
        );
        if tool_call.get("type").is_some()
            && accumulator
                .kind
                .as_deref()
                .is_some_and(|kind| kind != "function")
        {
            accumulator.invalid = true;
            push_error(
                &mut state.contract_errors,
                "invalid_tool_type",
                &format!("{path}/type"),
            );
        }

        if let Some(function) = tool_call.get("function") {
            let Some(function) = function.as_object() else {
                accumulator.invalid = true;
                push_error(
                    &mut state.contract_errors,
                    "invalid_function",
                    &format!("{path}/function"),
                );
                continue;
            };
            merge_optional_string(
                &mut accumulator.name,
                function.get("name"),
                "tool_name",
                &format!("{path}/function/name"),
                &mut accumulator.invalid,
                &mut state.contract_errors,
            );
            if let Some(arguments) = function.get("arguments") {
                if let Some(fragment) = arguments.as_str() {
                    accumulator.arguments_seen = true;
                    accumulator.arguments.push_str(fragment);
                } else {
                    accumulator.invalid = true;
                    push_error(
                        &mut state.contract_errors,
                        "invalid_argument_fragment",
                        &format!("{path}/function/arguments"),
                    );
                }
            }
        }
    }
}

fn merge_optional_string(
    accumulated: &mut Option<String>,
    incoming: Option<&Value>,
    field: &str,
    path: &str,
    invalid: &mut bool,
    errors: &mut Vec<String>,
) {
    let Some(incoming) = incoming else {
        return;
    };
    let Some(incoming) = incoming.as_str() else {
        *invalid = true;
        push_error(errors, &format!("invalid_{field}"), path);
        return;
    };

    match accumulated {
        None => *accumulated = Some(incoming.to_owned()),
        Some(existing) if existing != incoming => {
            *invalid = true;
            push_error(errors, &format!("conflicting_{field}"), path);
        }
        Some(_) => {}
    }
}

fn normal_finish(state: &ParserState) -> bool {
    matches!(
        (state.finish_reason.as_deref(), state.tool_calls.is_empty()),
        (Some("stop"), true) | (Some("tool_calls"), false)
    )
}

fn build_assistant_turn(state: &mut ParserState) -> Option<AssistantTurn> {
    if !state.saw_chunk {
        return None;
    }

    let mut history_calls = Vec::new();
    let mut normalized_calls = Vec::new();
    let mut finalization_errors = Vec::new();
    let indexes_are_contiguous = state
        .tool_calls
        .keys()
        .copied()
        .enumerate()
        .all(|(position, index)| position == index);

    for (position, (index, accumulator)) in state.tool_calls.iter().enumerate() {
        let base_path = format!("/choices/0/delta/tool_calls/{position}");
        if *index != position {
            finalization_errors.push(contract_error(
                "non_contiguous_tool_index",
                &format!("{base_path}/index"),
            ));
        }

        let mut history_call = Map::new();
        let id_is_non_empty = accumulator
            .id
            .as_deref()
            .is_some_and(|id| !id.trim().is_empty());
        if let Some(id) = &accumulator.id {
            history_call.insert("id".into(), Value::String(id.clone()));
            if !id_is_non_empty {
                finalization_errors
                    .push(contract_error("empty_tool_id", &format!("{base_path}/id")));
            }
        } else {
            finalization_errors.push(contract_error(
                "missing_tool_id",
                &format!("{base_path}/id"),
            ));
        }
        if let Some(kind) = &accumulator.kind {
            history_call.insert("type".into(), Value::String(kind.clone()));
        } else {
            finalization_errors.push(contract_error(
                "missing_tool_type",
                &format!("{base_path}/type"),
            ));
        }

        let mut function = Map::new();
        let name_is_non_empty = accumulator
            .name
            .as_deref()
            .is_some_and(|name| !name.trim().is_empty());
        if let Some(name) = &accumulator.name {
            function.insert("name".into(), Value::String(name.clone()));
            if !name_is_non_empty {
                finalization_errors.push(contract_error(
                    "empty_tool_name",
                    &format!("{base_path}/function/name"),
                ));
            }
        } else {
            finalization_errors.push(contract_error(
                "missing_tool_name",
                &format!("{base_path}/function/name"),
            ));
        }
        if accumulator.arguments_seen {
            function.insert(
                "arguments".into(),
                Value::String(accumulator.arguments.clone()),
            );
        } else {
            finalization_errors.push(contract_error(
                "missing_arguments",
                &format!("{base_path}/function/arguments"),
            ));
        }
        if !function.is_empty() {
            history_call.insert("function".into(), Value::Object(function));
        }
        history_calls.push(Value::Object(history_call));

        let arguments = serde_json::from_str::<Value>(&accumulator.arguments).ok();
        let valid_arguments = arguments.as_ref().is_some_and(Value::is_object);
        if accumulator.arguments_seen && !valid_arguments {
            finalization_errors.push(contract_error(
                "invalid_arguments",
                &format!("{base_path}/function/arguments"),
            ));
        }

        if indexes_are_contiguous
            && id_is_non_empty
            && name_is_non_empty
            && !accumulator.invalid
            && accumulator.kind.as_deref() == Some("function")
            && let (Some(id), Some(name), Some(arguments)) =
                (&accumulator.id, &accumulator.name, arguments)
            && arguments.is_object()
        {
            normalized_calls.push(ToolCall {
                index: *index,
                correlation: ToolCorrelation::Required(id.clone()),
                name: name.clone(),
                arguments,
            });
        }
    }

    for error in finalization_errors {
        if !state.contract_errors.contains(&error) {
            state.contract_errors.push(error);
        }
    }

    let mut history = Map::from_iter([
        ("role".into(), Value::String("assistant".into())),
        (
            "content".into(),
            if state.text.is_empty() {
                Value::Null
            } else {
                Value::String(state.text.clone())
            },
        ),
    ]);
    if !history_calls.is_empty() {
        history.insert("tool_calls".into(), Value::Array(history_calls));
    }

    Some(AssistantTurn {
        history: ProtocolHistory::OpenAiChat(Value::Object(history)),
        tool_calls: normalized_calls,
        final_text: state.text.clone(),
    })
}

fn json_index(value: Option<&Value>) -> Option<usize> {
    usize::try_from(value?.as_u64()?).ok()
}

fn contract_error(code: &str, path: &str) -> String {
    debug_assert!(path.starts_with('/'));
    format!("chat.{code}:{path}")
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

    use super::parse;
    use crate::protocol::stream::{AssistantTurn, ProtocolHistory, ToolCall, ToolCorrelation};

    const TOOL_STREAM: &[u8] = include_bytes!("../fixtures/openai_chat_tool.sse");
    const FINAL_STREAM: &[u8] = include_bytes!("../fixtures/openai_chat_final.sse");
    const MISSING_TERMINAL: &[u8] = include_bytes!("../fixtures/openai_chat_missing_terminal.sse");
    const TRUNCATED: &[u8] = include_bytes!("../fixtures/openai_chat_truncated.sse");
    const API_ERROR: &[u8] = include_bytes!("../fixtures/openai_chat_error.sse");
    const INVALID_ARGUMENTS: &[u8] =
        include_bytes!("../fixtures/openai_chat_invalid_arguments.sse");

    #[tokio::test]
    async fn parses_argument_fragments_and_preserves_official_assistant_history() {
        let body = complete_fixture(TOOL_STREAM);
        let parsed = parse(&body).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert_eq!(parsed.stream_end_signal, StreamEndSignal::OpenAiDone);
        assert_eq!(parsed.model_stop_reason.as_deref(), Some("tool_calls"));
        assert_eq!(parsed.event_count, 5, "four JSON events plus [DONE]");
        assert!(parsed.contract_errors.is_empty());
        assert_eq!(
            parsed.assistant_turn,
            Some(AssistantTurn {
                history: ProtocolHistory::OpenAiChat(json!({
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_weather_046",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"city\":\"Beijing\"}"
                        }
                    }]
                })),
                tool_calls: vec![ToolCall {
                    index: 0,
                    correlation: ToolCorrelation::Required("call_weather_046".into()),
                    name: "get_weather".into(),
                    arguments: json!({"city": "Beijing"}),
                }],
                final_text: String::new(),
            })
        );
    }

    #[tokio::test]
    async fn parses_final_text_and_stop_terminal() {
        let body = complete_fixture(FINAL_STREAM);
        let parsed = parse(&body).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert_eq!(parsed.stream_end_signal, StreamEndSignal::OpenAiDone);
        assert_eq!(parsed.model_stop_reason.as_deref(), Some("stop"));
        assert_eq!(parsed.event_count, 4, "three JSON events plus [DONE]");
        assert!(parsed.contract_errors.is_empty());
        assert_eq!(
            parsed.assistant_turn,
            Some(AssistantTurn {
                history: ProtocolHistory::OpenAiChat(json!({
                    "role": "assistant",
                    "content": "MODEL_DOCTOR_CASE_046_OK"
                })),
                tool_calls: vec![],
                final_text: "MODEL_DOCTOR_CASE_046_OK".into(),
            })
        );
    }

    #[tokio::test]
    async fn clean_eof_without_done_is_missing_terminal_event() {
        let body = complete_fixture(MISSING_TERMINAL);
        let parsed = parse(&body).await;

        assert_eq!(
            parsed.stream_termination,
            StreamTermination::MissingTerminalEvent
        );
        assert_eq!(parsed.stream_end_signal, StreamEndSignal::None);
        assert_eq!(parsed.model_stop_reason.as_deref(), Some("stop"));
        assert_eq!(parsed.event_count, 2);
    }

    #[tokio::test]
    async fn trailing_partial_sse_frame_is_malformed_stream() {
        let parsed = parse(TRUNCATED).await;

        assert_eq!(
            parsed.stream_termination,
            StreamTermination::MalformedStream
        );
        assert_eq!(parsed.stream_end_signal, StreamEndSignal::None);
        assert_eq!(
            parsed.event_count, 1,
            "only the dispatched event is counted"
        );
        assert_eq!(
            parsed.contract_errors,
            ["chat.truncated_sse:/events/1".to_owned()]
        );
    }

    #[tokio::test]
    async fn malformed_event_json_is_malformed_stream() {
        let parsed = parse(b"data: {not-json}\n\n").await;

        assert_eq!(
            parsed.stream_termination,
            StreamTermination::MalformedStream
        );
        assert_eq!(parsed.event_count, 1, "the SSE event decoded successfully");
        assert_eq!(
            parsed.contract_errors,
            ["chat.malformed_event_json:/events/0/data".to_owned()]
        );
    }

    #[tokio::test]
    async fn top_level_api_error_is_protocol_error_without_leaking_details() {
        let body = complete_fixture(API_ERROR);
        let parsed = parse(&body).await;

        assert_eq!(parsed.stream_termination, StreamTermination::ProtocolError);
        assert_eq!(parsed.stream_end_signal, StreamEndSignal::None);
        assert_eq!(parsed.event_count, 1);
        assert_eq!(parsed.contract_errors, ["chat.api_error:/error".to_owned()]);
        assert!(!format!("{parsed:?}").contains("sensitive upstream detail"));
    }

    #[tokio::test]
    async fn malformed_accumulated_arguments_keep_completed_wire_lifecycle() {
        let body = complete_fixture(INVALID_ARGUMENTS);
        let parsed = parse(&body).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert_eq!(parsed.stream_end_signal, StreamEndSignal::OpenAiDone);
        assert_eq!(parsed.model_stop_reason.as_deref(), Some("tool_calls"));
        assert_eq!(
            parsed.contract_errors,
            [
                "chat.invalid_arguments:/choices/0/delta/tool_calls/0/function/arguments"
                    .to_owned()
            ]
        );
        assert_eq!(
            parsed
                .assistant_turn
                .as_ref()
                .expect("native history remains available")
                .tool_calls,
            vec![]
        );
    }

    #[tokio::test]
    async fn rejects_conflicting_choice_and_tool_indexes() {
        let duplicate_choice = stream(&[
            chunk(json!([
                {"index": 0, "delta": {}, "finish_reason": null},
                {"index": 0, "delta": {}, "finish_reason": null}
            ])),
            chunk(json!([{"index": 0, "delta": {}, "finish_reason": "stop"}])),
            "[DONE]".into(),
        ]);
        let duplicate = parse(duplicate_choice.as_bytes()).await;
        assert!(
            duplicate
                .contract_errors
                .contains(&"chat.duplicate_choice_index:/events/0/data/choices/1/index".to_owned())
        );

        let duplicate_tool = stream(&[
            chunk(json!([{
                "index": 0,
                "delta": {"tool_calls": [
                    {"index": 0, "id": "call_a", "type": "function", "function": {"name": "get_weather", "arguments": "{}"}},
                    {"index": 0, "id": "call_b", "type": "function", "function": {"name": "get_time", "arguments": "{}"}}
                ]},
                "finish_reason": null
            }])),
            chunk(json!([{"index": 0, "delta": {}, "finish_reason": "tool_calls"}])),
            "[DONE]".into(),
        ]);
        let duplicate = parse(duplicate_tool.as_bytes()).await;
        assert!(
            duplicate.contract_errors.contains(
                &"chat.duplicate_tool_index:/events/0/data/choices/0/delta/tool_calls/1/index"
                    .to_owned()
            )
        );
    }

    #[tokio::test]
    async fn rejects_conflicting_tool_ids_without_echoing_them() {
        let body = tool_conflict_stream("different-private-id");
        let parsed = parse(body.as_bytes()).await;

        assert!(parsed.contract_errors.contains(
            &"chat.conflicting_tool_id:/events/1/data/choices/0/delta/tool_calls/0/id".to_owned()
        ));
        assert!(!format!("{parsed:?}").contains("different-private-id"));
    }

    #[tokio::test]
    async fn requires_stable_typed_envelope_identity() {
        let body = stream(&[
            r#"{"id":"chatcmpl-stable","object":"chat.completion.chunk","created":1787041140,"model":"gpt-test","choices":[{"index":0,"delta":{"role":"assistant","content":"OK"},"finish_reason":null}]}"#.into(),
            r#"{"id":"chatcmpl-stable","object":"chat.completion.chunk","created":1787041140,"model":"private-model-value","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#.into(),
            "[DONE]".into(),
        ]);
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert!(
            parsed
                .contract_errors
                .contains(&"chat.conflicting_model:/events/1/data/model".to_owned())
        );
        assert!(!format!("{parsed:?}").contains("private-model-value"));

        let invalid = stream(&[
            r#"{"id":7,"object":"wrong","created":"now","model":false,"choices":{}}"#.into(),
            "[DONE]".into(),
        ]);
        let invalid = parse(invalid.as_bytes()).await;
        assert_eq!(
            invalid.stream_termination,
            StreamTermination::ModelIncomplete
        );
        for expected in [
            "chat.invalid_id:/events/0/data/id",
            "chat.invalid_object:/events/0/data/object",
            "chat.invalid_created:/events/0/data/created",
            "chat.invalid_model:/events/0/data/model",
            "chat.invalid_choices:/events/0/data/choices",
        ] {
            assert!(
                invalid.contract_errors.contains(&expected.to_owned()),
                "{expected}"
            );
        }
    }

    #[tokio::test]
    async fn non_normal_finish_reason_is_model_incomplete() {
        let body = stream(&[
            chunk(json!([{
                "index": 0,
                "delta": {"role": "assistant", "content": "partial"},
                "finish_reason": "length"
            }])),
            "[DONE]".into(),
        ]);
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(
            parsed.stream_termination,
            StreamTermination::ModelIncomplete
        );
        assert_eq!(parsed.stream_end_signal, StreamEndSignal::OpenAiDone);
        assert_eq!(parsed.model_stop_reason.as_deref(), Some("length"));
    }

    #[tokio::test]
    async fn counts_and_ignores_forward_compatible_extension_events() {
        let mut body = "event: ping\ndata: {\"type\":\"ping\"}\n\n".to_owned();
        body.push_str(std::str::from_utf8(FINAL_STREAM).expect("fixture is UTF-8"));
        body.push('\n');
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert_eq!(parsed.event_count, 5, "ping plus every Chat SSE event");
        assert!(parsed.contract_errors.is_empty());
    }

    #[tokio::test]
    async fn selects_choice_index_zero_instead_of_the_first_array_position() {
        let body = stream(&[
            chunk(json!([
                {"index": 1, "delta": {"content": "IGNORE"}, "finish_reason": null},
                {"index": 0, "delta": {"role": "assistant", "content": "RIGHT"}, "finish_reason": null}
            ])),
            chunk(json!([
                {"index": 1, "delta": {"content": "ALSO_IGNORE"}, "finish_reason": "stop"},
                {"index": 0, "delta": {}, "finish_reason": "stop"}
            ])),
            "[DONE]".into(),
        ]);
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert_eq!(
            parsed.assistant_turn.expect("selected turn").final_text,
            "RIGHT"
        );
        assert!(parsed.contract_errors.is_empty());
    }

    #[tokio::test]
    async fn accepts_usage_only_chunk_between_normal_finish_and_done() {
        let prefix = std::str::from_utf8(FINAL_STREAM)
            .expect("fixture is UTF-8")
            .strip_suffix("data: [DONE]\n")
            .expect("fixture ends in [DONE]");
        let usage = r#"data: {"id":"chatcmpl-final","object":"chat.completion.chunk","created":1787041141,"model":"gpt-test","choices":[],"usage":{"prompt_tokens":12,"completion_tokens":4,"total_tokens":16}}

data: [DONE]

"#;
        let parsed = parse(format!("{prefix}{usage}").as_bytes()).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert_eq!(parsed.model_stop_reason.as_deref(), Some("stop"));
        assert_eq!(parsed.event_count, 5);
        assert!(parsed.contract_errors.is_empty());
    }

    #[tokio::test]
    async fn accumulates_distinct_parallel_tool_indexes_independently() {
        let body = stream(&[
            chunk(json!([{
                "index": 0,
                "delta": {"role": "assistant", "tool_calls": [
                    {
                        "index": 1,
                        "id": "call_time",
                        "type": "function",
                        "function": {"name": "get_time", "arguments": "{\"zone\":\"UTC\"}"}
                    },
                    {
                        "index": 0,
                        "id": "call_weather",
                        "type": "function",
                        "function": {"name": "get_weather", "arguments": "{\"city\":\"Beijing\"}"}
                    }
                ]},
                "finish_reason": null
            }])),
            chunk(json!([{"index": 0, "delta": {}, "finish_reason": "tool_calls"}])),
            "[DONE]".into(),
        ]);
        let parsed = parse(body.as_bytes()).await;
        let calls = &parsed.assistant_turn.expect("tool turn").tool_calls;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert!(parsed.contract_errors.is_empty());
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].index, 0);
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[1].index, 1);
        assert_eq!(calls[1].name, "get_time");
    }

    #[tokio::test]
    async fn appends_identical_argument_fragments_without_deduplicating() {
        let body = stream(&[
            chunk(json!([{
                "index": 0,
                "delta": {"tool_calls": [{
                    "index": 0,
                    "id": "call_repeat",
                    "type": "function",
                    "function": {"name": "echo", "arguments": "{\"text\":\"same"}
                }]},
                "finish_reason": null
            }])),
            chunk(json!([{
                "index": 0,
                "delta": {"tool_calls": [{
                    "index": 0,
                    "function": {"arguments": "same\"}"}
                }]},
                "finish_reason": null
            }])),
            chunk(json!([{"index": 0, "delta": {}, "finish_reason": "tool_calls"}])),
            "[DONE]".into(),
        ]);
        let parsed = parse(body.as_bytes()).await;
        let call = &parsed.assistant_turn.expect("tool turn").tool_calls[0];

        assert_eq!(call.arguments, json!({"text": "samesame"}));
        assert!(parsed.contract_errors.is_empty());
    }

    #[tokio::test]
    async fn finish_reason_seals_the_selected_choice_against_later_deltas() {
        let body = stream(&[
            chunk(json!([{
                "index": 0,
                "delta": {"role": "assistant", "content": "KEPT"},
                "finish_reason": null
            }])),
            chunk(json!([{"index": 0, "delta": {}, "finish_reason": "stop"}])),
            chunk(json!([{
                "index": 0,
                "delta": {
                    "content": "PRIVATE_POST_FINISH_TEXT",
                    "tool_calls": [{
                        "index": 0,
                        "id": "private-post-finish-id",
                        "type": "function",
                        "function": {"name": "private_post_finish_tool", "arguments": "{}"}
                    }]
                },
                "finish_reason": "stop"
            }])),
            "[DONE]".into(),
        ]);
        let parsed = parse(body.as_bytes()).await;
        let debug = format!("{parsed:?}");

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert_eq!(parsed.assistant_turn.expect("turn").final_text, "KEPT");
        for expected in [
            "chat.post_finish_content:/events/2/data/choices/0/delta/content",
            "chat.post_finish_tool_calls:/events/2/data/choices/0/delta/tool_calls",
            "chat.duplicate_finish_reason:/events/2/data/choices/0/finish_reason",
        ] {
            assert!(
                parsed.contract_errors.contains(&expected.to_owned()),
                "{expected}"
            );
        }
        assert!(!debug.contains("PRIVATE_POST_FINISH_TEXT"));
        assert!(!debug.contains("private-post-finish-id"));
        assert!(!debug.contains("private_post_finish_tool"));
    }

    #[tokio::test]
    async fn allows_extension_events_after_finish_before_done() {
        let mut body = stream(&[
            chunk(json!([{
                "index": 0,
                "delta": {"role": "assistant", "content": "OK"},
                "finish_reason": null
            }])),
            chunk(json!([{"index": 0, "delta": {}, "finish_reason": "stop"}])),
        ]);
        body.push_str(&named_event("ping", r#"{"type":"ping"}"#));
        body.push_str("data: [DONE]\n\n");
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert_eq!(parsed.event_count, 4);
        assert!(parsed.contract_errors.is_empty());
    }

    #[tokio::test]
    async fn done_marker_is_recognized_only_on_default_or_message_events() {
        let mut ping_body = stream(&[
            chunk(json!([{
                "index": 0,
                "delta": {"role": "assistant", "content": "OK"},
                "finish_reason": null
            }])),
            chunk(json!([{"index": 0, "delta": {}, "finish_reason": "stop"}])),
        ]);
        ping_body.push_str(&named_event("ping", "[DONE]"));
        let ping = parse(ping_body.as_bytes()).await;
        assert_eq!(
            ping.stream_termination,
            StreamTermination::MissingTerminalEvent
        );
        assert_eq!(ping.stream_end_signal, StreamEndSignal::None);

        let error = parse(named_event("error", "[DONE]").as_bytes()).await;
        assert_eq!(error.stream_termination, StreamTermination::ProtocolError);
        assert_eq!(error.stream_end_signal, StreamEndSignal::None);
        assert_eq!(
            error.contract_errors,
            ["chat.api_error:/events/0".to_owned()]
        );
    }

    #[tokio::test]
    async fn malformed_error_event_keeps_malformed_precedence_and_api_error() {
        let parsed = parse(named_event("error", "{not-json").as_bytes()).await;

        assert_eq!(
            parsed.stream_termination,
            StreamTermination::MalformedStream
        );
        assert_eq!(
            parsed.contract_errors,
            [
                "chat.api_error:/events/0".to_owned(),
                "chat.malformed_event_json:/events/0/data".to_owned(),
            ]
        );
    }

    #[tokio::test]
    async fn malformed_error_after_done_still_uses_event_type_precedence() {
        let mut body = "data: [DONE]\n\n".to_owned();
        body.push_str(&named_event("error", "{private-bad-json"));
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(
            parsed.stream_termination,
            StreamTermination::MalformedStream
        );
        for expected in [
            "chat.early_done:/events/0",
            "chat.api_error:/events/1",
            "chat.malformed_event_json:/events/1/data",
        ] {
            assert!(
                parsed.contract_errors.contains(&expected.to_owned()),
                "{expected}"
            );
        }
        assert!(!format!("{:?}", parsed.contract_errors).contains("private-bad-json"));
    }

    #[tokio::test]
    async fn malformed_message_after_done_keeps_malformed_precedence() {
        let body = stream(&[
            chunk(json!([{
                "index": 0,
                "delta": {"role": "assistant", "content": "KEPT"},
                "finish_reason": "stop"
            }])),
            "[DONE]".into(),
            "{PRIVATE_BAD_JSON".into(),
        ]);
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(
            parsed.stream_termination,
            StreamTermination::MalformedStream
        );
        assert_eq!(parsed.stream_end_signal, StreamEndSignal::OpenAiDone);
        assert_eq!(
            parsed.contract_errors,
            [
                "chat.early_done:/events/1".to_owned(),
                "chat.malformed_event_json:/events/2/data".to_owned(),
            ]
        );
        assert_eq!(
            parsed.assistant_turn.as_ref().expect("turn").final_text,
            "KEPT"
        );
        assert!(!format!("{parsed:?}").contains("PRIVATE_BAD_JSON"));
    }

    #[tokio::test]
    async fn api_error_object_after_done_is_recorded() {
        let body = stream(&[
            chunk(json!([{
                "index": 0,
                "delta": {"role": "assistant", "content": "KEPT"},
                "finish_reason": "stop"
            }])),
            "[DONE]".into(),
            json!({"error": {"message": "PRIVATE_API_DETAIL"}}).to_string(),
        ]);
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(parsed.stream_termination, StreamTermination::ProtocolError);
        assert_eq!(parsed.stream_end_signal, StreamEndSignal::OpenAiDone);
        assert_eq!(
            parsed.contract_errors,
            [
                "chat.early_done:/events/1".to_owned(),
                "chat.api_error:/events/2/data/error".to_owned(),
            ]
        );
        assert_eq!(
            parsed.assistant_turn.as_ref().expect("turn").final_text,
            "KEPT"
        );
        assert!(!format!("{parsed:?}").contains("PRIVATE_API_DETAIL"));
    }

    #[tokio::test]
    async fn early_done_seals_the_stream_and_rejects_later_model_data() {
        let body = stream(&[
            chunk(json!([{
                "index": 0,
                "delta": {"role": "assistant", "content": "KEPT"},
                "finish_reason": null
            }])),
            "[DONE]".into(),
            chunk(json!([{
                "index": 0,
                "delta": {"content": "PRIVATE_AFTER_DONE"},
                "finish_reason": "stop"
            }])),
            "[DONE]".into(),
        ]);
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(parsed.stream_termination, StreamTermination::ProtocolError);
        assert_eq!(
            parsed
                .assistant_turn
                .as_ref()
                .expect("partial turn")
                .final_text,
            "KEPT"
        );
        assert!(
            parsed
                .contract_errors
                .contains(&"chat.early_done:/events/1".to_owned())
        );
        assert!(
            parsed
                .contract_errors
                .contains(&"chat.duplicate_done:/events/3".to_owned())
        );
        assert!(!format!("{parsed:?}").contains("PRIVATE_AFTER_DONE"));
    }

    #[tokio::test]
    async fn duplicate_done_is_a_protocol_error() {
        let body = stream(&[
            chunk(json!([{
                "index": 0,
                "delta": {"role": "assistant", "content": "OK"},
                "finish_reason": null
            }])),
            chunk(json!([{"index": 0, "delta": {}, "finish_reason": "stop"}])),
            "[DONE]".into(),
            "[DONE]".into(),
        ]);
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(parsed.stream_termination, StreamTermination::ProtocolError);
        assert!(
            parsed
                .contract_errors
                .contains(&"chat.duplicate_done:/events/3".to_owned())
        );
    }

    #[tokio::test]
    async fn rejects_invalid_or_repeated_usage_only_chunks() {
        let terminal = chunk(json!([{
            "index": 0,
            "delta": {"role": "assistant", "content": "OK"},
            "finish_reason": "stop"
        }]));
        let valid_usage = usage_chunk(Some(json!({"total_tokens": 1})));

        let early =
            parse(stream(&[valid_usage.clone(), terminal.clone(), "[DONE]".into()]).as_bytes())
                .await;
        assert!(
            early
                .contract_errors
                .contains(&"chat.usage_before_finish:/events/0/data/choices".to_owned())
        );

        let missing =
            parse(stream(&[terminal.clone(), usage_chunk(None), "[DONE]".into()]).as_bytes()).await;
        assert!(
            missing
                .contract_errors
                .contains(&"chat.invalid_usage_chunk:/events/1/data/usage".to_owned())
        );

        let empty = parse(
            stream(&[
                terminal.clone(),
                usage_chunk(Some(json!({}))),
                "[DONE]".into(),
            ])
            .as_bytes(),
        )
        .await;
        assert!(
            empty
                .contract_errors
                .contains(&"chat.invalid_usage_chunk:/events/1/data/usage".to_owned())
        );

        let repeated = parse(
            stream(&[terminal, valid_usage.clone(), valid_usage, "[DONE]".into()]).as_bytes(),
        )
        .await;
        assert_eq!(repeated.stream_termination, StreamTermination::Completed);
        assert!(
            repeated
                .contract_errors
                .contains(&"chat.duplicate_usage_chunk:/events/2/data/usage".to_owned())
        );
    }

    #[tokio::test]
    async fn content_chunk_rejects_non_null_usage() {
        let body = stream(&[
            chunk_with_usage(
                json!([{
                    "index": 0,
                    "delta": {"role": "assistant", "content": "OK"},
                    "finish_reason": null
                }]),
                json!({"total_tokens": 1}),
            ),
            chunk(json!([{"index": 0, "delta": {}, "finish_reason": "stop"}])),
            "[DONE]".into(),
        ]);
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert_eq!(
            parsed.contract_errors,
            ["chat.unexpected_usage:/events/0/data/usage".to_owned()]
        );
        assert_eq!(parsed.assistant_turn.expect("turn").final_text, "OK");
    }

    #[tokio::test]
    async fn finish_chunk_rejects_non_null_usage() {
        let body = stream(&[
            chunk(json!([{
                "index": 0,
                "delta": {"role": "assistant", "content": "OK"},
                "finish_reason": null
            }])),
            chunk_with_usage(
                json!([{"index": 0, "delta": {}, "finish_reason": "stop"}]),
                json!({"total_tokens": 1}),
            ),
            "[DONE]".into(),
        ]);
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert_eq!(
            parsed.contract_errors,
            ["chat.unexpected_usage:/events/1/data/usage".to_owned()]
        );
        assert_eq!(parsed.assistant_turn.expect("turn").final_text, "OK");
    }

    #[tokio::test]
    async fn sparse_or_large_tool_indexes_never_produce_executable_calls() {
        let sparse_body = tool_index_stream(&[0, 2]);
        let sparse = parse(sparse_body.as_bytes()).await;
        assert_eq!(sparse.stream_termination, StreamTermination::Completed);
        assert!(
            sparse
                .assistant_turn
                .expect("tool turn")
                .tool_calls
                .is_empty()
        );
        assert!(sparse.contract_errors.contains(
            &"chat.non_contiguous_tool_index:/choices/0/delta/tool_calls/1/index".to_owned()
        ));

        let large_body = tool_index_stream(&[999_999]);
        let large = parse(large_body.as_bytes()).await;
        assert!(
            large
                .assistant_turn
                .expect("tool turn")
                .tool_calls
                .is_empty()
        );
        assert_eq!(
            large.contract_errors,
            ["chat.non_contiguous_tool_index:/choices/0/delta/tool_calls/0/index".to_owned()]
        );
        assert!(
            !large
                .contract_errors
                .iter()
                .any(|error| error.contains("999999"))
        );
    }

    #[tokio::test]
    async fn blank_tool_id_or_name_is_not_executable() {
        let body = stream(&[
            chunk(json!([{
                "index": 0,
                "delta": {"role": "assistant", "tool_calls": [{
                    "index": 0,
                    "id": "   ",
                    "type": "function",
                    "function": {"name": "\t", "arguments": "{}"}
                }]},
                "finish_reason": null
            }])),
            chunk(json!([{"index": 0, "delta": {}, "finish_reason": "tool_calls"}])),
            "[DONE]".into(),
        ]);
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert!(
            parsed
                .assistant_turn
                .expect("tool turn")
                .tool_calls
                .is_empty()
        );
        assert!(
            parsed
                .contract_errors
                .contains(&"chat.empty_tool_id:/choices/0/delta/tool_calls/0/id".to_owned())
        );
        assert!(parsed.contract_errors.contains(
            &"chat.empty_tool_name:/choices/0/delta/tool_calls/0/function/name".to_owned()
        ));
    }

    fn stream(events: &[String]) -> String {
        events
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect()
    }

    fn complete_fixture(fixture: &[u8]) -> Vec<u8> {
        let mut body = fixture.to_vec();
        body.push(b'\n');
        body
    }

    fn named_event(name: &str, data: &str) -> String {
        format!("event: {name}\ndata: {data}\n\n")
    }

    fn usage_chunk(usage: Option<serde_json::Value>) -> String {
        let mut value = json!({
            "id": "chatcmpl-test",
            "object": "chat.completion.chunk",
            "created": 1787041140,
            "model": "gpt-test",
            "choices": []
        });
        if let Some(usage) = usage {
            value["usage"] = usage;
        }
        value.to_string()
    }

    fn chunk(choices: serde_json::Value) -> String {
        json!({
            "id": "chatcmpl-test",
            "object": "chat.completion.chunk",
            "created": 1787041140,
            "model": "gpt-test",
            "choices": choices
        })
        .to_string()
    }

    fn chunk_with_usage(choices: serde_json::Value, usage: serde_json::Value) -> String {
        json!({
            "id": "chatcmpl-test",
            "object": "chat.completion.chunk",
            "created": 1787041140,
            "model": "gpt-test",
            "choices": choices,
            "usage": usage
        })
        .to_string()
    }

    fn tool_conflict_stream(second_id: &str) -> String {
        stream(&[
            chunk(json!([{
                "index": 0,
                "delta": {"role": "assistant", "tool_calls": [{
                    "index": 0,
                    "id": "call_weather_046",
                    "type": "function",
                    "function": {"name": "get_weather", "arguments": "{\"city\":"}
                }]},
                "finish_reason": null
            }])),
            chunk(json!([{
                "index": 0,
                "delta": {"tool_calls": [{
                    "index": 0,
                    "id": second_id,
                    "function": {"arguments": "\"Beijing\"}"}
                }]},
                "finish_reason": null
            }])),
            chunk(json!([{"index": 0, "delta": {}, "finish_reason": "tool_calls"}])),
            "[DONE]".into(),
        ])
    }

    fn tool_index_stream(indexes: &[usize]) -> String {
        let tool_calls = indexes
            .iter()
            .map(|index| {
                json!({
                    "index": index,
                    "id": format!("call_{index}"),
                    "type": "function",
                    "function": {
                        "name": format!("tool_{index}"),
                        "arguments": "{}"
                    }
                })
            })
            .collect::<Vec<_>>();
        stream(&[
            chunk(json!([{
                "index": 0,
                "delta": {"role": "assistant", "tool_calls": tool_calls},
                "finish_reason": null
            }])),
            chunk(json!([{"index": 0, "delta": {}, "finish_reason": "tool_calls"}])),
            "[DONE]".into(),
        ])
    }
}
