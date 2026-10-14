use agent_check::{
    assessment,
    checks::evaluate_schema,
    concurrency::{self, ProbeOptions},
    config::{ModelConfig, Protocol},
    dataset::SourceSnapshot,
    engine::atomic_json,
    judgment, measure,
    model::{CHECK_VERSION, Status},
    registry::builtin_specs,
    transport::{RequestFailure, Transport},
};
use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

#[test]
fn schema_compatibility_preserves_content_validation_and_native_format_fact() {
    let expected = SourceSnapshot::generate(CHECK_VERSION)
        .expected(None)
        .unwrap();
    let text = serde_json::to_string(&expected).unwrap();
    let strict = evaluate_schema(&text, &expected, Instant::now(), vec![]);
    assert_eq!(strict.status, Status::Passed);
    assert_eq!(strict.facts["native_format_satisfied"], true);
    let compatible = evaluate_schema(
        &format!("```json\n{text}\n```"),
        &expected,
        Instant::now(),
        vec![],
    );
    assert_eq!(compatible.status, Status::Passed);
    assert_eq!(compatible.facts["native_format_satisfied"], false);
    assert_eq!(compatible.findings[0].code, "schema_compatibility");
    let mut wrong = expected.clone();
    wrong.records[0].version += 1;
    let wrong = evaluate_schema(
        &format!("```json\n{}\n```", serde_json::to_string(&wrong).unwrap()),
        &expected,
        Instant::now(),
        vec![],
    );
    assert_eq!(wrong.status, Status::Failed);
    for bad in [
        "```json\n{broken\n```".to_owned(),
        format!("```json\n{text}\n```\n```json\n{text}\n```"),
    ] {
        assert_eq!(
            evaluate_schema(&bad, &expected, Instant::now(), vec![]).status,
            Status::Failed
        );
    }
}

#[derive(Default)]
struct Load {
    active: AtomicUsize,
    total: AtomicUsize,
    peak: AtomicUsize,
    limit: usize,
    invalid: bool,
    temporary_failure_at: usize,
    slow_after_requests: usize,
}
async fn respond(
    State(state): State<Arc<Load>>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let request_number = state.total.fetch_add(1, Ordering::SeqCst) + 1;
    let active = state.active.fetch_add(1, Ordering::SeqCst) + 1;
    state.peak.fetch_max(active, Ordering::SeqCst);
    let delay = if state.slow_after_requests > 0 && request_number > state.slow_after_requests {
        1_000
    } else {
        75
    };
    tokio::time::sleep(Duration::from_millis(delay)).await;
    state.active.fetch_sub(1, Ordering::SeqCst);
    if active > state.limit || request_number == state.temporary_failure_at {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error":{"message":"concurrency limited"}})),
        );
    }
    let responses = body.get("input").is_some();
    let text = if state.invalid {
        " \n\t"
    } else {
        "你好，祝你今天愉快。"
    };
    // Deliberately omit usage: lack of token accounting must never prevent probing.
    let reply = if responses {
        json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":text}]}]})
    } else {
        json!({"choices":[{"message":{"role":"assistant","content":text},"finish_reason":"stop"}]})
    };
    (StatusCode::OK, Json(reply))
}
async fn probe(
    limit: usize,
    invalid: bool,
    protocol: Protocol,
) -> (agent_check::model::CheckResult, Arc<Load>) {
    probe_with_options(limit, invalid, protocol, 0, ProbeOptions::default()).await
}
async fn probe_with_options(
    limit: usize,
    invalid: bool,
    protocol: Protocol,
    temporary_failure_at: usize,
    options: ProbeOptions,
) -> (agent_check::model::CheckResult, Arc<Load>) {
    probe_with_slow_wave(limit, invalid, protocol, temporary_failure_at, options, 0).await
}
async fn probe_with_slow_wave(
    limit: usize,
    invalid: bool,
    protocol: Protocol,
    temporary_failure_at: usize,
    options: ProbeOptions,
    slow_after_requests: usize,
) -> (agent_check::model::CheckResult, Arc<Load>) {
    let state = Arc::new(Load {
        limit,
        invalid,
        temporary_failure_at,
        slow_after_requests,
        ..Default::default()
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = ModelConfig::new(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "test",
        "placeholder".into(),
        Some(protocol),
    )
    .unwrap();
    let router = Router::new()
        .route("/v1/chat/completions", post(respond))
        .route("/v1/responses", post(respond))
        .with_state(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    let transport = Transport::new(config, dir.path(), CancellationToken::new()).unwrap();
    let result = concurrency::probe(
        &transport,
        Instant::now() + Duration::from_secs(3600),
        None,
        options,
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        result.evidence_ids.len()
    );
    server.abort();
    (result, state)
}

#[tokio::test]
async fn first_unsuccessful_wave_stops_without_refinement_or_retries() {
    let (result, _) = probe_with_options(
        9,
        false,
        Protocol::ChatCompletions,
        5,
        ProbeOptions::default(),
    )
    .await;
    assert_eq!(result.facts["first_unsuccessful_concurrency"], 4);
    assert_eq!(result.facts["recommended_concurrency"], 2);
    assert_eq!(result.facts["request_count"], 7);
    assert_eq!(result.facts["maximum_confirmed"], false);
}

#[tokio::test]
async fn request_guard_never_masquerades_as_service_maximum() {
    let (result, _) = probe_with_options(
        100,
        false,
        Protocol::ChatCompletions,
        0,
        ProbeOptions {
            max_requests: 70,
            ..Default::default()
        },
    )
    .await;
    assert_eq!(result.facts["verified_concurrency_lower_bound"], 32);
    assert_eq!(result.facts["maximum_concurrency"], Value::Null);
    assert_eq!(result.facts["maximum_confirmed"], false);
    assert_eq!(result.facts["stop_reason"], "request_budget");
    assert_eq!(result.status, Status::Incomplete);
}

#[tokio::test]
async fn both_protocols_recommend_the_previous_doubled_level_and_continue_above_32() {
    for protocol in [Protocol::ChatCompletions, Protocol::Responses] {
        for (limit, recommended, rejected) in [(37, 32, 64), (65, 64, 128)] {
            let (result, state) = probe(limit, false, protocol).await;
            assert_eq!(
                result.status,
                Status::Passed,
                "limit={limit}, stop_reason={}",
                result.facts["stop_reason"]
            );
            assert_eq!(result.facts["recommended_concurrency"], recommended);
            assert_eq!(
                result.facts["verified_concurrency_lower_bound"],
                recommended
            );
            assert_eq!(result.facts["maximum_confirmed"], false);
            assert_eq!(result.facts["maximum_concurrency"], Value::Null);
            assert_eq!(result.facts["estimate_complete"], true);
            assert_eq!(result.facts["first_unsuccessful_concurrency"], rejected);
            assert_eq!(state.peak.load(Ordering::SeqCst), rejected);
            assert_eq!(state.total.load(Ordering::SeqCst), rejected * 2 - 1);
            assert_eq!(
                state.total.load(Ordering::SeqCst),
                result.evidence_ids.len()
            );
            let levels: Vec<_> = result.facts["rounds"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["concurrency"].as_u64().unwrap())
                .collect();
            assert!(levels.windows(2).all(|pair| pair[1] == pair[0] * 2));
            assert!(result.title.starts_with(&format!("建议并发 {recommended}")));
        }
    }
}

#[tokio::test]
async fn correct_but_slow_responses_recommend_the_previous_level() {
    for protocol in [Protocol::ChatCompletions, Protocol::Responses] {
        for (max_mean_ms, max_tail_ms, reason, recommended, rejected) in [
            (500, 20_000, "mean_latency", 2, 4),
            (10_000, 500, "max_latency", 2, 4),
            (10_000, 500, "p95_latency", 16, 32),
        ] {
            let (result, state) = probe_with_slow_wave(
                100,
                false,
                protocol,
                0,
                ProbeOptions {
                    max_mean_ms,
                    max_tail_ms,
                    ..Default::default()
                },
                rejected - 1,
            )
            .await;
            assert_eq!(result.status, Status::Passed);
            assert_eq!(result.facts["recommended_concurrency"], recommended);
            assert_eq!(result.facts["first_unsuccessful_concurrency"], rejected);
            assert_eq!(result.facts["first_unsuccessful_reason"], reason);
            assert_eq!(state.total.load(Ordering::SeqCst), rejected * 2 - 1);
            let round = result.facts["rounds"].as_array().unwrap().last().unwrap();
            assert_eq!(round["successful"], rejected);
            assert_eq!(round["failed"], 0);
            assert_eq!(round["performance_passed"], false);
            assert_eq!(round["failure_reason"], reason);
            assert!(result.title.contains("响应过慢"));
            assert!(result.findings[0].fact.contains("平均"));
        }
    }
}

#[tokio::test]
async fn slow_single_request_does_not_produce_a_recommendation() {
    let (result, state) = probe_with_options(
        100,
        false,
        Protocol::ChatCompletions,
        0,
        ProbeOptions {
            max_mean_ms: 1,
            ..Default::default()
        },
    )
    .await;
    assert_eq!(result.status, Status::Incomplete);
    assert_eq!(result.facts["recommended_concurrency"], Value::Null);
    assert_eq!(result.facts["verified_concurrency_lower_bound"], 0);
    assert_eq!(result.facts["rounds"][0]["successful"], 1);
    assert_eq!(result.facts["first_unsuccessful_reason"], "mean_latency");
    assert_eq!(state.total.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn rate_limit_stops_on_the_first_failed_level_and_is_not_hidden_by_retries() {
    let (result, state) = probe(5, false, Protocol::ChatCompletions).await;
    assert_eq!(result.facts["verified_concurrency_lower_bound"], 4);
    assert_eq!(result.facts["recommended_concurrency"], 4);
    assert_eq!(result.facts["first_unsuccessful_concurrency"], 8);
    assert_eq!(result.facts["rounds"][3]["rate_limited"], 3);
    assert_eq!(state.total.load(Ordering::SeqCst), 15);
    assert!(
        result.facts["rounds"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|r| r["requests"].as_array().unwrap())
            .all(|r| r["attempts"] == 1)
    );
    assert_eq!(result.facts["maximum_confirmed"], false);
}

#[tokio::test]
async fn http_success_with_empty_text_does_not_count_as_capacity() {
    let (result, state) = probe(100, true, Protocol::ChatCompletions).await;
    assert_eq!(result.status, Status::Incomplete);
    assert_eq!(result.facts["verified_concurrency_lower_bound"], 0);
    assert_eq!(
        result.facts["rounds"][0]["requests"][0]["reason"],
        "invalid_response"
    );
    assert_eq!(state.total.load(Ordering::SeqCst), 1);
    assert_eq!(result.facts["maximum_confirmed"], false);
}

#[tokio::test]
async fn request_waits_beyond_sixty_seconds_and_times_out_at_180() {
    let arrived = Arc::new(tokio::sync::Notify::new());
    async fn hang(State(arrived): State<Arc<tokio::sync::Notify>>) -> &'static str {
        arrived.notify_one();
        std::future::pending::<()>().await;
        ""
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = ModelConfig::new(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "test",
        "placeholder".into(),
        None,
    )
    .unwrap();
    let router = Router::new()
        .route("/v1/chat/completions", post(hang))
        .with_state(arrived.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    let transport = Transport::new(config, dir.path(), CancellationToken::new()).unwrap();
    let request = tokio::spawn(async move {
        transport
            .send(json!({}), false, Instant::now() + Duration::from_secs(300))
            .await
            .unwrap()
    });
    arrived.notified().await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    assert!(!request.is_finished());
    tokio::time::advance(Duration::from_secs(120)).await;
    let outcome = request.await.unwrap();
    assert!(matches!(outcome.failure, Some(RequestFailure::Timeout)));
    assert_eq!(outcome.evidence.attempts.len(), 1);
    assert_eq!(outcome.evidence.request_timeout_seconds, Some(180));
    server.abort();
    tokio::time::resume();
}

#[tokio::test]
async fn tool_request_waits_past_180_and_360_seconds_but_stops_at_600() {
    use agent_check::model::CheckId;
    let plan = builtin_specs();
    for (id, seconds) in [
        (CheckId::Response, 360),
        (CheckId::Tools, 1200),
        (CheckId::Schema, 180),
        (CheckId::Context, 900),
        (CheckId::Performance, 3600),
    ] {
        assert_eq!(
            plan.iter().find(|s| s.id == id).unwrap().budget_seconds,
            seconds
        );
    }
    assert_eq!(agent_check::transport::REQUEST_BUDGET.as_secs(), 180);
    assert_eq!(agent_check::engine::DIRECT_BUDGET.as_secs(), 6240);
    for protocol in [Protocol::ChatCompletions, Protocol::Responses] {
        let arrived = Arc::new(tokio::sync::Notify::new());
        async fn hang(State(arrived): State<Arc<tokio::sync::Notify>>) -> &'static str {
            arrived.notify_one();
            std::future::pending::<()>().await;
            ""
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = ModelConfig::new(
            &format!("http://{}/v1", listener.local_addr().unwrap()),
            "test",
            "placeholder".into(),
            Some(protocol),
        )
        .unwrap();
        let app = Router::new()
            .route("/v1/chat/completions", post(hang))
            .route("/v1/responses", post(hang))
            .with_state(arrived.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let directory = tempfile::tempdir().unwrap();
        let transport = Transport::new(config, directory.path(), CancellationToken::new()).unwrap();
        let deadline = agent_check::engine::interface_deadline(
            agent_check::model::CheckId::Tools,
            Instant::now() + Duration::from_secs(3600),
        );
        let request = tokio::spawn(async move {
            agent_check::checks::tools(&transport, deadline)
                .await
                .unwrap()
        });
        arrived.notified().await;
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(181)).await;
        tokio::task::yield_now().await;
        assert!(
            !request.is_finished(),
            "tool request stopped at the old 180-second limit"
        );
        tokio::time::advance(Duration::from_secs(181)).await;
        tokio::task::yield_now().await;
        assert!(
            !request.is_finished(),
            "tool request stopped at the old 360-second stage budget"
        );
        tokio::time::advance(Duration::from_secs(239)).await;
        let check = request.await.unwrap();
        assert_eq!(check.evidence_ids.len(), 1);
        let row: Value = serde_json::from_slice(
            &std::fs::read(
                directory
                    .path()
                    .join(format!("{}.json", check.evidence_ids[0])),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(row["failure"], "timeout");
        assert_eq!(row["request_timeout_seconds"], 600);
        server.abort();
        tokio::time::resume();
    }
}

#[tokio::test]
async fn cancelled_probe_does_not_launch_requests_or_report_a_limit() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let config =
        ModelConfig::new("http://127.0.0.1:9/v1", "test", "placeholder".into(), None).unwrap();
    let transport = Transport::new(config, dir.path(), cancel).unwrap();
    let result = measure::performance(&transport, Instant::now() + Duration::from_secs(1080))
        .await
        .unwrap();
    assert_eq!(result.status, Status::Incomplete);
    assert!(result.evidence_ids.is_empty());
    assert_eq!(result.facts["stop_reason"], "cancelled");
}

struct TextReply {
    reply: Value,
    active: AtomicUsize,
    requests: std::sync::Mutex<Vec<Value>>,
}
async fn text_reply(
    State(state): State<Arc<TextReply>>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    state.requests.lock().unwrap().push(body);
    let active = state.active.fetch_add(1, Ordering::SeqCst) + 1;
    tokio::time::sleep(Duration::from_millis(30)).await;
    state.active.fetch_sub(1, Ordering::SeqCst);
    if active > 2 {
        (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error":{"message":"limited"}})),
        )
    } else {
        (StatusCode::OK, Json(state.reply.clone()))
    }
}
fn text_envelope(protocol: Protocol, text: &str) -> Value {
    match protocol {
        Protocol::ChatCompletions => {
            json!({"choices":[{"message":{"role":"assistant","content":text},"finish_reason":"stop"}]})
        }
        Protocol::Responses => {
            json!({"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}]})
        }
    }
}
async fn probe_text_reply(
    protocol: Protocol,
    reply: Value,
) -> (
    tempfile::TempDir,
    agent_check::model::CheckResult,
    Arc<TextReply>,
) {
    let state = Arc::new(TextReply {
        reply,
        active: AtomicUsize::new(0),
        requests: Default::default(),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = ModelConfig::new(
        &format!("http://{}/v1", listener.local_addr().unwrap()),
        "test",
        "placeholder".into(),
        Some(protocol),
    )
    .unwrap();
    let app = Router::new()
        .route("/v1/chat/completions", post(text_reply))
        .route("/v1/responses", post(text_reply))
        .with_state(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let directory = tempfile::tempdir().unwrap();
    let transport = Transport::new(
        config,
        &directory.path().join("http"),
        CancellationToken::new(),
    )
    .unwrap();
    let check = concurrency::probe(
        &transport,
        Instant::now() + Duration::from_secs(3600),
        None,
        ProbeOptions {
            max_requests: 7,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    server.abort();
    (directory, check, state)
}
fn adjudicate_probe(directory: &std::path::Path, check: &mut agent_check::model::CheckResult) {
    atomic_json(&directory.join("observations/performance.json"), check).unwrap();
    let index = judgment::index_evidence(directory).unwrap();
    let spec = builtin_specs()
        .into_iter()
        .find(|s| s.id == check.id)
        .unwrap();
    let evaluation = assessment::builtin_evaluation(&spec, check, directory, &index).unwrap();
    check.facts = evaluation.facts;
    judgment::seal(&spec, check, evaluation.claims, directory, &index).unwrap();
}
#[tokio::test]
async fn plain_text_and_explanations_pass_both_probe_and_evidence_judgment() {
    for protocol in [Protocol::ChatCompletions, Protocol::Responses] {
        for text in [
            "你好！",
            "**连接正常。**",
            "We need answer exactly JSON. Need obey no extra.{\"ok\":true,\"probe_id\":\"2e3a49151a54465298481c9d472ffae1\"}",
        ] {
            let (directory, mut check, state) =
                probe_text_reply(protocol, text_envelope(protocol, text)).await;
            assert_eq!(check.status, Status::Passed, "{protocol:?}: {text}");
            assert_eq!(check.facts["recommended_concurrency"], 2);
            assert_eq!(check.facts["request_count"], 7);
            let requests = state.requests.lock().unwrap();
            assert_eq!(requests.len(), 7);
            for request in requests.iter() {
                let content = request
                    .get("messages")
                    .or_else(|| request.get("input"))
                    .unwrap()[0]["content"]
                    .as_str()
                    .unwrap();
                assert_eq!(content, "请用一句简短的中文打个招呼。");
                assert!(request.get("tools").is_none());
                assert!(request.get("response_format").is_none());
                assert!(request.pointer("/text/format").is_none());
                assert_eq!(
                    request
                        .get("max_tokens")
                        .or_else(|| request.get("max_output_tokens"))
                        .unwrap(),
                    2048
                );
            }
            drop(requests);
            adjudicate_probe(directory.path(), &mut check);
            assert_eq!(check.status, Status::Passed);
            assert_eq!(check.facts["recommended_concurrency"], 2);
            assert_eq!(check.facts["workload_type"], "short_text");
            assert_eq!(check.facts["maximum_confirmed"], false);
        }
    }
}
#[tokio::test]
async fn legacy_json_workloads_are_not_reinterpreted_as_plain_text_runs() {
    for protocol in [Protocol::ChatCompletions, Protocol::Responses] {
        let (directory, check, _) =
            probe_text_reply(protocol, text_envelope(protocol, "你好")).await;
        for id in &check.evidence_ids {
            let path = directory.path().join("http").join(format!("{id}.json"));
            let mut row: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let field = if protocol == Protocol::ChatCompletions {
                "messages"
            } else {
                "input"
            };
            row["request"][field][0]["content"] = json!(
                "这是并发请求测试。请原样返回以下 JSON，不增加字段或解释：{\"ok\":true,\"probe_id\":\"legacy\"}"
            );
            atomic_json(&path, &row).unwrap();
        }
        let index = judgment::index_evidence(directory.path()).unwrap();
        let spec = builtin_specs()
            .into_iter()
            .find(|s| s.id == check.id)
            .unwrap();
        assert!(assessment::builtin_evaluation(&spec, &check, directory.path(), &index).is_err());
    }
}

#[tokio::test]
async fn http_200_without_a_completed_text_response_is_not_capacity_evidence() {
    for protocol in [Protocol::ChatCompletions, Protocol::Responses] {
        let mut truncated = text_envelope(protocol, "partial");
        let mut refusal = text_envelope(protocol, "");
        let mut reasoning_only = text_envelope(protocol, "");
        let mut tool = text_envelope(protocol, "calling a tool");
        match protocol {
            Protocol::ChatCompletions => {
                truncated["choices"][0]["finish_reason"] = json!("length");
                refusal["choices"][0]["message"]["refusal"] = json!("declined");
                reasoning_only["choices"][0]["message"]["reasoning_content"] =
                    json!("thinking only");
                tool["choices"][0]["finish_reason"] = json!("tool_calls");
                tool["choices"][0]["message"]["tool_calls"] = json!([{"id":"call-extra","type":"function","function":{"name":"unrequested","arguments":"{}"}}]);
            }
            Protocol::Responses => {
                truncated["status"] = json!("incomplete");
                refusal["output"][0]["content"] = json!([{"type":"refusal","refusal":"declined"}]);
                reasoning_only["output"] = json!([{"type":"reasoning","summary":[{"type":"summary_text","text":"thinking only"}]}]);
                tool["output"].as_array_mut().unwrap().push(json!({"type":"function_call","id":"fc-extra","call_id":"call-extra","name":"unrequested","arguments":"{}"}));
            }
        }
        for reply in [
            text_envelope(protocol, ""),
            text_envelope(protocol, " \n\t"),
            truncated,
            refusal,
            reasoning_only,
            tool,
            json!({"error":{"message":"service error"}}),
        ] {
            let (directory, mut check, state) = probe_text_reply(protocol, reply).await;
            assert_eq!(check.status, Status::Incomplete);
            assert_eq!(check.facts["recommended_concurrency"], Value::Null);
            assert_eq!(state.requests.lock().unwrap().len(), 1);
            adjudicate_probe(directory.path(), &mut check);
            assert_eq!(check.status, Status::Incomplete);
            assert_eq!(check.facts["recommended_concurrency"], Value::Null);
            assert_eq!(check.facts["verified_concurrency_lower_bound"], 0);
        }
    }
}
