use agent_check::{
    assessment, checks,
    concurrency::{self, ProbeOptions},
    config::{ModelConfig, Protocol},
    dataset::{Record, RecordFile, SourceSnapshot},
    engine::atomic_json,
    judgment::{self, Outcome},
    measure,
    model::{CheckId, CheckResult, FailureOrigin, Finding, Status, TaskResult, ToolEvent},
    registry::builtin_specs,
    transport::{HttpAttempt, HttpEvidence, Transport},
};
use axum::{
    Json, Router, body::Body, extract::State, http::StatusCode, response::Response, routing::post,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Deserialize)]
struct Case {
    id: String,
    check: CheckId,
    variant: String,
    expected: Status,
}
struct Service {
    case: Case,
    requests: AtomicUsize,
    active: AtomicUsize,
}
fn calls(ids: Vec<String>, number: usize) -> Vec<Value> {
    vec![
        json!({"id":format!("call_{number}"),"type":"function","function":{"name":"get_records","arguments":json!({"query":{"ids":ids,"kind":"records"}}).to_string()}}),
    ]
}
fn wire(
    protocol: Protocol,
    text: &str,
    calls: &[Value],
    finish: &str,
    usage: bool,
    input: u64,
) -> Value {
    match protocol {
        Protocol::ChatCompletions => {
            let mut message = json!({"role":"assistant","content":text,"tool_calls":calls});
            if finish == "refusal" {
                message["refusal"] = json!("本次请求被拒绝");
            }
            let mut value = json!({"model":"fixture","choices":[{"index":0,"message":message,"finish_reason":if finish=="refusal"{"stop"}else{finish}}]});
            if usage {
                value["usage"] = json!({"prompt_tokens":input,"completion_tokens":1,"completion_tokens_details":{"reasoning_tokens":0}});
            }
            value
        }
        Protocol::Responses => {
            let mut output = vec![];
            if !text.is_empty() || calls.is_empty() {
                output.push(json!({"type":"message","id":"message_fixture","role":"assistant","status":"completed","content":[if finish=="refusal"{json!({"type":"refusal","refusal":"本次请求被拒绝"})}else{json!({"type":"output_text","text":text})}]}));
            }
            for call in calls {
                output.push(json!({"type":"function_call","id":format!("fc_{}",call["id"].as_str().unwrap()),"call_id":call["id"],"name":call["function"]["name"],"arguments":call["function"]["arguments"],"status":"completed"}));
            }
            let mut value = json!({"model":"fixture","status":if finish=="length"{"incomplete"}else{"completed"},"output":output});
            if usage {
                value["usage"] = json!({"input_tokens":input,"output_tokens":1,"output_tokens_details":{"reasoning_tokens":0}});
            }
            value
        }
    }
}
fn stream(protocol: Protocol, value: &Value) -> String {
    let mut frames = vec![];
    match protocol {
        Protocol::ChatCompletions => {
            let message = &value["choices"][0]["message"];
            let mut delta = message.clone();
            if let Some(calls) = delta["tool_calls"].as_array_mut() {
                for (i, call) in calls.iter_mut().enumerate() {
                    call["index"] = json!(i);
                }
            }
            frames.push(json!({"model":"fixture","choices":[{"index":0,"delta":delta,"finish_reason":value["choices"][0]["finish_reason"]}],"usage":value["usage"]}));
        }
        Protocol::Responses => {
            for (i, item) in value["output"].as_array().unwrap().iter().enumerate() {
                if item["type"] == "message" {
                    for c in item["content"].as_array().unwrap() {
                        if c["type"] == "output_text" {
                            frames.push(
                                json!({"type":"response.output_text.delta","delta":c["text"]}),
                            );
                        }
                    }
                }
                if item["type"] == "function_call" {
                    frames.push(
                        json!({"type":"response.output_item.added","output_index":i,"item":item}),
                    );
                    frames.push(json!({"type":"response.function_call_arguments.delta","item_id":item["id"],"output_index":i,"delta":item["arguments"]}));
                }
            }
            frames.push(json!({"type":if value["status"]=="completed"{"response.completed"}else{"response.incomplete"},"response":value}));
        }
    }
    frames
        .iter()
        .map(|v| format!("data: {v}\n\n"))
        .collect::<String>()
        + "data: [DONE]\n\n"
}
fn http_response(status: StatusCode, body: Value) -> Response {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}
async fn respond(State(state): State<Arc<Service>>, Json(body): Json<Value>) -> Response {
    let number = state.requests.fetch_add(1, Ordering::SeqCst) + 1;
    let variant = state.case.variant.as_str();
    if variant == "unauthorized" {
        return http_response(
            StatusCode::UNAUTHORIZED,
            json!({"error":{"message":"invalid configuration"}}),
        );
    }
    if variant == "retry_then_ok" && number == 1 {
        return http_response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error":{"message":"temporary"}}),
        );
    }
    let protocol = if body.get("input").is_some() {
        Protocol::Responses
    } else {
        Protocol::ChatCompletions
    };
    let messages = body
        .get("messages")
        .or_else(|| body.get("input"))
        .unwrap()
        .as_array()
        .unwrap();
    let prompt = messages[0]["content"].as_str().unwrap_or("");
    let mut text = "连接正常".to_owned();
    let mut tool_calls = vec![];
    let mut finish = "stop";
    let mut include_usage = true;
    let mut input = 1;
    match state.case.check {
        CheckId::Response => {
            if variant == "empty" || (variant == "stream_empty" && body["stream"] == true) {
                text.clear();
            }
            if variant == "truncated" {
                finish = "length";
            }
            if variant == "refusal" {
                finish = "refusal";
            }
        }
        CheckId::Tools => {
            let mut returned = BTreeMap::new();
            let mut returns = 0;
            for item in messages
                .iter()
                .filter(|m| m["role"] == "tool" || m["type"] == "function_call_output")
            {
                returns += 1;
                let content = item
                    .get("content")
                    .or_else(|| item.get("output"))
                    .unwrap()
                    .as_str()
                    .unwrap();
                let data: Value = serde_json::from_str(content).unwrap();
                for r in data["records"].as_array().unwrap() {
                    returned.insert(r["id"].as_str().unwrap().to_owned(), r.clone());
                }
            }
            let wanted = ["R-01", "R-07", "R-12"];
            if variant == "no_call" {
                text = "我已经完成".into();
            } else if returned.is_empty() {
                tool_calls = if variant == "serial" {
                    calls(vec![wanted[0].into()], number)
                } else if variant == "parallel" {
                    let mut list = calls(vec![wanted[0].into()], number * 10);
                    list.extend(calls(
                        vec![wanted[1].into(), wanted[2].into()],
                        number * 10 + 1,
                    ));
                    list
                } else {
                    calls(wanted.iter().map(|v| (*v).into()).collect(), number)
                };
                if variant == "wrong_argument_type" {
                    tool_calls[0]["function"]["arguments"] =
                        json!(json!({"query":{"ids":"R-01","kind":"records"}}).to_string());
                }
                text.clear();
                finish = "tool_calls";
            } else if variant == "serial" && returned.len() < 3 {
                tool_calls = calls(
                    vec![
                        wanted
                            .iter()
                            .find(|id| !returned.contains_key(**id))
                            .unwrap()
                            .to_string(),
                    ],
                    number,
                );
                text.clear();
                finish = "tool_calls";
            } else if variant == "repeated_query" && returns == 1 {
                tool_calls = calls(vec![wanted[0].into()], number);
                text.clear();
                finish = "tool_calls";
            } else {
                let mut records: Vec<_> = returned.into_values().collect();
                if variant == "wrong_value" {
                    records[0]["owner"] = json!("incorrect");
                }
                text = if variant == "prose_only" {
                    "已经查询记录。".into()
                } else {
                    json!({"records":records}).to_string()
                };
            }
        }
        CheckId::Schema => {
            let mut data: Value = serde_json::from_str(prompt.split_once('：').unwrap().1).unwrap();
            if variant == "wrong_value" {
                data["records"][0]["version"] = json!(999);
            }
            if variant == "reordered" {
                data["records"].as_array_mut().unwrap().reverse();
            }
            if variant == "duplicate_record" {
                let row = data["records"][0].clone();
                data["records"].as_array_mut().unwrap().push(row);
            }
            text = match variant {
                "malformed" => "{broken".into(),
                "fenced" => format!("```json\n{data}\n```"),
                _ => data.to_string(),
            };
            if variant == "unsolicited_call" {
                tool_calls = calls(vec!["R-01".into()], 1);
                finish = "tool_calls";
            }
        }
        CheckId::Context => {
            input = prompt.matches(" x").count() as u64 + 57;
            let reservation = body
                .get("max_tokens")
                .or_else(|| body.get("max_output_tokens"))
                .unwrap()
                .as_u64()
                .unwrap();
            let limit = match variant {
                "below_minimum" => 65536,
                "above_minimum" => 262144,
                _ => 131072,
            };
            if variant == "unrelated_rejection" && input + reservation > 200000 {
                return http_response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    json!({"error":{"code":"temporary_error","message":"busy"}}),
                );
            }
            if input + reservation > limit && variant != "unrelated_rejection" {
                return http_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error":{"code":"context_length_exceeded","input_tokens":input,"maximum_context_length":limit}}),
                );
            }
            include_usage = variant != "missing_usage";
        }
        CheckId::Performance => {
            let active = state.active.fetch_add(1, Ordering::SeqCst) + 1;
            tokio::time::sleep(Duration::from_millis(20)).await;
            state.active.fetch_sub(1, Ordering::SeqCst);
            if matches!(variant, "rate_limit" | "extra_text") && active > 2 {
                return http_response(
                    StatusCode::TOO_MANY_REQUESTS,
                    json!({"error":{"message":"limited"}}),
                );
            }
            text = if variant == "empty_response" && number > 3 {
                String::new()
            } else if variant == "extra_text" {
                "我先简短确认一下：你好，连接正常。".into()
            } else {
                "你好。".into()
            };
            include_usage = false;
        }
        _ => unreachable!(),
    }
    let value = wire(protocol, &text, &tool_calls, finish, include_usage, input);
    if body["stream"] == true {
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/event-stream")
            .body(Body::from(stream(protocol, &value)))
            .unwrap()
    } else {
        http_response(StatusCode::OK, value)
    }
}
fn forced_finding() -> Finding {
    Finding {
        code: "forced_label".into(),
        title: "观察层声称失败".into(),
        fact: "标签不能替代证据".into(),
        impact: String::new(),
        suggestion: String::new(),
        origin: FailureOrigin::ModelOutput,
        evidence_ids: vec![],
    }
}
async fn execute_http(case: &Case, protocol: Protocol, dir: &Path) -> CheckResult {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let state = Arc::new(Service {
        case: case.clone(),
        requests: AtomicUsize::new(0),
        active: AtomicUsize::new(0),
    });
    let router = Router::new()
        .route("/v1/chat/completions", post(respond))
        .route("/v1/responses", post(respond))
        .with_state(state);
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let token = CancellationToken::new();
    if case.variant == "cancelled" {
        token.cancel();
    }
    let cfg = ModelConfig::new(
        &format!("http://{address}/v1"),
        "fixture",
        "placeholder".into(),
        Some(protocol),
    )
    .unwrap();
    let transport = Transport::new(cfg, &dir.join("http"), token).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3600);
    let mut result = match case.check {
        CheckId::Response => checks::response(&transport, deadline).await.unwrap(),
        CheckId::Tools => checks::tools(&transport, deadline).await.unwrap(),
        CheckId::Schema => checks::schema(&transport, deadline).await.unwrap(),
        CheckId::Context => measure::context(&transport, deadline).await.unwrap(),
        CheckId::Performance => concurrency::probe(
            &transport,
            deadline,
            None,
            ProbeOptions {
                max_requests: if matches!(case.variant.as_str(), "budget" | "false_pass_label") {
                    3
                } else {
                    15
                },
                ..Default::default()
            },
        )
        .await
        .unwrap(),
        _ => unreachable!(),
    };
    server.abort();
    if case.variant == "false_failure_label" {
        result.status = Status::Failed;
        result.title = "观察层声称失败".into();
        result.findings = vec![forced_finding()];
    }
    if case.variant == "false_pass_label" {
        result.status = Status::Passed;
        result.title = "声称最大并发 999".into();
        result
            .facts
            .insert("recommended_concurrency".into(), json!(999));
        result.facts.insert("maximum_confirmed".into(), json!(true));
        result
            .facts
            .insert("maximum_concurrency".into(), json!(999));
    }
    result
}
fn fixture_snapshot() -> SourceSnapshot {
    let files: BTreeMap<_, _> = (0..3)
        .map(|file| {
            let records = (0..4)
                .map(|row| {
                    let i = file * 4 + row + 1;
                    Record {
                        id: format!("R-{i:02}"),
                        owner: format!("owner-{i}"),
                        version: 1,
                    }
                })
                .collect();
            (format!("source-{}.json", file + 1), RecordFile { records })
        })
        .collect();
    let sha256 = format!("{:x}", Sha256::digest(serde_json::to_vec(&files).unwrap()));
    SourceSnapshot {
        seed: "independent-fixture".into(),
        files,
        sha256,
    }
}
fn event(id: &str, name: &str, path: &str, error: bool) -> ToolEvent {
    ToolEvent {
        call_id: id.into(),
        name: name.into(),
        arguments: json!({"path":path}),
        result: if error {
            json!({"content":[{"type":"text","text":"AGENTCHECK_RECOVERABLE_READ_fixture: temporary failure"}]})
        } else {
            json!({"content":[{"type":"text","text":"fixture operation completed"}]})
        },
        is_error: error,
    }
}
fn push_event(trace: &mut Vec<Value>, event: &ToolEvent, conflicting: bool) {
    trace.push(json!({"type":"tool_execution_start","toolCallId":event.call_id,"toolName":event.name,"args":if conflicting{json!({"path":"unrelated.txt"})}else{event.arguments.clone()}}));
    trace.push(json!({"type":"tool_execution_end","toolCallId":event.call_id,"toolName":event.name,"result":event.result,"isError":event.is_error}));
}
fn finish_turn(trace: &mut Vec<Value>, normal: bool) {
    trace.push(json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"done"}],"stopReason":if normal{"stop"}else{"error"}}}));
    trace.push(json!({"type":"agent_end","isTerminal":true}));
}
fn execute_agent(case: &Case, protocol: Protocol, dir: &Path) -> CheckResult {
    let snapshot = fixture_snapshot();
    let first_records: Vec<_> = snapshot
        .files
        .values()
        .flat_map(|f| f.records.clone())
        .collect();
    let first = json!({"records":first_records});
    let mut check = CheckResult::pending(case.check);
    check.status = Status::Passed;
    check.finished = true;
    check.title = "观察层声称通过".into();
    for repetition in 1..=2 {
        let task_id = format!("{}-{repetition}", case.check.key());
        let folder = dir.join("agent").join(&task_id);
        let mut output = first.clone();
        let mut trace = vec![json!({"type":"ready"}), json!({"type":"agent_start"})];
        let mut events = vec![];
        if case.variant == "reused_pending_id" {
            let old = event("read-2", "read", "inputs/source-2.json", false);
            push_event(&mut trace, &old, false);
            events.push(old);
        }
        if case.check == CheckId::Recovery {
            let failed = event("failed-read", "read", "inputs/source-1.json", true);
            push_event(&mut trace, &failed, false);
            events.push(failed);
            trace.push(json!({"type":"message_end","message":{"role":"assistant","stopReason":"toolUse","content":[{"type":"toolCall","id":if case.variant=="reused_pending_id"{"read-2"}else{"read-1"},"name":"read","arguments":{"path":if case.variant=="reused_pending_id"{"inputs/source-2.json"}else{"inputs/source-1.json"}}}]}}));
        }
        for i in 1..=3 {
            if (case.variant == "missing_source" && i == 3)
                || (case.variant == "reused_pending_id" && i == 2)
            {
                continue;
            }
            let e = event(
                &format!("read-{i}"),
                "read",
                &format!("inputs/source-{i}.json"),
                false,
            );
            push_event(
                &mut trace,
                &e,
                case.variant == "conflicting_arguments" && i == 1,
            );
            events.push(e);
        }
        let write = event("write-1", "write", "result.json", false);
        push_event(&mut trace, &write, false);
        events.push(write);
        finish_turn(&mut trace, case.variant != "abnormal_end");
        if case.check == CheckId::Multiturn {
            atomic_json(&folder.join("stage1-result.json"), &first).unwrap();
            if case.variant != "first_only" {
                output["records"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"id":"R-13","owner":"陈晨","version":1}));
                if case.variant != "missing_update" {
                    output["records"][6]["version"] = json!(2);
                }
                trace.push(json!({"type":"agent_start"}));
                if case.variant != "no_second_write" {
                    let write = event("write-2", "write", "result.json", false);
                    push_event(&mut trace, &write, false);
                    events.push(write);
                }
                finish_turn(&mut trace, true);
            }
        }
        if case.variant == "wrong_value" {
            output["records"][0]["owner"] = json!("incorrect");
        }
        if case.variant == "reordered" {
            output["records"].as_array_mut().unwrap().reverse();
        }
        let http_id = format!("http-fixture-{task_id}");
        let marker = if case.variant == "not_delivered" {
            "marker not in a tool result"
        } else {
            "AGENTCHECK_RECOVERABLE_READ_fixture: temporary failure"
        };
        let tool_id = if case.variant == "reused_pending_id" {
            "read-2"
        } else {
            "read-1"
        };
        let returned = if case.check == CheckId::Recovery && case.variant != "no_new_action" {
            vec![
                json!({"id":tool_id,"type":"function","function":{"name":"read","arguments":json!({"path":if tool_id=="read-2"{"inputs/source-2.json"}else{"inputs/source-1.json"}}).to_string()}}),
            ]
        } else {
            vec![]
        };
        let request = match protocol {
            Protocol::ChatCompletions => {
                json!({"model":"fixture","messages":[{"role":"tool","tool_call_id":"failed-read","content":marker}],"stream":false})
            }
            Protocol::Responses => {
                json!({"model":"fixture","input":[{"type":"function_call_output","call_id":"failed-read","output":marker}],"stream":false})
            }
        };
        let evidence = HttpEvidence {
            id: http_id.clone(),
            endpoint: format!("http://fixture/v1{}", protocol.suffix()),
            protocol,
            request,
            stream: false,
            elapsed_ms: 1,
            attempts: vec![HttpAttempt {
                number: 1,
                elapsed_ms: 1,
                http_status: Some(200),
                headers: BTreeMap::new(),
                response_body: wire(
                    protocol,
                    "",
                    &returned,
                    if returned.is_empty() {
                        "stop"
                    } else {
                        "tool_calls"
                    },
                    true,
                    1,
                )
                .to_string(),
                error: None,
            }],
            parse_error: None,
            failure: None,
        };
        atomic_json(
            &folder.join("http").join(format!("{http_id}.json")),
            &evidence,
        )
        .unwrap();
        atomic_json(&folder.join("source-snapshot.json"), &snapshot).unwrap();
        atomic_json(&folder.join("workspace/result.json"), &output).unwrap();
        atomic_json(&folder.join("rpc.json"), &trace).unwrap();
        let failed = case.variant == "false_failure_label";
        check.tasks.push(TaskResult {
            id: task_id,
            finished: true,
            kind: case.check,
            repetition,
            status: if failed {
                Status::Failed
            } else {
                Status::Passed
            },
            title: "观察层标签".into(),
            elapsed_ms: 1,
            normal_end: true,
            fault_delivered: true,
            recovery_observed: true,
            tool_events: events,
            evidence_ids: vec![http_id.clone()],
            findings: if failed {
                vec![forced_finding()]
            } else {
                vec![]
            },
        });
        check.evidence_ids.push(http_id);
    }
    check
}
#[tokio::test]
async fn all_registered_requirements_match_independent_acceptance_labels_on_both_protocols() {
    let data: Value =
        serde_json::from_str(include_str!("fixtures/acceptance-matrix.json")).unwrap();
    let cases: Vec<Case> = serde_json::from_value(data["cases"].clone()).unwrap();
    let mut failures = vec![];
    let mut results = vec![];
    for protocol in [Protocol::ChatCompletions, Protocol::Responses] {
        for case in &cases {
            let dir = tempfile::tempdir().unwrap();
            let mut check = if case.check.is_agent() {
                execute_agent(case, protocol, dir.path())
            } else {
                execute_http(case, protocol, dir.path()).await
            };
            atomic_json(
                &dir.path()
                    .join("observations")
                    .join(format!("{}.json", case.check.key())),
                &check,
            )
            .unwrap();
            let index = judgment::index_evidence(dir.path()).unwrap();
            let spec = builtin_specs()
                .into_iter()
                .find(|s| s.id == case.check)
                .unwrap();
            let claims = assessment::builtin_claims(&spec, &check, dir.path(), &index);
            let mut error = None;
            match claims {
                Ok(claims) => {
                    judgment::seal(&spec, &mut check, claims, dir.path(), &index).unwrap()
                }
                Err(e) => {
                    error = Some(e.to_string());
                    check.status = Status::Incomplete;
                }
            }
            let matched = check.status == case.expected;
            if !matched {
                failures.push(format!(
                    "{} / {:?}: expected {:?}, got {:?} ({:?})",
                    case.id, protocol, case.expected, check.status, error
                ));
            }
            if let Some(a) = &check.assessment {
                for r in &a.requirements {
                    if r.outcome != Outcome::Unknown {
                        assert!(!r.evidence_ids.is_empty());
                        assert!(r.evidence_ids.iter().all(|id| index.contains_key(id)));
                    }
                }
            }
            results.push(json!({"id":case.id,"protocol":protocol,"expected":case.expected,"actual":check.status,"matched":matched,"error":error}));
        }
    }
    if let Some(path) = std::env::var_os("AGENTCHECK_ACCEPTANCE_OUTPUT") {
        atomic_json(Path::new(&path),&json!({"matrix_version":data["version"],"scope":data["scope"],"cases":results,"mismatches":failures})).unwrap();
    }
    assert!(
        failures.is_empty(),
        "Acceptance mismatches:\n{}",
        failures.join("\n")
    );
}
