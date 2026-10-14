use crate::{
    config::{ModelConfig, Protocol},
    dataset::{self, SourceSnapshot},
    model::{CheckId, FailureOrigin, Finding, TaskResult, ToolEvent},
    transport::{RequestFailure, Transport},
};
use anyhow::{Context, Result, bail};
use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::post,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub const TASK_BUDGET: Duration = Duration::from_secs(240);
const FAULT_TEXT: &str = "AGENTCHECK_RECOVERABLE_READ";
#[derive(Clone)]
struct ProxyState {
    transport: Transport,
    deadline: Instant,
    evidence: Arc<Mutex<Vec<String>>>,
    delivered: Arc<Mutex<bool>>,
    fault_marker: String,
    proxy_key: String,
    continuation_calls: Arc<Mutex<std::collections::BTreeSet<String>>>,
}
async fn proxy_request(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let expected = format!("Bearer {}", state.proxy_key);
    if headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        != Some(expected.as_str())
    {
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::from("{\"error\":{\"message\":\"Unauthorized\"}}"))
            .unwrap();
    }
    if body["model"] != state.transport.config().model {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::from("{\"error\":{\"message\":\"Unexpected model\"}}"))
            .unwrap();
    }
    let fault_in_context = fault_in_tool_context(
        &body,
        state.transport.config().protocol,
        &state.fault_marker,
    );
    let stream = body["stream"].as_bool().unwrap_or(false);
    match state.transport.send(body, stream, state.deadline).await {
        Ok(outcome) => {
            state
                .evidence
                .lock()
                .await
                .push(outcome.evidence.id.clone());
            let last = outcome.evidence.attempts.last();
            if fault_in_context && last.is_some_and(|a| a.http_status == Some(200)) {
                *state.delivered.lock().await = true;
                if let Some(response) = &outcome.response {
                    state
                        .continuation_calls
                        .lock()
                        .await
                        .extend(response.calls.iter().map(|call| call.id.clone()));
                }
            }
            let status = last
                .and_then(|a| a.http_status)
                .unwrap_or(match outcome.failure {
                    Some(RequestFailure::Timeout) => 504,
                    _ => 502,
                });
            let content_type = last
                .and_then(|a| a.headers.get("content-type"))
                .map(String::as_str)
                .unwrap_or("application/json");
            let body=last.map(|a|a.response_body.clone()).filter(|b|!b.is_empty()).unwrap_or_else(||json!({"error":{"message":outcome.failure.map(|v|v.title()).unwrap_or_else(||"Request failed".into())}}).to_string());
            Response::builder()
                .status(status)
                .header("content-type", content_type)
                .body(Body::from(body))
                .unwrap()
        }
        Err(_) => Response::builder()
            .status(500)
            .body(Body::from(
                "{\"error\":{\"message\":\"Evidence write failed\"}}",
            ))
            .unwrap(),
    }
}
fn fault_in_tool_context(body: &Value, protocol: Protocol, marker: &str) -> bool {
    if marker.is_empty() {
        return false;
    }
    let items = if protocol == Protocol::ChatCompletions {
        body["messages"].as_array()
    } else {
        body["input"].as_array()
    };
    items.into_iter().flatten().any(|item| {
        let tool = if protocol == Protocol::ChatCompletions {
            item["role"] == "tool"
        } else {
            item["type"] == "function_call_output"
        };
        tool && serde_json::to_string(item).is_ok_and(|v| v.contains(marker))
    })
}

struct RpcSession {
    child: Child,
    process_group: Option<u32>,
    stdin: ChildStdin,
    stdout: tokio::io::Lines<BufReader<ChildStdout>>,
    transcript: Vec<Value>,
    tools: BTreeMap<String, ToolEvent>,
    normal_end: bool,
    assistant_ended: bool,
    config: ModelConfig,
    step: Option<Arc<dyn Fn(String) + Send + Sync>>,
}
impl RpcSession {
    async fn command(&mut self, value: Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(&value)?;
        bytes.push(b'\n');
        self.stdin.write_all(&bytes).await?;
        self.stdin.flush().await?;
        Ok(())
    }
    async fn next(&mut self, deadline: Instant, cancel: &CancellationToken) -> Result<Value> {
        let raw = tokio::select! {
            _=cancel.cancelled()=>bail!("CANCELLED"),
            value=tokio::time::timeout(deadline.saturating_duration_since(Instant::now()),self.stdout.next_line())=>value.context("TASK_TIMEOUT")??.context("OMP_EOF")?,
        };
        if raw.len() > 1_048_576 {
            bail!("OMP RPC 超出约定帧大小");
        }
        let raw = self.config.redact(&raw);
        let frame: Value = serde_json::from_str(&raw).context("OMP RPC 返回无效 JSON")?;
        self.transcript.push(frame.clone());
        Ok(frame)
    }
    async fn wait_ready(&mut self, deadline: Instant, cancel: &CancellationToken) -> Result<()> {
        loop {
            let frame = self.next(deadline, cancel).await?;
            if frame["type"] == "ready" {
                return Ok(());
            }
            if frame["type"] == "extension_error" {
                bail!("OMP 扩展加载失败");
            }
        }
    }
    async fn turn(
        &mut self,
        prompt: String,
        deadline: Instant,
        cancel: &CancellationToken,
    ) -> Result<()> {
        self.normal_end = false;
        self.assistant_ended = false;
        let mut agent_started = false;
        let id = Uuid::new_v4().to_string();
        self.command(json!({"id":id,"type":"prompt","message":prompt}))
            .await?;
        loop {
            let frame = self.next(deadline, cancel).await?;
            match frame["type"].as_str() {
                Some("agent_start") => agent_started = true,
                Some("response") if frame["id"] == id => {
                    if frame["success"] == false {
                        bail!("OMP 拒绝任务请求");
                    }
                    if frame["data"]["agentInvoked"] == false {
                        bail!("OMP 接受请求但未启动模型任务");
                    }
                }
                Some("extension_error") => bail!("OMP 扩展执行失败"),
                Some("auto_retry_start" | "retry_fallback_applied" | "model_changed") => {
                    bail!("OMP 触发了未允许的内部重试或模型切换")
                }
                Some("tool_execution_start") => {
                    let call_id = frame["toolCallId"]
                        .as_str()
                        .context("工具执行记录缺少 ID")?
                        .to_owned();
                    let name = frame["toolName"].as_str().unwrap_or("unknown").to_owned();
                    if !["read", "glob", "grep", "write", "edit"].contains(&name.as_str()) {
                        bail!("OMP 启用了检测范围外的工具");
                    }
                    if let Some(step) = &self.step {
                        step(format!("调用 {name} 文件工具"));
                    }
                    self.tools.insert(
                        call_id.clone(),
                        ToolEvent {
                            call_id,
                            name,
                            arguments: frame["args"].clone(),
                            result: Value::Null,
                            is_error: true,
                        },
                    );
                }
                Some("tool_execution_end") => {
                    let id = frame["toolCallId"]
                        .as_str()
                        .context("工具完成记录缺少 ID")?;
                    if let Some(tool) = self.tools.get_mut(id) {
                        tool.result = frame["result"].clone();
                        tool.is_error = frame["isError"].as_bool().unwrap_or(true);
                    } else {
                        // Blocked tools can omit a start event. Keep the delivered native error.
                        self.tools.insert(
                            id.into(),
                            ToolEvent {
                                call_id: id.into(),
                                name: frame["toolName"].as_str().unwrap_or("unknown").into(),
                                arguments: frame["args"].clone(),
                                result: frame["result"].clone(),
                                is_error: frame["isError"].as_bool().unwrap_or(true),
                            },
                        );
                    }
                }
                Some("message_end") if frame["message"]["role"] == "assistant" => {
                    self.assistant_ended = frame["message"]["stopReason"] == "stop";
                }
                Some("agent_end") if agent_started && frame["isTerminal"] != false => {
                    if let Some(last) = frame["messages"].as_array().and_then(|messages| {
                        messages.iter().rev().find(|m| m["role"] == "assistant")
                    }) {
                        self.assistant_ended = last["stopReason"] == "stop";
                    }
                    self.normal_end = self.assistant_ended;
                    return Ok(());
                }
                _ => {}
            }
        }
    }
    async fn stop(&mut self) {
        let _ = self.command(json!({"type":"abort"})).await;
        if let Some(pid) = self.process_group.take() {
            #[cfg(unix)]
            {
                // The child was started in its own process group, never the CLI's group.
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            }
            #[cfg(windows)]
            {
                let system = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
                let mut kill = Command::new(PathBuf::from(system).join("System32/taskkill.exe"));
                kill.args(["/PID", &pid.to_string(), "/T", "/F"])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                let _ = tokio::time::timeout(Duration::from_secs(3), kill.status()).await;
            }
        }
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}
impl Drop for RpcSession {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.process_group.take() {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
        let _ = self.child.start_kill();
    }
}
struct ProxyGuard {
    abort: tokio::task::AbortHandle,
    cancel: CancellationToken,
}
impl Drop for ProxyGuard {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.abort.abort();
    }
}

pub fn development_runtime() -> Result<PathBuf> {
    if let Some(path) = crate::runtime::component("omp")? {
        return Ok(path);
    }
    let target = env!("AGENTCHECK_TARGET");
    let binary = if cfg!(windows) { "omp.exe" } else { "omp" };
    let path = std::env::var_os("AGENTCHECK_OMP_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(".cache/omp")
                .join(crate::model::OMP_VERSION)
                .join(target)
                .join(binary)
        });
    if !path.is_file() {
        bail!("开发运行环境未准备，请运行 scripts/prepare-runtime.mjs");
    }
    use sha2::{Digest, Sha256};
    let manifest: Value = serde_json::from_str(include_str!("../runtime/pi-release.json"))?;
    let expected = manifest["assets"][target]["sha256"]
        .as_str()
        .context("OMP 清单缺少本平台的校验值")?;
    let actual = format!("{:x}", Sha256::digest(std::fs::read(&path)?));
    if actual != expected {
        bail!("OMP 运行环境校验失败，拒绝使用未确认的版本");
    }
    Ok(path)
}

pub async fn run_task(
    config: ModelConfig,
    kind: CheckId,
    repetition: u8,
    folder: &Path,
    cancel: CancellationToken,
    step: Option<Arc<dyn Fn(String) + Send + Sync>>,
) -> Result<TaskResult> {
    if !kind.is_agent() || !(1..=2).contains(&repetition) {
        bail!("无效的 OMP 任务");
    }
    let cancel = cancel.child_token();
    let started = Instant::now();
    let deadline = started + TASK_BUDGET;
    let snapshot = SourceSnapshot::for_omp("agentcheck/11");
    let workdir = folder.join("workspace");
    snapshot.write_inputs(&workdir)?;
    let config_dir = folder.join("config");
    std::fs::create_dir_all(&config_dir)?;
    let evidence_dir = folder.join("http");
    std::fs::create_dir_all(&evidence_dir)?;
    std::fs::write(
        folder.join("source-snapshot.json"),
        serde_json::to_vec_pretty(&snapshot)?,
    )?;
    let marker = format!("{FAULT_TEXT}_{}", Uuid::new_v4().simple());
    let evidence = Arc::new(Mutex::new(vec![]));
    let delivered = Arc::new(Mutex::new(false));
    let continuation_calls = Arc::new(Mutex::new(std::collections::BTreeSet::new()));
    let transport = Transport::new(config.clone(), &evidence_dir, cancel.clone())?;
    let proxy_key = Uuid::new_v4().to_string();
    let proxy = ProxyState {
        transport,
        deadline,
        proxy_key: proxy_key.clone(),
        continuation_calls: continuation_calls.clone(),
        evidence: evidence.clone(),
        delivered: delivered.clone(),
        fault_marker: if kind == CheckId::Recovery {
            marker.clone()
        } else {
            String::new()
        },
    };
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let router = Router::new()
        .route("/v1/chat/completions", post(proxy_request))
        .route("/v1/responses", post(proxy_request))
        .with_state(proxy);
    let proxy_stop = CancellationToken::new();
    let stop = proxy_stop.clone();
    let mut server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(stop.cancelled_owned())
            .await
    });
    let _proxy_guard = ProxyGuard {
        abort: server.abort_handle(),
        cancel: cancel.clone(),
    };
    let api = if config.protocol == Protocol::Responses {
        "openai-responses"
    } else {
        "openai-completions"
    };
    let settings = json!({"retry":{"enabled":false,"maxRetries":0,"modelFallback":false},"compaction":{"enabled":false,"asyncEnabled":false,"midTurnEnabled":false},"memory":{"enabled":false},"startup":{"quiet":true},"lsp":{"enabled":false},"todo":{"enabled":false},"task":{"enabled":false},"bash":{"enabled":false,"autoBackground":{"enabled":false}},"launch":{"enabled":false},"read":{"summarize":{"enabled":false}},"providers":{"openai":{"serviceTier":"none"}}});
    std::fs::write(
        config_dir.join("config.yml"),
        serde_json::to_vec_pretty(&settings)?,
    )?;
    let model = json!({"providers":{"agentcheck":{"baseUrl":format!("http://{address}/v1"),"apiKey":"AGENTCHECK_PROXY_TOKEN","api":api,"models":[{"id":config.model,"name":config.model,"reasoning":false,"input":["text"],"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},"contextWindow":131072,"maxTokens":8192,"compat":{"maxTokensField":"max_tokens","supportsStore":false,"supportsDeveloperRole":false,"supportsReasoningEffort":false}}]}}});
    std::fs::write(
        config_dir.join("models.yml"),
        serde_json::to_vec_pretty(&model)?,
    )?;
    let extension = folder.join("isolation.ts");
    let code = r#"import { resolve, relative, isAbsolute } from 'node:path';
export default function(pi) {
  let injected=false;
  pi.on('tool_call', async (event,ctx) => {
    if (!['read','glob','grep','write','edit'].includes(event.toolName)) return {block:true,reason:'Only file tools are permitted in this task.'};
    const input=event.input||{};
    const candidate=input.path||input.file_path||input.filePath||'.';
    if(typeof candidate==='string') {
      if(/^[a-z][a-z0-9+.-]*:\/\//i.test(candidate)) return {block:true,reason:'Only local workspace files are available.'};
      const path=resolve(ctx.cwd,candidate.split('?')[0]);
      const rel=relative(resolve(ctx.cwd),path);
      if(rel==='..'||rel.startsWith('../')||rel.startsWith('..\\')||isAbsolute(rel)) return {block:true,reason:'This task can access only its workspace.'};
    }
    if(typeof input.pattern==='string'&&input.pattern.split(/[\\/]/).includes('..')) return {block:true,reason:'Parent-directory patterns are not available.'};
    if(process.env.AGENTCHECK_INJECT_FAILURE==='1'&&!injected&&event.toolName==='read') {
      injected=true;return {block:true,reason:process.env.AGENTCHECK_FAULT_MARKER+': temporary file read failure. Retry the same read; the file is available on the next attempt.'};
    }
  });
}
"#;
    std::fs::write(&extension, code)?;
    let selector = format!("agentcheck/{}", config.model);
    let mut command = Command::new(development_runtime()?);
    command
        .current_dir(&workdir)
        .args([
            "--mode",
            "rpc",
            "--no-session",
            "--no-title",
            "--no-extensions",
            "--no-skills",
            "--no-rules",
            "--no-lsp",
            "--no-pty",
            "--no-prewalk",
            "--tools",
            "read,glob,grep,write,edit",
            "--thinking",
            "off",
            "--approval-mode",
            "yolo",
            "--model",
            &selector,
            "--smol",
            &selector,
            "--slow",
            &selector,
            "--plan",
            &selector,
        ])
        .arg("--config")
        .arg(config_dir.join("config.yml"))
        .arg("-e")
        .arg(&extension)
        .env("PI_CODING_AGENT_DIR", &config_dir)
        .env("AGENTCHECK_PROXY_TOKEN", &proxy_key)
        .env(
            "AGENTCHECK_INJECT_FAILURE",
            if kind == CheckId::Recovery { "1" } else { "0" },
        )
        .env("AGENTCHECK_FAULT_MARKER", &marker)
        .env_remove("MODEL_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("PI_SMOL_MODEL")
        .env_remove("PI_SLOW_MODEL")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            proxy_stop.cancel();
            server.abort();
            return Err(error.into());
        }
    };
    let stderr = child.stderr.take().unwrap();
    let redact = config.clone();
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut output = String::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if output.len() < 1_048_576 {
                output.push_str(&redact.redact(&line));
                output.push('\n');
            }
        }
        output
    });
    let stdin = child.stdin.take().unwrap();
    let stdout = BufReader::new(child.stdout.take().unwrap()).lines();
    let process_group = child.id();
    let mut session = RpcSession {
        child,
        process_group,
        stdin,
        stdout,
        transcript: vec![],
        tools: BTreeMap::new(),
        normal_end: false,
        assistant_ended: false,
        config: config.clone(),
        step,
    };
    let mut first_valid = true;
    let mut first_content_valid = true;
    let execution = async {
        session.wait_ready(deadline, &cancel).await?;
        session.turn(snapshot.prompt(), deadline, &cancel).await?;
        if kind == CheckId::Multiturn {
            let Ok(output) = dataset::read_result(&workdir) else {
                first_valid = false;
                first_content_valid = false;
                return Ok(());
            };
            let events = session
                .transcript
                .iter()
                .filter(|frame| frame["type"] == "tool_execution_end")
                .filter_map(|frame| {
                    frame["toolCallId"]
                        .as_str()
                        .and_then(|id| session.tools.get(id))
                })
                .cloned()
                .collect::<Vec<_>>();
            let validation = dataset::validate_task(
                &snapshot,
                None,
                &output,
                kind,
                &events,
                session.normal_end,
                false,
            )?;
            std::fs::write(
                folder.join("stage1-result.json"),
                serde_json::to_vec_pretty(&output)?,
            )?;
            first_content_valid = validation.content_valid;
            first_valid = validation.passed;
            if first_valid {
                session
                    .turn(snapshot.continuation_prompt(), deadline, &cancel)
                    .await?;
            }
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let was_cancelled = cancel.is_cancelled();
    session.stop().await;
    cancel.cancel();
    proxy_stop.cancel();
    if tokio::time::timeout(Duration::from_secs(3), &mut server)
        .await
        .is_err()
    {
        server.abort();
    }
    let stderr = stderr_task.await.unwrap_or_default();
    std::fs::write(folder.join("stderr.txt"), stderr)?;
    std::fs::write(
        folder.join("rpc.json"),
        serde_json::to_vec_pretty(&session.transcript)?,
    )?;
    let events = std::mem::take(&mut session.tools)
        .into_values()
        .collect::<Vec<_>>();
    // Tool events must retain execution order rather than lexical call-ID order.
    let mut ordered = vec![];
    for frame in &session.transcript {
        if frame["type"] == "tool_execution_end" {
            if let Some(event) = events
                .iter()
                .find(|event| frame["toolCallId"] == event.call_id)
            {
                ordered.push(event.clone());
            }
        }
    }
    let fault_delivered = *delivered.lock().await;
    let recovered =
        continued_after_fault(&ordered, &*continuation_calls.lock().await, config.protocol);
    let mut result = TaskResult {
        finished: false,
        id: format!("{:?}-{repetition}", kind).to_lowercase(),
        kind,
        repetition,
        status: crate::model::Status::Incomplete,
        title: String::new(),
        elapsed_ms: started.elapsed().as_millis() as u64,
        normal_end: session.normal_end,
        fault_delivered,
        recovery_observed: recovered,
        tool_events: ordered,
        evidence_ids: evidence.lock().await.clone(),
        findings: vec![],
    };
    match execution {
        Err(error) => {
            let message = config.redact(&error.to_string());
            let cancelled = was_cancelled;
            let timeout = message.contains("TASK_TIMEOUT");
            result.finished = timeout && !cancelled;
            result.status = if timeout {
                crate::model::Status::Failed
            } else {
                crate::model::Status::Incomplete
            };
            result.title = if cancelled {
                "任务已取消".into()
            } else if timeout {
                "任务执行超时".into()
            } else {
                "任务运行受阻，尚未取得完整结果".into()
            };
            result.findings.push(finding(
                if timeout {
                    "task_timeout"
                } else {
                    "runtime_blocked"
                },
                &result.title,
                &message,
                if timeout {
                    FailureOrigin::Service
                } else {
                    FailureOrigin::Runtime
                },
                &result.evidence_ids,
            ));
        }
        Ok(()) => {
            result.finished = true;
            let changes = (kind == CheckId::Multiturn && first_valid).then(|| snapshot.changes());
            let output = dataset::read_result(&workdir);
            if let Ok(output) = output {
                let validation = dataset::validate_task(
                    &snapshot,
                    changes.as_ref(),
                    &output,
                    kind,
                    &result.tool_events,
                    result.normal_end,
                    fault_delivered,
                )?;
                std::fs::write(
                    folder.join("validation.json"),
                    serde_json::to_vec_pretty(&validation)?,
                )?;
                result.status = task_status(
                    &validation,
                    first_content_valid,
                    first_valid && (kind != CheckId::Recovery || (fault_delivered && recovered)),
                    result.normal_end,
                );
                result.title = if result.status == crate::model::Status::Passed {
                    match kind {
                        CheckId::Continuous => "连续执行多个工具，任务结果正确",
                        CheckId::Recovery => "工具出错后恢复，任务结果正确",
                        _ => "指定字段修改生效，其余内容保持不变",
                    }
                    .into()
                } else if !result.normal_end {
                    "任务未正常结束，内容能力尚未确认".into()
                } else if !fault_delivered && kind == CheckId::Recovery {
                    "尚未确认读取故障已送达模型".into()
                } else if kind == CheckId::Recovery && !recovered {
                    "没有故障送达后继续执行成功的工具证据".into()
                } else if !first_content_valid {
                    "第一轮结果不符合要求".into()
                } else if !first_valid {
                    "第一轮过程证据不足，第二轮未执行".into()
                } else if let Some(difference) = validation.differences.first() {
                    if difference.field == "version"
                        && kind == CheckId::Multiturn
                        && snapshot
                            .records()
                            .iter()
                            .any(|r| r.id == difference.id && json!(r.version) == difference.actual)
                    {
                        "指定修改未生效，仍保留旧内容".into()
                    } else if difference.field == "version" {
                        "结果中的版本号不符合要求".into()
                    } else {
                        "任务结果存在遗漏或字段错误".into()
                    }
                } else if !validation.content_valid {
                    "结果文件格式、字段或类型不符合要求".into()
                } else {
                    "缺少实际读取或写入证据".into()
                };
                if result.status != crate::model::Status::Passed {
                    if kind == CheckId::Recovery && recovered {
                        result.title = format!("工具已恢复；{}", result.title);
                    }
                    let fact = serde_json::to_string(&json!({
                        "validation": validation,
                        "fault_delivered": fault_delivered,
                        "recovery_observed": recovered,
                    }))?;
                    result.findings.push(finding(
                        "task_validation",
                        &result.title,
                        &fact,
                        if result.status == crate::model::Status::Incomplete {
                            FailureOrigin::Detector
                        } else {
                            FailureOrigin::ModelOutput
                        },
                        &result.evidence_ids,
                    ));
                }
            } else {
                result.status = if result.normal_end {
                    crate::model::Status::Failed
                } else {
                    crate::model::Status::Incomplete
                };
                result.title = if result.normal_end {
                    "未生成有效结果文件"
                } else {
                    "任务未正常结束，结果文件尚未确认"
                }
                .into();
                if kind == CheckId::Recovery && recovered {
                    result.title = format!("工具已恢复；{}", result.title);
                }
                result.findings.push(finding(
                    "missing_result",
                    &result.title,
                    "任务结束后 result.json 缺失或不是有效 JSON",
                    if result.normal_end {
                        FailureOrigin::ModelOutput
                    } else {
                        FailureOrigin::Runtime
                    },
                    &result.evidence_ids,
                ));
            }
        }
    }
    std::fs::write(
        folder.join("task-result.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    Ok(result)
}
fn task_status(
    validation: &dataset::Validation,
    first_content_valid: bool,
    process_valid: bool,
    normal_end: bool,
) -> crate::model::Status {
    if !normal_end {
        crate::model::Status::Incomplete
    } else if !validation.content_valid || !first_content_valid {
        crate::model::Status::Failed
    } else if !validation.passed || !process_valid {
        crate::model::Status::Incomplete
    } else {
        crate::model::Status::Passed
    }
}
pub(crate) fn wire_call_id(protocol: Protocol, runtime_id: &str) -> &str {
    if protocol == Protocol::Responses {
        runtime_id
            .split_once('|')
            .map_or(runtime_id, |(call, _)| call)
    } else {
        runtime_id
    }
}
fn continued_after_fault(
    events: &[ToolEvent],
    calls: &std::collections::BTreeSet<String>,
    protocol: Protocol,
) -> bool {
    events.iter().any(|event| {
        calls.contains(wire_call_id(protocol, &event.call_id))
            && !event.is_error
            && ["read", "glob", "grep", "write", "edit"].contains(&event.name.as_str())
    })
}
fn finding(
    code: &str,
    title: &str,
    fact: &str,
    origin: FailureOrigin,
    evidence: &[String],
) -> Finding {
    Finding {
        code: code.into(),
        title: title.into(),
        fact: fact.into(),
        impact: "智能体任务未得到完整、正确且可确认的结果。".into(),
        suggestion: "请根据检测记录检查对应问题，处理后重新运行检测。".into(),
        origin,
        evidence_ids: evidence.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unverified_first_turn_process_is_not_a_model_content_failure() {
        let mut validation = dataset::Validation {
            passed: false,
            content_valid: true,
            differences: vec![],
            process_errors: vec!["来源证据不足".into()],
        };
        assert_eq!(
            task_status(&validation, true, false, true),
            crate::model::Status::Incomplete
        );
        assert_eq!(
            task_status(&validation, false, false, true),
            crate::model::Status::Failed
        );
        validation.process_errors.clear();
        validation.passed = true;
        assert_eq!(
            task_status(&validation, true, false, true),
            crate::model::Status::Incomplete
        );
        assert_eq!(
            task_status(&validation, true, true, true),
            crate::model::Status::Passed
        );
    }
    #[test]
    fn abnormal_termination_does_not_turn_partial_output_into_a_model_failure() {
        let validation = dataset::Validation {
            passed: false,
            content_valid: false,
            differences: vec![],
            process_errors: vec!["任务未正常结束".into()],
        };
        assert_eq!(
            task_status(&validation, false, false, false),
            crate::model::Status::Incomplete
        );
        assert_eq!(
            task_status(&validation, false, true, true),
            crate::model::Status::Failed
        );
    }
    #[test]
    fn pending_calls_are_not_mistaken_for_actions_after_error_delivery() {
        let earlier = ToolEvent {
            call_id: "old".into(),
            name: "read".into(),
            arguments: json!({}),
            result: json!({}),
            is_error: false,
        };
        let later = ToolEvent {
            call_id: "new".into(),
            name: "write".into(),
            arguments: json!({}),
            result: json!({}),
            is_error: false,
        };
        let calls = std::collections::BTreeSet::from(["new".into()]);
        assert!(!continued_after_fault(
            std::slice::from_ref(&earlier),
            &calls,
            Protocol::ChatCompletions,
        ));
        assert!(continued_after_fault(
            &[earlier, later],
            &calls,
            Protocol::ChatCompletions
        ));
    }
    #[test]
    fn fault_is_proven_by_forwarded_tool_context_not_assistant_claim() {
        let marker = "AGENTCHECK_RECOVERABLE_READ_nonce";
        assert!(!fault_in_tool_context(
            &json!({"messages":[{"role":"assistant","content":marker}]}),
            Protocol::ChatCompletions,
            marker
        ));
        assert!(fault_in_tool_context(
            &json!({"messages":[{"role":"tool","content":marker}]}),
            Protocol::ChatCompletions,
            marker
        ));
        assert!(fault_in_tool_context(
            &json!({"input":[{"type":"function_call_output","call_id":"x","output":marker}]}),
            Protocol::Responses,
            marker
        ));
    }
}
