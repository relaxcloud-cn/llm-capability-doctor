use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::conclusion::{CustomerConclusionReport, build_default_customer_report};
use crate::ingress::{ConnectionConfig, redact_endpoint};
use crate::records::{
    AttemptKind, EventInput, EventKind, ModuleResult, ModuleResultState, OverallConclusion,
    add_attempt, add_event, add_evidence, complete_run, create_run, set_module_result,
    set_overall_conclusion, start_run, stop_run,
};
use crate::records::{AttemptRecord, CreateRunInput, DetectionRecord};
use crate::transport::{ChatCompletionsRequest, ChatCompletionsTransport};

pub const CLI_VERSION: &str = "cli/v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliConfiguration {
    pub redacted_endpoint: String,
    pub model: String,
    pub api_key_provided: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliRunRequest {
    pub endpoint: String,
    pub model: String,
    pub api_key: Option<String>,
    pub selected_modules: Option<Vec<String>>,
    pub stop_after: Option<String>,
    pub run_id: String,
    pub started_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModuleRunResult {
    pub state: ModuleResultState,
    pub reason: Option<String>,
    pub evidence_kind: String,
    pub evidence_summary: String,
    pub evidence_payload: serde_json::Value,
}

pub trait ModuleExecutor {
    fn execute(&mut self, module_id: &str, record: &DetectionRecord) -> ModuleRunResult;

    fn execution_origin(&self) -> &'static str {
        "custom_executor"
    }
}

#[derive(Debug, Default)]
pub struct UnavailableExecutor;

impl ModuleExecutor for UnavailableExecutor {
    fn execution_origin(&self) -> &'static str {
        "unavailable_executor"
    }

    fn execute(&mut self, module_id: &str, _record: &DetectionRecord) -> ModuleRunResult {
        ModuleRunResult {
            state: ModuleResultState::Inconclusive,
            reason: Some(format!(
                "{module_id} 执行器尚未接入当前 CLI；保留配置和范围，不伪造测量结果"
            )),
            evidence_kind: "cli_execution_boundary".into(),
            evidence_summary: "当前版本仅完成 CLI 编排，真实模块执行由后续执行器接入".into(),
            evidence_payload: json!({"execution": "unavailable"}),
        }
    }
}

pub struct LiveExecutor {
    transport: ChatCompletionsTransport,
}

impl std::fmt::Debug for LiveExecutor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LiveExecutor")
            .field("transport", &self.transport)
            .finish()
    }
}

impl LiveExecutor {
    pub fn new(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
        timeout: std::time::Duration,
    ) -> Result<Self, String> {
        Ok(Self {
            transport: ChatCompletionsTransport::new(endpoint, model, api_key, timeout)?,
        })
    }
}

impl ModuleExecutor for LiveExecutor {
    fn execution_origin(&self) -> &'static str {
        "real_service"
    }

    fn execute(&mut self, module_id: &str, _record: &DetectionRecord) -> ModuleRunResult {
        let request = ChatCompletionsRequest {
            module_id: module_id.into(),
            prompt: probe_prompt(module_id).into(),
            max_tokens: 64,
            stream: false,
        };
        let response = self.transport.send(request.clone());
        let evidence_payload = self.transport.evidence_payload(&request, &response);
        let summary = format!(
            "真实 Chat Completions 响应：HTTP {:?}，耗时 {} ms",
            response.status, response.elapsed_ms
        );
        if let Some(error) = response.error {
            return ModuleRunResult {
                state: ModuleResultState::InvalidExecution,
                reason: Some(format!("{error}；保留为环境/传输异常，不归因模型")),
                evidence_kind: "real_service_transport_error".into(),
                evidence_summary: summary,
                evidence_payload,
            };
        }
        let status = response.status.unwrap_or_default();
        if matches!(status, 401 | 403) {
            return ModuleRunResult {
                state: ModuleResultState::InvalidExecution,
                reason: Some(format!("服务返回 HTTP {status}；认证或权限异常不归因模型")),
                evidence_kind: "real_service_auth_error".into(),
                evidence_summary: summary,
                evidence_payload,
            };
        }
        if status == 429 || status >= 500 {
            return ModuleRunResult {
                state: ModuleResultState::Inconclusive,
                reason: Some(format!(
                    "服务返回 HTTP {status}；限流或服务异常保留为 inconclusive"
                )),
                evidence_kind: "real_service_retryable_error".into(),
                evidence_summary: summary,
                evidence_payload,
            };
        }
        if !(200..300).contains(&status) {
            return ModuleRunResult {
                state: ModuleResultState::Inconclusive,
                reason: Some(format!(
                    "服务返回 HTTP {status}；未将未知错误包装为模块失败"
                )),
                evidence_kind: "real_service_http_error".into(),
                evidence_summary: summary,
                evidence_payload,
            };
        }
        if !is_chat_completion_shape(response.parsed.as_ref()) {
            return ModuleRunResult {
                state: ModuleResultState::Fail,
                reason: Some(
                    "HTTP 成功但响应缺少可识别的 Chat Completions choices/message 结构".into(),
                ),
                evidence_kind: "real_service_protocol_failure".into(),
                evidence_summary: summary,
                evidence_payload,
            };
        }
        let state = match module_id {
            "agent" => ModuleResultState::Inconclusive,
            "capability" | "performance" | "specification" => ModuleResultState::Inconclusive,
            _ => ModuleResultState::Pass,
        };
        let reason = match state {
            ModuleResultState::Inconclusive => Some(
                "已完成真实服务 HTTP 冒烟，但当前入口尚未覆盖该模块的完整固定样本；不伪造完整通过"
                    .into(),
            ),
            _ => Some("真实服务响应通过协议结构检查".into()),
        };
        ModuleRunResult {
            state,
            reason,
            evidence_kind: "real_service_response".into(),
            evidence_summary: summary,
            evidence_payload,
        }
    }
}

fn probe_prompt(module_id: &str) -> &'static str {
    match module_id {
        "ingress" => "请只回复：接入检查通过。",
        "specification" => "请只回复：规格冒烟检查通过。",
        "capability" => "请只回复：能力冒烟检查通过。",
        "performance" => "请只回复：性能冒烟检查通过。",
        "agent" => "请只回复：智能体冒烟检查通过。",
        "baseline" => "请只回复：基线冒烟检查通过。",
        _ => "请只回复：检查通过。",
    }
}

fn is_chat_completion_shape(value: Option<&serde_json::Value>) -> bool {
    value
        .and_then(|value| value.get("choices"))
        .and_then(serde_json::Value::as_array)
        .is_some_and(|choices| {
            choices.first().is_some_and(|choice| {
                choice.get("message").is_some() || choice.get("text").is_some()
            })
        })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CliRunReport {
    pub version: String,
    pub configuration: CliConfiguration,
    pub selected_modules: Vec<String>,
    pub unselected_modules: Vec<String>,
    pub record: DetectionRecord,
    pub overall: Option<OverallConclusion>,
    pub customer_conclusion: CustomerConclusionReport,
    pub execution_origin: String,
    pub limitations: Vec<String>,
}

impl CliRunRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.endpoint.trim().is_empty() {
            return Err("缺少服务 URL；请使用 --url 提供目标地址".into());
        }
        if self.model.trim().is_empty() {
            return Err("缺少模型名称；请使用 --model 提供配置值".into());
        }
        let selected = normalized_selection(self.selected_modules.clone())?;
        if let Some(stop_after) = &self.stop_after {
            if !selected.iter().any(|module| module == stop_after) {
                return Err(format!("--stop-after 的项目未被选中：{stop_after}"));
            }
        }
        Ok(())
    }
}

pub fn default_module_ids() -> Vec<String> {
    crate::records::MODULE_IDS
        .iter()
        .map(|module| (*module).into())
        .collect()
}

pub fn normalized_selection(selection: Option<Vec<String>>) -> Result<Vec<String>, String> {
    let modules = selection.unwrap_or_else(default_module_ids);
    if modules.is_empty() {
        return Err("至少选择一个检测项目，或省略选择以执行当前版本全部正式项目".into());
    }
    let mut seen = BTreeSet::new();
    for module in &modules {
        if !crate::records::MODULE_IDS.contains(&module.as_str()) {
            return Err(format!("未知检测项目：{module}"));
        }
        if !seen.insert(module.clone()) {
            return Err(format!("检测项目重复选择：{module}"));
        }
    }
    Ok(modules)
}

pub fn run_with_executor<E: ModuleExecutor>(
    request: CliRunRequest,
    executor: &mut E,
) -> Result<CliRunReport, String> {
    request.validate()?;
    let selected = normalized_selection(request.selected_modules.clone())?;
    let selected_set = selected.iter().cloned().collect::<BTreeSet<_>>();
    let unselected = default_module_ids()
        .into_iter()
        .filter(|module| !selected_set.contains(module))
        .collect::<Vec<_>>();
    let config = ConnectionConfig::new(request.endpoint.clone(), request.model.clone());
    let mut record = create_run(CreateRunInput {
        id: request.run_id.clone(),
        now: request.started_at.clone(),
        target: config_target(&config),
        selected_modules: Some(selected.clone()),
    });
    start_run(&mut record, &request.started_at)?;
    let stop_index = request
        .stop_after
        .as_deref()
        .and_then(|module| selected.iter().position(|item| item == module));
    let mut stopped = false;
    for (index, module_id) in selected.iter().enumerate() {
        if stop_index.is_some_and(|stop_index| index > stop_index) {
            break;
        }
        let result = executor.execute(module_id, &record);
        let evidence = add_evidence(
            &mut record,
            &format!("cli-{module_id}-{index}"),
            &result.evidence_kind,
            &request.started_at,
            json!({
                "module": module_id,
                "summary": result.evidence_summary,
                "origin": "cli_orchestration",
                "payload": result.evidence_payload,
            }),
        );
        let attempt_id = format!("attempt-{module_id}-{index}");
        add_attempt(
            &mut record,
            AttemptRecord {
                id: attempt_id.clone(),
                module_id: module_id.clone(),
                kind: AttemptKind::Initial,
                started_at: request.started_at.clone(),
                ended_at: Some(request.started_at.clone()),
                supersedes_attempt_id: None,
                evidence_refs: vec![evidence.id.clone()],
            },
        )?;
        add_event(
            &mut record,
            EventInput {
                id: format!("event-{module_id}-{index}"),
                kind: EventKind::System,
                occurred_at: request.started_at.clone(),
                summary: result
                    .reason
                    .clone()
                    .unwrap_or_else(|| format!("{module_id} 执行完成")),
                incident_id: None,
                attempt_id: Some(attempt_id.clone()),
                evidence_refs: vec![evidence.id.clone()],
            },
        )?;
        set_module_result(
            &mut record,
            ModuleResult {
                module_id: module_id.clone(),
                state: result.state,
                reason: result.reason,
                attempt_refs: vec![attempt_id],
                evidence_refs: vec![evidence.id],
                incident_refs: Vec::new(),
            },
            &request.started_at,
        )?;
        if stop_index == Some(index) {
            stop_run(
                &mut record,
                "CLI requested stop-after; remaining selected projects remain unverified",
                &request.started_at,
            )?;
            stopped = true;
            break;
        }
    }
    if !stopped {
        complete_run(&mut record, &request.started_at)?;
    }
    let overall = if stopped {
        OverallConclusion::Inconclusive
    } else {
        overall_for_record(&record)
    };
    let overall_evidence_refs = record
        .evidence
        .iter()
        .map(|evidence| evidence.id.clone())
        .collect();
    set_overall_conclusion(
        &mut record,
        overall,
        overall_evidence_refs,
        &request.started_at,
    )?;
    let customer_conclusion = build_default_customer_report(&record)?;
    let execution_origin = executor.execution_origin();
    let execution_limitation = if execution_origin == "real_service" {
        "当前真实执行器已完成 Chat Completions 冒烟请求；尚未覆盖所有模块的完整固定样本时，模块保持 inconclusive。"
    } else {
        "未接入真实执行器的模块保持 inconclusive，不伪造测量结果。"
    };
    Ok(CliRunReport {
        version: CLI_VERSION.into(),
        configuration: CliConfiguration {
            redacted_endpoint: redact_endpoint(&request.endpoint),
            model: request.model,
            api_key_provided: request.api_key.is_some(),
        },
        selected_modules: selected,
        unselected_modules: unselected,
        record,
        overall: Some(overall),
        customer_conclusion,
        execution_origin: execution_origin.into(),
        limitations: vec![
            "CLI 保存的是实际选择、状态和证据入口；模块状态只由执行器返回的事实决定。".into(),
            execution_limitation.into(),
            "停止运行只保留停止前事实，剩余项目保持 unverified，不包装成完成全测。".into(),
            "API 密钥只用于配置存在性标记，不写入记录或输出。".into(),
        ],
    })
}

pub fn render_text(report: &CliRunReport) -> String {
    let mut lines = vec![
        format!("模型：{}", report.configuration.model),
        format!("地址：{}", report.configuration.redacted_endpoint),
        format!("选择：{}", report.selected_modules.join(", ")),
        format!("未选：{}", report.unselected_modules.join(", ")),
        format!("运行：{}", lifecycle_label(report.record.lifecycle)),
        format!("结论：{}", report.customer_conclusion.text),
    ];
    lines.extend(report.record.module_results.iter().map(|result| {
        format!(
            "项目 {}：{}{}",
            result.module_id,
            module_result_label(result.state),
            result
                .reason
                .as_deref()
                .map_or(String::new(), |reason| format!("（{reason}）"))
        )
    }));
    lines.join("\n")
}

pub fn render_report(
    report: &CliRunReport,
    format: OutputFormat,
) -> Result<String, serde_json::Error> {
    match format {
        OutputFormat::Text => Ok(render_text(report)),
        OutputFormat::Json => serde_json::to_string_pretty(report),
    }
}

pub fn write_report(path: impl AsRef<Path>, content: &str) -> Result<(), String> {
    fs::write(path, content).map_err(|error| format!("写入 CLI 报告失败：{error}"))
}

pub fn generated_run_id() -> String {
    format!("run-{}", unix_timestamp())
}

pub fn generated_timestamp() -> String {
    format!("unix:{}", unix_timestamp())
}

fn config_target(config: &ConnectionConfig) -> crate::records::ServiceSnapshotInput {
    crate::records::ServiceSnapshotInput {
        endpoint_fingerprint: redact_endpoint(&config.endpoint),
        model: config.model.clone(),
        protocol: config.protocol.clone(),
        auth_mode: config.auth_mode.clone(),
        client_version: config.client_version.clone(),
        environment: config.environment.clone(),
    }
}

fn overall_for_record(record: &DetectionRecord) -> OverallConclusion {
    if record
        .module_results
        .iter()
        .any(|result| result.state == ModuleResultState::Fail)
    {
        OverallConclusion::Limited
    } else if record.plan.iter().any(|entry| {
        entry.state == crate::records::ModuleSelectionState::Selected
            && record
                .module_results
                .iter()
                .find(|result| result.module_id == entry.module_id)
                .is_some_and(|result| {
                    matches!(
                        result.state,
                        ModuleResultState::Inconclusive
                            | ModuleResultState::InvalidExecution
                            | ModuleResultState::Unverified
                    )
                })
    }) {
        OverallConclusion::Inconclusive
    } else {
        OverallConclusion::Usable
    }
}

fn lifecycle_label(state: crate::records::LifecycleState) -> &'static str {
    match state {
        crate::records::LifecycleState::Planned => "planned",
        crate::records::LifecycleState::Running => "running",
        crate::records::LifecycleState::Stopping => "stopping",
        crate::records::LifecycleState::Stopped => "stopped",
        crate::records::LifecycleState::Completed => "completed",
    }
}

fn module_result_label(state: ModuleResultState) -> &'static str {
    match state {
        ModuleResultState::Pass => "pass",
        ModuleResultState::Fail => "fail",
        ModuleResultState::Unsupported => "unsupported",
        ModuleResultState::Inconclusive => "inconclusive",
        ModuleResultState::InvalidExecution => "invalid_execution",
        ModuleResultState::NotApplicable => "not_applicable",
        ModuleResultState::NotSelected => "not_selected",
        ModuleResultState::Unverified => "unverified",
    }
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after Unix epoch")
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::{
        CreateRunInput, LifecycleState, ModuleSelectionState, ServiceSnapshotInput, create_run,
    };
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[derive(Debug)]
    struct TestExecutor {
        calls: Vec<String>,
        state: ModuleResultState,
    }

    impl ModuleExecutor for TestExecutor {
        fn execute(&mut self, module_id: &str, _record: &DetectionRecord) -> ModuleRunResult {
            self.calls.push(module_id.into());
            ModuleRunResult {
                state: self.state,
                reason: None,
                evidence_kind: "test_module".into(),
                evidence_summary: "controlled test result".into(),
                evidence_payload: json!({"fixture": true}),
            }
        }
    }

    fn request(selection: Option<Vec<String>>) -> CliRunRequest {
        CliRunRequest {
            endpoint: "https://api.example.test/v1/chat".into(),
            model: "model-a".into(),
            api_key: Some("secret".into()),
            selected_modules: selection,
            stop_after: None,
            run_id: "run-cli".into(),
            started_at: "2026-09-11T00:00:00Z".into(),
        }
    }

    fn executor_record() -> DetectionRecord {
        create_run(CreateRunInput {
            id: "run-live-executor".into(),
            now: "2026-09-11T00:00:00Z".into(),
            target: ServiceSnapshotInput {
                endpoint_fingerprint: "127.0.0.1".into(),
                model: "model-a".into(),
                protocol: "chat-completions".into(),
                auth_mode: "bearer".into(),
                client_version: "test".into(),
                environment: BTreeMap::new(),
            },
            selected_modules: None,
        })
    }

    fn mock_server(
        response_status: u16,
        response_body: &'static str,
        connections: usize,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let handle = thread::spawn(move || {
            for _ in 0..connections {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 16 * 1024];
                let size = stream.read(&mut request).unwrap_or(0);
                captured
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&request[..size]).into_owned());
                let response = format!(
                    "HTTP/1.1 {response_status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                    response_body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests.lock().unwrap().clone()
        });
        (format!("http://{address}/v1/chat/completions"), handle)
    }

    #[test]
    fn defaults_to_all_formal_modules_and_rejects_unknown_or_duplicate_selection() {
        assert_eq!(normalized_selection(None).unwrap(), default_module_ids());
        assert!(normalized_selection(Some(vec!["unknown".into()])).is_err());
        assert!(normalized_selection(Some(vec!["agent".into(), "agent".into()])).is_err());
    }

    #[test]
    fn live_executor_runs_selected_modules_and_links_real_evidence() {
        let (endpoint, server) = mock_server(
            200,
            r#"{"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}]}"#,
            2,
        );
        let mut executor = LiveExecutor::new(
            endpoint,
            "model-a",
            "secret-value",
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        let report = run_with_executor(
            CliRunRequest {
                selected_modules: Some(vec!["ingress".into(), "baseline".into()]),
                ..request(None)
            },
            &mut executor,
        )
        .unwrap();
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|request| {
            request.contains("authorization: Bearer secret-value")
                || request.contains("Authorization: Bearer secret-value")
        }));
        assert_eq!(report.execution_origin, "real_service");
        assert_eq!(report.record.evidence.len(), 2);
        assert!(
            report
                .record
                .evidence
                .iter()
                .all(|evidence| evidence.payload.to_string().contains("transport_version"))
        );
        assert!(
            !serde_json::to_string(&report)
                .unwrap()
                .contains("secret-value")
        );
    }

    #[test]
    fn live_executor_keeps_auth_and_rate_limit_as_non_model_states() {
        let (auth_endpoint, auth_server) = mock_server(401, r#"{"error":"unauthorized"}"#, 1);
        let mut auth_executor = LiveExecutor::new(
            auth_endpoint,
            "model-a",
            "secret-value",
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        let auth_result = auth_executor.execute("ingress", &executor_record());
        auth_server.join().unwrap();
        assert_eq!(auth_result.state, ModuleResultState::InvalidExecution);
        assert!(auth_result.reason.unwrap().contains("不归因模型"));

        let (rate_endpoint, rate_server) = mock_server(429, r#"{"error":"slow"}"#, 3);
        let mut rate_executor = LiveExecutor::new(
            rate_endpoint,
            "model-a",
            "secret-value",
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        let rate_result = rate_executor.execute("performance", &executor_record());
        rate_server.join().unwrap();
        assert_eq!(rate_result.state, ModuleResultState::Inconclusive);
    }

    #[test]
    fn live_executor_marks_invalid_json_as_protocol_failure() {
        let (endpoint, server) = mock_server(200, "not-json", 1);
        let mut executor = LiveExecutor::new(
            endpoint,
            "model-a",
            "secret-value",
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        let result = executor.execute("specification", &executor_record());
        server.join().unwrap();
        assert_eq!(result.state, ModuleResultState::Fail);
        assert!(result.reason.unwrap().contains("响应缺少"));
    }

    #[test]
    fn selected_modules_are_saved_and_unselected_modules_are_explicit() {
        let mut executor = TestExecutor {
            calls: Vec::new(),
            state: ModuleResultState::Pass,
        };
        let report = run_with_executor(
            request(Some(vec!["capability".into(), "performance".into()])),
            &mut executor,
        )
        .unwrap();
        assert_eq!(executor.calls, vec!["capability", "performance"]);
        assert_eq!(report.selected_modules, vec!["capability", "performance"]);
        assert!(
            report
                .record
                .plan
                .iter()
                .any(|entry| entry.module_id == "agent"
                    && entry.state == ModuleSelectionState::NotSelected)
        );
        assert_eq!(report.record.lifecycle, LifecycleState::Completed);
        assert_eq!(report.overall, Some(OverallConclusion::Usable));
    }

    #[test]
    fn default_unavailable_executor_keeps_full_run_inconclusive_without_fake_pass() {
        let mut executor = UnavailableExecutor;
        let report = run_with_executor(request(None), &mut executor).unwrap();
        assert_eq!(report.record.lifecycle, LifecycleState::Completed);
        assert_eq!(report.overall, Some(OverallConclusion::Inconclusive));
        assert!(
            report
                .record
                .module_results
                .iter()
                .filter(|result| result.state == ModuleResultState::Inconclusive)
                .count()
                == crate::records::MODULE_IDS.len()
        );
        let text = render_text(&report);
        assert!(text.contains("inconclusive"));
        assert!(!text.contains("secret"));
    }

    #[test]
    fn stop_after_preserves_pre_stop_facts_and_leaves_remaining_projects_unverified() {
        let mut executor = TestExecutor {
            calls: Vec::new(),
            state: ModuleResultState::Pass,
        };
        let mut cli_request = request(None);
        cli_request.stop_after = Some("capability".into());
        let report = run_with_executor(cli_request, &mut executor).unwrap();
        assert_eq!(report.record.lifecycle, LifecycleState::Stopped);
        assert_eq!(
            executor.calls,
            vec!["ingress", "specification", "capability"]
        );
        assert!(
            report
                .record
                .module_results
                .iter()
                .any(|result| result.module_id == "performance"
                    && result.state == ModuleResultState::Unverified)
        );
        assert_eq!(report.overall, Some(OverallConclusion::Inconclusive));
    }

    #[test]
    fn validates_configuration_and_supports_json_output_file() {
        let mut executor = TestExecutor {
            calls: Vec::new(),
            state: ModuleResultState::NotApplicable,
        };
        let report =
            run_with_executor(request(Some(vec!["baseline".into()])), &mut executor).unwrap();
        let json = render_report(&report, OutputFormat::Json).unwrap();
        assert!(json.contains("selected_modules"));
        assert!(json.contains("api_key_provided"));
        assert!(!json.contains("secret"));
        let mut invalid = request(Some(vec!["baseline".into()]));
        invalid.model.clear();
        assert!(run_with_executor(invalid, &mut executor).is_err());
    }
}
