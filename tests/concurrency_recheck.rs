use agent_check::{
    assessment,
    concurrency::{self, ProbeOptions},
    config::{ModelConfig, Protocol},
    engine::atomic_json,
    judgment,
    model::{CheckId, CheckResult, Status},
    registry::builtin_specs,
    transport::Transport,
};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

async fn run_case(
    protocol: Protocol,
    behavior: &'static str,
    cap: usize,
) -> (tempfile::TempDir, CheckResult, usize) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let count = Arc::new(AtomicUsize::new(0));
    let cancel = CancellationToken::new();
    let server_count = count.clone();
    let server_cancel = cancel.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let count = server_count.clone();
            let cancel = server_cancel.clone();
            tokio::spawn(async move {
                let mut bytes = Vec::new();
                loop {
                    let mut chunk = [0; 2048];
                    let n = socket.read(&mut chunk).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&chunk[..n]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let number = count.fetch_add(1, Ordering::SeqCst) + 1;
                tokio::time::sleep(Duration::from_millis(20)).await;
                let drop_connection = number == 5
                    && matches!(behavior, "recover" | "repeat" | "cancel" | "mixed")
                    || number == 9 && behavior == "repeat";
                if drop_connection {
                    if behavior == "cancel" {
                        cancel.cancel();
                    }
                    socket.shutdown().await.unwrap();
                    return;
                }
                let limited = number >= 12;
                let blank =
                    behavior == "invalid" && number == 5 || behavior == "mixed" && number == 6;
                let text = if blank { "" } else { "你好" };
                let body = if limited {
                    json!({"error":{"message":"limited"}})
                } else if protocol == Protocol::Responses {
                    json!({"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}]})
                } else {
                    json!({"choices":[{"message":{"role":"assistant","content":text},"finish_reason":"stop"}]})
                };
                let body = body.to_string();
                let status = if limited {
                    "429 Too Many Requests"
                } else {
                    "200 OK"
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let config = ModelConfig::new(&endpoint, "test", "placeholder".into(), Some(protocol)).unwrap();
    let transport = Transport::new(config, &dir.path().join("http"), cancel).unwrap();
    let check = concurrency::probe(
        &transport,
        Instant::now() + Duration::from_secs(600),
        None,
        ProbeOptions {
            max_requests: cap,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    server.abort();
    (dir, check, count.load(Ordering::SeqCst))
}

fn seal(dir: &std::path::Path, check: &mut CheckResult) {
    atomic_json(&dir.join("observations/performance.json"), check).unwrap();
    let evidence = judgment::index_evidence(dir).unwrap();
    let spec = builtin_specs()
        .into_iter()
        .find(|s| s.id == CheckId::Performance)
        .unwrap();
    let evaluation = assessment::builtin_evaluation(&spec, check, dir, &evidence).unwrap();
    check.facts = evaluation.facts;
    judgment::seal(&spec, check, evaluation.claims, dir, &evidence).unwrap();
}

#[tokio::test]
async fn whole_wave_recheck_keeps_first_failure_and_continues_expansion() {
    for protocol in [Protocol::ChatCompletions, Protocol::Responses] {
        let (dir, mut check, sent) = run_case(protocol, "recover", 100).await;
        assert_eq!(sent, 19);
        assert_eq!(
            check.facts["rounds"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["concurrency"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![1, 2, 4, 4, 8]
        );
        assert_eq!(check.facts["rounds"][2]["failed"], 1);
        assert_eq!(check.facts["rounds"][3]["phase"], "recheck");
        assert_eq!(check.facts["rounds"][3]["successful"], 4);
        let first = &check.facts["rounds"][2];
        let repeat = &check.facts["rounds"][3];
        assert!(
            repeat["started_offset_ms"].as_u64().unwrap()
                >= first["started_offset_ms"].as_u64().unwrap()
                    + first["elapsed_ms"].as_u64().unwrap()
                    + 1000
        );
        seal(dir.path(), &mut check);
        assert_eq!(check.status, Status::Passed);
        assert_eq!(check.facts["recommended_concurrency"], 4);
        assert_eq!(check.facts["network_rechecks"], 1);
        assert_eq!(check.facts["connection_errors"], 1);
        assert_eq!(check.facts["recovered_levels"], json!([4]));
        assert_eq!(check.facts["request_count"], 19);
        assert!(!check.assessment.as_ref().unwrap().conditions.is_empty());
    }
}

#[tokio::test]
async fn persistent_connections_stop_after_one_recheck_and_keep_trial_lower_bound() {
    for protocol in [Protocol::ChatCompletions, Protocol::Responses] {
        let (dir, mut check, sent) = run_case(protocol, "repeat", 100).await;
        assert_eq!(sent, 11);
        seal(dir.path(), &mut check);
        assert_eq!(check.status, Status::Passed);
        assert_eq!(check.facts["recommended_concurrency"], 2);
        assert_eq!(check.facts["recommendation_basis"], "verified_trial");
        assert_eq!(check.facts["estimate_complete"], false);
        assert_eq!(check.facts["connection_stability"], "unresolved");
        assert_eq!(check.facts["connection_errors"], 2);
        assert_eq!(check.facts["network_rechecks"], 1);
        assert_eq!(check.facts["maximum_confirmed"], false);
    }
}

#[tokio::test]
async fn content_errors_and_mixed_failures_are_not_retried_or_promoted() {
    for behavior in ["invalid", "mixed"] {
        let (dir, mut check, sent) = run_case(Protocol::ChatCompletions, behavior, 100).await;
        assert_eq!(sent, 7);
        assert_eq!(check.facts["network_rechecks"], 0);
        seal(dir.path(), &mut check);
        assert_eq!(check.status, Status::Incomplete);
        assert_eq!(check.facts["recommended_concurrency"], Value::Null);
    }
}

#[tokio::test]
async fn budget_limits_preserve_first_connection_error_without_partial_recheck() {
    let (dir, mut check, sent) = run_case(Protocol::ChatCompletions, "recover", 7).await;
    assert_eq!(sent, 7);
    assert_eq!(check.facts["stop_reason"], "request_budget");
    seal(dir.path(), &mut check);
    assert_eq!(check.facts["network_rechecks"], 0);
    assert_eq!(check.facts["recommendation_basis"], "verified_trial");
    assert_eq!(check.facts["recommended_concurrency"], 2);
    assert_eq!(check.facts["estimate_complete"], false);
}

#[tokio::test]
async fn cancellation_does_not_launch_a_recheck_or_publish_a_recommendation() {
    let (_, check, sent) = run_case(Protocol::ChatCompletions, "cancel", 100).await;
    assert_eq!(sent, 7);
    assert_eq!(check.facts["network_rechecks"], 0);
    assert_eq!(check.facts["recommended_concurrency"], Value::Null);
    assert_eq!(check.status, Status::Incomplete);
}
