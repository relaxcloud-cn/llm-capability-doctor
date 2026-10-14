use agent_check::{
    config::{ModelConfig, Protocol},
    preflight,
    transport::HttpEvidence,
};
use axum::{Router, extract::State, http::StatusCode, routing::post};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio_util::sync::CancellationToken;

type ResponseState = (Arc<AtomicUsize>, Arc<Vec<(StatusCode, String)>>);
struct Service {
    url: String,
    count: Arc<AtomicUsize>,
    job: tokio::task::JoinHandle<()>,
}
impl Drop for Service {
    fn drop(&mut self) {
        self.job.abort();
    }
}
async fn service(replies: Vec<(StatusCode, String)>) -> Service {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let count = Arc::new(AtomicUsize::new(0));
    async fn respond(
        State((count, replies)): State<ResponseState>,
    ) -> (StatusCode, [(String, String); 1], String) {
        let index = count.fetch_add(1, Ordering::SeqCst).min(replies.len() - 1);
        (
            replies[index].0,
            [("retry-after".into(), "0".into())],
            replies[index].1.clone(),
        )
    }
    let router = Router::new()
        .route("/v1/chat/completions", post(respond))
        .route("/v1/responses", post(respond))
        .with_state((count.clone(), Arc::new(replies)));
    let job = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Service { url, count, job }
}
fn evidence(root: &std::path::Path) -> HttpEvidence {
    let folder = std::fs::read_dir(root.join("startup"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let file = std::fs::read_dir(folder.join("http"))
        .or_else(|_| std::fs::read_dir(&folder))
        .unwrap()
        .find_map(|entry| {
            let path = entry.unwrap().path();
            (path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("http-")
                && path.extension().is_some_and(|v| v == "json"))
            .then_some(path)
        })
        .unwrap();
    serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap()
}
#[tokio::test]
async fn both_protocols_preserve_original_http_error_body_without_rewording() {
    for protocol in [Protocol::ChatCompletions, Protocol::Responses] {
        for (status, body) in [
            (
                StatusCode::UNAUTHORIZED,
                "{\n  \"error\": {\"message\": \"quota-specific authentication detail\", \"code\": \"vendor_code\"}\n}\n",
            ),
            (
                StatusCode::FORBIDDEN,
                "permission denied\nproject access is disabled\trequest: 123\n",
            ),
            (
                StatusCode::NOT_FOUND,
                "{\"error\":\"deployment custom-name is missing\"}",
            ),
            (
                StatusCode::BAD_GATEWAY,
                "<html><body>upstream unavailable</body></html>",
            ),
            (
                StatusCode::OK,
                "{\"error\":{\"message\":\"application-level error\"}}",
            ),
            (
                StatusCode::OK,
                "gateway returned non-JSON text\noriginal detail",
            ),
        ] {
            let server = service(vec![(status, body.into())]).await;
            let root = tempfile::tempdir().unwrap();
            let config = ModelConfig::new(
                &server.url,
                "test",
                "private-api-key".into(),
                Some(protocol),
            )
            .unwrap();
            let error = preflight::check(&config, root.path(), CancellationToken::new())
                .await
                .unwrap_err()
                .to_string();
            assert_eq!(error, format!("HTTP {}\n{body}", status.as_u16()));
            let saved = evidence(root.path());
            assert_eq!(saved.attempts.last().unwrap().response_body, body);
            assert_eq!(
                server.count.load(Ordering::SeqCst),
                if status.as_u16() >= 500 { 3 } else { 1 }
            );
        }
    }
}
#[tokio::test]
async fn final_retry_response_is_shown_and_earlier_attempts_are_retained() {
    let server = service(vec![
        (StatusCode::SERVICE_UNAVAILABLE, "first failure".into()),
        (StatusCode::SERVICE_UNAVAILABLE, "second failure".into()),
        (
            StatusCode::TOO_MANY_REQUESTS,
            "{\"error\":\"final quota failure\"}".into(),
        ),
    ])
    .await;
    let root = tempfile::tempdir().unwrap();
    let config = ModelConfig::new(&server.url, "test", "private-key".into(), None).unwrap();
    let error = preflight::check(&config, root.path(), CancellationToken::new())
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(error, "HTTP 429\n{\"error\":\"final quota failure\"}");
    let saved = evidence(root.path());
    assert_eq!(saved.attempts.len(), 3);
    assert_eq!(saved.attempts[0].response_body, "first failure");
}
#[tokio::test]
async fn raw_errors_redact_configured_credentials_including_json_escaping() {
    let secret = "private-\"quoted-key\"";
    let body =
        serde_json::json!({"error":{"message":format!("Invalid key: {secret}")}}).to_string();
    let server = service(vec![(StatusCode::UNAUTHORIZED, body.clone())]).await;
    let root = tempfile::tempdir().unwrap();
    let config = ModelConfig::new(&server.url, "test", secret.into(), None).unwrap();
    let error = preflight::check(&config, root.path(), CancellationToken::new())
        .await
        .unwrap_err()
        .to_string();
    let expected = body.replace("private-\\\"quoted-key\\\"", "[REDACTED]");
    assert_eq!(error, format!("HTTP 401\n{expected}"));
    assert_eq!(evidence(root.path()).attempts[0].response_body, expected);
}
#[tokio::test]
async fn empty_http_body_uses_status_without_guessing_the_cause() {
    let server = service(vec![(StatusCode::UNAUTHORIZED, String::new())]).await;
    let root = tempfile::tempdir().unwrap();
    let config = ModelConfig::new(&server.url, "test", "private-key".into(), None).unwrap();
    let error = preflight::check(&config, root.path(), CancellationToken::new())
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(error, "HTTP 401（响应正文为空）");
}
#[tokio::test]
async fn normal_responses_still_pass_and_cancellation_is_not_a_server_error() {
    for (protocol, body) in [
        (
            Protocol::ChatCompletions,
            serde_json::json!({"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}),
        ),
        (
            Protocol::Responses,
            serde_json::json!({"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}]}),
        ),
    ] {
        let server = service(vec![(StatusCode::OK, body.to_string())]).await;
        let root = tempfile::tempdir().unwrap();
        let config =
            ModelConfig::new(&server.url, "test", "private-key".into(), Some(protocol)).unwrap();
        assert!(
            preflight::check(&config, root.path(), CancellationToken::new())
                .await
                .is_ok()
        );
        assert_eq!(server.count.load(Ordering::SeqCst), 1);
        let token = CancellationToken::new();
        token.cancel();
        let error = preflight::check(&config, root.path(), token)
            .await
            .unwrap_err()
            .to_string();
        assert_eq!(error, "连接检查已取消");
        assert_eq!(server.count.load(Ordering::SeqCst), 1);
    }
}
#[test]
fn escaped_and_standalone_query_credentials_are_redacted() {
    let config = ModelConfig::new(
        "https://example.org/v1?token=query%2Fprivate",
        "test",
        "private-key".into(),
        None,
    )
    .unwrap();
    assert_eq!(
        config.redact("token=query%2Fprivate | query/private | query%2Fprivate | private-key"),
        "[REDACTED] | [REDACTED] | [REDACTED] | [REDACTED]"
    );
    let config = ModelConfig::new(
        "https://example.org/v1?timeout=10&stream=true",
        "test",
        "private-key".into(),
        None,
    )
    .unwrap();
    let response = "{\"version\":10,\"ok\":true}";
    assert_eq!(config.redact(response), response);
    assert_eq!(config.redact("timeout=10&stream=true"), "[REDACTED]");
}
#[test]
fn terminal_preserves_line_breaks_but_does_not_execute_control_sequences() {
    let text = "HTTP 401\r\n{\n\t\"error\": \"raw text\"\n}\n";
    assert_eq!(agent_check::terminal::safe_error_text(text), text);
    let safe = agent_check::terminal::safe_error_text("raw\u{1b}[2J\rforged\u{9b}m");
    assert!(!safe.contains('\u{1b}') && !safe.contains('\r') && !safe.contains('\u{9b}'));
    assert!(safe.contains("\\u{1b}[2J") && safe.contains("\\rforged"));
}
#[tokio::test]
async fn network_failure_retains_the_underlying_error_without_credentials() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let root = tempfile::tempdir().unwrap();
    let config = ModelConfig::new(
        &format!("http://{address}/v1?token=private-query"),
        "test",
        "private-api-key".into(),
        None,
    )
    .unwrap();
    let error = preflight::check(&config, root.path(), CancellationToken::new())
        .await
        .unwrap_err()
        .to_string();
    let saved = evidence(root.path());
    let last = saved.attempts.last().unwrap();
    assert_eq!(Some(error.as_str()), last.error.as_deref());
    assert!(error.contains("error"));
    assert_ne!(error, "无法连接模型服务");
    assert!(!error.contains("private-query"));
    assert!(!error.contains("private-api-key"));
    assert!(last.http_status.is_none() && last.response_body.is_empty());
}
