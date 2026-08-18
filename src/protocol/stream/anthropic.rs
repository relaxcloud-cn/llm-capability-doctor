use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use crate::evidence::{StreamEndSignal, StreamTermination};

use super::framing::decode_sse_chunks;
use super::{AssistantTurn, ProtocolHistory, StreamParseResult, ToolCall, ToolCorrelation};

#[derive(Clone, Copy)]
enum Terminal {
    MessageStop,
    ProtocolError,
}

#[derive(Default)]
struct ParserState {
    saw_message_start: bool,
    message_identity_valid: bool,
    blocks: BTreeMap<usize, BlockAccumulator>,
    open_blocks: BTreeSet<usize>,
    tool_ids: BTreeMap<String, usize>,
    next_block_index: usize,
    saw_message_delta: bool,
    stop_reason: Option<String>,
    stop_reason_path: Option<String>,
    terminal: Option<Terminal>,
    saw_message_stop: bool,
    contract_errors: Vec<String>,
}

#[derive(Default)]
struct BlockAccumulator {
    native: Map<String, Value>,
    kind: Option<String>,
    tool_id: Option<String>,
    tool_name: Option<String>,
    initial_input: Option<Value>,
    resolved_input: Option<Value>,
    input_fragments: String,
    input_invalid: bool,
    text: String,
    thinking: String,
    signature: String,
    citations: Vec<Value>,
    write_citations_array: bool,
    closed: bool,
    invalid: bool,
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
        let explicit_error = event.event == "error";
        let value = match serde_json::from_str::<Value>(&event.data) {
            Ok(value) => value,
            Err(_) => {
                record_event_after_terminal(&mut state, event_index, explicit_error);
                if explicit_error {
                    push_error(
                        &mut state.contract_errors,
                        "api_error",
                        &format!("/events/{event_index}"),
                    );
                    state.terminal = Some(Terminal::ProtocolError);
                }
                malformed_stream = true;
                push_error(
                    &mut state.contract_errors,
                    "malformed_event_json",
                    &format!("/events/{event_index}/data"),
                );
                continue;
            }
        };
        let Some(payload) = value.as_object() else {
            record_event_after_terminal(&mut state, event_index, explicit_error);
            if explicit_error {
                push_error(
                    &mut state.contract_errors,
                    "api_error",
                    &format!("/events/{event_index}"),
                );
                state.terminal = Some(Terminal::ProtocolError);
            }
            push_error(
                &mut state.contract_errors,
                "invalid_event",
                &format!("/events/{event_index}/data"),
            );
            continue;
        };
        let Some(data_type) = payload.get("type").and_then(Value::as_str) else {
            record_event_after_terminal(&mut state, event_index, explicit_error);
            if explicit_error {
                push_error(
                    &mut state.contract_errors,
                    "api_error",
                    &format!("/events/{event_index}"),
                );
                state.terminal = Some(Terminal::ProtocolError);
            }
            push_error(
                &mut state.contract_errors,
                "invalid_event_type",
                &format!("/events/{event_index}/data/type"),
            );
            continue;
        };

        if event.event.is_empty() || event.event == "message" {
            record_event_after_terminal(&mut state, event_index, explicit_error);
            push_error(
                &mut state.contract_errors,
                "missing_event_name",
                &format!("/events/{event_index}/event"),
            );
            if explicit_error {
                push_error(
                    &mut state.contract_errors,
                    "api_error",
                    &format!("/events/{event_index}"),
                );
                state.terminal = Some(Terminal::ProtocolError);
            }
            continue;
        }
        if event.event != data_type {
            record_event_after_terminal(&mut state, event_index, explicit_error);
            push_error(
                &mut state.contract_errors,
                "event_type_mismatch",
                &format!("/events/{event_index}/data/type"),
            );
            if explicit_error {
                push_error(
                    &mut state.contract_errors,
                    "api_error",
                    &format!("/events/{event_index}"),
                );
                state.terminal = Some(Terminal::ProtocolError);
            }
            continue;
        }

        if data_type == "message_stop" {
            state.saw_message_stop = true;
        }

        if state.terminal.is_some() {
            record_event_after_terminal(&mut state, event_index, explicit_error);
            if explicit_error {
                push_error(
                    &mut state.contract_errors,
                    "api_error",
                    &format!("/events/{event_index}"),
                );
                state.terminal = Some(Terminal::ProtocolError);
            }
            continue;
        }

        if explicit_error {
            push_error(
                &mut state.contract_errors,
                "api_error",
                &format!("/events/{event_index}"),
            );
            state.terminal = Some(Terminal::ProtocolError);
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

    if !malformed_stream && !matches!(state.terminal, Some(Terminal::ProtocolError)) {
        validate_completed_lifecycle(&mut state);
    }
    let assistant_turn = build_assistant_turn(&mut state);
    let stop_reason_is_normal = stop_reason_is_normal(&state);
    let stream_termination = if malformed_stream {
        StreamTermination::MalformedStream
    } else {
        match state.terminal {
            Some(Terminal::ProtocolError) => StreamTermination::ProtocolError,
            None => StreamTermination::MissingTerminalEvent,
            Some(Terminal::MessageStop) if stop_reason_is_normal => StreamTermination::Completed,
            Some(Terminal::MessageStop) => StreamTermination::ModelIncomplete,
        }
    };

    StreamParseResult {
        assistant_turn,
        stream_termination,
        stream_end_signal: if state.saw_message_stop {
            StreamEndSignal::AnthropicMessageStop
        } else {
            StreamEndSignal::None
        },
        model_stop_reason: state.stop_reason,
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
    payload: &Map<String, Value>,
    event_index: usize,
) {
    match event_type {
        "message_start" => process_message_start(state, payload, event_index),
        "content_block_start" => process_block_start(state, payload, event_index),
        "content_block_delta" => process_block_delta(state, payload, event_index),
        "content_block_stop" => process_block_stop(state, payload, event_index),
        "message_delta" => process_message_delta(state, payload, event_index),
        "message_stop" => process_message_stop(state, event_index),
        "ping" => {}
        _ => {}
    }
}

fn process_message_start(
    state: &mut ParserState,
    payload: &Map<String, Value>,
    event_index: usize,
) {
    if state.saw_message_start {
        push_error(
            &mut state.contract_errors,
            "duplicate_message_start",
            &format!("/events/{event_index}/event"),
        );
        return;
    }
    if event_index != 0 {
        push_error(
            &mut state.contract_errors,
            "message_start_not_first",
            &format!("/events/{event_index}/event"),
        );
    }
    state.saw_message_start = true;
    state.message_identity_valid = true;

    let base = format!("/events/{event_index}/data/message");
    let Some(message) = payload.get("message").and_then(Value::as_object) else {
        state.message_identity_valid = false;
        push_error(&mut state.contract_errors, "invalid_message", &base);
        return;
    };
    validate_nonempty_field(state, message, "id", "invalid_message_id", &base);
    validate_exact_field(
        state,
        message,
        "type",
        "message",
        "invalid_message_type",
        &base,
    );
    validate_exact_field(
        state,
        message,
        "role",
        "assistant",
        "invalid_message_role",
        &base,
    );
    validate_nonempty_field(state, message, "model", "invalid_message_model", &base);
    if !matches!(message.get("content"), Some(Value::Array(content)) if content.is_empty()) {
        state.message_identity_valid = false;
        push_error(
            &mut state.contract_errors,
            "invalid_initial_content",
            &format!("{base}/content"),
        );
    }
    for field in ["stop_reason", "stop_sequence"] {
        if message.get(field) != Some(&Value::Null) {
            state.message_identity_valid = false;
            push_error(
                &mut state.contract_errors,
                "invalid_initial_stop",
                &format!("{base}/{field}"),
            );
        }
    }
    validate_usage(
        &mut state.contract_errors,
        message.get("usage"),
        &format!("{base}/usage"),
        true,
    );
}

fn validate_nonempty_field(
    state: &mut ParserState,
    object: &Map<String, Value>,
    field: &str,
    code: &str,
    base: &str,
) {
    if object
        .get(field)
        .and_then(Value::as_str)
        .is_none_or(|value| value.trim().is_empty())
    {
        state.message_identity_valid = false;
        push_error(&mut state.contract_errors, code, &format!("{base}/{field}"));
    }
}

fn validate_exact_field(
    state: &mut ParserState,
    object: &Map<String, Value>,
    field: &str,
    expected: &str,
    code: &str,
    base: &str,
) {
    if object.get(field).and_then(Value::as_str) != Some(expected) {
        state.message_identity_valid = false;
        push_error(&mut state.contract_errors, code, &format!("{base}/{field}"));
    }
}

fn process_block_start(state: &mut ParserState, payload: &Map<String, Value>, event_index: usize) {
    require_message_start(state, event_index);
    let base = format!("/events/{event_index}/data");
    if state.saw_message_delta {
        push_error(
            &mut state.contract_errors,
            "block_after_message_delta",
            &format!("{base}/type"),
        );
    }
    let Some(index) = event_index_value(state, payload, &base) else {
        return;
    };
    if !state.open_blocks.is_empty() {
        push_error(
            &mut state.contract_errors,
            "overlapping_block",
            &format!("{base}/type"),
        );
        for open_index in state.open_blocks.clone() {
            if let Some(block) = state.blocks.get_mut(&open_index) {
                block.invalid = true;
            }
        }
    }
    if state.blocks.contains_key(&index) {
        push_error(
            &mut state.contract_errors,
            "duplicate_block_index",
            &format!("{base}/index"),
        );
        if let Some(block) = state.blocks.get_mut(&index) {
            block.invalid = true;
        }
        return;
    }

    let mut block = match payload.get("content_block").and_then(Value::as_object) {
        Some(block) => block_accumulator(block, &base, &mut state.contract_errors),
        None => {
            push_error(
                &mut state.contract_errors,
                "invalid_content_block",
                &format!("{base}/content_block"),
            );
            BlockAccumulator {
                invalid: true,
                ..BlockAccumulator::default()
            }
        }
    };
    if index != state.next_block_index {
        block.invalid = true;
        push_error(
            &mut state.contract_errors,
            "non_contiguous_index",
            &format!("{base}/index"),
        );
    } else {
        state.next_block_index = state.next_block_index.saturating_add(1);
    }
    if !state.open_blocks.is_empty() || state.saw_message_delta {
        block.invalid = true;
    }

    if block.kind.as_deref() == Some("tool_use")
        && let Some(tool_id) = block.tool_id.clone()
    {
        if let Some(previous_index) = state.tool_ids.get(&tool_id).copied() {
            block.invalid = true;
            if let Some(previous) = state.blocks.get_mut(&previous_index) {
                previous.invalid = true;
            }
            push_error(
                &mut state.contract_errors,
                "duplicate_tool_use_id",
                &format!("{base}/content_block/id"),
            );
        } else {
            state.tool_ids.insert(tool_id, index);
        }
    }

    state.blocks.insert(index, block);
    state.open_blocks.insert(index);
}

fn block_accumulator(
    native: &Map<String, Value>,
    base: &str,
    errors: &mut Vec<String>,
) -> BlockAccumulator {
    let mut block = BlockAccumulator {
        native: native.clone(),
        ..BlockAccumulator::default()
    };
    block.kind = required_nonempty_string(
        native.get("type"),
        "invalid_block_type",
        &format!("{base}/content_block/type"),
        &mut block.invalid,
        errors,
    );

    match block.kind.as_deref() {
        Some("tool_use") => {
            block.tool_id = required_nonempty_string(
                native.get("id"),
                "invalid_tool_use_id",
                &format!("{base}/content_block/id"),
                &mut block.invalid,
                errors,
            );
            block.tool_name = required_nonempty_string(
                native.get("name"),
                "invalid_tool_name",
                &format!("{base}/content_block/name"),
                &mut block.invalid,
                errors,
            );
            match native.get("input") {
                Some(input) if input.is_object() => block.initial_input = Some(input.clone()),
                _ => {
                    block.invalid = true;
                    block.input_invalid = true;
                    push_error(
                        errors,
                        "invalid_initial_input",
                        &format!("{base}/content_block/input"),
                    );
                }
            }
        }
        Some("text") => {
            match native.get("text").and_then(Value::as_str) {
                Some(text) => block.text.push_str(text),
                None => {
                    block.invalid = true;
                    push_error(
                        errors,
                        "invalid_text",
                        &format!("{base}/content_block/text"),
                    );
                }
            }
            match native.get("citations") {
                None | Some(Value::Null) => {}
                Some(Value::Array(citations)) => {
                    block.citations = citations.clone();
                    block.write_citations_array = true;
                }
                Some(_) => {
                    block.invalid = true;
                    push_error(
                        errors,
                        "invalid_citations",
                        &format!("{base}/content_block/citations"),
                    );
                }
            }
        }
        Some("thinking") => {
            match native.get("thinking").and_then(Value::as_str) {
                Some(thinking) => block.thinking.push_str(thinking),
                None => {
                    block.invalid = true;
                    push_error(
                        errors,
                        "invalid_thinking",
                        &format!("{base}/content_block/thinking"),
                    );
                }
            }
            match native.get("signature").and_then(Value::as_str) {
                Some(signature) => block.signature = signature.to_owned(),
                None => {
                    block.invalid = true;
                    push_error(
                        errors,
                        "invalid_signature",
                        &format!("{base}/content_block/signature"),
                    );
                }
            }
        }
        _ => {}
    }
    block
}

fn process_block_delta(state: &mut ParserState, payload: &Map<String, Value>, event_index: usize) {
    require_message_start(state, event_index);
    let base = format!("/events/{event_index}/data");
    if state.saw_message_delta {
        push_error(
            &mut state.contract_errors,
            "block_after_message_delta",
            &format!("{base}/type"),
        );
    }
    let Some(index) = event_index_value(state, payload, &base) else {
        return;
    };
    let Some(block) = state.blocks.get_mut(&index) else {
        push_error(
            &mut state.contract_errors,
            "unknown_block_index",
            &format!("{base}/index"),
        );
        return;
    };
    if !state.open_blocks.contains(&index) || block.closed {
        block.invalid = true;
        push_error(
            &mut state.contract_errors,
            "block_not_open",
            &format!("{base}/index"),
        );
        return;
    }
    if state.saw_message_delta {
        block.invalid = true;
    }
    let Some(delta) = payload.get("delta").and_then(Value::as_object) else {
        block.invalid = true;
        push_error(
            &mut state.contract_errors,
            "invalid_delta",
            &format!("{base}/delta"),
        );
        return;
    };
    let delta_type = delta.get("type").and_then(Value::as_str);
    match (block.kind.as_deref(), delta_type) {
        (Some("tool_use"), Some("input_json_delta")) => {
            if let Some(fragment) = delta.get("partial_json").and_then(Value::as_str) {
                block.input_fragments.push_str(fragment);
            } else {
                block.invalid = true;
                block.input_invalid = true;
                push_error(
                    &mut state.contract_errors,
                    "invalid_input_fragment",
                    &format!("{base}/delta/partial_json"),
                );
            }
        }
        (Some("text"), Some("text_delta")) => {
            if let Some(text) = delta.get("text").and_then(Value::as_str) {
                block.text.push_str(text);
            } else {
                block.invalid = true;
                push_error(
                    &mut state.contract_errors,
                    "invalid_text_delta",
                    &format!("{base}/delta/text"),
                );
            }
        }
        (Some("text"), Some("citations_delta")) => {
            if let Some(citation) = delta.get("citation").filter(|value| value.is_object()) {
                block.citations.push(citation.clone());
                block.write_citations_array = true;
            } else {
                block.invalid = true;
                push_error(
                    &mut state.contract_errors,
                    "invalid_citation_delta",
                    &format!("{base}/delta/citation"),
                );
            }
        }
        (Some("thinking"), Some("thinking_delta")) => {
            if let Some(thinking) = delta.get("thinking").and_then(Value::as_str) {
                block.thinking.push_str(thinking);
            } else {
                block.invalid = true;
                push_error(
                    &mut state.contract_errors,
                    "invalid_thinking_delta",
                    &format!("{base}/delta/thinking"),
                );
            }
        }
        (Some("thinking"), Some("signature_delta")) => {
            if let Some(signature) = delta.get("signature").and_then(Value::as_str) {
                block.signature = signature.to_owned();
            } else {
                block.invalid = true;
                push_error(
                    &mut state.contract_errors,
                    "invalid_signature_delta",
                    &format!("{base}/delta/signature"),
                );
            }
        }
        (_, Some(delta_type)) if is_known_delta_type(delta_type) => {
            block.invalid = true;
            push_error(
                &mut state.contract_errors,
                "unexpected_block_delta",
                &format!("{base}/delta/type"),
            );
        }
        (_, Some(_)) => {}
        (_, None) => {
            block.invalid = true;
            push_error(
                &mut state.contract_errors,
                "invalid_delta_type",
                &format!("{base}/delta/type"),
            );
        }
    }
}

fn is_known_delta_type(delta_type: &str) -> bool {
    matches!(
        delta_type,
        "text_delta"
            | "citations_delta"
            | "input_json_delta"
            | "thinking_delta"
            | "signature_delta"
    )
}

fn process_block_stop(state: &mut ParserState, payload: &Map<String, Value>, event_index: usize) {
    require_message_start(state, event_index);
    let base = format!("/events/{event_index}/data");
    if state.saw_message_delta {
        push_error(
            &mut state.contract_errors,
            "block_after_message_delta",
            &format!("{base}/type"),
        );
    }
    let Some(index) = event_index_value(state, payload, &base) else {
        return;
    };
    let Some(block) = state.blocks.get_mut(&index) else {
        push_error(
            &mut state.contract_errors,
            "unknown_block_index",
            &format!("{base}/index"),
        );
        return;
    };
    if !state.open_blocks.remove(&index) || block.closed {
        block.invalid = true;
        push_error(
            &mut state.contract_errors,
            "block_not_open",
            &format!("{base}/index"),
        );
        return;
    }
    if state.saw_message_delta {
        block.invalid = true;
    }
    block.closed = true;
    finalize_block(block);
}

fn finalize_block(block: &mut BlockAccumulator) {
    match block.kind.as_deref() {
        Some("tool_use") => {
            let input = if block.input_fragments.is_empty() {
                block.initial_input.clone()
            } else {
                serde_json::from_str::<Value>(&block.input_fragments).ok()
            };
            if input.as_ref().is_none_or(|value| !value.is_object()) {
                block.invalid = true;
                block.input_invalid = true;
            } else {
                block.resolved_input = input;
            }
        }
        Some("text") => {
            block
                .native
                .insert("text".into(), Value::String(block.text.clone()));
            if block.write_citations_array {
                block
                    .native
                    .insert("citations".into(), Value::Array(block.citations.clone()));
            }
        }
        Some("thinking") => {
            block
                .native
                .insert("thinking".into(), Value::String(block.thinking.clone()));
            block
                .native
                .insert("signature".into(), Value::String(block.signature.clone()));
        }
        _ => {}
    }
}

fn process_message_delta(
    state: &mut ParserState,
    payload: &Map<String, Value>,
    event_index: usize,
) {
    require_message_start(state, event_index);
    let base = format!("/events/{event_index}/data");
    if !state.open_blocks.is_empty() {
        push_error(
            &mut state.contract_errors,
            "message_delta_with_open_block",
            &format!("{base}/type"),
        );
        for index in state.open_blocks.clone() {
            if let Some(block) = state.blocks.get_mut(&index) {
                block.invalid = true;
            }
        }
    }
    state.saw_message_delta = true;
    let Some(delta) = payload.get("delta").and_then(Value::as_object) else {
        push_error(
            &mut state.contract_errors,
            "invalid_message_delta",
            &format!("{base}/delta"),
        );
        return;
    };
    validate_usage(
        &mut state.contract_errors,
        payload.get("usage"),
        &format!("{base}/usage"),
        false,
    );

    if let Some(value) = delta.get("stop_reason") {
        let path = format!("{base}/delta/stop_reason");
        match value.as_str() {
            Some(reason) if !reason.trim().is_empty() => {
                if state.stop_reason.is_some() {
                    push_error(&mut state.contract_errors, "duplicate_stop_reason", &path);
                } else {
                    state.stop_reason = Some(reason.to_owned());
                    state.stop_reason_path = Some(path);
                }
            }
            _ => push_error(&mut state.contract_errors, "invalid_stop_reason", &path),
        }
    }
}

fn process_message_stop(state: &mut ParserState, event_index: usize) {
    require_message_start(state, event_index);
    if !state.saw_message_delta {
        push_error(
            &mut state.contract_errors,
            "missing_message_delta",
            &format!("/events/{event_index}/data"),
        );
    }
    if !state.open_blocks.is_empty() {
        push_error(
            &mut state.contract_errors,
            "message_stop_with_open_block",
            &format!("/events/{event_index}/data"),
        );
        for index in state.open_blocks.clone() {
            if let Some(block) = state.blocks.get_mut(&index) {
                block.invalid = true;
            }
        }
    }
    state.saw_message_stop = true;
    state.terminal = Some(Terminal::MessageStop);
}

fn validate_usage(errors: &mut Vec<String>, value: Option<&Value>, base: &str, start: bool) {
    let Some(usage) = value.and_then(Value::as_object) else {
        push_error(errors, "invalid_usage", base);
        return;
    };
    if start && usage.get("input_tokens").and_then(Value::as_u64).is_none() {
        push_error(
            errors,
            "invalid_input_tokens",
            &format!("{base}/input_tokens"),
        );
    }
    if usage.get("output_tokens").and_then(Value::as_u64).is_none() {
        push_error(
            errors,
            "invalid_output_tokens",
            &format!("{base}/output_tokens"),
        );
    }
}

fn validate_completed_lifecycle(state: &mut ParserState) {
    if !state.saw_message_start {
        push_error(
            &mut state.contract_errors,
            "missing_message_start",
            "/events/0",
        );
    }
    for index in state.open_blocks.clone() {
        if let Some(block) = state.blocks.get_mut(&index) {
            block.invalid = true;
        }
        push_error(&mut state.contract_errors, "unclosed_block", "/content");
    }
    if state.saw_message_stop && state.stop_reason.is_none() {
        push_error(
            &mut state.contract_errors,
            "missing_stop_reason",
            "/message/stop_reason",
        );
    }
    if state.stop_reason.is_some() && !stop_reason_is_normal(state) {
        let path = state
            .stop_reason_path
            .clone()
            .unwrap_or_else(|| "/message/stop_reason".into());
        push_error(&mut state.contract_errors, "invalid_stop_reason", &path);
    }
}

fn stop_reason_is_normal(state: &ParserState) -> bool {
    let has_tool = state
        .blocks
        .values()
        .any(|block| block.kind.as_deref() == Some("tool_use"));
    matches!(
        (has_tool, state.stop_reason.as_deref()),
        (true, Some("tool_use")) | (false, Some("end_turn"))
    )
}

fn build_assistant_turn(state: &mut ParserState) -> Option<AssistantTurn> {
    if !state.saw_message_start {
        return None;
    }
    let mut history = Vec::with_capacity(state.blocks.len());
    let mut tool_calls = Vec::new();
    let mut final_text = String::new();
    let mut finalization_errors = Vec::new();

    for (position, (index, block)) in state.blocks.iter_mut().enumerate() {
        if !block.closed {
            block.invalid = true;
        }
        if block.kind.as_deref() == Some("tool_use") {
            if block.input_invalid {
                finalization_errors.push(contract_error(
                    "invalid_tool_input",
                    &format!("/content/{position}/input"),
                ));
            }
            if let Some(input) = block.resolved_input.clone() {
                block.native.insert("input".into(), input.clone());
                if !block.invalid
                    && block.closed
                    && let (Some(tool_id), Some(name)) = (&block.tool_id, &block.tool_name)
                {
                    tool_calls.push(ToolCall {
                        index: *index,
                        correlation: ToolCorrelation::Required(tool_id.clone()),
                        name: name.clone(),
                        arguments: input,
                    });
                }
            }
        } else if block.kind.as_deref() == Some("text") {
            final_text.push_str(&block.text);
        }
        history.push(Value::Object(block.native.clone()));
    }

    for error in finalization_errors {
        if !state.contract_errors.contains(&error) {
            state.contract_errors.push(error);
        }
    }
    if !state.message_identity_valid
        || !state.contract_errors.is_empty()
        || state.stop_reason.as_deref() != Some("tool_use")
    {
        tool_calls.clear();
    }

    Some(AssistantTurn {
        history: ProtocolHistory::Anthropic(history),
        tool_calls,
        final_text,
    })
}

fn require_message_start(state: &mut ParserState, event_index: usize) {
    if !state.saw_message_start {
        push_error(
            &mut state.contract_errors,
            "event_before_message_start",
            &format!("/events/{event_index}/event"),
        );
    }
}

fn event_index_value(
    state: &mut ParserState,
    payload: &Map<String, Value>,
    base: &str,
) -> Option<usize> {
    let index = payload
        .get("index")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok());
    if index.is_none() {
        push_error(
            &mut state.contract_errors,
            "invalid_block_index",
            &format!("{base}/index"),
        );
    }
    index
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

fn record_event_after_terminal(state: &mut ParserState, event_index: usize, terminal: bool) {
    if state.terminal.is_some() {
        push_error(
            &mut state.contract_errors,
            if terminal {
                "duplicate_terminal"
            } else {
                "event_after_terminal"
            },
            &format!("/events/{event_index}/event"),
        );
    }
}

fn contract_error(code: &str, path: &str) -> String {
    debug_assert!(path.starts_with('/'));
    format!("anthropic.{code}:{path}")
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

    const TOOL_STREAM: &[u8] = include_bytes!("../fixtures/anthropic_tool.sse");
    const FINAL_STREAM: &[u8] = include_bytes!("../fixtures/anthropic_final.sse");
    const MISSING_TERMINAL: &[u8] = include_bytes!("../fixtures/anthropic_missing_terminal.sse");
    const TRUNCATED: &[u8] = include_bytes!("../fixtures/anthropic_truncated.sse");
    const API_ERROR: &[u8] = include_bytes!("../fixtures/anthropic_error.sse");
    const INVALID_ORDER: &[u8] = include_bytes!("../fixtures/anthropic_invalid_order.sse");

    #[tokio::test]
    async fn reconstructs_ordered_native_blocks_and_fragmented_tool_input() {
        let parsed = parse(&complete_fixture(TOOL_STREAM)).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert_eq!(
            parsed.stream_end_signal,
            StreamEndSignal::AnthropicMessageStop
        );
        assert_eq!(parsed.model_stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(parsed.event_count, 12);
        assert!(parsed.contract_errors.is_empty());
        assert_eq!(
            parsed.assistant_turn,
            Some(AssistantTurn {
                history: ProtocolHistory::Anthropic(vec![
                    json!({"type": "text", "text": "Checking weather."}),
                    json!({
                        "type": "tool_use",
                        "id": "toolu_weather_046",
                        "name": "get_weather",
                        "input": {"city": "Beijing"}
                    }),
                ]),
                tool_calls: vec![ToolCall {
                    index: 1,
                    correlation: ToolCorrelation::Required("toolu_weather_046".into()),
                    name: "get_weather".into(),
                    arguments: json!({"city": "Beijing"}),
                }],
                final_text: "Checking weather.".into(),
            })
        );
    }

    #[tokio::test]
    async fn reconstructs_final_text_and_requires_end_turn() {
        let parsed = parse(&complete_fixture(FINAL_STREAM)).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert_eq!(
            parsed.stream_end_signal,
            StreamEndSignal::AnthropicMessageStop
        );
        assert_eq!(parsed.model_stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(parsed.event_count, 7);
        assert!(parsed.contract_errors.is_empty());
        assert_eq!(
            parsed.assistant_turn,
            Some(AssistantTurn {
                history: ProtocolHistory::Anthropic(vec![json!({
                    "type": "text",
                    "text": "MODEL_DOCTOR_CASE_046_OK"
                })]),
                tool_calls: vec![],
                final_text: "MODEL_DOCTOR_CASE_046_OK".into(),
            })
        );
    }

    #[tokio::test]
    async fn preserves_null_citations_without_a_citation_delta() {
        let body = b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_null_citations\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-test\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\",\"citations\":null}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"done\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":2}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
        let parsed = parse(body).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert!(parsed.contract_errors.is_empty());
        assert_eq!(
            parsed.assistant_turn.expect("assistant turn").history,
            ProtocolHistory::Anthropic(vec![json!({
                "type": "text",
                "text": "done",
                "citations": null
            })])
        );
    }

    #[tokio::test]
    async fn preserves_thinking_blocks_and_replaces_the_streamed_signature() {
        let body = b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_thinking\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-test\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\",\"signature\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"step one; \"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"step two\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"sig-old\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"sig-final\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\",\"text\":\"\",\"citations\":[]}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"done\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"citations_delta\",\"citation\":{\"type\":\"char_location\",\"cited_text\":\"source\"}}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":4,\"future_counter\":9}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
        let parsed = parse(body).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert!(parsed.contract_errors.is_empty());
        assert_eq!(
            parsed.assistant_turn.expect("assistant turn").history,
            ProtocolHistory::Anthropic(vec![
                json!({
                    "type": "thinking",
                    "thinking": "step one; step two",
                    "signature": "sig-final"
                }),
                json!({
                    "type": "text",
                    "text": "done",
                    "citations": [{"type": "char_location", "cited_text": "source"}]
                }),
            ])
        );
    }

    #[tokio::test]
    async fn rejects_known_delta_types_that_do_not_match_the_open_block() {
        let tool_fixture = std::str::from_utf8(TOOL_STREAM).expect("fixture UTF-8");
        let tool_with_text_delta = tool_fixture.replacen(
            "{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"city\\\":\\\"\"}",
            "{\"type\":\"text_delta\",\"text\":\"wrong\"}",
            1,
        );
        let final_fixture = std::str::from_utf8(FINAL_STREAM).expect("fixture UTF-8");
        let text_with_input_delta = final_fixture.replacen(
            "{\"type\":\"text_delta\",\"text\":\"MODEL_DOCTOR_CASE_\"}",
            "{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}",
            1,
        );

        for (body, expected) in [
            (
                tool_with_text_delta,
                "anthropic.unexpected_block_delta:/events/7/data/delta/type",
            ),
            (
                text_with_input_delta,
                "anthropic.unexpected_block_delta:/events/2/data/delta/type",
            ),
        ] {
            let parsed = parse(&complete_fixture(body.as_bytes())).await;
            assert_eq!(parsed.stream_termination, StreamTermination::Completed);
            assert!(
                parsed.contract_errors.contains(&expected.to_owned()),
                "missing {expected}: {:?}",
                parsed.contract_errors
            );
            assert!(
                parsed
                    .assistant_turn
                    .expect("assistant turn")
                    .tool_calls
                    .is_empty()
            );
        }

        let extension_delta = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"future_delta\",\"future_value\":true}}\n\n";
        let body = tool_fixture.replacen(
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1",
            &format!(
                "{extension_delta}event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":1"
            ),
            1,
        );
        let parsed = parse(&complete_fixture(body.as_bytes())).await;
        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert!(parsed.contract_errors.is_empty());
        assert_eq!(
            parsed
                .assistant_turn
                .expect("assistant turn")
                .tool_calls
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn rejects_duplicate_tool_use_ids_across_distinct_blocks() {
        let fixture = std::str::from_utf8(TOOL_STREAM).expect("fixture UTF-8");
        let duplicate = "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":2,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_weather_046\",\"name\":\"get_time\",\"input\":{}}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":2}\n\n";
        let body = fixture.replacen(
            "event: message_delta",
            &format!("{duplicate}event: message_delta"),
            1,
        );
        let parsed = parse(&complete_fixture(body.as_bytes())).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert!(parsed.contract_errors.contains(
            &"anthropic.duplicate_tool_use_id:/events/10/data/content_block/id".to_owned()
        ));
        assert!(
            parsed
                .assistant_turn
                .expect("history")
                .tool_calls
                .is_empty()
        );
    }

    #[tokio::test]
    async fn accepts_multiple_message_deltas_and_new_usage_fields() {
        let fixture = std::str::from_utf8(FINAL_STREAM).expect("fixture UTF-8");
        let partial = "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{},\"usage\":{\"output_tokens\":8,\"future_counter\":1}}\n\n";
        let body = fixture.replacen(
            "event: message_delta",
            &format!("{partial}event: message_delta"),
            1,
        );
        let parsed = parse(&complete_fixture(body.as_bytes())).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert_eq!(parsed.model_stop_reason.as_deref(), Some("end_turn"));
        assert!(parsed.contract_errors.is_empty());
    }

    #[tokio::test]
    async fn distinguishes_missing_terminal_truncation_and_api_error() {
        let missing = parse(&complete_fixture(MISSING_TERMINAL)).await;
        assert_eq!(
            missing.stream_termination,
            StreamTermination::MissingTerminalEvent
        );
        assert_eq!(missing.stream_end_signal, StreamEndSignal::None);
        assert!(missing.contract_errors.is_empty());

        let truncated = parse(TRUNCATED).await;
        assert_eq!(
            truncated.stream_termination,
            StreamTermination::MalformedStream
        );
        assert_eq!(
            truncated.contract_errors,
            ["anthropic.truncated_sse:/events/1".to_owned()]
        );

        let error = parse(&complete_fixture(API_ERROR)).await;
        assert_eq!(error.stream_termination, StreamTermination::ProtocolError);
        assert_eq!(
            error.contract_errors,
            ["anthropic.api_error:/events/0".to_owned()]
        );
        assert!(!format!("{error:?}").contains("sensitive upstream detail"));
    }

    #[tokio::test]
    async fn completed_wire_with_invalid_order_has_no_executable_call() {
        let parsed = parse(&complete_fixture(INVALID_ORDER)).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert_eq!(
            parsed.stream_end_signal,
            StreamEndSignal::AnthropicMessageStop
        );
        for expected in [
            "anthropic.non_contiguous_index:/events/1/data/index",
            "anthropic.overlapping_block:/events/2/data/type",
            "anthropic.duplicate_block_index:/events/2/data/index",
            "anthropic.unknown_block_index:/events/3/data/index",
        ] {
            assert!(
                parsed.contract_errors.contains(&expected.to_owned()),
                "missing {expected}: {:?}",
                parsed.contract_errors
            );
        }
        assert!(
            parsed
                .assistant_turn
                .expect("native history")
                .tool_calls
                .is_empty()
        );
    }

    #[tokio::test]
    async fn rejects_lifecycle_and_input_mutations_with_stable_errors() {
        let fixture = std::str::from_utf8(TOOL_STREAM).expect("fixture UTF-8");
        let cases = [
            (
                fixture.replacen("event: message_start", "event: ping", 1),
                "anthropic.event_type_mismatch:/events/0/data/type",
                StreamTermination::Completed,
            ),
            (
                fixture.replacen("\"role\":\"assistant\"", "\"role\":\"user\"", 1),
                "anthropic.invalid_message_role:/events/0/data/message/role",
                StreamTermination::Completed,
            ),
            (
                fixture.replacen("\"index\":1,\"delta\"", "\"index\":0,\"delta\"", 1),
                "anthropic.block_not_open:/events/7/data/index",
                StreamTermination::Completed,
            ),
            (
                fixture.replacen("Beijing\\\"}", "Beijing", 1),
                "anthropic.invalid_tool_input:/content/1/input",
                StreamTermination::Completed,
            ),
            (
                fixture.replacen(
                    "\"stop_reason\":\"tool_use\"",
                    "\"stop_reason\":\"end_turn\"",
                    1,
                ),
                "anthropic.invalid_stop_reason:/events/10/data/delta/stop_reason",
                StreamTermination::ModelIncomplete,
            ),
            (
                fixture.replacen("\"output_tokens\":18", "\"output_tokens\":-1", 1),
                "anthropic.invalid_output_tokens:/events/10/data/usage/output_tokens",
                StreamTermination::Completed,
            ),
        ];

        for (body, expected, termination) in cases {
            let parsed = parse(&complete_fixture(body.as_bytes())).await;
            assert_eq!(parsed.stream_termination, termination);
            assert!(
                parsed.contract_errors.contains(&expected.to_owned()),
                "missing {expected}: {:?}",
                parsed.contract_errors
            );
            assert!(
                parsed
                    .assistant_turn
                    .as_ref()
                    .is_none_or(|turn| turn.tool_calls.is_empty())
            );
        }
    }

    #[tokio::test]
    async fn requires_message_start_first_and_only_once() {
        let start = first_frame(TOOL_STREAM);
        let body = format!("{start}\n\n{}", std::str::from_utf8(TOOL_STREAM).unwrap());
        let parsed = parse(&complete_fixture(body.as_bytes())).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert!(
            parsed
                .contract_errors
                .contains(&"anthropic.duplicate_message_start:/events/1/event".to_owned())
        );

        let ping_first = format!(
            "event: ping\ndata: {{\"type\":\"ping\"}}\n\n{}",
            std::str::from_utf8(FINAL_STREAM).unwrap()
        );
        let parsed = parse(&complete_fixture(ping_first.as_bytes())).await;
        assert!(
            parsed
                .contract_errors
                .contains(&"anthropic.message_start_not_first:/events/1/event".to_owned())
        );
    }

    #[tokio::test]
    async fn explicit_error_name_wins_even_when_data_type_mismatches() {
        let body = b"event: error\ndata: {\"type\":\"private-mismatch\",\"error\":{\"message\":\"PRIVATE\"}}\n\n";
        let parsed = parse(body).await;

        assert_eq!(parsed.stream_termination, StreamTermination::ProtocolError);
        assert_eq!(
            parsed.contract_errors,
            [
                "anthropic.event_type_mismatch:/events/0/data/type".to_owned(),
                "anthropic.api_error:/events/0".to_owned(),
            ]
        );
        assert!(!format!("{parsed:?}").contains("PRIVATE"));
    }

    #[tokio::test]
    async fn observes_a_valid_message_stop_after_an_error_without_accepting_forgeries() {
        let error = "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"PRIVATE\"}}\n\n";
        let parsed = parse(
            format!("{error}event: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n")
                .as_bytes(),
        )
        .await;
        assert_eq!(parsed.stream_termination, StreamTermination::ProtocolError);
        assert_eq!(
            parsed.stream_end_signal,
            StreamEndSignal::AnthropicMessageStop
        );
        assert!(!format!("{parsed:?}").contains("PRIVATE"));

        for forged in [
            "event: model_doctor.extension\ndata: {\"type\":\"message_stop\"}\n\n",
            "event: message_stop\ndata: {\"type\":\"model_doctor.extension\"}\n\n",
        ] {
            let parsed = parse(format!("{error}{forged}").as_bytes()).await;
            assert_eq!(parsed.stream_termination, StreamTermination::ProtocolError);
            assert_eq!(parsed.stream_end_signal, StreamEndSignal::None);
        }
    }

    #[tokio::test]
    async fn forged_error_data_type_does_not_override_extension_event_name() {
        let body = b"event: model_doctor.extension\ndata: {\"type\":\"error\",\"message\":\"PRIVATE\"}\n\n";
        let parsed = parse(body).await;

        assert_eq!(
            parsed.stream_termination,
            StreamTermination::MissingTerminalEvent
        );
        assert!(
            parsed
                .contract_errors
                .contains(&"anthropic.event_type_mismatch:/events/0/data/type".to_owned())
        );
        assert!(!format!("{parsed:?}").contains("PRIVATE"));
    }

    #[tokio::test]
    async fn terminal_seals_state_and_later_error_has_priority() {
        let mut completed = String::from_utf8(complete_fixture(FINAL_STREAM)).unwrap();
        completed.push_str(
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"PRIVATE_AFTER\"}}\n\n",
        );
        let parsed = parse(completed.as_bytes()).await;
        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert!(
            parsed
                .contract_errors
                .contains(&"anthropic.event_after_terminal:/events/7/event".to_owned())
        );
        assert!(!format!("{parsed:?}").contains("PRIVATE_AFTER"));

        completed.push_str(
            "event: error\ndata: {\"type\":\"private-mismatch\",\"message\":\"PRIVATE_ERROR\"}\n\n",
        );
        let parsed = parse(completed.as_bytes()).await;
        assert_eq!(parsed.stream_termination, StreamTermination::ProtocolError);
        assert_eq!(
            parsed.stream_end_signal,
            StreamEndSignal::AnthropicMessageStop
        );
        assert!(
            parsed
                .contract_errors
                .contains(&"anthropic.api_error:/events/8".to_owned())
        );
        assert!(!format!("{parsed:?}").contains("PRIVATE_ERROR"));
    }

    #[tokio::test]
    async fn malformed_event_after_terminal_keeps_malformed_precedence() {
        let mut body = String::from_utf8(complete_fixture(FINAL_STREAM)).unwrap();
        body.push_str("event: model_doctor.extension\ndata: {PRIVATE_BAD_JSON}\n\n");
        let parsed = parse(&complete_fixture(body.as_bytes())).await;

        assert_eq!(
            parsed.stream_termination,
            StreamTermination::MalformedStream
        );
        assert_eq!(
            parsed.stream_end_signal,
            StreamEndSignal::AnthropicMessageStop
        );
        assert!(
            parsed
                .contract_errors
                .contains(&"anthropic.event_after_terminal:/events/7/event".to_owned())
        );
        assert!(!format!("{parsed:?}").contains("PRIVATE_BAD_JSON"));
    }

    #[tokio::test]
    async fn sparse_large_index_uses_bounded_storage_and_position_paths() {
        let body = std::str::from_utf8(TOOL_STREAM)
            .unwrap()
            .replace("\"index\":1", "\"index\":999999999");
        let parsed = parse(&complete_fixture(body.as_bytes())).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert!(
            parsed
                .contract_errors
                .contains(&"anthropic.non_contiguous_index:/events/6/data/index".to_owned())
        );
        assert!(parsed.contract_errors.iter().all(|error| {
            !error.starts_with("anthropic.invalid_tool_input:/content/999999999/")
        }));
        assert!(
            parsed
                .assistant_turn
                .expect("history")
                .tool_calls
                .is_empty()
        );
    }

    fn first_frame(body: &[u8]) -> &str {
        std::str::from_utf8(body)
            .expect("fixture UTF-8")
            .split_once("\n\n")
            .expect("fixture has a first frame")
            .0
    }

    fn complete_fixture(body: &[u8]) -> Vec<u8> {
        let mut body = body.to_vec();
        if !body.ends_with(b"\n\n") {
            body.push(b'\n');
        }
        body
    }
}
