use std::collections::BTreeSet;

use chrono::DateTime;
use serde_json::{Map, Value};

use crate::evidence::{StreamEndSignal, StreamTermination};

use super::framing::{FramingError, NdjsonItem, decode_ndjson_chunks_partial};
use super::{AssistantTurn, ProtocolHistory, StreamParseResult, ToolCall, ToolCorrelation};

const METRIC_FIELDS: [&str; 6] = [
    "total_duration",
    "load_duration",
    "prompt_eval_count",
    "prompt_eval_duration",
    "eval_count",
    "eval_duration",
];

#[derive(Debug)]
struct PendingToolCall {
    native_index: Option<usize>,
    correlation: ToolCorrelation,
    name: String,
    arguments: Value,
}

#[derive(Default)]
struct ParserState {
    history: Map<String, Value>,
    final_text: String,
    thinking: String,
    raw_tool_calls: Vec<Value>,
    pending_tool_calls: Vec<PendingToolCall>,
    seen_tool_ids: BTreeSet<String>,
    seen_tool_indexes: BTreeSet<usize>,
    saw_indexed_call: bool,
    saw_unindexed_call: bool,
    invalid_call_set: bool,
    saw_message: bool,
    saw_thinking: bool,
    saw_tool_calls: bool,
    saw_done: bool,
    done_reason: Option<String>,
    sealed: bool,
    protocol_error: bool,
    contract_errors: Vec<String>,
}

pub(crate) async fn parse(body: &[u8]) -> StreamParseResult {
    let decoded = decode_ndjson_chunks_partial(vec![body.to_vec()]);
    let mut state = ParserState::default();
    let mut event_count = 0;
    let mut malformed_stream = false;

    for (record_index, item) in decoded.items.iter().enumerate() {
        let record = match item {
            NdjsonItem::Record(record) => record,
            NdjsonItem::Error(error) => {
                malformed_stream = true;
                state.sealed = true;
                push_error(
                    &mut state.contract_errors,
                    framing_error_code(error),
                    &format!("/records/{record_index}"),
                );
                continue;
            }
        };
        event_count += 1;
        let Some(record) = record.as_object() else {
            continue;
        };
        let sealed_before_record = state.sealed;

        if record.contains_key("error") {
            if sealed_before_record {
                push_error(
                    &mut state.contract_errors,
                    "record_after_terminal",
                    &format!("/records/{record_index}"),
                );
            }
            push_error(
                &mut state.contract_errors,
                "api_error",
                &format!("/records/{record_index}/error"),
            );
            state.protocol_error = true;
            state.sealed = true;
            continue;
        }

        let envelope = validate_record(&mut state, record, record_index);
        if sealed_before_record {
            push_error(
                &mut state.contract_errors,
                "record_after_terminal",
                &format!("/records/{record_index}"),
            );
            state.protocol_error = true;
            observe_terminal(
                &mut state,
                envelope.done,
                envelope.done_reason,
                record_index,
            );
            continue;
        }

        if let Some(message) = envelope.message {
            process_message(&mut state, message, record_index);
        }
        observe_terminal(
            &mut state,
            envelope.done,
            envelope.done_reason,
            record_index,
        );
    }

    if state.saw_indexed_call && state.saw_unindexed_call {
        push_error(
            &mut state.contract_errors,
            "mixed_function_indexes",
            "/message/tool_calls",
        );
        state.invalid_call_set = true;
    }

    let stream_termination = if malformed_stream {
        StreamTermination::MalformedStream
    } else if state.protocol_error {
        StreamTermination::ProtocolError
    } else if !state.saw_done {
        StreamTermination::MissingTerminalEvent
    } else {
        match state.done_reason.as_deref() {
            None | Some("stop") => StreamTermination::Completed,
            Some(_) => StreamTermination::ModelIncomplete,
        }
    };
    let stream_end_signal = if state.saw_done {
        StreamEndSignal::OllamaDone
    } else {
        StreamEndSignal::None
    };
    let assistant_turn = build_assistant_turn(&mut state);

    StreamParseResult {
        assistant_turn,
        stream_termination,
        stream_end_signal,
        model_stop_reason: state.done_reason,
        event_count,
        contract_errors: state.contract_errors,
    }
}

fn framing_error_code(error: &FramingError) -> &'static str {
    match error {
        FramingError::IncompleteNdjson { .. } => "truncated_ndjson",
        FramingError::InvalidNdjson { .. } => "malformed_ndjson",
        FramingError::NdjsonNotObject { .. } => "non_object_record",
        FramingError::InvalidSse { .. } => "invalid_ndjson",
    }
}

struct ValidatedEnvelope<'a> {
    message: Option<&'a Map<String, Value>>,
    done: Option<bool>,
    done_reason: DoneReason,
}

#[derive(Default)]
struct DoneReason {
    present: bool,
    value: Option<String>,
}

fn validate_record<'a>(
    state: &mut ParserState,
    record: &'a Map<String, Value>,
    record_index: usize,
) -> ValidatedEnvelope<'a> {
    let base = format!("/records/{record_index}");
    if record
        .get("model")
        .and_then(Value::as_str)
        .is_none_or(|model| model.trim().is_empty())
    {
        push_error(
            &mut state.contract_errors,
            "invalid_model",
            &format!("{base}/model"),
        );
    }

    let valid_created_at = record
        .get("created_at")
        .and_then(Value::as_str)
        .is_some_and(|timestamp| DateTime::parse_from_rfc3339(timestamp).is_ok());
    if !valid_created_at {
        push_error(
            &mut state.contract_errors,
            "invalid_created_at",
            &format!("{base}/created_at"),
        );
    }

    let message = record.get("message").and_then(Value::as_object);
    if message.is_none() {
        push_error(
            &mut state.contract_errors,
            "invalid_message",
            &format!("{base}/message"),
        );
    }

    let done = record.get("done").and_then(Value::as_bool);
    if done.is_none() {
        push_error(
            &mut state.contract_errors,
            "invalid_done",
            &format!("{base}/done"),
        );
    }

    for field in METRIC_FIELDS {
        if record
            .get(field)
            .is_some_and(|value| value.as_u64().is_none())
        {
            push_error(
                &mut state.contract_errors,
                "invalid_metric",
                &format!("{base}/{field}"),
            );
        }
    }

    let done_reason = match record.get("done_reason") {
        None => DoneReason::default(),
        Some(Value::String(reason)) if !reason.is_empty() => DoneReason {
            present: true,
            value: Some(reason.clone()),
        },
        Some(_) => {
            push_error(
                &mut state.contract_errors,
                "invalid_done_reason",
                &format!("{base}/done_reason"),
            );
            DoneReason {
                present: true,
                value: None,
            }
        }
    };

    ValidatedEnvelope {
        message,
        done,
        done_reason,
    }
}

fn observe_terminal(
    state: &mut ParserState,
    done: Option<bool>,
    done_reason: DoneReason,
    record_index: usize,
) {
    if done != Some(true) {
        if done_reason.present {
            push_error(
                &mut state.contract_errors,
                "done_reason_before_terminal",
                &format!("/records/{record_index}/done_reason"),
            );
        }
        return;
    }

    if state.saw_done {
        push_error(
            &mut state.contract_errors,
            "duplicate_done",
            &format!("/records/{record_index}/done"),
        );
        state.protocol_error = true;
    } else {
        state.saw_done = true;
        state.done_reason = done_reason.value;
    }
    state.sealed = true;
}

fn process_message(state: &mut ParserState, message: &Map<String, Value>, record_index: usize) {
    let base = format!("/records/{record_index}/message");
    state.saw_message = true;

    match message.get("role").and_then(Value::as_str) {
        Some("assistant") => {
            state
                .history
                .insert("role".into(), Value::String("assistant".into()));
        }
        _ => push_error(
            &mut state.contract_errors,
            "invalid_role",
            &format!("{base}/role"),
        ),
    }

    match message.get("content").and_then(Value::as_str) {
        Some(content) => {
            state.final_text.push_str(content);
            state
                .history
                .insert("content".into(), Value::String(state.final_text.clone()));
        }
        None => push_error(
            &mut state.contract_errors,
            "invalid_content",
            &format!("{base}/content"),
        ),
    }

    if let Some(thinking) = message.get("thinking") {
        match thinking.as_str() {
            Some(thinking) => {
                state.saw_thinking = true;
                state.thinking.push_str(thinking);
                state
                    .history
                    .insert("thinking".into(), Value::String(state.thinking.clone()));
            }
            None => push_error(
                &mut state.contract_errors,
                "invalid_thinking",
                &format!("{base}/thinking"),
            ),
        }
    }

    process_images(state, message.get("images"), &base);
    process_tool_calls(state, message.get("tool_calls"), record_index);

    for (key, value) in message {
        if !matches!(
            key.as_str(),
            "role" | "content" | "thinking" | "images" | "tool_calls"
        ) {
            state.history.insert(key.clone(), value.clone());
        }
    }
}

fn process_images(state: &mut ParserState, images: Option<&Value>, base: &str) {
    let Some(images) = images else {
        return;
    };
    match images {
        Value::Null => {
            state.history.entry("images").or_insert(Value::Null);
        }
        Value::Array(incoming) => {
            let history_images = state
                .history
                .entry("images")
                .or_insert_with(|| Value::Array(Vec::new()));
            if !history_images.is_array() {
                *history_images = Value::Array(Vec::new());
            }
            history_images
                .as_array_mut()
                .expect("images was normalized to an array")
                .extend(incoming.iter().cloned());
        }
        _ => push_error(
            &mut state.contract_errors,
            "invalid_images",
            &format!("{base}/images"),
        ),
    }
}

fn process_tool_calls(state: &mut ParserState, tool_calls: Option<&Value>, record_index: usize) {
    let Some(tool_calls) = tool_calls else {
        return;
    };
    if tool_calls.is_null() {
        state.history.entry("tool_calls").or_insert(Value::Null);
        return;
    }
    let Some(tool_calls) = tool_calls.as_array() else {
        push_error(
            &mut state.contract_errors,
            "invalid_tool_calls",
            &format!("/records/{record_index}/message/tool_calls"),
        );
        return;
    };

    state.saw_tool_calls = true;
    for (position, raw_call) in tool_calls.iter().enumerate() {
        state.raw_tool_calls.push(raw_call.clone());
        process_tool_call(state, raw_call, record_index, position);
    }
    state.history.insert(
        "tool_calls".into(),
        Value::Array(state.raw_tool_calls.clone()),
    );
}

fn process_tool_call(
    state: &mut ParserState,
    raw_call: &Value,
    record_index: usize,
    position: usize,
) {
    let base = format!("/records/{record_index}/message/tool_calls/{position}");
    let Some(call) = raw_call.as_object() else {
        push_error(&mut state.contract_errors, "invalid_tool_call", &base);
        return;
    };
    let Some(function) = call.get("function").and_then(Value::as_object) else {
        push_error(
            &mut state.contract_errors,
            "invalid_function",
            &format!("{base}/function"),
        );
        return;
    };
    let Some(name) = function
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
    else {
        push_error(
            &mut state.contract_errors,
            "invalid_function_name",
            &format!("{base}/function/name"),
        );
        return;
    };
    let Some(arguments) = function.get("arguments").filter(|value| value.is_object()) else {
        push_error(
            &mut state.contract_errors,
            "invalid_function_arguments",
            &format!("{base}/function/arguments"),
        );
        return;
    };

    let correlation = match call.get("id") {
        None => ToolCorrelation::Optional(None),
        Some(Value::String(id)) if !id.trim().is_empty() => {
            if !state.seen_tool_ids.insert(id.clone()) {
                push_error(
                    &mut state.contract_errors,
                    "duplicate_tool_call_id",
                    &format!("{base}/id"),
                );
                state.invalid_call_set = true;
            }
            ToolCorrelation::Optional(Some(id.clone()))
        }
        Some(_) => {
            push_error(
                &mut state.contract_errors,
                "invalid_tool_call_id",
                &format!("{base}/id"),
            );
            return;
        }
    };

    let native_index = match function.get("index") {
        None => {
            state.saw_unindexed_call = true;
            None
        }
        Some(value) => {
            let Some(index) = value.as_u64().and_then(|index| usize::try_from(index).ok()) else {
                push_error(
                    &mut state.contract_errors,
                    "invalid_function_index",
                    &format!("{base}/function/index"),
                );
                return;
            };
            state.saw_indexed_call = true;
            if !state.seen_tool_indexes.insert(index) {
                push_error(
                    &mut state.contract_errors,
                    "duplicate_function_index",
                    &format!("{base}/function/index"),
                );
                state.invalid_call_set = true;
            }
            Some(index)
        }
    };

    state.pending_tool_calls.push(PendingToolCall {
        native_index,
        correlation,
        name: name.into(),
        arguments: arguments.clone(),
    });
}

fn build_assistant_turn(state: &mut ParserState) -> Option<AssistantTurn> {
    if !state.saw_message {
        return None;
    }
    if state.saw_thinking {
        state
            .history
            .insert("thinking".into(), Value::String(state.thinking.clone()));
    }
    if state.saw_tool_calls {
        state.history.insert(
            "tool_calls".into(),
            Value::Array(state.raw_tool_calls.clone()),
        );
    }

    let mut pending = std::mem::take(&mut state.pending_tool_calls);
    let tool_calls = if state.invalid_call_set {
        Vec::new()
    } else if state.saw_indexed_call {
        pending.sort_by_key(|call| call.native_index);
        pending
            .into_iter()
            .map(|call| ToolCall {
                index: call.native_index.expect("all executable calls are indexed"),
                correlation: call.correlation,
                name: call.name,
                arguments: call.arguments,
            })
            .collect()
    } else {
        pending
            .into_iter()
            .enumerate()
            .map(|(index, call)| ToolCall {
                index,
                correlation: call.correlation,
                name: call.name,
                arguments: call.arguments,
            })
            .collect()
    };

    Some(AssistantTurn {
        history: ProtocolHistory::Ollama(Value::Object(state.history.clone())),
        tool_calls,
        final_text: state.final_text.clone(),
    })
}

fn push_error(errors: &mut Vec<String>, code: &str, path: &str) {
    errors.push(format!("ollama.{code}:{path}"));
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::evidence::{StreamEndSignal, StreamTermination};
    use crate::protocol::stream::{ProtocolHistory, ToolCorrelation};

    use super::parse;

    const TOOL_STREAM: &[u8] = include_bytes!("../fixtures/ollama_tool.ndjson");
    const FINAL_STREAM: &[u8] = include_bytes!("../fixtures/ollama_final.ndjson");
    const MISSING_TERMINAL: &[u8] = include_bytes!("../fixtures/ollama_missing_terminal.ndjson");
    const TRUNCATED: &[u8] = include_bytes!("../fixtures/ollama_truncated.ndjson");
    const API_ERROR: &[u8] = include_bytes!("../fixtures/ollama_error.ndjson");
    const INCOMPLETE: &[u8] = include_bytes!("../fixtures/ollama_incomplete.ndjson");

    #[tokio::test]
    async fn accumulates_official_tool_stream_and_preserves_native_history() {
        let parsed = parse(TOOL_STREAM).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert_eq!(parsed.stream_end_signal, StreamEndSignal::OllamaDone);
        assert_eq!(parsed.model_stop_reason.as_deref(), Some("stop"));
        assert_eq!(parsed.event_count, 3);
        assert!(parsed.contract_errors.is_empty());

        let turn = parsed.assistant_turn.expect("assistant turn");
        assert_eq!(turn.final_text, "Calling tools.");
        assert_eq!(turn.tool_calls.len(), 2);
        assert_eq!(turn.tool_calls[0].index, 0);
        assert_eq!(turn.tool_calls[0].name, "get_time");
        assert_eq!(
            turn.tool_calls[0].correlation,
            ToolCorrelation::Optional(Some("call_time_046".into()))
        );
        assert_eq!(turn.tool_calls[0].arguments, json!({"zone": "UTC"}));
        assert_eq!(turn.tool_calls[1].index, 1);
        assert_eq!(turn.tool_calls[1].name, "get_weather");
        assert_eq!(
            turn.tool_calls[1].correlation,
            ToolCorrelation::Optional(Some("call_weather_046".into()))
        );
        assert_eq!(
            turn.tool_calls[1].arguments,
            json!({"city": "Beijing", "units": {"temperature": "celsius"}})
        );
        assert_eq!(
            turn.history,
            ProtocolHistory::Ollama(json!({
                "role": "assistant",
                "thinking": "Need weather. ",
                "content": "Calling tools.",
                "future_message": {"trace": 1},
                "tool_calls": [
                    {
                        "id": "call_weather_046",
                        "type": "function",
                        "function": {
                            "index": 1,
                            "name": "get_weather",
                            "arguments": {"city": "Beijing", "units": {"temperature": "celsius"}},
                            "future_function": "keep"
                        },
                        "future_call": "keep"
                    },
                    {
                        "id": "call_time_046",
                        "type": "function",
                        "function": {
                            "index": 0,
                            "name": "get_time",
                            "arguments": {"zone": "UTC"}
                        }
                    }
                ]
            }))
        );
    }

    #[tokio::test]
    async fn reconstructs_final_text_and_accepts_absent_done_reason() {
        let parsed = parse(FINAL_STREAM).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert_eq!(parsed.stream_end_signal, StreamEndSignal::OllamaDone);
        assert_eq!(parsed.model_stop_reason, None);
        assert!(parsed.contract_errors.is_empty());
        let turn = parsed.assistant_turn.expect("assistant turn");
        assert_eq!(turn.final_text, "MODEL_DOCTOR_CASE_046_OK");
        assert_eq!(
            turn.history,
            ProtocolHistory::Ollama(json!({
                "role": "assistant",
                "thinking": "hidden",
                "content": "MODEL_DOCTOR_CASE_046_OK",
                "images": null,
                "future_message": "kept"
            }))
        );
    }

    #[tokio::test]
    async fn distinguishes_missing_incomplete_truncated_and_api_error() {
        let missing = parse(MISSING_TERMINAL).await;
        assert_eq!(
            missing.stream_termination,
            StreamTermination::MissingTerminalEvent
        );
        assert_eq!(missing.stream_end_signal, StreamEndSignal::None);
        assert_eq!(missing.event_count, 1);

        let incomplete = parse(INCOMPLETE).await;
        assert_eq!(
            incomplete.stream_termination,
            StreamTermination::ModelIncomplete
        );
        assert_eq!(incomplete.stream_end_signal, StreamEndSignal::OllamaDone);
        assert_eq!(incomplete.model_stop_reason.as_deref(), Some("length"));
        assert!(incomplete.contract_errors.is_empty());

        let truncated = parse(TRUNCATED.strip_suffix(b"\n").unwrap_or(TRUNCATED)).await;
        assert_eq!(
            truncated.stream_termination,
            StreamTermination::MalformedStream
        );
        assert_eq!(truncated.stream_end_signal, StreamEndSignal::None);
        assert_eq!(truncated.event_count, 1);
        assert_eq!(
            truncated.contract_errors,
            ["ollama.truncated_ndjson:/records/1"]
        );

        let error = parse(API_ERROR).await;
        assert_eq!(error.stream_termination, StreamTermination::ProtocolError);
        assert_eq!(error.stream_end_signal, StreamEndSignal::None);
        assert_eq!(error.event_count, 1);
        assert_eq!(error.contract_errors, ["ollama.api_error:/records/0/error"]);
        assert!(!error.contract_errors.join(" ").contains("sensitive"));
    }

    #[tokio::test]
    async fn requires_official_record_envelope_and_rfc3339_timestamp() {
        let cases = [
            (
                json!({"created_at": timestamp(), "message": assistant(""), "done": true}),
                "ollama.invalid_model:/records/0/model",
            ),
            (
                json!({"model": "", "created_at": timestamp(), "message": assistant(""), "done": true}),
                "ollama.invalid_model:/records/0/model",
            ),
            (
                json!({"model": "qwen3", "created_at": "PRIVATE_TIME", "message": assistant(""), "done": true}),
                "ollama.invalid_created_at:/records/0/created_at",
            ),
            (
                json!({"model": "qwen3", "created_at": timestamp(), "done": true}),
                "ollama.invalid_message:/records/0/message",
            ),
            (
                json!({"model": "qwen3", "created_at": timestamp(), "message": assistant("")}),
                "ollama.invalid_done:/records/0/done",
            ),
        ];

        for (record, expected) in cases {
            let parsed = parse(stream(&[record]).as_bytes()).await;
            assert!(
                parsed.contract_errors.contains(&expected.to_owned()),
                "{expected}"
            );
            assert!(!parsed.contract_errors.join(" ").contains("PRIVATE_TIME"));
        }
    }

    #[tokio::test]
    async fn rejects_whitespace_only_model_function_name_and_call_id() {
        let blank_model = normal(assistant("ok"), true, None);
        let mut blank_model = blank_model.as_object().expect("record").clone();
        blank_model.insert("model".into(), json!(" \t"));
        let parsed = parse(stream(&[Value::Object(blank_model)]).as_bytes()).await;
        assert!(
            parsed
                .contract_errors
                .contains(&"ollama.invalid_model:/records/0/model".to_owned())
        );

        let blank_name = json!({
            "id": "call_1",
            "function": {"name": " \n", "arguments": {}}
        });
        let parsed = parse(stream(&[tool_record(vec![blank_name], true)]).as_bytes()).await;
        assert!(
            parsed.contract_errors.contains(
                &"ollama.invalid_function_name:/records/0/message/tool_calls/0/function/name"
                    .to_owned()
            )
        );

        let blank_id = json!({
            "id": " \t",
            "function": {"name": "get_weather", "arguments": {}}
        });
        let parsed = parse(stream(&[tool_record(vec![blank_id], true)]).as_bytes()).await;
        assert!(parsed.contract_errors.contains(
            &"ollama.invalid_tool_call_id:/records/0/message/tool_calls/0/id".to_owned()
        ));
    }

    #[tokio::test]
    async fn validates_message_fields_but_keeps_extension_fields_forward_compatible() {
        let record = normal(
            json!({
                "role": "PRIVATE_ROLE",
                "content": 7,
                "thinking": false,
                "tool_calls": "PRIVATE_CALLS",
                "images": "PRIVATE_IMAGES",
                "future_extension": {"nested": true}
            }),
            true,
            Some(json!("stop")),
        );
        let parsed = parse(stream(&[record]).as_bytes()).await;

        for expected in [
            "ollama.invalid_role:/records/0/message/role",
            "ollama.invalid_content:/records/0/message/content",
            "ollama.invalid_thinking:/records/0/message/thinking",
            "ollama.invalid_tool_calls:/records/0/message/tool_calls",
            "ollama.invalid_images:/records/0/message/images",
        ] {
            assert!(
                parsed.contract_errors.contains(&expected.to_owned()),
                "{expected}"
            );
        }
        assert!(!parsed.contract_errors.join(" ").contains("PRIVATE"));
    }

    #[tokio::test]
    async fn validates_each_optional_timing_and_count_field_independently() {
        for field in [
            "total_duration",
            "load_duration",
            "prompt_eval_count",
            "prompt_eval_duration",
            "eval_count",
            "eval_duration",
        ] {
            let mut record = normal(assistant("ok"), true, None);
            record
                .as_object_mut()
                .expect("record")
                .insert(field.into(), json!(-1));
            let parsed = parse(stream(&[record]).as_bytes()).await;
            assert!(
                parsed
                    .contract_errors
                    .contains(&format!("ollama.invalid_metric:/records/0/{field}"))
            );
        }

        let valid = normal(assistant("ok"), true, None);
        let parsed = parse(stream(&[valid]).as_bytes()).await;
        assert!(parsed.contract_errors.is_empty());
    }

    #[tokio::test]
    async fn rejects_invalid_tool_shapes_without_inventing_executable_calls() {
        let cases = [
            (
                json!({}),
                "ollama.invalid_function:/records/0/message/tool_calls/0/function",
            ),
            (
                json!({"function": {"arguments": {}}}),
                "ollama.invalid_function_name:/records/0/message/tool_calls/0/function/name",
            ),
            (
                json!({"function": {"name": "", "arguments": {}}}),
                "ollama.invalid_function_name:/records/0/message/tool_calls/0/function/name",
            ),
            (
                json!({"function": {"name": "get_weather"}}),
                "ollama.invalid_function_arguments:/records/0/message/tool_calls/0/function/arguments",
            ),
            (
                json!({"function": {"name": "get_weather", "arguments": "PRIVATE_ARG"}}),
                "ollama.invalid_function_arguments:/records/0/message/tool_calls/0/function/arguments",
            ),
            (
                json!({"function": {"name": "get_weather", "arguments": null}}),
                "ollama.invalid_function_arguments:/records/0/message/tool_calls/0/function/arguments",
            ),
            (
                json!({"function": {"name": "get_weather", "arguments": []}}),
                "ollama.invalid_function_arguments:/records/0/message/tool_calls/0/function/arguments",
            ),
            (
                json!({"id": "", "function": {"name": "get_weather", "arguments": {}}}),
                "ollama.invalid_tool_call_id:/records/0/message/tool_calls/0/id",
            ),
            (
                json!({"id": null, "function": {"name": "get_weather", "arguments": {}}}),
                "ollama.invalid_tool_call_id:/records/0/message/tool_calls/0/id",
            ),
            (
                json!({"function": {"index": -1, "name": "get_weather", "arguments": {}}}),
                "ollama.invalid_function_index:/records/0/message/tool_calls/0/function/index",
            ),
            (
                json!({"function": {"index": "PRIVATE_INDEX", "name": "get_weather", "arguments": {}}}),
                "ollama.invalid_function_index:/records/0/message/tool_calls/0/function/index",
            ),
        ];

        for (call, expected) in cases {
            let parsed = parse(stream(&[tool_record(vec![call], true)]).as_bytes()).await;
            assert!(
                parsed.contract_errors.contains(&expected.to_owned()),
                "{expected}"
            );
            assert!(
                parsed
                    .assistant_turn
                    .expect("native history")
                    .tool_calls
                    .is_empty()
            );
            assert!(!parsed.contract_errors.join(" ").contains("PRIVATE_ARG"));
        }
    }

    #[tokio::test]
    async fn treats_call_ids_as_optional_but_rejects_duplicates() {
        let no_ids = vec![call(None, None, "first"), call(None, None, "second")];
        let parsed = parse(stream(&[tool_record(no_ids, true)]).as_bytes()).await;
        let calls = parsed.assistant_turn.expect("turn").tool_calls;
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].correlation, ToolCorrelation::Optional(None));
        assert_eq!(calls[1].correlation, ToolCorrelation::Optional(None));

        let duplicate = vec![
            call(Some("PRIVATE_DUPLICATE_ID"), None, "first"),
            call(Some("PRIVATE_DUPLICATE_ID"), None, "second"),
        ];
        let parsed = parse(stream(&[tool_record(duplicate, true)]).as_bytes()).await;
        assert!(parsed.contract_errors.contains(
            &"ollama.duplicate_tool_call_id:/records/0/message/tool_calls/1/id".to_owned()
        ));
        assert!(
            parsed
                .assistant_turn
                .expect("history")
                .tool_calls
                .is_empty()
        );
        assert!(
            !parsed
                .contract_errors
                .join(" ")
                .contains("PRIVATE_DUPLICATE_ID")
        );
    }

    #[tokio::test]
    async fn orders_indexed_calls_and_uses_encounter_order_when_all_are_unindexed() {
        let indexed = vec![
            call(None, Some(9), "ninth"),
            call(None, Some(1_000_000_000), "sparse"),
            call(None, Some(2), "second"),
        ];
        let parsed = parse(stream(&[tool_record(indexed, true)]).as_bytes()).await;
        let calls = parsed.assistant_turn.expect("turn").tool_calls;
        assert_eq!(
            calls.iter().map(|call| call.index).collect::<Vec<_>>(),
            [2, 9, 1_000_000_000]
        );
        assert_eq!(
            calls
                .iter()
                .map(|call| call.name.as_str())
                .collect::<Vec<_>>(),
            ["second", "ninth", "sparse"]
        );

        let unindexed = vec![call(None, None, "first"), call(None, None, "second")];
        let parsed = parse(stream(&[tool_record(unindexed, true)]).as_bytes()).await;
        let calls = parsed.assistant_turn.expect("turn").tool_calls;
        assert_eq!(
            calls.iter().map(|call| call.index).collect::<Vec<_>>(),
            [0, 1]
        );
        assert_eq!(calls[0].name, "first");
        assert_eq!(calls[1].name, "second");
    }

    #[tokio::test]
    async fn rejects_mixed_or_duplicate_function_indexes_for_the_whole_turn() {
        let cases = [
            (
                vec![call(None, Some(0), "first"), call(None, None, "second")],
                "ollama.mixed_function_indexes:/message/tool_calls",
            ),
            (
                vec![call(None, Some(3), "first"), call(None, Some(3), "second")],
                "ollama.duplicate_function_index:/records/0/message/tool_calls/1/function/index",
            ),
        ];

        for (calls, expected) in cases {
            let parsed = parse(stream(&[tool_record(calls, true)]).as_bytes()).await;
            assert!(
                parsed.contract_errors.contains(&expected.to_owned()),
                "{expected}"
            );
            assert!(
                parsed
                    .assistant_turn
                    .expect("history")
                    .tool_calls
                    .is_empty()
            );
        }
    }

    #[tokio::test]
    async fn appends_complete_tool_calls_across_records_instead_of_merging_arguments() {
        let records = [
            tool_record(vec![call(None, None, "first")], false),
            tool_record(vec![call(None, None, "second")], true),
        ];
        let parsed = parse(stream(&records).as_bytes()).await;
        let calls = parsed.assistant_turn.expect("turn").tool_calls;
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "first");
        assert_eq!(calls[1].name, "second");
        assert_eq!(calls[0].arguments, json!({"name": "first"}));
    }

    #[tokio::test]
    async fn validates_done_reason_position_and_value() {
        let records = [
            normal(assistant("before"), false, Some(json!("stop"))),
            normal(assistant("after"), true, Some(json!("stop"))),
        ];
        let parsed = parse(stream(&records).as_bytes()).await;
        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert!(
            parsed
                .contract_errors
                .contains(&"ollama.done_reason_before_terminal:/records/0/done_reason".to_owned())
        );

        let unknown = parse(
            stream(&[normal(
                assistant("partial"),
                true,
                Some(json!("future_limit")),
            )])
            .as_bytes(),
        )
        .await;
        assert_eq!(
            unknown.stream_termination,
            StreamTermination::ModelIncomplete
        );
        assert_eq!(unknown.model_stop_reason.as_deref(), Some("future_limit"));

        for reason in [Value::Null, json!(""), json!(7)] {
            let parsed =
                parse(stream(&[normal(assistant("ok"), true, Some(reason))]).as_bytes()).await;
            assert_eq!(parsed.stream_termination, StreamTermination::Completed);
            assert!(
                parsed
                    .contract_errors
                    .contains(&"ollama.invalid_done_reason:/records/0/done_reason".to_owned())
            );
        }
    }

    #[tokio::test]
    async fn terminal_seals_history_but_later_done_remains_observable() {
        let records = [
            normal(assistant("before"), true, Some(json!("stop"))),
            normal(
                assistant("PRIVATE_AFTER_TERMINAL"),
                true,
                Some(json!("stop")),
            ),
        ];
        let parsed = parse(stream(&records).as_bytes()).await;

        assert_eq!(parsed.stream_termination, StreamTermination::ProtocolError);
        assert_eq!(parsed.stream_end_signal, StreamEndSignal::OllamaDone);
        assert!(
            parsed
                .contract_errors
                .contains(&"ollama.record_after_terminal:/records/1".to_owned())
        );
        assert!(
            parsed
                .contract_errors
                .contains(&"ollama.duplicate_done:/records/1/done".to_owned())
        );
        assert_eq!(
            parsed.assistant_turn.expect("sealed turn").final_text,
            "before"
        );
    }

    #[tokio::test]
    async fn api_error_seals_history_and_later_done_is_only_an_observed_signal() {
        let body = format!(
            "{}\n{{\"error\":\"PRIVATE_ERROR\"}}\n{}\n",
            normal(assistant("before"), false, None),
            normal(assistant("PRIVATE_AFTER_ERROR"), true, Some(json!("stop")))
        );
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(parsed.stream_termination, StreamTermination::ProtocolError);
        assert_eq!(parsed.stream_end_signal, StreamEndSignal::OllamaDone);
        assert_eq!(parsed.model_stop_reason.as_deref(), Some("stop"));
        assert_eq!(
            parsed.assistant_turn.expect("sealed turn").final_text,
            "before"
        );
        assert!(
            parsed
                .contract_errors
                .contains(&"ollama.api_error:/records/1/error".to_owned())
        );
        assert!(
            parsed
                .contract_errors
                .contains(&"ollama.record_after_terminal:/records/2".to_owned())
        );
        assert!(!parsed.contract_errors.join(" ").contains("PRIVATE"));
    }

    #[tokio::test]
    async fn malformed_record_after_done_has_precedence_and_retains_done_signal() {
        let body = format!(
            "{}\n{{\"model\":",
            normal(assistant("complete"), true, Some(json!("stop")))
        );
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(
            parsed.stream_termination,
            StreamTermination::MalformedStream
        );
        assert_eq!(parsed.stream_end_signal, StreamEndSignal::OllamaDone);
        assert_eq!(parsed.model_stop_reason.as_deref(), Some("stop"));
        assert_eq!(parsed.event_count, 1);
        assert_eq!(
            parsed.contract_errors,
            ["ollama.truncated_ndjson:/records/1"]
        );
    }

    #[tokio::test]
    async fn malformed_record_seals_history_but_later_done_remains_observable() {
        let after_error = normal(
            json!({
                "role": "assistant",
                "content": "PRIVATE_AFTER_MALFORMED",
                "tool_calls": [{
                    "id": "PRIVATE_CALL_ID",
                    "function": {
                        "name": "PRIVATE_TOOL_NAME",
                        "arguments": {"secret": "PRIVATE_ARGUMENT"}
                    }
                }]
            }),
            true,
            Some(json!("stop")),
        );
        let body = format!(
            "{}\nPRIVATE_BAD_JSON\n{after_error}\n",
            normal(assistant("before"), false, None)
        );
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(
            parsed.stream_termination,
            StreamTermination::MalformedStream
        );
        assert_eq!(parsed.stream_end_signal, StreamEndSignal::OllamaDone);
        assert_eq!(parsed.model_stop_reason.as_deref(), Some("stop"));
        assert_eq!(parsed.event_count, 2);
        assert!(
            parsed
                .contract_errors
                .contains(&"ollama.malformed_ndjson:/records/1".to_owned())
        );
        assert!(
            parsed
                .contract_errors
                .contains(&"ollama.record_after_terminal:/records/2".to_owned())
        );

        let turn = parsed.assistant_turn.expect("pre-error history");
        assert_eq!(turn.final_text, "before");
        assert!(turn.tool_calls.is_empty());
        assert_eq!(
            turn.history,
            ProtocolHistory::Ollama(json!({"role": "assistant", "content": "before"}))
        );
        assert!(!parsed.contract_errors.join(" ").contains("PRIVATE"));
    }

    #[tokio::test]
    async fn multiple_framing_errors_and_truncated_tail_keep_wire_order_and_done_signal() {
        let body = format!(
            "{}\nPRIVATE_BAD_ONE\n[]\n{}\n{{\"tail\":",
            normal(assistant("before"), false, None),
            normal(assistant("PRIVATE_AFTER_ERRORS"), true, Some(json!("stop")))
        );
        let parsed = parse(body.as_bytes()).await;

        assert_eq!(
            parsed.stream_termination,
            StreamTermination::MalformedStream
        );
        assert_eq!(parsed.stream_end_signal, StreamEndSignal::OllamaDone);
        assert_eq!(parsed.event_count, 2);
        assert_eq!(
            parsed.contract_errors,
            [
                "ollama.malformed_ndjson:/records/1",
                "ollama.non_object_record:/records/2",
                "ollama.record_after_terminal:/records/3",
                "ollama.truncated_ndjson:/records/4",
            ]
        );
        assert_eq!(
            parsed.assistant_turn.expect("sealed history").final_text,
            "before"
        );
        assert!(!parsed.contract_errors.join(" ").contains("PRIVATE"));
    }

    #[tokio::test]
    async fn invalid_json_or_non_object_after_done_retains_the_observed_signal() {
        for (suffix, expected) in [
            ("not-json\n", "ollama.malformed_ndjson:/records/1"),
            ("[]\n", "ollama.non_object_record:/records/1"),
        ] {
            let body = format!(
                "{}\n{suffix}",
                normal(assistant("complete"), true, Some(json!("stop")))
            );
            let parsed = parse(body.as_bytes()).await;

            assert_eq!(
                parsed.stream_termination,
                StreamTermination::MalformedStream
            );
            assert_eq!(parsed.stream_end_signal, StreamEndSignal::OllamaDone);
            assert_eq!(parsed.event_count, 1);
            assert_eq!(parsed.contract_errors, [expected]);
        }
    }

    #[tokio::test]
    async fn accepts_null_or_array_images_and_null_tool_calls() {
        let records = [
            normal(
                json!({"role": "assistant", "content": "a", "images": null, "tool_calls": null}),
                false,
                None,
            ),
            normal(
                json!({"role": "assistant", "content": "b", "images": ["one", "two"]}),
                true,
                None,
            ),
        ];
        let parsed = parse(stream(&records).as_bytes()).await;

        assert_eq!(parsed.stream_termination, StreamTermination::Completed);
        assert!(parsed.contract_errors.is_empty());
        assert_eq!(parsed.assistant_turn.expect("turn").final_text, "ab");
    }

    #[tokio::test]
    async fn empty_stream_is_missing_terminal_without_an_assistant_turn() {
        let parsed = parse(b"").await;
        assert_eq!(
            parsed.stream_termination,
            StreamTermination::MissingTerminalEvent
        );
        assert_eq!(parsed.stream_end_signal, StreamEndSignal::None);
        assert_eq!(parsed.event_count, 0);
        assert!(parsed.assistant_turn.is_none());
    }

    fn timestamp() -> &'static str {
        "2026-08-18T00:00:00Z"
    }

    fn assistant(content: &str) -> Value {
        json!({"role": "assistant", "content": content})
    }

    fn normal(message: Value, done: bool, done_reason: Option<Value>) -> Value {
        let mut record = serde_json::Map::from_iter([
            ("model".into(), json!("qwen3")),
            ("created_at".into(), json!(timestamp())),
            ("message".into(), message),
            ("done".into(), json!(done)),
        ]);
        if let Some(reason) = done_reason {
            record.insert("done_reason".into(), reason);
        }
        Value::Object(record)
    }

    fn tool_record(calls: Vec<Value>, done: bool) -> Value {
        normal(
            json!({"role": "assistant", "content": "", "tool_calls": calls}),
            done,
            done.then(|| json!("stop")),
        )
    }

    fn call(id: Option<&str>, index: Option<u64>, name: &str) -> Value {
        let mut function = serde_json::Map::from_iter([
            ("name".into(), json!(name)),
            ("arguments".into(), json!({"name": name})),
        ]);
        if let Some(index) = index {
            function.insert("index".into(), json!(index));
        }
        let mut call = serde_json::Map::from_iter([("function".into(), Value::Object(function))]);
        if let Some(id) = id {
            call.insert("id".into(), json!(id));
        }
        Value::Object(call)
    }

    fn stream(records: &[Value]) -> String {
        records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    }
}
