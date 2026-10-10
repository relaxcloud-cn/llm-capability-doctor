use agent_check::{
    checks::evaluate_schema,
    config::{ModelConfig, Protocol},
    dataset::SourceSnapshot,
    measure,
    model::{CHECK_VERSION, Status},
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
}
async fn respond(
    State(state): State<Arc<Load>>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    state.total.fetch_add(1, Ordering::SeqCst);
    let active = state.active.fetch_add(1, Ordering::SeqCst) + 1;
    state.peak.fetch_max(active, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(75)).await;
    state.active.fetch_sub(1, Ordering::SeqCst);
    if active > state.limit {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error":{"message":"concurrency limited"}})),
        );
    }
    let responses = body.get("input").is_some();
    let prompt = if responses {
        &body["input"][0]["content"]
    } else {
        &body["messages"][0]["content"]
    };
    let (_, expected) = prompt.as_str().unwrap().split_once('：').unwrap();
    let text = if state.invalid { "wrong" } else { expected };
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
    let state = Arc::new(Load {
        limit,
        invalid,
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
    let result = measure::performance(&transport, Instant::now() + Duration::from_secs(1080))
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
async fn both_protocols_probe_real_overlap_without_token_usage_and_report_lower_bound() {
    for protocol in [Protocol::ChatCompletions, Protocol::Responses] {
        let (result, state) = probe(100, false, protocol).await;
        assert_eq!(result.status, Status::Passed);
        assert_eq!(result.facts["verified_concurrency_lower_bound"], 32);
        assert_eq!(result.facts["maximum_confirmed"], false);
        assert_eq!(result.facts["maximum_concurrency"], Value::Null);
        assert_eq!(result.facts["rounds"].as_array().unwrap().len(), 6);
        assert_eq!(state.total.load(Ordering::SeqCst), 63);
        assert_eq!(state.peak.load(Ordering::SeqCst), 32);
    }
}

#[tokio::test]
async fn rate_limit_stops_escalation_and_is_not_hidden_by_retries() {
    let (result, state) = probe(5, false, Protocol::ChatCompletions).await;
    assert_eq!(result.facts["verified_concurrency_lower_bound"], 4);
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
async fn http_success_with_wrong_content_does_not_count_as_capacity() {
    let (result, state) = probe(100, true, Protocol::ChatCompletions).await;
    assert_eq!(result.status, Status::Incomplete);
    assert_eq!(result.facts["verified_concurrency_lower_bound"], 0);
    assert_eq!(
        result.facts["rounds"][0]["requests"][0]["reason"],
        "invalid_response"
    );
    assert_eq!(state.total.load(Ordering::SeqCst), 1);
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
    server.abort();
    tokio::time::resume();
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
