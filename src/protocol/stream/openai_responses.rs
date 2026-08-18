use std::collections::BTreeMap;

use serde_json::Value;

use crate::evidence::{StreamEndSignal, StreamTermination};

use super::framing::decode_sse_chunks;
use super::{AssistantTurn, ProtocolHistory, StreamParseResult, ToolCall, ToolCorrelation};

#[derive(Default)]
struct ParserState {
    response_id: Option<String>,
    saw_created: bool,
    last_sequence_number: Option<u64>,
    items: BTreeMap<usize, ItemAccumulator>,
    final_text: String,
    terminal: Option<Terminal>,
    saw_response_completed: bool,
    model_stop_reason: Option<String>,
    contract_errors: Vec<String>,
}

#[derive(Clone, Copy)]
enum Terminal {
    Completed,
    Incomplete,
    ProtocolError,
}

#[derive(Default)]
struct ItemAccumulator {
    id: Option<String>,
    kind: Option<String>,
    call_id: Option<String>,
    name: Option<String>,
    argument_deltas: String,
    done_arguments: Option<String>,
    arguments_done: bool,
    texts: BTreeMap<usize, TextAccumulator>,
    closed: bool,
    invalid: bool,
}

#[derive(Default)]
struct TextAccumulator {
    deltas: String,
    done: bool,
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
        let value = match serde_json::from_str::<Value>(&event.data) {
            Ok(value) => value,
            Err(_) => {
                malformed_stream = true;
                record_event_after_terminal(&mut state, event_index);
                if event.event == "error" {
                    push_error(
                        &mut state.contract_errors,
                        "api_error",
                        &format!("/events/{event_index}"),
                    );
                }
                push_error(
                    &mut state.contract_errors,
                    "malformed_event_json",
                    &format!("/events/{event_index}/data"),
                );
                continue;
            }
        };

        let Some(payload) = value.as_object() else {
            record_event_after_terminal(&mut state, event_index);
            push_error(
                &mut state.contract_errors,
                "invalid_event",
                &format!("/events/{event_index}/data"),
            );
            continue;
        };

        validate_sequence_number(&mut state, payload, event_index);

        let Some(data_type) = payload.get("type").and_then(Value::as_str) else {
            record_event_after_terminal(&mut state, event_index);
            push_error(
                &mut state.contract_errors,
                "invalid_event_type",
                &format!("/events/{event_index}/data/type"),
            );
            continue;
        };

        if event.event.is_empty() || event.event == "message" {
            record_event_after_terminal(&mut state, event_index);
            push_error(
                &mut state.contract_errors,
                "missing_event_name",
                &format!("/events/{event_index}/event"),
            );
            continue;
        }
        if event.event != data_type {
            record_event_after_terminal(&mut state, event_index);
            push_error(
                &mut state.contract_errors,
                "event_type_mismatch",
                &format!("/events/{event_index}/data/type"),
            );
            record_explicit_failure_event(&mut state, &event.event, event_index);
            continue;
        }

        if data_type == "response.completed" {
            state.saw_response_completed = true;
        }

        if state.terminal.is_some() {
            let code = if is_terminal_event(data_type) {
                "duplicate_terminal"
            } else {
                "event_after_terminal"
            };
            push_error(
                &mut state.contract_errors,
                code,
                &format!("/events/{event_index}/event"),
            );
            match data_type {
                "error" => {
                    push_error(
                        &mut state.contract_errors,
                        "api_error",
                        &format!("/events/{event_index}"),
                    );
                    state.terminal = Some(Terminal::ProtocolError);
                }
                "response.failed" => {
                    push_error(
                        &mut state.contract_errors,
                        "response_failed",
                        &format!("/events/{event_index}"),
                    );
                    state.terminal = Some(Terminal::ProtocolError);
                }
                _ => {}
            }
            continue;
        }

        process_event(&mut state, data_type, payload, event_index);
    }

    if decoded.trailing_incomplete_frame {
        malformed_stream = true;
        push_error(
            &mut state.contract_errors,
            "truncated_sse",
            &format!("/events/{event_count}"),
        );
    }

    let assistant_turn = build_assistant_turn(&mut state);
    let stream_termination = if malformed_stream {
        StreamTermination::MalformedStream
    } else {
        match state.terminal {
            Some(Terminal::Completed) => StreamTermination::Completed,
            Some(Terminal::Incomplete) => StreamTermination::ModelIncomplete,
            Some(Terminal::ProtocolError) => StreamTermination::ProtocolError,
            None => StreamTermination::MissingTerminalEvent,
        }
    };
    let stream_end_signal = if state.saw_response_completed {
        StreamEndSignal::OpenAiResponseCompleted
    } else {
        StreamEndSignal::None
    };

    StreamParseResult {
        assistant_turn,
        stream_termination,
        stream_end_signal,
        model_stop_reason: state.model_stop_reason,
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

fn process_event(
    state: &mut ParserState,
    event_type: &str,
    payload: &serde_json::Map<String, Value>,
    event_index: usize,
) {
    match event_type {
        "response.created" => process_created(state, payload, event_index),
        "response.in_progress" | "response.queued" => {
            require_created(state, event_index);
            correlate_response(state, payload, event_index);
        }
        "response.output_item.added" => process_item_added(state, payload, event_index),
        "response.function_call_arguments.delta" => {
            process_argument_delta(state, payload, event_index)
        }
        "response.function_call_arguments.done" => {
            process_arguments_done(state, payload, event_index)
        }
        "response.output_text.delta" => process_text_delta(state, payload, event_index),
        "response.output_text.done" => process_text_done(state, payload, event_index),
        "response.output_item.done" => process_item_done(state, payload, event_index),
        "response.completed" => process_completed(state, payload, event_index),
        "response.incomplete" => process_incomplete(state, payload, event_index),
        "response.failed" => process_failed(state, payload, event_index),
        "error" => {
            push_error(
                &mut state.contract_errors,
                "api_error",
                &format!("/events/{event_index}"),
            );
            state.terminal = Some(Terminal::ProtocolError);
        }
        _ => {}
    }
}

fn process_created(
    state: &mut ParserState,
    payload: &serde_json::Map<String, Value>,
    event_index: usize,
) {
    if event_index != 0 {
        push_error(
            &mut state.contract_errors,
            "created_not_first",
            &format!("/events/{event_index}/event"),
        );
    }
    if state.saw_created {
        push_error(
            &mut state.contract_errors,
            "duplicate_created",
            &format!("/events/{event_index}/event"),
        );
        return;
    }
    if !state.items.is_empty() {
        push_error(
            &mut state.contract_errors,
            "late_created",
            &format!("/events/{event_index}/event"),
        );
    }
    state.saw_created = true;
    correlate_response(state, payload, event_index);
    require_response_status(state, payload, event_index, "in_progress", false);
}

fn process_item_added(
    state: &mut ParserState,
    payload: &serde_json::Map<String, Value>,
    event_index: usize,
) {
    require_created(state, event_index);
    let base = format!("/events/{event_index}/data");
    let Some(output_index) = json_index(payload.get("output_index")) else {
        push_error(
            &mut state.contract_errors,
            "invalid_output_index",
            &format!("{base}/output_index"),
        );
        return;
    };
    let Some(item) = payload.get("item").and_then(Value::as_object) else {
        push_error(
            &mut state.contract_errors,
            "invalid_item",
            &format!("{base}/item"),
        );
        return;
    };
    if state.items.contains_key(&output_index) {
        push_error(
            &mut state.contract_errors,
            "duplicate_output_index",
            &format!("{base}/output_index"),
        );
        return;
    }

    let mut accumulated = ItemAccumulator::default();
    accumulated.id = required_nonempty_string(
        item.get("id"),
        "invalid_item_id",
        &format!("{base}/item/id"),
        &mut accumulated.invalid,
        &mut state.contract_errors,
    );
    accumulated.kind = required_nonempty_string(
        item.get("type"),
        "invalid_item_type",
        &format!("{base}/item/type"),
        &mut accumulated.invalid,
        &mut state.contract_errors,
    );
    if item.get("status").and_then(Value::as_str) != Some("in_progress") {
        accumulated.invalid = true;
        push_error(
            &mut state.contract_errors,
            "invalid_item_status",
            &format!("{base}/item/status"),
        );
    }

    if accumulated.kind.as_deref() == Some("function_call") {
        accumulated.call_id = optional_nonempty_string(
            item.get("call_id"),
            "invalid_call_id",
            &format!("{base}/item/call_id"),
            &mut accumulated.invalid,
            &mut state.contract_errors,
        );
        accumulated.name = optional_nonempty_string(
            item.get("name"),
            "invalid_tool_name",
            &format!("{base}/item/name"),
            &mut accumulated.invalid,
            &mut state.contract_errors,
        );
        match item.get("arguments") {
            Some(Value::String(arguments)) if arguments.is_empty() => {}
            Some(Value::String(_)) => {
                accumulated.invalid = true;
                push_error(
                    &mut state.contract_errors,
                    "nonempty_initial_arguments",
                    &format!("{base}/item/arguments"),
                );
            }
            Some(_) => {
                accumulated.invalid = true;
                push_error(
                    &mut state.contract_errors,
                    "invalid_arguments",
                    &format!("{base}/item/arguments"),
                );
            }
            None => {}
        }
    }

    state.items.insert(output_index, accumulated);
}

fn process_argument_delta(
    state: &mut ParserState,
    payload: &serde_json::Map<String, Value>,
    event_index: usize,
) {
    require_created(state, event_index);
    let base = format!("/events/{event_index}/data");
    let Some(output_index) = event_output_index(state, payload, &base) else {
        return;
    };
    let item_id = payload.get("item_id").and_then(Value::as_str);
    let delta = payload.get("delta").and_then(Value::as_str);
    if item_id.is_none() {
        push_error(
            &mut state.contract_errors,
            "invalid_item_id",
            &format!("{base}/item_id"),
        );
    }
    if delta.is_none() {
        push_error(
            &mut state.contract_errors,
            "invalid_argument_delta",
            &format!("{base}/delta"),
        );
    }

    let (items, errors) = (&mut state.items, &mut state.contract_errors);
    let Some(item) = items.get_mut(&output_index) else {
        push_error(
            errors,
            "unknown_output_index",
            &format!("{base}/output_index"),
        );
        return;
    };
    let correlated = correlate_item_reference(item, item_id, &base, errors);
    if item.kind.as_deref() != Some("function_call") {
        item.invalid = true;
        push_error(errors, "wrong_item_type", &format!("{base}/item_id"));
        return;
    }
    if item.arguments_done || item.closed {
        item.invalid = true;
        push_error(
            errors,
            "argument_delta_after_done",
            &format!("{base}/delta"),
        );
        return;
    }
    if correlated && let Some(delta) = delta {
        item.argument_deltas.push_str(delta);
    }
}

fn process_arguments_done(
    state: &mut ParserState,
    payload: &serde_json::Map<String, Value>,
    event_index: usize,
) {
    require_created(state, event_index);
    let base = format!("/events/{event_index}/data");
    let Some(output_index) = event_output_index(state, payload, &base) else {
        return;
    };
    let item_id = payload.get("item_id").and_then(Value::as_str);
    let name = payload.get("name").and_then(Value::as_str);
    let arguments = payload.get("arguments").and_then(Value::as_str);
    if item_id.is_none() {
        push_error(
            &mut state.contract_errors,
            "invalid_item_id",
            &format!("{base}/item_id"),
        );
    }
    if name.is_none_or(|value| value.trim().is_empty()) {
        push_error(
            &mut state.contract_errors,
            "invalid_tool_name",
            &format!("{base}/name"),
        );
    }
    if arguments.is_none() {
        push_error(
            &mut state.contract_errors,
            "invalid_arguments",
            &format!("{base}/arguments"),
        );
    }

    let (items, errors) = (&mut state.items, &mut state.contract_errors);
    let Some(item) = items.get_mut(&output_index) else {
        push_error(
            errors,
            "unknown_output_index",
            &format!("{base}/output_index"),
        );
        return;
    };
    let correlated = correlate_item_reference(item, item_id, &base, errors);
    if item.kind.as_deref() != Some("function_call") {
        item.invalid = true;
        push_error(errors, "wrong_item_type", &format!("{base}/item_id"));
        return;
    }
    if item.arguments_done {
        item.invalid = true;
        push_error(
            errors,
            "duplicate_arguments_done",
            &format!("{base}/arguments"),
        );
        return;
    }
    item.arguments_done = true;
    if !correlated {
        item.invalid = true;
    }
    merge_identity(
        &mut item.name,
        name,
        "conflicting_tool_name",
        &format!("{base}/name"),
        &mut item.invalid,
        errors,
    );
    if let Some(arguments) = arguments {
        if item.argument_deltas != arguments {
            item.invalid = true;
            push_error(errors, "arguments_mismatch", &format!("{base}/arguments"));
        }
        item.done_arguments = Some(arguments.to_owned());
    } else {
        item.invalid = true;
    }
}

fn process_text_delta(
    state: &mut ParserState,
    payload: &serde_json::Map<String, Value>,
    event_index: usize,
) {
    require_created(state, event_index);
    let base = format!("/events/{event_index}/data");
    let Some(output_index) = event_output_index(state, payload, &base) else {
        return;
    };
    let Some(content_index) = content_index(state, payload, &base) else {
        return;
    };
    let item_id = payload.get("item_id").and_then(Value::as_str);
    let delta = payload.get("delta").and_then(Value::as_str);
    if item_id.is_none() {
        push_error(
            &mut state.contract_errors,
            "invalid_item_id",
            &format!("{base}/item_id"),
        );
    }
    if delta.is_none() {
        push_error(
            &mut state.contract_errors,
            "invalid_text_delta",
            &format!("{base}/delta"),
        );
    }

    let (items, final_text, errors) = (
        &mut state.items,
        &mut state.final_text,
        &mut state.contract_errors,
    );
    let Some(item) = items.get_mut(&output_index) else {
        push_error(
            errors,
            "unknown_output_index",
            &format!("{base}/output_index"),
        );
        return;
    };
    let correlated = correlate_item_reference(item, item_id, &base, errors);
    if item.kind.as_deref() != Some("message") {
        item.invalid = true;
        push_error(errors, "wrong_item_type", &format!("{base}/item_id"));
        return;
    }
    if item.closed {
        item.invalid = true;
        push_error(errors, "text_after_item_done", &format!("{base}/delta"));
        return;
    }
    let text = item.texts.entry(content_index).or_default();
    if text.done {
        item.invalid = true;
        push_error(errors, "text_delta_after_done", &format!("{base}/delta"));
        return;
    }
    if correlated && let Some(delta) = delta {
        text.deltas.push_str(delta);
        final_text.push_str(delta);
    }
}

fn process_text_done(
    state: &mut ParserState,
    payload: &serde_json::Map<String, Value>,
    event_index: usize,
) {
    require_created(state, event_index);
    let base = format!("/events/{event_index}/data");
    let Some(output_index) = event_output_index(state, payload, &base) else {
        return;
    };
    let Some(content_index) = content_index(state, payload, &base) else {
        return;
    };
    let item_id = payload.get("item_id").and_then(Value::as_str);
    let text_value = payload.get("text").and_then(Value::as_str);
    if item_id.is_none() {
        push_error(
            &mut state.contract_errors,
            "invalid_item_id",
            &format!("{base}/item_id"),
        );
    }
    if text_value.is_none() {
        push_error(
            &mut state.contract_errors,
            "invalid_text",
            &format!("{base}/text"),
        );
    }

    let (items, errors) = (&mut state.items, &mut state.contract_errors);
    let Some(item) = items.get_mut(&output_index) else {
        push_error(
            errors,
            "unknown_output_index",
            &format!("{base}/output_index"),
        );
        return;
    };
    let correlated = correlate_item_reference(item, item_id, &base, errors);
    if item.kind.as_deref() != Some("message") {
        item.invalid = true;
        push_error(errors, "wrong_item_type", &format!("{base}/item_id"));
        return;
    }
    let text = item.texts.entry(content_index).or_default();
    if text.done {
        item.invalid = true;
        push_error(errors, "duplicate_text_done", &format!("{base}/text"));
        return;
    }
    text.done = true;
    if !correlated {
        item.invalid = true;
    }
    if let Some(text_value) = text_value
        && text.deltas != text_value
    {
        item.invalid = true;
        push_error(errors, "text_mismatch", &format!("{base}/text"));
    }
}

fn process_item_done(
    state: &mut ParserState,
    payload: &serde_json::Map<String, Value>,
    event_index: usize,
) {
    require_created(state, event_index);
    let base = format!("/events/{event_index}/data");
    let Some(output_index) = event_output_index(state, payload, &base) else {
        return;
    };
    let Some(snapshot) = payload.get("item").and_then(Value::as_object) else {
        push_error(
            &mut state.contract_errors,
            "invalid_item",
            &format!("{base}/item"),
        );
        return;
    };

    let (items, errors) = (&mut state.items, &mut state.contract_errors);
    let Some(item) = items.get_mut(&output_index) else {
        push_error(
            errors,
            "unknown_output_index",
            &format!("{base}/output_index"),
        );
        return;
    };
    if item.closed {
        item.invalid = true;
        push_error(errors, "duplicate_item_done", &format!("{base}/item"));
        return;
    }
    correlate_snapshot(item, snapshot, &format!("{base}/item"), errors);
    if snapshot.get("status").and_then(Value::as_str) != Some("completed") {
        item.invalid = true;
        push_error(
            errors,
            "invalid_item_status",
            &format!("{base}/item/status"),
        );
    }

    if item.kind.as_deref() == Some("function_call") {
        let snapshot_arguments = snapshot.get("arguments").and_then(Value::as_str);
        merge_identity(
            &mut item.call_id,
            snapshot.get("call_id").and_then(Value::as_str),
            "conflicting_call_id",
            &format!("{base}/item/call_id"),
            &mut item.invalid,
            errors,
        );
        merge_identity(
            &mut item.name,
            snapshot.get("name").and_then(Value::as_str),
            "conflicting_tool_name",
            &format!("{base}/item/name"),
            &mut item.invalid,
            errors,
        );
        if !item.arguments_done {
            item.invalid = true;
            push_error(
                errors,
                "missing_arguments_done",
                &format!("{base}/item/arguments"),
            );
        }
        if snapshot_arguments.is_none() {
            item.invalid = true;
            push_error(
                errors,
                "invalid_arguments",
                &format!("{base}/item/arguments"),
            );
        } else if item.done_arguments.as_deref() != snapshot_arguments {
            item.invalid = true;
            push_error(
                errors,
                "arguments_mismatch",
                &format!("{base}/item/arguments"),
            );
        }
    } else if item.kind.as_deref() == Some("message") && item.texts.values().any(|text| !text.done)
    {
        item.invalid = true;
        push_error(errors, "text_not_done", &format!("{base}/item/content"));
    }
    item.closed = true;
}

fn process_completed(
    state: &mut ParserState,
    payload: &serde_json::Map<String, Value>,
    event_index: usize,
) {
    require_created(state, event_index);
    correlate_response(state, payload, event_index);
    let status_is_completed =
        require_response_status(state, payload, event_index, "completed", true);
    validate_terminal_items(state, payload, event_index);
    if status_is_completed {
        state.model_stop_reason = Some("completed".into());
        state.terminal = Some(Terminal::Completed);
    } else {
        state.terminal = Some(Terminal::ProtocolError);
    }
}

fn process_incomplete(
    state: &mut ParserState,
    payload: &serde_json::Map<String, Value>,
    event_index: usize,
) {
    require_created(state, event_index);
    correlate_response(state, payload, event_index);
    if require_response_status(state, payload, event_index, "incomplete", true) {
        state.model_stop_reason = Some("incomplete".into());
        state.terminal = Some(Terminal::Incomplete);
    } else {
        state.terminal = Some(Terminal::ProtocolError);
    }
}

fn process_failed(
    state: &mut ParserState,
    payload: &serde_json::Map<String, Value>,
    event_index: usize,
) {
    require_created(state, event_index);
    correlate_response(state, payload, event_index);
    let status_is_failed = require_response_status(state, payload, event_index, "failed", true);
    if status_is_failed {
        state.model_stop_reason = Some("failed".into());
    }
    push_error(
        &mut state.contract_errors,
        "response_failed",
        &format!("/events/{event_index}/data/response/status"),
    );
    state.terminal = Some(Terminal::ProtocolError);
}

fn correlate_response(
    state: &mut ParserState,
    payload: &serde_json::Map<String, Value>,
    event_index: usize,
) {
    let base = format!("/events/{event_index}/data/response");
    let Some(response) = payload.get("response").and_then(Value::as_object) else {
        push_error(&mut state.contract_errors, "invalid_response", &base);
        return;
    };
    let id = response.get("id").and_then(Value::as_str);
    if id.is_none_or(|value| value.trim().is_empty()) {
        push_error(
            &mut state.contract_errors,
            "invalid_response_id",
            &format!("{base}/id"),
        );
        return;
    }
    let id = id.expect("non-empty response id");
    match &state.response_id {
        None => state.response_id = Some(id.to_owned()),
        Some(expected) if expected != id => push_error(
            &mut state.contract_errors,
            "conflicting_response_id",
            &format!("{base}/id"),
        ),
        Some(_) => {}
    }
}

fn require_response_status(
    state: &mut ParserState,
    payload: &serde_json::Map<String, Value>,
    event_index: usize,
    expected: &str,
    terminal: bool,
) -> bool {
    let valid = payload
        .get("response")
        .and_then(Value::as_object)
        .and_then(|response| response.get("status"))
        .and_then(Value::as_str)
        == Some(expected);
    if !valid {
        push_error(
            &mut state.contract_errors,
            if terminal {
                "invalid_terminal_status"
            } else {
                "invalid_response_status"
            },
            &format!("/events/{event_index}/data/response/status"),
        );
    }
    valid
}

fn validate_terminal_items(
    state: &mut ParserState,
    payload: &serde_json::Map<String, Value>,
    event_index: usize,
) {
    let base = format!("/events/{event_index}/data/response/output");
    let output = payload
        .get("response")
        .and_then(Value::as_object)
        .and_then(|response| response.get("output"))
        .and_then(Value::as_array);
    let Some(output) = output else {
        push_error(&mut state.contract_errors, "invalid_terminal_output", &base);
        return;
    };

    let (items, errors) = (&mut state.items, &mut state.contract_errors);
    for (output_index, item) in items.iter_mut() {
        if !item.closed {
            item.invalid = true;
            push_error(errors, "unclosed_output_item", &base);
        }
        let Some(snapshot) = output.get(*output_index).and_then(Value::as_object) else {
            item.invalid = true;
            push_error(errors, "missing_terminal_item", &base);
            continue;
        };
        correlate_snapshot(item, snapshot, &format!("{base}/{output_index}"), errors);
        if item.kind.as_deref() == Some("function_call") {
            compare_identity(
                item.call_id.as_deref(),
                snapshot.get("call_id").and_then(Value::as_str),
                "conflicting_call_id",
                &format!("{base}/{output_index}/call_id"),
                &mut item.invalid,
                errors,
            );
            compare_identity(
                item.name.as_deref(),
                snapshot.get("name").and_then(Value::as_str),
                "conflicting_tool_name",
                &format!("{base}/{output_index}/name"),
                &mut item.invalid,
                errors,
            );
            compare_identity(
                item.done_arguments.as_deref(),
                snapshot.get("arguments").and_then(Value::as_str),
                "arguments_mismatch",
                &format!("{base}/{output_index}/arguments"),
                &mut item.invalid,
                errors,
            );
        }
    }
    if output.len() != items.len() {
        push_error(errors, "terminal_output_mismatch", &base);
    }
}

fn correlate_snapshot(
    item: &mut ItemAccumulator,
    snapshot: &serde_json::Map<String, Value>,
    base: &str,
    errors: &mut Vec<String>,
) {
    compare_identity(
        item.id.as_deref(),
        snapshot.get("id").and_then(Value::as_str),
        "conflicting_item_id",
        &format!("{base}/id"),
        &mut item.invalid,
        errors,
    );
    compare_identity(
        item.kind.as_deref(),
        snapshot.get("type").and_then(Value::as_str),
        "conflicting_item_type",
        &format!("{base}/type"),
        &mut item.invalid,
        errors,
    );
}

fn correlate_item_reference(
    item: &mut ItemAccumulator,
    incoming: Option<&str>,
    base: &str,
    errors: &mut Vec<String>,
) -> bool {
    let valid = incoming.is_some_and(|incoming| item.id.as_deref() == Some(incoming));
    if !valid {
        item.invalid = true;
        push_error(errors, "conflicting_item_id", &format!("{base}/item_id"));
    }
    valid
}

fn merge_identity(
    expected: &mut Option<String>,
    incoming: Option<&str>,
    code: &str,
    path: &str,
    invalid: &mut bool,
    errors: &mut Vec<String>,
) {
    match (expected.as_deref(), incoming) {
        (None, Some(value)) if !value.trim().is_empty() => *expected = Some(value.to_owned()),
        (Some(expected), Some(value)) if expected == value => {}
        _ => {
            *invalid = true;
            push_error(errors, code, path);
        }
    }
}

fn compare_identity(
    expected: Option<&str>,
    incoming: Option<&str>,
    code: &str,
    path: &str,
    invalid: &mut bool,
    errors: &mut Vec<String>,
) {
    if expected.is_none() || expected != incoming {
        *invalid = true;
        push_error(errors, code, path);
    }
}

fn required_nonempty_string(
    value: Option<&Value>,
    code: &str,
    path: &str,
    invalid: &mut bool,
    errors: &mut Vec<String>,
) -> Option<String> {
    match value.and_then(Value::as_str) {
        Some(value) if !value.trim().is_empty() => Some(value.to_owned()),
        _ => {
            *invalid = true;
            push_error(errors, code, path);
            None
        }
    }
}

fn optional_nonempty_string(
    value: Option<&Value>,
    code: &str,
    path: &str,
    invalid: &mut bool,
    errors: &mut Vec<String>,
) -> Option<String> {
    match value {
        None => None,
        Some(Value::String(value)) if !value.trim().is_empty() => Some(value.clone()),
        Some(_) => {
            *invalid = true;
            push_error(errors, code, path);
            None
        }
    }
}

fn event_output_index(
    state: &mut ParserState,
    payload: &serde_json::Map<String, Value>,
    base: &str,
) -> Option<usize> {
    let index = json_index(payload.get("output_index"));
    if index.is_none() {
        push_error(
            &mut state.contract_errors,
            "invalid_output_index",
            &format!("{base}/output_index"),
        );
    }
    index
}

fn content_index(
    state: &mut ParserState,
    payload: &serde_json::Map<String, Value>,
    base: &str,
) -> Option<usize> {
    let index = json_index(payload.get("content_index"));
    if index.is_none() {
        push_error(
            &mut state.contract_errors,
            "invalid_content_index",
            &format!("{base}/content_index"),
        );
    }
    index
}

fn require_created(state: &mut ParserState, event_index: usize) {
    if !state.saw_created {
        push_error(
            &mut state.contract_errors,
            "event_before_created",
            &format!("/events/{event_index}/event"),
        );
    }
}

fn validate_sequence_number(
    state: &mut ParserState,
    payload: &serde_json::Map<String, Value>,
    event_index: usize,
) {
    let path = format!("/events/{event_index}/data/sequence_number");
    let Some(sequence_number) = payload.get("sequence_number").and_then(Value::as_u64) else {
        push_error(&mut state.contract_errors, "invalid_sequence_number", &path);
        return;
    };
    if state
        .last_sequence_number
        .is_some_and(|previous| sequence_number <= previous)
    {
        push_error(
            &mut state.contract_errors,
            "non_increasing_sequence_number",
            &path,
        );
        return;
    }
    state.last_sequence_number = Some(sequence_number);
}

fn record_event_after_terminal(state: &mut ParserState, event_index: usize) {
    if state.terminal.is_some() {
        push_error(
            &mut state.contract_errors,
            "event_after_terminal",
            &format!("/events/{event_index}/event"),
        );
    }
}

fn record_explicit_failure_event(state: &mut ParserState, event_name: &str, event_index: usize) {
    let code = match event_name {
        "error" => "api_error",
        "response.failed" => "response_failed",
        _ => return,
    };
    push_error(
        &mut state.contract_errors,
        code,
        &format!("/events/{event_index}"),
    );
    state.terminal = Some(Terminal::ProtocolError);
}

fn build_assistant_turn(state: &mut ParserState) -> Option<AssistantTurn> {
    let response_id = state.response_id.clone()?;
    let mut tool_calls = Vec::new();
    let mut finalization_errors = Vec::new();

    for (position, (output_index, item)) in state.items.iter().enumerate() {
        if item.kind.as_deref() != Some("function_call") {
            continue;
        }
        let base = format!("/output/{position}");
        let call_id_valid = item
            .call_id
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        let name_valid = item
            .name
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        if !call_id_valid {
            finalization_errors.push(contract_error(
                "missing_call_id",
                &format!("{base}/call_id"),
            ));
        }
        if !name_valid {
            finalization_errors.push(contract_error("missing_tool_name", &format!("{base}/name")));
        }
        let arguments = item
            .done_arguments
            .as_deref()
            .and_then(|arguments| serde_json::from_str::<Value>(arguments).ok());
        if item.done_arguments.is_some()
            && arguments.as_ref().is_none_or(|value| !value.is_object())
        {
            finalization_errors.push(contract_error(
                "invalid_arguments",
                &format!("{base}/arguments"),
            ));
        }

        if !item.invalid
            && item.closed
            && item.arguments_done
            && call_id_valid
            && name_valid
            && let (Some(call_id), Some(name), Some(arguments)) =
                (&item.call_id, &item.name, arguments)
            && arguments.is_object()
        {
            tool_calls.push(ToolCall {
                index: *output_index,
                correlation: ToolCorrelation::Required(call_id.clone()),
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

    Some(AssistantTurn {
        history: ProtocolHistory::OpenAiResponses { response_id },
        tool_calls,
        final_text: state.final_text.clone(),
    })
}

fn json_index(value: Option<&Value>) -> Option<usize> {
    usize::try_from(value?.as_u64()?).ok()
}

fn is_terminal_event(event_type: &str) -> bool {
    matches!(
        event_type,
        "response.completed" | "response.incomplete" | "response.failed" | "error"
    )
}

fn contract_error(code: &str, path: &str) -> String {
    debug_assert!(path.starts_with('/'));
    format!("responses.{code}:{path}")
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

    const TOOL_STREAM: &[u8] = include_bytes!("../fixtures/openai_responses_tool.sse");
    const FINAL_STREAM: &[u8] = include_bytes!("../fixtures/openai_responses_final.sse");
    const MISSING_TERMINAL: &[u8] =
        include_bytes!("../fixtures/openai_responses_missing_terminal.sse");
    const TRUNCATED: &[u8] = include_bytes!("../fixtures/openai_responses_truncated.sse");
    const API_ERROR: &[u8] = include_bytes!("../fixtures/openai_responses_error.sse");
    const INCOMPLETE: &[u8] = include_bytes!("../fixtures/openai_responses_incomplete.sse");
    const BAD_CORRELATION: &[u8] =
        include_bytes!("../fixtures/openai_responses_bad_correlation.sse");

    #[tokio::test]
    async fn parses_official_function_call_sequence_and_preserves_response_id() {
        let parsed = parse(&complete_fixture(TOOL_STREAM)).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert_eq!(
            parsed.stream_end_signal,
            StreamEndSignal::OpenAiResponseCompleted
        );
        assert_eq!(parsed.model_stop_reason.as_deref(), Some("completed"));
        assert_eq!(parsed.event_count, 7);
        assert!(parsed.contract_errors.is_empty());
        assert_eq!(
            parsed.assistant_turn,
            Some(AssistantTurn {
                history: ProtocolHistory::OpenAiResponses {
                    response_id: "resp_tool_046".into(),
                },
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
    async fn accumulates_official_output_text_events() {
        let parsed = parse(&complete_fixture(FINAL_STREAM)).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert_eq!(parsed.model_stop_reason.as_deref(), Some("completed"));
        assert_eq!(parsed.event_count, 9);
        assert!(parsed.contract_errors.is_empty());
        assert_eq!(
            parsed.assistant_turn,
            Some(AssistantTurn {
                history: ProtocolHistory::OpenAiResponses {
                    response_id: "resp_final_046".into(),
                },
                tool_calls: vec![],
                final_text: "MODEL_DOCTOR_CASE_046_OK".into(),
            })
        );
    }

    #[tokio::test]
    async fn classifies_fixture_failures_without_leaking_model_values() {
        let missing = parse(&complete_fixture(MISSING_TERMINAL)).await;
        assert_eq!(
            missing.stream_termination,
            StreamTermination::MissingTerminalEvent
        );
        assert_eq!(missing.stream_end_signal, StreamEndSignal::None);
        assert_eq!(missing.event_count, 7);
        assert!(missing.contract_errors.is_empty());

        let truncated = parse(TRUNCATED).await;
        assert_eq!(
            truncated.stream_termination,
            StreamTermination::MalformedStream
        );
        assert_eq!(
            truncated.contract_errors,
            ["responses.truncated_sse:/events/1".to_owned()]
        );

        let error = parse(&complete_fixture(API_ERROR)).await;
        assert_eq!(error.stream_termination, StreamTermination::ProtocolError);
        assert_eq!(
            error.contract_errors,
            ["responses.api_error:/events/0".to_owned()]
        );
        assert!(!format!("{error:?}").contains("sensitive upstream detail"));

        let incomplete = parse(&complete_fixture(INCOMPLETE)).await;
        assert_eq!(
            incomplete.stream_termination,
            StreamTermination::ModelIncomplete
        );
        assert_eq!(incomplete.model_stop_reason.as_deref(), Some("incomplete"));
    }

    #[tokio::test]
    async fn completed_wire_with_bad_correlation_stays_completed_and_non_executable() {
        let parsed = parse(&complete_fixture(BAD_CORRELATION)).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert_eq!(
            parsed.stream_end_signal,
            StreamEndSignal::OpenAiResponseCompleted
        );
        assert!(
            parsed
                .contract_errors
                .contains(&"responses.conflicting_item_id:/events/2/data/item_id".to_owned())
        );
        assert!(
            parsed
                .assistant_turn
                .as_ref()
                .expect("native history")
                .tool_calls
                .is_empty()
        );
        assert!(!format!("{parsed:?}").contains("private-wrong-item-id"));
    }

    #[tokio::test]
    async fn validates_event_names_and_every_sequence_number() {
        let body = format!(
            "{}{}{}",
            response_event(
                "response.created",
                json!({
                    "type": "response.created",
                    "response": {"id": "resp_sequence", "status": "in_progress"},
                    "sequence_number": 10
                }),
            ),
            response_event(
                "response.future_extension",
                json!({
                    "type": "private-mismatched-type",
                    "sequence_number": 10
                }),
            ),
            response_event(
                "response.completed",
                json!({
                    "type": "response.completed",
                    "response": {"id": "resp_sequence", "status": "completed", "output": []},
                    "sequence_number": 11
                }),
            ),
        );
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        for expected in [
            "responses.event_type_mismatch:/events/1/data/type",
            "responses.non_increasing_sequence_number:/events/1/data/sequence_number",
        ] {
            assert!(
                parsed.contract_errors.contains(&expected.to_owned()),
                "missing {expected}: {:?}",
                parsed.contract_errors
            );
        }
        assert!(!format!("{parsed:?}").contains("private-mismatched-type"));
    }

    #[tokio::test]
    async fn accepts_a_correlated_extension_but_requires_created_as_first_frame() {
        let extension = response_event(
            "response.future_extension",
            json!({"type": "response.future_extension", "sequence_number": 11}),
        );
        let valid = format!(
            "{}{}{}",
            response_event(
                "response.created",
                json!({
                    "type": "response.created",
                    "response": {"id": "resp_extension", "status": "in_progress"},
                    "sequence_number": 10
                }),
            ),
            extension,
            response_event(
                "response.completed",
                json!({
                    "type": "response.completed",
                    "response": {"id": "resp_extension", "status": "completed", "output": []},
                    "sequence_number": 12
                }),
            ),
        );
        let parsed = parse(valid.as_bytes()).await;
        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert!(parsed.contract_errors.is_empty());

        let late_created = format!(
            "{}{}{}",
            response_event(
                "response.future_extension",
                json!({"type": "response.future_extension", "sequence_number": 1}),
            ),
            response_event(
                "response.created",
                json!({
                    "type": "response.created",
                    "response": {"id": "resp_late", "status": "in_progress"},
                    "sequence_number": 2
                }),
            ),
            response_event(
                "response.completed",
                json!({
                    "type": "response.completed",
                    "response": {"id": "resp_late", "status": "completed", "output": []},
                    "sequence_number": 3
                }),
            ),
        );
        let parsed = parse(late_created.as_bytes()).await;
        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert!(
            parsed
                .contract_errors
                .contains(&"responses.created_not_first:/events/1/event".to_owned())
        );
    }

    #[tokio::test]
    async fn requires_done_arguments_to_equal_all_delta_fragments() {
        let fixture = std::str::from_utf8(TOOL_STREAM).expect("fixture is UTF-8");
        let changed = fixture.replace(
            r#""arguments":"{\"city\":\"Beijing\"}","sequence_number":4"#,
            r#""arguments":"{}","sequence_number":4"#,
        );
        assert_ne!(changed, fixture, "the done event must be mutated");
        let parsed = parse(&complete_fixture(changed.as_bytes())).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert!(
            parsed
                .contract_errors
                .contains(&"responses.arguments_mismatch:/events/4/data/arguments".to_owned())
        );
        assert!(
            parsed
                .assistant_turn
                .expect("history")
                .tool_calls
                .is_empty()
        );
    }

    #[tokio::test]
    async fn distinguishes_invalid_completed_status_and_failed_terminals() {
        let invalid_completed = terminal_stream("response.completed", "in_progress", 2);
        let parsed = parse(invalid_completed.as_bytes()).await;
        assert_eq!(parsed.stream_termination, StreamTermination::ProtocolError);
        assert_eq!(
            parsed.stream_end_signal,
            StreamEndSignal::OpenAiResponseCompleted
        );
        assert!(parsed.contract_errors.contains(
            &"responses.invalid_terminal_status:/events/1/data/response/status".to_owned()
        ));

        for (event, wrong_status) in [
            ("response.incomplete", "completed"),
            ("response.failed", "in_progress"),
        ] {
            let parsed = parse(terminal_stream(event, wrong_status, 2).as_bytes()).await;
            assert_eq!(parsed.stream_termination, StreamTermination::ProtocolError);
            assert_eq!(parsed.model_stop_reason, None);
            assert!(parsed.contract_errors.contains(
                &"responses.invalid_terminal_status:/events/1/data/response/status".to_owned()
            ));
        }

        let failed = terminal_stream("response.failed", "failed", 2);
        let parsed = parse(failed.as_bytes()).await;
        assert_eq!(parsed.stream_termination, StreamTermination::ProtocolError);
        assert_eq!(parsed.model_stop_reason.as_deref(), Some("failed"));
        assert!(
            parsed
                .contract_errors
                .contains(&"responses.response_failed:/events/1/data/response/status".to_owned())
        );
    }

    #[tokio::test]
    async fn terminal_seals_state_but_malformed_json_keeps_precedence() {
        let mut body = terminal_stream("response.completed", "completed", 2);
        body.push_str(&response_event(
            "response.output_text.delta",
            json!({
                "type": "response.output_text.delta",
                "item_id": "private-item-after-terminal",
                "output_index": 0,
                "content_index": 0,
                "delta": "PRIVATE_TEXT_AFTER_TERMINAL",
                "sequence_number": 3
            }),
        ));
        let parsed = parse(body.as_bytes()).await;
        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert!(
            parsed
                .contract_errors
                .contains(&"responses.event_after_terminal:/events/2/event".to_owned())
        );
        assert!(!format!("{parsed:?}").contains("PRIVATE_TEXT_AFTER_TERMINAL"));

        let mut malformed = terminal_stream("response.completed", "completed", 2);
        malformed.push_str("event: response.future\ndata: {PRIVATE_BAD_JSON}\n\n");
        let parsed = parse(malformed.as_bytes()).await;
        assert_eq!(
            parsed.stream_termination,
            StreamTermination::MalformedStream
        );
        assert_eq!(
            parsed.stream_end_signal,
            StreamEndSignal::OpenAiResponseCompleted
        );
        assert!(
            parsed
                .contract_errors
                .contains(&"responses.event_after_terminal:/events/2/event".to_owned())
        );
        assert!(!format!("{parsed:?}").contains("PRIVATE_BAD_JSON"));
    }

    #[tokio::test]
    async fn later_explicit_failures_upgrade_the_first_terminal_without_absorbing_payloads() {
        for (event, payload) in [
            (
                "error",
                json!({
                    "type": "error",
                    "code": "private-error-code",
                    "message": "PRIVATE_ERROR_AFTER_COMPLETED",
                    "param": null,
                    "sequence_number": 3
                }),
            ),
            (
                "response.failed",
                json!({
                    "type": "response.failed",
                    "response": {
                        "id": "private-response-id-after-completed",
                        "status": "failed"
                    },
                    "sequence_number": 3
                }),
            ),
        ] {
            let mut body = terminal_stream("response.completed", "completed", 2);
            body.push_str(&response_event(event, payload));
            let parsed = parse(body.as_bytes()).await;

            assert_eq!(parsed.stream_termination, StreamTermination::ProtocolError);
            assert_eq!(
                parsed.stream_end_signal,
                StreamEndSignal::OpenAiResponseCompleted
            );
            assert_eq!(parsed.model_stop_reason.as_deref(), Some("completed"));
            assert!(
                parsed
                    .contract_errors
                    .contains(&"responses.duplicate_terminal:/events/2/event".to_owned())
            );
            let debug = format!("{parsed:?}");
            assert!(!debug.contains("PRIVATE_ERROR_AFTER_COMPLETED"));
            assert!(!debug.contains("private-response-id-after-completed"));
        }
    }

    #[tokio::test]
    async fn mismatched_explicit_failure_event_names_still_classify_protocol_error() {
        for (event, stable_error) in [
            ("error", "responses.api_error:/events/0"),
            ("response.failed", "responses.response_failed:/events/0"),
        ] {
            let body = response_event(
                event,
                json!({
                    "type": "private-mismatched-type",
                    "message": "PRIVATE_STANDALONE_FAILURE",
                    "response": {
                        "id": "private-standalone-response-id",
                        "status": "failed"
                    },
                    "sequence_number": 1
                }),
            );
            let parsed = parse(body.as_bytes()).await;

            assert_eq!(parsed.stream_termination, StreamTermination::ProtocolError);
            assert_eq!(parsed.stream_end_signal, StreamEndSignal::None);
            assert!(
                parsed
                    .contract_errors
                    .contains(&"responses.event_type_mismatch:/events/0/data/type".to_owned())
            );
            assert!(parsed.contract_errors.contains(&stable_error.to_owned()));
            let debug = format!("{parsed:?}");
            assert!(!debug.contains("PRIVATE_STANDALONE_FAILURE"));
            assert!(!debug.contains("private-standalone-response-id"));
            assert!(!debug.contains("private-mismatched-type"));
        }
    }

    #[tokio::test]
    async fn mismatched_failure_after_completed_upgrades_without_losing_completed_signal() {
        for (event, stable_error) in [
            ("error", "responses.api_error:/events/2"),
            ("response.failed", "responses.response_failed:/events/2"),
        ] {
            let mut body = terminal_stream("response.completed", "completed", 2);
            body.push_str(&response_event(
                event,
                json!({
                    "type": "private-mismatched-type",
                    "message": "PRIVATE_FAILURE_AFTER_COMPLETED",
                    "response": {
                        "id": "private-post-completed-response-id",
                        "status": "failed"
                    },
                    "sequence_number": 3
                }),
            ));
            let parsed = parse(body.as_bytes()).await;

            assert_eq!(parsed.stream_termination, StreamTermination::ProtocolError);
            assert_eq!(
                parsed.stream_end_signal,
                StreamEndSignal::OpenAiResponseCompleted
            );
            assert_eq!(parsed.model_stop_reason.as_deref(), Some("completed"));
            assert!(
                parsed
                    .contract_errors
                    .contains(&"responses.event_type_mismatch:/events/2/data/type".to_owned())
            );
            assert!(parsed.contract_errors.contains(&stable_error.to_owned()));
            let debug = format!("{parsed:?}");
            assert!(!debug.contains("PRIVATE_FAILURE_AFTER_COMPLETED"));
            assert!(!debug.contains("private-post-completed-response-id"));
            assert!(!debug.contains("private-mismatched-type"));
        }
    }

    #[tokio::test]
    async fn forged_failure_data_type_does_not_override_a_non_failure_event_name() {
        let body = response_event(
            "response.future_extension",
            json!({
                "type": "error",
                "message": "PRIVATE_FORGED_ERROR",
                "sequence_number": 1
            }),
        );
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(
            parsed.stream_termination,
            StreamTermination::MissingTerminalEvent
        );
        assert_eq!(
            parsed.contract_errors,
            ["responses.event_type_mismatch:/events/0/data/type".to_owned()]
        );
        assert!(!format!("{parsed:?}").contains("PRIVATE_FORGED_ERROR"));
    }

    #[tokio::test]
    async fn completed_signal_is_observed_even_when_it_follows_another_terminal() {
        let mut body = terminal_stream("response.incomplete", "incomplete", 2);
        body.push_str(&response_event(
            "response.completed",
            json!({
                "type": "response.completed",
                "response": {
                    "id": "private-completed-id-after-incomplete",
                    "status": "completed",
                    "output": []
                },
                "sequence_number": 3
            }),
        ));
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(
            parsed.stream_termination,
            StreamTermination::ModelIncomplete
        );
        assert_eq!(
            parsed.stream_end_signal,
            StreamEndSignal::OpenAiResponseCompleted
        );
        assert_eq!(parsed.model_stop_reason.as_deref(), Some("incomplete"));
        assert!(
            parsed
                .contract_errors
                .contains(&"responses.duplicate_terminal:/events/2/event".to_owned())
        );
        assert!(!format!("{parsed:?}").contains("private-completed-id-after-incomplete"));
    }

    #[tokio::test]
    async fn large_sparse_output_indexes_do_not_allocate_or_leak_into_errors() {
        let body = format!(
            "{}{}{}",
            response_event(
                "response.created",
                json!({
                    "type": "response.created",
                    "response": {"id": "resp_sparse", "status": "in_progress"},
                    "sequence_number": 1
                }),
            ),
            response_event(
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": 999999,
                    "item": {
                        "id": "msg_sparse",
                        "type": "message",
                        "role": "assistant",
                        "content": [],
                        "status": "in_progress"
                    },
                    "sequence_number": 2
                }),
            ),
            response_event(
                "response.completed",
                json!({
                    "type": "response.completed",
                    "response": {"id": "resp_sparse", "status": "completed", "output": []},
                    "sequence_number": 3
                }),
            ),
        );
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert!(!format!("{:?}", parsed.contract_errors).contains("999999"));
    }

    fn complete_fixture(fixture: &[u8]) -> Vec<u8> {
        let mut body = fixture.to_vec();
        body.push(b'\n');
        body
    }

    fn response_event(name: &str, data: serde_json::Value) -> String {
        format!("event: {name}\ndata: {data}\n\n")
    }

    fn terminal_stream(event: &str, status: &str, sequence_number: u64) -> String {
        format!(
            "{}{}",
            response_event(
                "response.created",
                json!({
                    "type": "response.created",
                    "response": {"id": "resp_terminal", "status": "in_progress"},
                    "sequence_number": sequence_number - 1
                }),
            ),
            response_event(
                event,
                json!({
                    "type": event,
                    "response": {"id": "resp_terminal", "status": status, "output": []},
                    "sequence_number": sequence_number
                }),
            ),
        )
    }
}
