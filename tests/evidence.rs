use std::fs;
use std::time::Duration;

use chrono::{Local, TimeZone};
use httpmock::Method::POST;
use httpmock::MockServer;
use model_capability_doctor::audit::{
    AuditWriter, RequestEvidence, ResponseMetrics, RunMetadata, TestManifest,
};
use model_capability_doctor::http::{HttpExecutor, RequestInput};
use model_capability_doctor::protocol::{AuthMode, Protocol};
use model_capability_doctor::redaction::{Redactor, mask_api_key};
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;
use url::Url;

mod fixture_server;

use fixture_server::FixtureServer;

fn timestamp(second: u32) -> chrono::DateTime<Local> {
    Local
        .with_ymd_and_hms(2026, 7, 19, 10, 11, second)
        .single()
        .unwrap()
}

#[test]
fn redaction_masks_keys_urls_headers_bodies_and_errors() {
    let url = Url::parse(
        "https://model.example/v1/chat?api_key=query-secret&name=visible&ToKeN=token-secret",
    )
    .unwrap();
    let redactor = Redactor::new("fixture-key", &url);

    assert_eq!(mask_api_key("short"), "[MASKED]");
    assert_eq!(mask_api_key("fixture-key"), "fixt********-key");

    let safe_url = redactor.redact_url(&url);
    assert!(safe_url.contains("api_key=[REDACTED]"));
    assert!(safe_url.contains("ToKeN=[REDACTED]"));
    assert!(safe_url.contains("name=visible"));
    for secret in ["fixture-key", "query-secret", "token-secret"] {
        assert!(!safe_url.contains(secret));
    }

    let headers = redactor.redact_headers(&[
        ("Authorization".into(), "Bearer fixture-key".into()),
        ("proxy-authorization".into(), "proxy-secret".into()),
        ("Set-Cookie".into(), "session=query-secret".into()),
        ("Content-Type".into(), "application/json".into()),
    ]);
    assert_eq!(headers[0].1, "[REDACTED]");
    assert_eq!(headers[1].1, "[REDACTED]");
    assert_eq!(headers[2].1, "[REDACTED]");
    assert_eq!(headers[3].1, "application/json");

    let safe_text = redactor
        .redact_text("body fixture-key query-secret token-secret fixture-key remains useful");
    assert_eq!(safe_text.matches("[REDACTED]").count(), 4);
    assert!(safe_text.contains("remains useful"));
}

#[test]
fn audit_writer_emits_compatible_blocks_without_secrets() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("doctor.log");
    let url = Url::parse("https://model.example/v1/chat?api_key=query-secret").unwrap();
    let redactor = Redactor::new("fixture-key", &url);
    let metadata = RunMetadata {
        run_id: "MD-20260719-101112-7".into(),
        started_at: timestamp(12),
        url: url.clone(),
        model: "fixture-model".into(),
        masked_api_key: mask_api_key("fixture-key"),
        selected_test_count: 1,
        insecure: true,
    };
    let mut writer = AuditWriter::create(&path, metadata, redactor).unwrap();
    let request = RequestEvidence {
        request_id: "test-004".into(),
        started_at: timestamp(13),
        completed_at: timestamp(14),
        protocol: Protocol::OpenAiChat,
        auth_mode: AuthMode::Bearer,
        stream: false,
        url,
        timeout: Duration::from_secs(120),
        insecure: true,
        body: r#"{"model":"fixture-model","secret":"fixture-key"}"#.into(),
        metrics: ResponseMetrics {
            transport_exit_code: 0,
            http_status: Some(200),
            time_total: Duration::from_millis(250),
            time_starttransfer: Duration::from_millis(100),
            size_download: 45,
        },
        headers: vec![
            ("Content-Type".into(), "application/json".into()),
            ("Set-Cookie".into(), "session=query-secret".into()),
        ],
        error: "upstream echoed fixture-key".into(),
        response_body: br#"{"content":"fixture-key query-secret"}"#.to_vec(),
    };

    writer.append_request(&request).unwrap();
    assert!(writer.append_request(&request).is_err());
    writer
        .append_manifest(&TestManifest {
            id: "004".into(),
            name: "同步生成".into(),
            category: "接口与协议".into(),
            completed_at: timestamp(15),
            request_refs: vec!["test-004".into()],
        })
        .unwrap();
    writer
        .finish(Duration::from_secs(3), timestamp(16))
        .unwrap();
    drop(writer);

    let log = fs::read_to_string(&path).unwrap();
    for marker in [
        "========== MODEL DOCTOR RUN ==========",
        "collector_runtime: rust",
        "tls_verification: disabled",
        "========== REQUEST test-004 BEGIN ==========",
        "----- CURL COMMAND BEGIN -----",
        "----- REQUEST BODY BEGIN -----",
        "----- RESPONSE METRICS BEGIN -----",
        "----- RESPONSE HEADERS BEGIN -----",
        "----- CURL STDERR BEGIN -----",
        "----- RESPONSE BODY BEGIN -----",
        "========== REQUEST test-004 END ==========",
        "========== TEST-004 BEGIN ==========",
        "request_refs: test-004",
        "========== RUN SUMMARY ==========",
        "request_count: 1",
        "test_manifest_count: 1",
        "========== END ==========",
    ] {
        assert!(log.contains(marker), "missing {marker:?} in {log}");
    }
    assert!(log.contains("api_key: fixt********-key"));
    assert!(log.contains("--insecure"));
    assert!(log.contains("Set-Cookie: [REDACTED]"));
    assert!(!log.contains("fixture-key"));
    assert!(!log.contains("query-secret"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

fn request_input(server: &MockServer, path: &str, auth_mode: AuthMode) -> RequestInput {
    RequestInput {
        request_id: format!("native-{}", auth_mode),
        url: Url::parse(&server.url(path)).unwrap(),
        protocol: Protocol::OpenAiChat,
        auth_mode,
        body: br#"{"model":"fixture-model","messages":[]}"#.to_vec(),
        stream: false,
        api_key: "fixture-key".into(),
    }
}

#[tokio::test]
async fn native_http_collects_success_status_headers_body_and_metrics() {
    let server = MockServer::start_async().await;
    let endpoint = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/bearer")
                .header("accept", "application/json, text/event-stream")
                .header("content-type", "application/json")
                .header("authorization", "Bearer fixture-key")
                .body(r#"{"model":"fixture-model","messages":[]}"#);
            then.status(200)
                .header("content-type", "application/json")
                .header("x-fixture", "present")
                .body(r#"{"choices":[{"message":{"content":"ok"}}]}"#);
        })
        .await;
    let executor = HttpExecutor::new(Duration::from_secs(2), false).unwrap();

    let evidence = executor
        .execute(
            request_input(&server, "/bearer", AuthMode::Bearer),
            CancellationToken::new(),
        )
        .await;

    endpoint.assert_async().await;
    assert_eq!(evidence.metrics.transport_exit_code, 0);
    assert_eq!(evidence.metrics.http_status, Some(200));
    assert_eq!(evidence.metrics.size_download, evidence.response_body.len());
    assert!(evidence.metrics.time_total >= evidence.metrics.time_starttransfer);
    assert_eq!(
        String::from_utf8(evidence.response_body).unwrap(),
        r#"{"choices":[{"message":{"content":"ok"}}]}"#
    );
    assert!(evidence.error.is_empty());
    assert!(
        evidence
            .headers
            .iter()
            .any(|(name, value)| name == "x-fixture" && value == "present")
    );
}

#[tokio::test]
async fn native_http_sends_each_supported_authentication_header() {
    let server = MockServer::start_async().await;
    let bearer = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/bearer")
                .header("authorization", "Bearer fixture-key");
            then.status(200).body("{}");
        })
        .await;
    let api_key = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/api-key")
                .header("api-key", "fixture-key");
            then.status(200).body("{}");
        })
        .await;
    let anthropic = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/anthropic")
                .header("x-api-key", "fixture-key")
                .header("anthropic-version", "2023-06-01");
            then.status(200).body("{}");
        })
        .await;
    let google = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/google")
                .header("x-goog-api-key", "fixture-key");
            then.status(200).body("{}");
        })
        .await;
    let executor = HttpExecutor::new(Duration::from_secs(2), false).unwrap();

    for (path, mode) in [
        ("/bearer", AuthMode::Bearer),
        ("/api-key", AuthMode::ApiKey),
        ("/anthropic", AuthMode::XApiKey),
        ("/google", AuthMode::XGoogApiKey),
    ] {
        let evidence = executor
            .execute(request_input(&server, path, mode), CancellationToken::new())
            .await;
        assert_eq!(evidence.metrics.http_status, Some(200), "{mode}");
    }

    bearer.assert_async().await;
    api_key.assert_async().await;
    anthropic.assert_async().await;
    google.assert_async().await;
}

#[tokio::test]
async fn native_http_preserves_non_success_responses_as_evidence() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(POST).path("/unauthorized");
            then.status(401)
                .header("www-authenticate", "Bearer")
                .body(r#"{"error":"invalid key"}"#);
        })
        .await;
    let executor = HttpExecutor::new(Duration::from_secs(2), false).unwrap();

    let evidence = executor
        .execute(
            request_input(&server, "/unauthorized", AuthMode::Bearer),
            CancellationToken::new(),
        )
        .await;

    assert_eq!(evidence.metrics.transport_exit_code, 0);
    assert_eq!(evidence.metrics.http_status, Some(401));
    assert_eq!(evidence.response_body, br#"{"error":"invalid key"}"#);
    assert!(evidence.error.is_empty());
}

#[tokio::test]
async fn native_http_records_timeouts_without_panicking() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(POST).path("/slow");
            then.status(200)
                .delay(Duration::from_millis(200))
                .body("late");
        })
        .await;
    let executor = HttpExecutor::new(Duration::from_millis(30), false).unwrap();

    let evidence = executor
        .execute(
            request_input(&server, "/slow", AuthMode::Bearer),
            CancellationToken::new(),
        )
        .await;

    assert_ne!(evidence.metrics.transport_exit_code, 0);
    assert_eq!(evidence.metrics.http_status, None);
    assert!(!evidence.error.is_empty());
    assert!(evidence.metrics.time_total >= Duration::from_millis(20));
}

#[tokio::test]
async fn native_http_cancellation_stops_an_inflight_request() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(POST).path("/cancel");
            then.status(200).delay(Duration::from_secs(2)).body("late");
        })
        .await;
    let executor = HttpExecutor::new(Duration::from_secs(5), false).unwrap();
    let cancellation = CancellationToken::new();
    let started = std::time::Instant::now();
    let task = tokio::spawn({
        let cancellation = cancellation.clone();
        async move {
            executor
                .execute(
                    request_input(&server, "/cancel", AuthMode::Bearer),
                    cancellation,
                )
                .await
        }
    });

    tokio::time::sleep(Duration::from_millis(20)).await;
    cancellation.cancel();
    let evidence = task.await.unwrap();

    assert!(started.elapsed() < Duration::from_millis(500));
    assert_ne!(evidence.metrics.transport_exit_code, 0);
    assert!(evidence.error.to_ascii_lowercase().contains("cancel"));
}

#[test]
fn production_source_never_spawns_curl_or_bash() {
    fn source_files(path: &std::path::Path, output: &mut Vec<std::path::PathBuf>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                source_files(&path, output);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                output.push(path);
            }
        }
    }

    let mut files = Vec::new();
    source_files(std::path::Path::new("src"), &mut files);
    for path in files {
        let source = fs::read_to_string(&path).unwrap();
        assert!(
            !source.contains("Command::new(\"curl\")"),
            "{}",
            path.display()
        );
        assert!(
            !source.contains("Command::new(\"bash\")"),
            "{}",
            path.display()
        );
    }
}

fn manifest_request_refs(log: &str, id: &str) -> Vec<String> {
    let start = format!("========== TEST-{id} BEGIN ==========");
    let end = format!("========== TEST-{id} END ==========");
    let block = log
        .split_once(&start)
        .unwrap_or_else(|| panic!("missing {start}"))
        .1
        .split_once(&end)
        .unwrap_or_else(|| panic!("missing {end}"))
        .0;
    let refs = block
        .lines()
        .find_map(|line| line.strip_prefix("request_refs: "))
        .expect("missing request_refs");
    if refs.is_empty() {
        Vec::new()
    } else {
        refs.split(',').map(str::to_owned).collect()
    }
}

fn run_fixture(protocol: Protocol, only: &str) -> (std::process::Output, String, usize) {
    let server = FixtureServer::start(protocol);
    let directory = tempdir().unwrap();
    let log_path = directory.path().join("doctor.log");
    let output = assert_cmd::cargo::cargo_bin_cmd!("model-capability-doctor")
        .args([
            "--url",
            &server.url(),
            "--model",
            "fixture-model",
            "--api-key",
            "fixture-key",
            "--only",
            only,
            "--log-file",
            log_path.to_str().unwrap(),
            "--timeout",
            "2",
        ])
        .output()
        .unwrap();
    let log = fs::read_to_string(log_path).unwrap_or_default();
    let request_count = server.requests().len();
    (output, log, request_count)
}

#[test]
fn end_to_end_detects_all_five_protocols_and_writes_consistent_counts() {
    for (protocol, probe_count) in [
        (Protocol::OpenAiChat, 1),
        (Protocol::OpenAiResponses, 2),
        (Protocol::AnthropicMessages, 3),
        (Protocol::GeminiGenerateContent, 4),
        (Protocol::OllamaChat, 5),
    ] {
        let (output, log, request_count) = run_fixture(protocol, "004");
        assert!(
            output.status.success(),
            "{protocol}: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(log.contains(&format!("protocol: {protocol}")), "{protocol}");
        assert_eq!(manifest_request_refs(&log, "004"), ["test-004"]);
        let discovered = log
            .lines()
            .filter(|line| {
                line.starts_with("========== REQUEST ") && line.ends_with(" BEGIN ==========")
            })
            .count();
        assert_eq!(discovered, probe_count + 1, "{protocol}: {log}");
        assert_eq!(request_count, discovered, "{protocol}");
        assert!(log.contains(&format!("request_count: {discovered}")));
        assert!(log.contains("test_manifest_count: 1"));
    }
}

#[test]
fn end_to_end_reuses_probe_repeat_and_tool_evidence_references() {
    let only = "002,003,007,033,047,048,049,055,056,061,062";
    let (output, log, _) = run_fixture(Protocol::OpenAiChat, only);
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(manifest_request_refs(&log, "002"), ["protocol-1"]);
    assert_eq!(manifest_request_refs(&log, "003"), ["protocol-1"]);
    assert_eq!(manifest_request_refs(&log, "007"), ["protocol-1"]);
    assert_eq!(manifest_request_refs(&log, "033").len(), 2);
    for id in ["047", "048", "049"] {
        assert_eq!(manifest_request_refs(&log, id).len(), 2, "check {id}");
    }
    assert_eq!(manifest_request_refs(&log, "055").len(), 5);
    assert_eq!(
        manifest_request_refs(&log, "055"),
        manifest_request_refs(&log, "056")
    );
    assert_eq!(manifest_request_refs(&log, "061").len(), 2);
    assert_eq!(manifest_request_refs(&log, "062").len(), 2);
}

#[test]
fn end_to_end_concurrency_manifest_references_sixty_unique_requests() {
    let (output, log, _) = run_fixture(Protocol::OpenAiChat, "057");
    assert!(output.status.success());
    let refs = manifest_request_refs(&log, "057");
    let unique: std::collections::BTreeSet<&str> = refs.iter().map(String::as_str).collect();
    assert_eq!(refs.len(), 60);
    assert_eq!(unique.len(), 60);
    assert!(refs.contains(&"test-057-c4-1".into()));
    assert!(refs.contains(&"test-057-c32-32".into()));
}

#[test]
fn end_to_end_transport_errors_are_evidence_not_run_fatal() {
    let directory = tempdir().unwrap();
    let log_path = directory.path().join("doctor.log");
    let output = assert_cmd::cargo::cargo_bin_cmd!("model-capability-doctor")
        .args([
            "--url",
            "http://127.0.0.1:9/v1/model",
            "--model",
            "fixture-model",
            "--api-key",
            "fixture-key",
            "--only",
            "004",
            "--log-file",
            log_path.to_str().unwrap(),
            "--timeout",
            "1",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let log = fs::read_to_string(log_path).unwrap();
    assert!(log.contains("curl_exit_code: 1"));
    assert!(log.contains("========== TEST-004 BEGIN =========="));
    assert!(log.contains("========== RUN SUMMARY =========="));
}
