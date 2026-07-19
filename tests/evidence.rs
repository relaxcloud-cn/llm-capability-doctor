use std::fs;
use std::time::Duration;

use chrono::{Local, TimeZone};
use model_capability_doctor::audit::{
    AuditWriter, RequestEvidence, ResponseMetrics, RunMetadata, TestManifest,
};
use model_capability_doctor::protocol::{AuthMode, Protocol};
use model_capability_doctor::redaction::{Redactor, mask_api_key};
use tempfile::tempdir;
use url::Url;

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
