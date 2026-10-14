use agent_check::{
    config::Protocol,
    dataset::{SourceSnapshot, validate_task},
    model::{CheckId, ToolEvent},
    transport::parse_stream,
};
use serde_json::json;

#[test]
fn repeated_complete_stream_identifiers_preserve_the_original_call() {
    let chunks = [
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_function_va1ephc4joh9_1","function":{"name":"read","arguments":""}}]},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_function_va1ephc4joh9_1","function":{"name":"read","arguments":"{\"path\":"}}]},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_function_va1ephc4joh9_1","function":{"arguments":"\"inputs/source-1.json\"}"}}]},"finish_reason":"tool_calls"}]}),
    ];
    let stream = chunks
        .iter()
        .map(|c| format!("data: {c}\n\n"))
        .collect::<String>()
        + "data: [DONE]\n\n";
    let response = parse_stream(Protocol::ChatCompletions, stream.as_bytes()).unwrap();
    assert_eq!(response.calls[0].id, "call_function_va1ephc4joh9_1");
    assert_eq!(response.calls[0].name, "read");
    assert_eq!(response.calls[0].arguments["path"], "inputs/source-1.json");
}

#[test]
fn conflicting_stream_identifiers_are_not_silently_combined() {
    let stream = [
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"read","arguments":"{"}}]},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_b","function":{"arguments":"}"}}]},"finish_reason":"tool_calls"}]}),
    ].iter().map(|c| format!("data: {c}\n\n")).collect::<String>() + "data: [DONE]\n\n";
    assert!(parse_stream(Protocol::ChatCompletions, stream.as_bytes()).is_err());
}

#[test]
fn unrelated_successful_file_operations_do_not_prove_task_execution() {
    let snapshot = SourceSnapshot::for_omp("accuracy-regression");
    let output = json!(snapshot.expected(None).unwrap());
    let events = [
        ToolEvent {
            call_id: "read-1".into(),
            name: "read".into(),
            arguments: json!({"path":"unrelated.txt"}),
            result: json!({}),
            is_error: false,
        },
        ToolEvent {
            call_id: "write-1".into(),
            name: "write".into(),
            arguments: json!({"path":"unrelated-copy.txt"}),
            result: json!({}),
            is_error: false,
        },
    ];
    let validation = validate_task(
        &snapshot,
        None,
        &output,
        CheckId::Continuous,
        &events,
        true,
        false,
    )
    .unwrap();
    assert!(!validation.passed);
}

#[test]
fn native_edit_uses_resolved_result_path_without_requiring_write_arguments() {
    let mut event = ToolEvent {
        call_id: "edit-1".into(),
        name: "edit".into(),
        arguments: json!({"input":"[result.json#82E2]\nPUT 1:\n+{}"}),
        result: json!({"details":{"op":"update","path":"/tmp/workspace/result.json"}}),
        is_error: false,
    };
    assert!(agent_check::dataset::references_path(&event, "result.json"));
    event.arguments = json!({"path":"result.json"});
    event.result = json!({"details":{"path":"/tmp/workspace/unrelated.json"}});
    assert!(!agent_check::dataset::references_path(
        &event,
        "result.json"
    ));
}

#[test]
fn batch_reads_prove_each_source_from_delivered_payload_not_requested_paths() {
    let snapshot = SourceSnapshot::for_omp("batch-read-regression");
    let targets: Vec<_> = snapshot
        .files
        .keys()
        .map(|name| format!("inputs/{name}"))
        .collect();
    let sections: Vec<_> = snapshot
        .files
        .iter()
        .map(|(name, file)| {
            let lines = serde_json::to_string_pretty(file)
                .unwrap()
                .lines()
                .enumerate()
                .map(|(i, line)| format!("{}:{line}", i + 1))
                .collect::<Vec<_>>()
                .join("\n");
            format!("[inputs/{name}#AB12]\n{lines}")
        })
        .collect();
    let mut read = ToolEvent {
        call_id: "read-batch".into(),
        name: "read".into(),
        arguments: json!({"path":targets.join(";")}),
        result: json!({"content":[{"type":"text","text":sections.join("\n\n")}],"details":{"displayReadTargets":targets}}),
        is_error: false,
    };
    let write = ToolEvent {
        call_id: "write-result".into(),
        name: "write".into(),
        arguments: json!({"path":"result.json"}),
        result: json!({}),
        is_error: false,
    };
    let output = json!(snapshot.expected(None).unwrap());
    let validate = |read: ToolEvent| {
        validate_task(
            &snapshot,
            None,
            &output,
            CheckId::Continuous,
            &[read, write.clone()],
            true,
            false,
        )
        .unwrap()
    };
    assert!(validate(read.clone()).passed);
    read.result["content"][0]["text"] = json!("requested paths only");
    assert!(!validate(read.clone()).passed);
    read.result["content"][0]["text"] = json!(sections[0]);
    assert!(!validate(read.clone()).passed);
    read.result["content"][0]["text"] = json!(sections.join("\n\n"));
    read.result["details"]["displayReadTargets"] = json!([targets[0]]);
    assert!(!validate(read).passed);
}

#[test]
fn invalid_record_shape_is_a_content_failure_not_only_missing_process() {
    let snapshot = SourceSnapshot::for_omp("accuracy-regression");
    let validation = validate_task(
        &snapshot,
        None,
        &json!({"records":"not an array"}),
        CheckId::Continuous,
        &[],
        true,
        false,
    )
    .unwrap();
    assert!(!validation.content_valid);
    assert!(!validation.passed);
}
