use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_llm-capability-doctor")
}

#[test]
fn help_and_version_work_without_cache_or_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let cache = directory.path().join("unused");
    for flag in ["--help", "--version"] {
        let output = Command::new(binary())
            .arg(flag)
            .current_dir(directory.path())
            .env("AGENTCHECK_CACHE_DIR", &cache)
            .env_remove("MODEL_API_KEY")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(!cache.exists());
        assert!(String::from_utf8_lossy(&output.stdout).contains("agentcheck"));
    }
}

#[test]
fn missing_analyzer_keeps_evidence_and_html_without_external_rules() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_server = stop.clone();
    let server = std::thread::spawn(move || {
        while !stop_server.load(Ordering::Relaxed) {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut data = Vec::new();
            let mut buffer = [0; 4096];
            while let Ok(length) = stream.read(&mut buffer) {
                if length == 0 {
                    break;
                }
                data.extend_from_slice(&buffer[..length]);
                if let Some(header_end) = data.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&data[..header_end]).to_lowercase();
                    let body_length = headers
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("content-length:")
                                .map(|value| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if data.len() >= header_end + 4 + body_length {
                        break;
                    }
                }
            }
            let body = r#"{"id":"local-test","object":"chat.completion","model":"fixture","choices":[{"index":0,"message":{"role":"assistant","content":"OK"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":1,"total_tokens":11}}"#;
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(binary())
        .args([
            "--url",
            &format!("http://{address}/v1/chat/completions"),
            "--model",
            "fixture",
            "--no-gui",
            "--modules",
            "ingress",
            "--timeout-seconds",
            "2",
        ])
        .env("MODEL_API_KEY", "fixture-secret-not-for-logs")
        .env("OMP_BIN", directory.path().join("missing-omp"))
        .current_dir(directory.path())
        .output()
        .unwrap();
    let fallback = Command::new(binary())
        .args([
            "--url",
            &format!("http://{address}/v1/chat/completions"),
            "--model",
            "fixture",
            "--gui",
            "--modules",
            "ingress",
            "--report-dir",
            "fallback",
            "--timeout-seconds",
            "2",
        ])
        .env("MODEL_API_KEY", "fixture-secret-not-for-logs")
        .env("OMP_BIN", directory.path().join("missing-omp"))
        .env("AGENTCHECK_GUI_PATH", directory.path().join("missing-gui"))
        .current_dir(directory.path())
        .output()
        .unwrap();
    #[cfg(unix)]
    let detached_gui = {
        use std::os::unix::fs::PermissionsExt;
        let gui = directory.path().join("gui-helper");
        let pid_path = directory.path().join("gui.pid");
        std::fs::write(
            &gui,
            "#!/bin/sh\nprintf '%s' $$ > \"$AGENTCHECK_TEST_PID\"\nwhile [ $# -gt 0 ]; do\nif [ \"$1\" = \"--agentcheck-ready-file\" ]; then shift; printf ready > \"$1\"; fi\nshift\ndone\nexec /bin/sleep 5\n",
        )
        .unwrap();
        std::fs::set_permissions(&gui, std::fs::Permissions::from_mode(0o700)).unwrap();
        let started = std::time::Instant::now();
        let launched = Command::new(binary())
            .args([
                "--url",
                &format!("http://{address}/v1/chat/completions"),
                "--model",
                "fixture",
                "--gui",
            ])
            .env("MODEL_API_KEY", "fixture-secret-not-for-logs")
            .env("AGENTCHECK_GUI_PATH", &gui)
            .env("AGENTCHECK_TEST_PID", &pid_path)
            .current_dir(directory.path())
            .output()
            .unwrap();
        let elapsed = started.elapsed();
        if let Ok(pid) = std::fs::read_to_string(&pid_path) {
            let _ = Command::new("/bin/kill")
                .args(["-TERM", pid.trim()])
                .output();
        }
        (launched, elapsed)
    };
    stop.store(true, Ordering::Relaxed);
    server.join().unwrap();
    #[cfg(unix)]
    {
        assert!(detached_gui.0.status.success());
        assert!(String::from_utf8_lossy(&detached_gui.0.stderr).contains("GUI 已启动"));
        assert!(
            detached_gui.1 < Duration::from_secs(3),
            "GUI 不应保持 CLI 输出管道打开"
        );
    }
    assert!(fallback.status.success());
    assert!(String::from_utf8_lossy(&fallback.stderr).contains("回退到纯 CLI"));
    assert!(directory.path().join("fallback/report.html").is_file());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        directory
            .path()
            .join("agentcheck-report/run.json")
            .is_file()
    );
    assert!(
        directory
            .path()
            .join("agentcheck-report/report.html")
            .is_file()
    );
    assert!(
        directory
            .path()
            .join("agentcheck-report/module-input/ingress.input.json")
            .is_file()
    );
    let report = std::fs::read_to_string(
        directory
            .path()
            .join("agentcheck-report/module-report/ingress.report.json"),
    )
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&report).unwrap();
    // custom 报告模式下 verdict 由模块实测状态映射，mock 响应合法故为 pass。
    assert_eq!(value["verdict"], "pass");
    assert!(!report.contains("fixture-secret-not-for-logs"));
}

/// 按请求体分支的 mock：工具请求回 tool_calls、流式回 SSE 分块（含 usage）、
/// 畸形 messages 回 400 错误对象，普通请求回标准 chat.completion。
#[test]
fn baseline_probes_reach_real_scenarios() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_server = stop.clone();
    let server = std::thread::spawn(move || {
        while !stop_server.load(Ordering::Relaxed) {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut data = Vec::new();
            let mut buffer = [0; 8192];
            while let Ok(length) = stream.read(&mut buffer) {
                if length == 0 {
                    break;
                }
                data.extend_from_slice(&buffer[..length]);
                if let Some(header_end) = data.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&data[..header_end]).to_lowercase();
                    let body_length = headers
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("content-length:")
                                .map(|value| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if data.len() >= header_end + 4 + body_length {
                        break;
                    }
                }
            }
            let request_text = String::from_utf8_lossy(&data);
            let body_start = request_text
                .find("\r\n\r\n")
                .map(|index| index + 4)
                .unwrap_or(request_text.len());
            let request: serde_json::Value =
                serde_json::from_str(&request_text[body_start..]).unwrap_or_default();
            let streaming = request["stream"].as_bool() == Some(true);
            let with_tools = request["tools"].is_array();
            if request["messages"].is_string() {
                let body = r#"{"error":{"message":"messages must be an array","type":"invalid_request_error","param":"messages","code":null}}"#;
                let _ = write!(
                    stream,
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                continue;
            }
            if streaming {
                let tool_chunk = if with_tools {
                    "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"fixture\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"lookup_weather\",\"arguments\":\"{\\\"city\\\":\\\"北京\\\"}\"}}]},\"finish_reason\":null}]}\n\n"
                } else {
                    "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"fixture\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"你好\"},\"finish_reason\":null}]}\n\n"
                };
                let body = format!(
                    "data: {{\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"fixture\",\"choices\":[{{\"index\":0,\"delta\":{{\"role\":\"assistant\"}},\"finish_reason\":null}}]}}\n\n{tool_chunk}data: {{\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"fixture\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: {{\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"fixture\",\"choices\":[],\"usage\":{{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}}}\n\ndata: [DONE]\n\n"
                );
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                continue;
            }
            let body = if with_tools {
                r#"{"id":"local-test","object":"chat.completion","created":1,"model":"fixture","choices":[{"index":0,"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"lookup_weather","arguments":"{\"city\":\"北京\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":1,"total_tokens":11}}"#.to_string()
            } else {
                r#"{"id":"local-test","object":"chat.completion","created":1,"model":"fixture","choices":[{"index":0,"message":{"role":"assistant","content":"OK"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":1,"total_tokens":11}}"#.to_string()
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(binary())
        .args([
            "--url",
            &format!("http://{address}/v1/chat/completions"),
            "--model",
            "fixture",
            "--no-gui",
            "--modules",
            "baseline",
            "--timeout-seconds",
            "5",
        ])
        .env("MODEL_API_KEY", "fixture-secret-not-for-logs")
        .env("OMP_BIN", directory.path().join("missing-omp"))
        .current_dir(directory.path())
        .output()
        .unwrap();
    stop.store(true, Ordering::Relaxed);
    server.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(directory.path().join("agentcheck-report/run.json")).unwrap(),
    )
    .unwrap();
    let baseline = run["record"]["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["payload"]["module"] == "baseline")
        .expect("baseline evidence");
    let comparisons = baseline["payload"]["payload"]["report"]["comparisons"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(comparisons.len(), 14);
    for comparison in &comparisons {
        assert_eq!(
            comparison["status"], "same_structure",
            "{} 应对齐官方结构，实际：{}",
            comparison["scenario"], comparison
        );
    }
    let body_dump = serde_json::to_string(&baseline).unwrap();
    assert!(body_dump.contains("\"tool_choice\":\"required\""));
    assert!(body_dump.contains("\"include_usage\":true"));
    assert!(body_dump.contains("\"messages\":\"not-an-array\""));
}

/// 起一个按请求体分支出应答的 mock；respond 返回 (状态码, Content-Type, 响应体)。
fn spawn_mock(
    respond: impl Fn(&serde_json::Value) -> (u16, &'static str, String) + Send + 'static,
) -> (String, std::thread::JoinHandle<()>, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_server = stop.clone();
    let server = std::thread::spawn(move || {
        while !stop_server.load(Ordering::Relaxed) {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut data = Vec::new();
            let mut buffer = [0; 8192];
            while let Ok(length) = stream.read(&mut buffer) {
                if length == 0 {
                    break;
                }
                data.extend_from_slice(&buffer[..length]);
                if let Some(header_end) = data.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&data[..header_end]).to_lowercase();
                    let body_length = headers
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("content-length:")
                                .map(|value| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if data.len() >= header_end + 4 + body_length {
                        break;
                    }
                }
            }
            let text = String::from_utf8_lossy(&data);
            let body_start = text
                .find("\r\n\r\n")
                .map(|index| index + 4)
                .unwrap_or(text.len());
            let request: serde_json::Value =
                serde_json::from_str(&text[body_start..]).unwrap_or_default();
            let (status, content_type, body) = respond(&request);
            let reason = match status {
                200 => "OK",
                400 => "Bad Request",
                _ => "Error",
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (address, server, stop)
}

/// 红路径：故意偏离规范的服务必须在报告里被准确标出差异，
/// 且不受影响的其他场景仍应判为 same_structure。
#[test]
fn baseline_flags_deviating_service() {
    let tool_calls_message = r#"{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"lookup_weather","arguments":"{\"city\":\"北京\"}"}}]}"#;
    let (address, server, stop) = spawn_mock(move |request| {
        let streaming = request["stream"].as_bool() == Some(true);
        let with_tools = request["tools"].is_array();
        if request["messages"].is_string() {
            // 错误对象缺 error.type
            return (
                400,
                "application/json",
                r#"{"error":{"message":"messages must be an array","param":"messages"}}"#.into(),
            );
        }
        if streaming {
            let delta = if with_tools {
                r#"{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"lookup_weather","arguments":"{}"}}]}"#
            } else {
                r#"{"content":"你好"}"#
            };
            // 故意不发 usage 分块
            let body = format!(
                "data: {{\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"fixture\",\"choices\":[{{\"index\":0,\"delta\":{{\"role\":\"assistant\"}},\"finish_reason\":null}}]}}\n\ndata: {{\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"fixture\",\"choices\":[{{\"index\":0,\"delta\":{delta},\"finish_reason\":null}}]}}\n\ndata: {{\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"fixture\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n"
            );
            return (200, "text/event-stream", body);
        }
        if with_tools {
            return (
                200,
                "application/json",
                format!(
                    r#"{{"id":"t1","object":"chat.completion","created":1,"model":"fixture","choices":[{{"index":0,"message":{tool_calls_message},"finish_reason":"tool_calls"}}],"usage":{{"prompt_tokens":10,"completion_tokens":1,"total_tokens":11}}}}"#
                ),
            );
        }
        // 缺 id；usage.total_tokens 是字符串而非数字
        (
            200,
            "application/json",
            r#"{"object":"chat.completion","created":1,"model":"fixture","choices":[{"index":0,"message":{"role":"assistant","content":"OK"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":1,"total_tokens":"11"}}"#
                .into(),
        )
    });
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(binary())
        .args([
            "--url",
            &format!("http://{address}/v1/chat/completions"),
            "--model",
            "fixture",
            "--no-gui",
            "--modules",
            "baseline",
            "--timeout-seconds",
            "5",
        ])
        .env("MODEL_API_KEY", "fixture-secret-not-for-logs")
        .env("OMP_BIN", directory.path().join("missing-omp"))
        .current_dir(directory.path())
        .output()
        .unwrap();
    stop.store(true, Ordering::Relaxed);
    server.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(directory.path().join("agentcheck-report/run.json")).unwrap(),
    )
    .unwrap();
    let baseline = run["record"]["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["payload"]["module"] == "baseline")
        .expect("baseline evidence");
    let comparisons = baseline["payload"]["payload"]["report"]["comparisons"]
        .as_array()
        .unwrap();
    let status_of = |id: &str| {
        comparisons
            .iter()
            .find(|c| c["scenario"] == id)
            .unwrap_or_else(|| panic!("缺少 {id} 对比结果"))["status"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let kinds_of = |id: &str| {
        comparisons.iter().find(|c| c["scenario"] == id).unwrap()["differences"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| {
                format!(
                    "{}:{}",
                    d["kind"].as_str().unwrap(),
                    d["path"].as_str().unwrap()
                )
            })
            .collect::<Vec<_>>()
    };

    // 偏离项被准确标出
    assert_eq!(status_of("BC01"), "different");
    assert!(
        kinds_of("BC01")
            .iter()
            .any(|k| k == "missing_required_field:id"),
        "BC01 应报缺 id：{:?}",
        kinds_of("BC01")
    );
    assert_eq!(status_of("BC04"), "different");
    assert!(
        kinds_of("BC04")
            .iter()
            .any(|k| k == "type_mismatch:usage.total_tokens"),
        "BC04 应报 total_tokens 类型不符：{:?}",
        kinds_of("BC04")
    );
    assert_eq!(status_of("BC05"), "different");
    assert_eq!(status_of("BC14"), "different");
    assert!(
        kinds_of("BC14")
            .iter()
            .any(|k| k == "missing_required_field:error.type"),
        "BC14 应报缺 error.type：{:?}",
        kinds_of("BC14")
    );
    // 没有 usage 分块应诚实判 inconclusive 而非凭空判差异
    assert_eq!(status_of("BC13"), "inconclusive");

    // 不受缺陷影响的场景保持绿
    for id in [
        "BC02", "BC03", "BC06", "BC07", "BC08", "BC09", "BC10", "BC11", "BC12",
    ] {
        assert_eq!(status_of(id), "same_structure", "{id} 不应误报差异");
    }
}
