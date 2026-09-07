use assert_cmd::cargo::cargo_bin_cmd;
use httpmock::Method::POST;
use httpmock::MockServer;
use predicates::prelude::*;
use serde_json::json;

#[test]
fn default_cli_run_creates_self_analysis_artifacts() {
    let server = MockServer::start();
    let endpoint = server.url("/v1/chat/completions");
    let response = server.mock(|when, then| {
        when.method(POST).path("/v1/chat/completions");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "id": "response-1",
                "object": "chat.completion",
                "created": 1,
                "model": "test-model",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "ok"},
                    "finish_reason": "stop"
                }]
            }));
    });
    let directory = tempfile::tempdir().unwrap();
    let log_path = directory.path().join("doctor.log");

    cargo_bin_cmd!("model-capability-doctor")
        .env_remove("MODEL_API_KEY")
        .args([
            "--url",
            &endpoint,
            "--model",
            "test-model",
            "--api-key",
            "secret-key",
            "--log-file",
            log_path.to_str().unwrap(),
            "--timeout",
            "5",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("检测完成"))
        .stdout(predicate::str::contains("自分析完成"));

    assert!(log_path.exists());
    assert!(directory.path().join("doctor-self-analysis.json").exists());
    let markdown_path = directory.path().join("doctor-self-analysis.md");
    assert!(markdown_path.exists());
    assert!(response.calls() > 0);

    // Mock 不返回 usage，估计容量不能通过刚性门槛。
    let markdown = std::fs::read_to_string(&markdown_path).unwrap();
    assert!(markdown.contains("| 模型最低上下文要求（128K） | 无法判断 |"));
    assert!(markdown.contains("缺少服务端 usage"));
    assert!(markdown.contains("刚性 11 项，柔性 31 项"));
    let log = std::fs::read_to_string(&log_path).unwrap();
    assert!(log.contains("========== REQUEST test-018-calibration BEGIN =========="));
    assert!(log.contains("========== REQUEST test-018-probe-128000-a1 BEGIN =========="));
    assert!(log.contains("========== REQUEST test-018-probe-496000-a1 BEGIN =========="));
}
