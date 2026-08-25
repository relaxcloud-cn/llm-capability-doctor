use assert_cmd::cargo::cargo_bin_cmd;
use httpmock::Method::POST;
use httpmock::MockServer;
use predicates::prelude::*;
use serde_json::json;

#[test]
fn default_run_merges_opencodex_summary_into_main_report() {
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
        .stdout(predicate::str::contains("OpenCodex v2.7.42 模型输出兼容性"));

    assert!(
        directory
            .path()
            .join("doctor-opencodex-v2742.json")
            .exists()
    );
    let markdown =
        std::fs::read_to_string(directory.path().join("doctor-self-analysis.md")).unwrap();
    assert!(markdown.contains("## 协议兼容性"));
    assert!(markdown.contains("检测协议：`openai_chat`"));
    assert!(!directory.path().join("doctor-opencodex-v2742.md").exists());
    assert!(response.calls() > 46);
}
