use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};
use tokio_util::sync::CancellationToken;
struct Server {
    url: String,
    count: Arc<AtomicUsize>,
    cancel: CancellationToken,
    thread: Option<thread::JoinHandle<()>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
const RAW_PREFLIGHT_ERROR: &str = "{\n  \"error\": {\n    \"message\": \"bad key test-secret-key\",\n    \"code\": \"invalid_api_key\"\n  }\n}\n";
fn server(status: StatusCode) -> Server {
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    let address = socket.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let state = (count.clone(), status);
    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    let thread = thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async move {
            async fn respond(
                State((count, status)): State<(Arc<AtomicUsize>, StatusCode)>,
            ) -> (StatusCode, &'static str) {
                count.fetch_add(1, Ordering::SeqCst);
                (status, RAW_PREFLIGHT_ERROR)
            }
            axum::serve(
                tokio::net::TcpListener::from_std(socket).unwrap(),
                Router::new()
                    .route("/v1/chat/completions", post(respond))
                    .with_state(state),
            )
            .with_graceful_shutdown(stop.cancelled_owned())
            .await
            .unwrap();
        });
    });
    Server {
        url: format!("http://{address}/v1"),
        count,
        cancel,
        thread: Some(thread),
    }
}
#[test]
fn failed_preflight_stops_without_capability_result_and_keeps_stdout_json_clean() {
    let model = server(StatusCode::UNAUTHORIZED);
    let root = tempfile::tempdir().unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_agent-check"))
        .args([
            "--url",
            &model.url,
            "--model",
            "test-model",
            "--no-gui",
            "--json",
            "--report-dir",
            root.path().to_str().unwrap(),
        ])
        .env("MODEL_API_KEY", "test-secret-key")
        .env("CI", "1")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert!(result.stdout.is_empty());
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert!(stderr.contains("未开始正式检测"));
    let expected = RAW_PREFLIGHT_ERROR.replace("test-secret-key", "[REDACTED]");
    assert!(stderr.contains(&format!("HTTP 401\n{expected}")));
    assert_eq!(stderr.matches(expected.trim_end()).count(), 1);
    assert!(!stderr.contains("API 密钥未通过验证"));
    assert!(!stderr.contains("test-secret-key"));
    assert!(!stderr.contains('\x1b'));
    assert_eq!(model.count.load(Ordering::SeqCst), 1);
    assert!(
        !std::fs::read_dir(root.path()).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("AC-"))
    );
}
#[test]
fn report_auto_open_opt_out_is_a_supported_cli_option() {
    let result = Command::new(env!("CARGO_BIN_EXE_agent-check"))
        .args(["--no-open-report", "--help"])
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("--no-open-report")
    );
}
#[test]
fn mutually_exclusive_modes_fail_before_network() {
    let result = Command::new(env!("CARGO_BIN_EXE_agent-check"))
        .args(["--gui", "--no-gui"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
}
#[test]
fn worker_accepts_private_control_and_eof_shutdown_without_a_model() {
    let root = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_agent-check"))
        .args(["--worker", "--report-dir", root.path().to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    fn read(output: &mut BufReader<std::process::ChildStdout>) -> Value {
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }
    assert_eq!(read(&mut output)["type"], "ready");
    writeln!(input, "{}", json!({"id":"h","type":"history"})).unwrap();
    let reply = read(&mut output);
    assert_eq!(reply["id"], "h");
    assert_eq!(reply["data"], json!([]));
    writeln!(input,"{}",json!({"id":"bad","type":"submit","config":{"base_url":"not-a-url","model":"m","api_key":"test-secret-key","protocol":null}})).unwrap();
    let reply = read(&mut output);
    assert_eq!(reply["success"], false);
    assert!(!reply.to_string().contains("test-secret-key"));
    writeln!(input, "{}", json!({"id":"s","type":"state"})).unwrap();
    assert_eq!(read(&mut output)["data"]["busy"], false);
    drop(input);
    assert!(child.wait().unwrap().success());
}

#[test]
fn worker_returns_the_original_preflight_error_without_starting_checks() {
    let model = server(StatusCode::UNAUTHORIZED);
    let root = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_agent-check"))
        .args(["--worker", "--report-dir", root.path().to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap());
    let mut next = || {
        let mut line = String::new();
        lines.read_line(&mut line).unwrap();
        serde_json::from_str::<Value>(&line).unwrap()
    };
    assert_eq!(next()["type"], "ready");
    writeln!(input,"{}",json!({"id":"connection","type":"submit","config":{"base_url":model.url,"model":"test","api_key":"test-secret-key","protocol":"chat-completions"}})).unwrap();
    let reply = loop {
        let message = next();
        assert_ne!(message["type"], "event");
        if message["id"] == "connection" {
            break message;
        }
    };
    assert_eq!(reply["success"], false);
    assert_eq!(
        reply["error"],
        format!(
            "HTTP 401\n{}",
            RAW_PREFLIGHT_ERROR.replace("test-secret-key", "[REDACTED]")
        )
    );
    writeln!(input, "{}", json!({"id":"state","type":"state"})).unwrap();
    loop {
        let message = next();
        assert_ne!(message["type"], "event");
        if message["id"] == "state" {
            assert_eq!(message["data"]["busy"], false);
            assert!(message["data"]["run"].is_null());
            break;
        }
    }
    drop(input);
    assert!(child.wait().unwrap().success());
    assert_eq!(model.count.load(Ordering::SeqCst), 1);
    assert!(!std::fs::read_dir(root.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("AC-")
    }));
}
#[tokio::test]
async fn worker_rejects_duplicate_start_and_eof_cancels_active_request_preserving_result() {
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let count = Arc::new(AtomicUsize::new(0));
    async fn respond(State(count): State<Arc<AtomicUsize>>) -> Json<Value> {
        if count.fetch_add(1, Ordering::SeqCst) > 0 {
            std::future::pending::<()>().await;
        }
        Json(
            json!({"model":"control","choices":[{"message":{"role":"assistant","content":"Connected"},"finish_reason":"stop"}]}),
        )
    }
    let service = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/v1/chat/completions", post(respond))
                .with_state(count),
        )
        .await
        .unwrap();
    });
    let root = tempfile::tempdir().unwrap();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_agent-check"))
        .args(["--worker", "--report-dir", root.path().to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut lines = tokio::io::BufReader::new(child.stdout.take().unwrap()).lines();
    async fn next(
        lines: &mut tokio::io::Lines<tokio::io::BufReader<tokio::process::ChildStdout>>,
    ) -> Value {
        let line = tokio::time::timeout(Duration::from_secs(10), lines.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(!line.contains("test-private-key"));
        serde_json::from_str(&line).unwrap()
    }
    assert_eq!(next(&mut lines).await["type"], "ready");
    let config = json!({"base_url":url,"model":"control","api_key":"test-private-key"});
    let submit = json!({"id":"start","type":"submit","config":config});
    input
        .write_all(format!("{submit}\n").as_bytes())
        .await
        .unwrap();
    let id = loop {
        let message = next(&mut lines).await;
        if message["data"]["event"]["type"] == "started" {
            break message["data"]["event"]["run"]["id"]
                .as_str()
                .unwrap()
                .to_owned();
        }
    };
    let duplicate = json!({"id":"duplicate","type":"submit","config":config});
    input
        .write_all(format!("{duplicate}\n").as_bytes())
        .await
        .unwrap();
    loop {
        let message = next(&mut lines).await;
        if message["id"] == "duplicate" {
            assert_eq!(message["success"], false);
            break;
        }
    }
    // Losing the GUI pipe must stop the active request and still save a report.
    drop(input);
    let final_run = loop {
        let message = next(&mut lines).await;
        if message["data"]["event"]["type"] == "finished" {
            break message["data"]["event"]["run"].clone();
        }
    };
    assert_eq!(final_run["id"], id);
    assert_eq!(final_run["cancelled"], true);
    assert_eq!(final_run["verdict"], "incomplete");
    assert!(
        final_run["checks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|check| check["status"] != "failed")
    );
    assert!(root.path().join(&id).join("report.html").is_file());
    let persisted: Value =
        serde_json::from_slice(&std::fs::read(root.path().join(&id).join("result.json")).unwrap())
            .unwrap();
    assert_eq!(persisted, final_run);
    assert_eq!(
        agent_check::store::history(root.path(), None)
            .unwrap()
            .len(),
        1
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    service.abort();
}
