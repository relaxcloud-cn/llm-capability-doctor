#![cfg(unix)]

use std::fs;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use model_capability_doctor::protocol::Protocol;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use tempfile::tempdir;

mod fixture_server;

use fixture_server::FixtureServer;

#[test]
fn sigterm_exits_143_and_preserves_the_log_header() {
    assert_signal_exit(Signal::SIGTERM, 143);
}

#[test]
fn sigint_exits_130_and_preserves_the_log_header() {
    assert_signal_exit(Signal::SIGINT, 130);
}

fn assert_signal_exit(signal: Signal, expected_code: i32) {
    let server = FixtureServer::start_delayed(Protocol::OpenAiChat, Duration::from_secs(2));
    let directory = tempdir().unwrap();
    let log_path = directory.path().join("doctor.log");
    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("model-capability-doctor"))
        .env("NO_PROXY", "*")
        .env("no_proxy", "*")
        .args([
            "--url",
            &server.url(),
            "--model",
            "fixture-model",
            "--api-key",
            "fixture-key",
            "--only",
            "004",
            "--log-file",
            log_path.to_str().unwrap(),
            "--timeout",
            "5",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if log_path.exists()
            && fs::read_to_string(&log_path)
                .unwrap_or_default()
                .contains("log_schema: llm-capability-doctor.evidence.v1")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "collector did not write its log header"
        );
        thread::sleep(Duration::from_millis(10));
    }

    kill(Pid::from_raw(child.id() as i32), signal).unwrap();
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(expected_code), "status={status:?}");
    let log = fs::read_to_string(log_path).unwrap();
    assert!(log.contains("log_schema: llm-capability-doctor.evidence.v1"));
}
