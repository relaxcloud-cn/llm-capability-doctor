use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::agent::{
    AgentEvent, AgentEventKind, AgentExecutionInput, AgentOutcome, AgentRunOutcome, AgentRuntime,
    AgentScenarioSpec, ExecutionOrigin, PermissionDecision, PermissionEffect, PermissionEvent,
    assess_execution, build_report_for_record, fixed_agent_scenarios, judge_scenario,
    parse_agent_session,
};
use crate::baseline::{
    ActualSnapshot, ActualSource, BaselineScenario,
    build_report_for_record as build_baseline_report, fixed_baseline_catalog,
};
use crate::capability::{
    CapabilityResponse, CapabilitySettings, ExecutionState, ToolCall, build_scorecard,
    fixed_capability_catalog,
};
use crate::conclusion::{CustomerConclusionReport, build_default_customer_report};
use crate::context_probe;
use crate::ingress::{ConnectionConfig, redact_endpoint};
use crate::messages_probe;
use crate::performance::{
    ErrorKind, PerformanceConditions, PerformanceSample, ProtectionAction, ProtectionState,
    ResponseMode as PerformanceResponseMode, RunPhase, TerminalState, Timeline, TokenCountSource,
    build_report_for_record as build_performance_report_for_record, fixed_performance_plan,
};
use crate::records::{
    AttemptKind, EventInput, EventKind, ModuleResult, ModuleResultState, OverallConclusion,
    add_attempt, add_event, add_evidence, complete_run, create_run, set_module_result,
    set_overall_conclusion, start_run, stop_run,
};
use crate::records::{AttemptRecord, CreateRunInput, DetectionRecord};
use crate::specification::{
    AttemptKind as SpecificationAttemptKind, EvidenceOrigin, SpecCategory, SpecStatus,
    SpecificationObservation, SpecificationPlan, build_report, specification_plan,
};
use crate::stream_probe;
use crate::structured_probe;
use crate::tool_probe;
use crate::transport::{
    ChatCompletionsRequest, ChatCompletionsResponse, ChatCompletionsTransport, StreamResponse,
};

pub const CLI_VERSION: &str = "cli/v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    Text,
    Json,
}

/// 报告生成模式：custom 只把检测结论填进模板，不调用 AI 或 OhMyPi；
/// dynamic 额外用 OhMyPi+模型对模块证据做语义分析后填进同一模板。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum ReportMode {
    Custom,
    Dynamic,
}

impl ReportMode {
    /// CLI/GUI 参数透传用的字符串值，与 ValueEnum 的取值一致。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Custom => "custom",
            Self::Dynamic => "dynamic",
        }
    }
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

    fn execute_with_record(
        &mut self,
        module_id: &str,
        record: &mut DetectionRecord,
    ) -> ModuleRunResult {
        self.execute(module_id, record)
    }

    fn execute_with_record_progress(
        &mut self,
        module_id: &str,
        record: &mut DetectionRecord,
        _progress: &mut dyn FnMut(ProgressDetail),
    ) -> ModuleRunResult {
        self.execute_with_record(module_id, record)
    }

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

/// 单个性能请求的硬超时；挂死的请求记为超时样本而非拖垮整体检测。
const PERF_REQUEST_TIMEOUT_MS: u64 = 120_000;

pub struct LiveExecutor {
    transport: ChatCompletionsTransport,
    full: bool,
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
            full: false,
        })
    }

    pub fn new_full(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
        timeout: std::time::Duration,
    ) -> Result<Self, String> {
        let mut executor = Self::new(endpoint, model, api_key, timeout)?;
        executor.full = true;
        Ok(executor)
    }
}

impl ModuleExecutor for LiveExecutor {
    fn execution_origin(&self) -> &'static str {
        "real_service"
    }

    fn execute(&mut self, module_id: &str, record: &DetectionRecord) -> ModuleRunResult {
        if self.full {
            return self.execute_full(module_id, record);
        }
        self.execute_smoke(module_id)
    }

    fn execute_with_record(
        &mut self,
        module_id: &str,
        record: &mut DetectionRecord,
    ) -> ModuleRunResult {
        if self.full {
            if module_id == "agent" {
                return self.execute_agent(record);
            }
            if module_id == "performance" {
                return self.execute_performance(record);
            }
        }
        self.execute(module_id, record)
    }

    fn execute_with_record_progress(
        &mut self,
        module_id: &str,
        record: &mut DetectionRecord,
        progress: &mut dyn FnMut(ProgressDetail),
    ) -> ModuleRunResult {
        if !self.full {
            return self.execute_with_record(module_id, record);
        }
        match module_id {
            "specification" => self.execute_specification_with_progress(record, progress),
            "capability" => self.execute_capability_with_progress(record, progress),
            "performance" => self.execute_performance_with_progress(record, progress),
            "agent" => self.execute_agent_with_progress(record, progress),
            "baseline" => self.execute_baseline_with_progress(record, progress),
            _ => self.execute_with_record(module_id, record),
        }
    }
}

impl LiveExecutor {
    fn execute_smoke(&mut self, module_id: &str) -> ModuleRunResult {
        let request = ChatCompletionsRequest {
            module_id: module_id.into(),
            messages: None,
            tools: None,
            prompt: probe_prompt(module_id).into(),
            max_tokens: 64,
            stream: false,
            allow_retry: true,
            timeout_ms: None,
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

    fn execute_full(&mut self, module_id: &str, record: &DetectionRecord) -> ModuleRunResult {
        if module_id == "agent" {
            let mut owned_record = record.clone();
            return self.execute_agent(&mut owned_record);
        }
        if module_id == "performance" {
            let mut owned_record = record.clone();
            return self.execute_performance(&mut owned_record);
        }
        match module_id {
            "specification" => self.execute_specification(record),
            "capability" => self.execute_capability(record),
            "performance" => {
                let mut owned_record = record.clone();
                self.execute_performance(&mut owned_record)
            }
            "baseline" => self.execute_baseline(record),
            _ => self.execute_smoke(module_id),
        }
    }

    fn execute_specification(&mut self, record: &DetectionRecord) -> ModuleRunResult {
        let mut noop = |_detail: ProgressDetail| {};
        self.execute_specification_with_progress(record, &mut noop)
    }

    fn execute_specification_with_progress(
        &mut self,
        record: &DetectionRecord,
        progress: &mut dyn FnMut(ProgressDetail),
    ) -> ModuleRunResult {
        let plans = specification_plan();
        let total_samples: usize = plans.iter().map(|plan| plan.samples.len()).sum();
        let mut observations = Vec::new();
        let mut evidence = Vec::new();
        for plan in plans {
            if plan.category == SpecCategory::ContextCapacity {
                self.execute_context_probes(
                    &plan,
                    &mut observations,
                    &mut evidence,
                    total_samples,
                    progress,
                );
                continue;
            }
            if plan.category == SpecCategory::ToolCalls {
                self.execute_tool_call_probes(
                    &plan,
                    &mut observations,
                    &mut evidence,
                    total_samples,
                    progress,
                );
                continue;
            }
            if plan.category == SpecCategory::StructuredOutput {
                self.execute_structured_probes(
                    &plan,
                    &mut observations,
                    &mut evidence,
                    total_samples,
                    progress,
                );
                continue;
            }
            if plan.category == SpecCategory::MessagesAndTurns {
                self.execute_messages_probes(
                    &plan,
                    &mut observations,
                    &mut evidence,
                    total_samples,
                    progress,
                );
                continue;
            }
            if plan.category == SpecCategory::Streaming {
                self.execute_stream_probes(
                    &plan,
                    &mut observations,
                    &mut evidence,
                    total_samples,
                    progress,
                );
                continue;
            }
            for sample_id in plan.samples {
                progress(ProgressDetail {
                    index: observations.len(),
                    total: total_samples,
                    id: plan.category.id().into(),
                    message: format!(
                        "正在检测规格样本 {} / {}",
                        observations.len() + 1,
                        total_samples
                    ),
                });
                let request = ChatCompletionsRequest {
                    module_id: "specification".into(),
                    messages: None,
                    tools: None,
                    prompt: format!(
                        "规格样本 {sample_id}。{}",
                        plan.fixed_settings
                            .values()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("，")
                    ),
                    max_tokens: 256,
                    stream: plan.category.id() == "S07",
                    allow_retry: true,
                    timeout_ms: None,
                };
                let (response, stream) = if request.stream {
                    let stream = self.transport.send_stream(request.clone());
                    (stream.response.clone(), Some(stream))
                } else {
                    (self.transport.send(request.clone()), None)
                };
                let payload = stream.as_ref().map_or_else(
                    || self.transport.evidence_payload(&request, &response),
                    |stream| self.transport.stream_evidence_payload(&request, stream),
                );
                evidence.push(json!({"sample_id": sample_id, "payload": payload}));
                let (status, limitation) = if let Some(stream) = stream.as_ref() {
                    if response.error.is_some() {
                        (
                            SpecStatus::Inconclusive,
                            Some("流式传输失败，未归因模型".into()),
                        )
                    } else if stream.parse_errors.is_empty()
                        && stream.terminated
                        && is_chat_completion_shape(response.parsed.as_ref())
                    {
                        (
                            SpecStatus::Accepted,
                            Some("已完成该固定样本的真实流式请求；已记录事件和终止状态".into()),
                        )
                    } else if !stream.parse_errors.is_empty() {
                        (
                            SpecStatus::Failed,
                            Some(format!(
                                "客户端流式解析失败：{}",
                                stream.parse_errors.join("；")
                            )),
                        )
                    } else if !stream.terminated {
                        (SpecStatus::Failed, Some("服务未正常结束流式响应".into()))
                    } else {
                        (
                            SpecStatus::Failed,
                            Some("流式响应未形成可识别的对话结果".into()),
                        )
                    }
                } else if response.error.is_some() {
                    (
                        SpecStatus::Inconclusive,
                        Some("传输失败，未归因模型".into()),
                    )
                } else if response
                    .status
                    .is_some_and(|status| !(200..300).contains(&status))
                {
                    (
                        SpecStatus::Inconclusive,
                        Some("HTTP 非成功响应，未归因模型".into()),
                    )
                } else if is_chat_completion_shape(response.parsed.as_ref()) {
                    (
                        SpecStatus::Accepted,
                        Some("已完成该固定样本的真实请求；未据此宣称边界或长期稳定性".into()),
                    )
                } else {
                    (
                        SpecStatus::Failed,
                        Some("HTTP 成功但响应结构不可识别".into()),
                    )
                };
                observations.push(SpecificationObservation {
                    category: plan.category,
                    sample_id,
                    attempt: SpecificationAttemptKind::Initial,
                    conditions: plan.fixed_settings.clone(),
                    actual_output: response.parsed,
                    finish_reason: Some("observed".into()),
                    natural_end: status == SpecStatus::Accepted,
                    status,
                    evidence_refs: vec!["module-evidence".into()],
                    evidence_origin: EvidenceOrigin::RealExecution,
                    limitation,
                    verified_scope: None,
                });
                progress(ProgressDetail {
                    index: observations.len(),
                    total: total_samples,
                    id: plan.category.id().into(),
                    message: format!("已完成规格样本 {} / {}", observations.len(), total_samples),
                });
            }
        }
        let executed_samples = observations.len();
        let report = build_report(record.id.as_str(), observations).ok();
        let passed = report.as_ref().is_some_and(|report| {
            report
                .rows
                .iter()
                .all(|row| row.result != SpecStatus::Failed)
        });
        ModuleRunResult {
            state: if passed {
                ModuleResultState::Pass
            } else {
                ModuleResultState::Fail
            },
            reason: Some(format!(
                "已执行 {} 个规格固定样本；边界声明仍受观察限制",
                evidence.len()
            )),
            evidence_kind: "real_specification_report".into(),
            evidence_summary: format!(
                "规格固定样本 {} 个，报告 {}",
                evidence.len(),
                if report.is_some() {
                    "已生成"
                } else {
                    "生成失败"
                }
            ),
            evidence_payload: json!({"version": crate::specification::SPECIFICATION_VERSION, "planned_samples": total_samples, "executed_samples": executed_samples, "report": report, "evidence": evidence}),
        }
    }

    /// S01 上下文容量实测：先校准字符/token 密度，再对 64K/128K/256K/512K
    /// 四个固定档位各发送一次真实长度输入，按服务端 usage 判定各档是否通过。
    fn execute_context_probes(
        &mut self,
        plan: &SpecificationPlan,
        observations: &mut Vec<SpecificationObservation>,
        evidence: &mut Vec<Value>,
        total_samples: usize,
        progress: &mut dyn FnMut(ProgressDetail),
    ) {
        let calibration_prompt = context_probe::calibration_prompt();
        let calibration_chars = calibration_prompt.len();
        let calibration_request = ChatCompletionsRequest {
            module_id: "specification".into(),
            messages: None,
            tools: None,
            prompt: calibration_prompt,
            max_tokens: 64,
            stream: false,
            allow_retry: true,
            timeout_ms: None,
        };
        let calibration_response = self.transport.send(calibration_request.clone());
        evidence.push(json!({
            "sample_id": "context-calibration",
            "payload": self.transport.evidence_payload(&calibration_request, &calibration_response),
        }));
        let density = context_probe::extract_prompt_tokens(calibration_response.parsed.as_ref())
            .map(|tokens| context_probe::chars_per_token(calibration_chars, tokens))
            .unwrap_or(context_probe::FALLBACK_CHARS_PER_TOKEN);

        for sample_id in &plan.samples {
            let Some(target) = context_probe::tier_tokens(sample_id) else {
                continue;
            };
            progress(ProgressDetail {
                index: observations.len(),
                total: total_samples,
                id: plan.category.id().into(),
                message: format!(
                    "正在检测规格样本 {} / {}",
                    observations.len() + 1,
                    total_samples
                ),
            });
            let prompt = context_probe::build_probe_prompt(target, density);
            let estimated_tokens = (prompt.len() as f64 / density) as u64;
            let request = ChatCompletionsRequest {
                module_id: "specification".into(),
                messages: None,
                tools: None,
                prompt,
                max_tokens: 256,
                stream: false,
                allow_retry: true,
                timeout_ms: None,
            };
            let response = self.transport.send(request.clone());
            let finish_reason = response_finish_reason(response.parsed.as_ref());
            let outcome = context_probe::classify(&response, target, estimated_tokens);
            let mut payload = self.transport.evidence_payload(&request, &response);
            if let Some(request_field) = payload.get_mut("request") {
                let head: String = request.prompt.chars().take(64).collect();
                let tail: String = request
                    .prompt
                    .chars()
                    .rev()
                    .take(64)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                request_field["prompt"] = json!(format!(
                    "{head}…[填充文本共 {} 字符]…{tail}",
                    request.prompt.len()
                ));
            }
            evidence.push(json!({
                "sample_id": sample_id,
                "payload": payload,
            }));
            let (status, limitation, verified_scope, count_source) = match &outcome {
                context_probe::ProbeOutcome::Passed {
                    measured_tokens,
                    estimated,
                } => (
                    SpecStatus::VerifiedRange,
                    Some(if *estimated {
                        "响应缺少 usage，token 数按校准密度估算".to_string()
                    } else {
                        "仅验证到该实测值，不代表服务理论上限".to_string()
                    }),
                    Some(format!("input <= {measured_tokens} tokens")),
                    if *estimated { "estimate" } else { "usage" },
                ),
                context_probe::ProbeOutcome::Rejected { detail } => (
                    SpecStatus::Failed,
                    Some(format!("{detail}（可归因容量拒绝，构成上界证据）")),
                    None,
                    "usage",
                ),
                context_probe::ProbeOutcome::Inconclusive { detail } => (
                    SpecStatus::Inconclusive,
                    Some(detail.clone()),
                    None,
                    "usage",
                ),
                context_probe::ProbeOutcome::Malformed { detail } => {
                    (SpecStatus::Failed, Some(detail.clone()), None, "usage")
                }
            };
            let mut conditions = plan.fixed_settings.clone();
            conditions.insert("target_tokens".into(), target.to_string());
            conditions.insert("count_source".into(), count_source.into());
            observations.push(SpecificationObservation {
                category: plan.category,
                sample_id: sample_id.clone(),
                attempt: SpecificationAttemptKind::Initial,
                conditions,
                actual_output: response.parsed,
                natural_end: finish_reason.as_deref() == Some("stop"),
                finish_reason,
                status,
                evidence_refs: vec!["module-evidence".into()],
                evidence_origin: EvidenceOrigin::RealExecution,
                limitation,
                verified_scope,
            });
            progress(ProgressDetail {
                index: observations.len(),
                total: total_samples,
                id: plan.category.id().into(),
                message: format!("已完成规格样本 {} / {}", observations.len(), total_samples),
            });
        }
    }

    /// S04 工具调用实测：5 个固定样本各发 1 次带 tools/tool_choice 的真实
    /// 请求，校验调用形态与参数是否符合 schema 和样例规则。
    fn execute_tool_call_probes(
        &mut self,
        plan: &SpecificationPlan,
        observations: &mut Vec<SpecificationObservation>,
        evidence: &mut Vec<Value>,
        total_samples: usize,
        progress: &mut dyn FnMut(ProgressDetail),
    ) {
        let samples = tool_probe::samples();
        for sample_id in &plan.samples {
            let Some(sample) = samples.iter().find(|sample| sample.id == sample_id) else {
                continue;
            };
            progress(ProgressDetail {
                index: observations.len(),
                total: total_samples,
                id: plan.category.id().into(),
                message: format!(
                    "正在检测规格样本 {} / {}",
                    observations.len() + 1,
                    total_samples
                ),
            });
            let payload = tool_probe::request_payload(sample, self.transport.model_name());
            let response = self
                .transport
                .send_payload(payload.clone(), "specification");
            let finish_reason = response_finish_reason(response.parsed.as_ref());
            let outcome = tool_probe::classify(sample, &response);
            evidence.push(json!({
                "sample_id": sample_id,
                "payload": self.transport.raw_evidence_payload(&payload, "specification", &response),
            }));
            let (status, limitation, verified_scope) = match &outcome {
                tool_probe::ToolOutcome::Pass { detail } => (
                    SpecStatus::Effective,
                    Some(format!("{detail}；仅验证该样例形态，不代表任意工具场景")),
                    None,
                ),
                tool_probe::ToolOutcome::Violation { detail } => {
                    (SpecStatus::Failed, Some(detail.clone()), None)
                }
                tool_probe::ToolOutcome::Unsupported { detail } => {
                    (SpecStatus::Unsupported, Some(detail.clone()), None)
                }
                tool_probe::ToolOutcome::Malformed { detail } => {
                    (SpecStatus::Failed, Some(detail.clone()), None)
                }
                tool_probe::ToolOutcome::Inconclusive { detail } => {
                    (SpecStatus::Inconclusive, Some(detail.clone()), None)
                }
            };
            let mut conditions = plan.fixed_settings.clone();
            conditions.insert("tool_choice".into(), sample.tool_choice.to_string());
            observations.push(SpecificationObservation {
                category: plan.category,
                sample_id: sample_id.clone(),
                attempt: SpecificationAttemptKind::Initial,
                conditions,
                actual_output: response.parsed,
                natural_end: finish_reason.as_deref() == Some("stop"),
                finish_reason,
                status,
                evidence_refs: vec!["module-evidence".into()],
                evidence_origin: EvidenceOrigin::RealExecution,
                limitation,
                verified_scope,
            });
            progress(ProgressDetail {
                index: observations.len(),
                total: total_samples,
                id: plan.category.id().into(),
                message: format!("已完成规格样本 {} / {}", observations.len(), total_samples),
            });
        }
    }

    /// S05 结构化输出实测：同一短输入 × 三模式各 1 次（对照 / json_object /
    /// json_schema），只解析 message.content 原文，禁止修补后判过。
    fn execute_structured_probes(
        &mut self,
        plan: &SpecificationPlan,
        observations: &mut Vec<SpecificationObservation>,
        evidence: &mut Vec<Value>,
        total_samples: usize,
        progress: &mut dyn FnMut(ProgressDetail),
    ) {
        let samples = structured_probe::samples();
        for sample_id in &plan.samples {
            let Some(sample) = samples.iter().find(|sample| sample.id == sample_id) else {
                continue;
            };
            progress(ProgressDetail {
                index: observations.len(),
                total: total_samples,
                id: plan.category.id().into(),
                message: format!(
                    "正在检测规格样本 {} / {}",
                    observations.len() + 1,
                    total_samples
                ),
            });
            let payload = structured_probe::request_payload(sample, self.transport.model_name());
            let response = self
                .transport
                .send_payload(payload.clone(), "specification");
            let finish_reason = response_finish_reason(response.parsed.as_ref());
            let outcome = structured_probe::classify(sample, &response);
            evidence.push(json!({
                "sample_id": sample_id,
                "payload": self.transport.raw_evidence_payload(&payload, "specification", &response),
            }));
            let (status, limitation, verified_scope) = match &outcome {
                structured_probe::StructuredOutcome::Pass { detail } => (
                    SpecStatus::Effective,
                    Some(format!(
                        "{detail}；仅覆盖该冻结 schema 子集，不代表任意 schema"
                    )),
                    None,
                ),
                structured_probe::StructuredOutcome::Violation { detail } => {
                    (SpecStatus::Failed, Some(detail.clone()), None)
                }
                structured_probe::StructuredOutcome::Unsupported { detail } => {
                    (SpecStatus::Unsupported, Some(detail.clone()), None)
                }
                structured_probe::StructuredOutcome::Malformed { detail } => {
                    (SpecStatus::Failed, Some(detail.clone()), None)
                }
                structured_probe::StructuredOutcome::Inconclusive { detail } => {
                    (SpecStatus::Inconclusive, Some(detail.clone()), None)
                }
            };
            let mut conditions = plan.fixed_settings.clone();
            conditions.insert(
                "response_format".into(),
                sample
                    .response_format
                    .as_ref()
                    .map(Value::to_string)
                    .unwrap_or_else(|| "未设置".into()),
            );
            observations.push(SpecificationObservation {
                category: plan.category,
                sample_id: sample_id.clone(),
                attempt: SpecificationAttemptKind::Initial,
                conditions,
                actual_output: response.parsed,
                natural_end: finish_reason.as_deref() == Some("stop"),
                finish_reason,
                status,
                evidence_refs: vec!["module-evidence".into()],
                evidence_origin: EvidenceOrigin::RealExecution,
                limitation,
                verified_scope,
            });
            progress(ProgressDetail {
                index: observations.len(),
                total: total_samples,
                id: plan.category.id().into(),
                message: format!("已完成规格样本 {} / {}", observations.len(), total_samples),
            });
        }
    }

    /// S06 消息与多轮输入实测：4 个固定消息序列各 1 次，验证角色接受与
    /// 同请求历史标记回引（不跨请求构造记忆）。
    fn execute_messages_probes(
        &mut self,
        plan: &SpecificationPlan,
        observations: &mut Vec<SpecificationObservation>,
        evidence: &mut Vec<Value>,
        total_samples: usize,
        progress: &mut dyn FnMut(ProgressDetail),
    ) {
        let samples = messages_probe::samples();
        for sample_id in &plan.samples {
            let Some(sample) = samples.iter().find(|sample| sample.id == sample_id) else {
                continue;
            };
            progress(ProgressDetail {
                index: observations.len(),
                total: total_samples,
                id: plan.category.id().into(),
                message: format!(
                    "正在检测规格样本 {} / {}",
                    observations.len() + 1,
                    total_samples
                ),
            });
            let payload = messages_probe::request_payload(sample, self.transport.model_name());
            let response = self
                .transport
                .send_payload(payload.clone(), "specification");
            let finish_reason = response_finish_reason(response.parsed.as_ref());
            let outcome = messages_probe::classify(sample, &response);
            evidence.push(json!({
                "sample_id": sample_id,
                "payload": self.transport.raw_evidence_payload(&payload, "specification", &response),
            }));
            let (status, limitation, verified_scope) = match &outcome {
                messages_probe::MessagesOutcome::Pass { detail } => (
                    SpecStatus::Effective,
                    Some(format!("{detail}；仅覆盖已测角色组合与同请求历史模式")),
                    None,
                ),
                messages_probe::MessagesOutcome::Violation { detail } => {
                    (SpecStatus::Failed, Some(detail.clone()), None)
                }
                messages_probe::MessagesOutcome::NotApplicable { detail } => {
                    (SpecStatus::NotApplicable, Some(detail.clone()), None)
                }
                messages_probe::MessagesOutcome::Malformed { detail } => {
                    (SpecStatus::Failed, Some(detail.clone()), None)
                }
                messages_probe::MessagesOutcome::Inconclusive { detail } => {
                    (SpecStatus::Inconclusive, Some(detail.clone()), None)
                }
            };
            observations.push(SpecificationObservation {
                category: plan.category,
                sample_id: sample_id.clone(),
                attempt: SpecificationAttemptKind::Initial,
                conditions: plan.fixed_settings.clone(),
                actual_output: response.parsed,
                natural_end: finish_reason.as_deref() == Some("stop"),
                finish_reason,
                status,
                evidence_refs: vec!["module-evidence".into()],
                evidence_origin: EvidenceOrigin::RealExecution,
                limitation,
                verified_scope,
            });
            progress(ProgressDetail {
                index: observations.len(),
                total: total_samples,
                id: plan.category.id().into(),
                message: format!("已完成规格样本 {} / {}", observations.len(), total_samples),
            });
        }
    }

    /// S07 流式输出实测：文本标记流 + 工具增量流各 1 次真实 stream:true
    /// 请求，验证语义增量、归组组装与正常终止。
    fn execute_stream_probes(
        &mut self,
        plan: &SpecificationPlan,
        observations: &mut Vec<SpecificationObservation>,
        evidence: &mut Vec<Value>,
        total_samples: usize,
        progress: &mut dyn FnMut(ProgressDetail),
    ) {
        let samples = stream_probe::samples();
        for sample_id in &plan.samples {
            let Some(sample) = samples.iter().find(|sample| sample.id == sample_id) else {
                continue;
            };
            progress(ProgressDetail {
                index: observations.len(),
                total: total_samples,
                id: plan.category.id().into(),
                message: format!(
                    "正在检测规格样本 {} / {}",
                    observations.len() + 1,
                    total_samples
                ),
            });
            let payload = stream_probe::request_payload(sample, self.transport.model_name());
            let stream = self
                .transport
                .send_stream_payload(payload.clone(), true, None);
            let outcome = stream_probe::classify(sample, &stream);
            evidence.push(json!({
                "sample_id": sample_id,
                "payload": self.transport.raw_stream_evidence_payload(&payload, "specification", &stream),
            }));
            let finish_reason = stream
                .events
                .iter()
                .rev()
                .find_map(|event| event.finish_reason.clone());
            let (status, limitation, verified_scope) = match &outcome {
                stream_probe::StreamOutcome::Pass { detail } => (
                    SpecStatus::Effective,
                    Some(format!(
                        "{detail}；仅覆盖该固定流式场景，不代表性能与稳定性"
                    )),
                    None,
                ),
                stream_probe::StreamOutcome::Violation { detail } => {
                    (SpecStatus::Failed, Some(detail.clone()), None)
                }
                stream_probe::StreamOutcome::Unsupported { detail } => {
                    (SpecStatus::Unsupported, Some(detail.clone()), None)
                }
                stream_probe::StreamOutcome::Malformed { detail } => {
                    (SpecStatus::Failed, Some(detail.clone()), None)
                }
                stream_probe::StreamOutcome::Inconclusive { detail } => {
                    (SpecStatus::Inconclusive, Some(detail.clone()), None)
                }
            };
            observations.push(SpecificationObservation {
                category: plan.category,
                sample_id: sample_id.clone(),
                attempt: SpecificationAttemptKind::Initial,
                conditions: plan.fixed_settings.clone(),
                actual_output: stream.response.parsed,
                natural_end: stream.terminated,
                finish_reason,
                status,
                evidence_refs: vec!["module-evidence".into()],
                evidence_origin: EvidenceOrigin::RealExecution,
                limitation,
                verified_scope,
            });
            progress(ProgressDetail {
                index: observations.len(),
                total: total_samples,
                id: plan.category.id().into(),
                message: format!("已完成规格样本 {} / {}", observations.len(), total_samples),
            });
        }
    }

    fn execute_capability(&mut self, record: &DetectionRecord) -> ModuleRunResult {
        let mut noop = |_detail: ProgressDetail| {};
        self.execute_capability_with_progress(record, &mut noop)
    }

    fn execute_capability_with_progress(
        &mut self,
        record: &DetectionRecord,
        progress: &mut dyn FnMut(ProgressDetail),
    ) -> ModuleRunResult {
        let samples = fixed_capability_catalog();
        let mut responses = Vec::with_capacity(samples.len());
        let mut evidence = Vec::with_capacity(samples.len());
        for sample in &samples {
            progress(ProgressDetail {
                index: evidence.len(),
                total: samples.len(),
                id: sample.id.clone(),
                message: format!(
                    "正在检测能力样本 {} / {}",
                    evidence.len() + 1,
                    samples.len()
                ),
            });
            let messages = sample.messages.as_ref().map(|turns| {
                turns
                    .iter()
                    .map(|turn| json!({"role": turn.role, "content": turn.content}))
                    .collect::<Vec<serde_json::Value>>()
            });
            let request = ChatCompletionsRequest {
                module_id: "capability".into(),
                prompt: sample.prompt.clone(),
                messages,
                tools: sample.tools.clone(),
                max_tokens: 1024,
                stream: false,
                allow_retry: true,
                timeout_ms: None,
            };
            let response = self.transport.send(request.clone());
            let text = completion_text(response.parsed.as_ref());
            let tool_calls = parse_tool_calls(response.parsed.as_ref());
            let truncated =
                response_finish_reason(response.parsed.as_ref()).as_deref() == Some("length");
            let execution = if response.error.is_some()
                || !response
                    .status
                    .is_some_and(|status| (200..300).contains(&status))
            {
                ExecutionState::Invalid {
                    reason: "真实服务请求未得到可评分响应".into(),
                }
            } else {
                ExecutionState::Valid
            };
            responses.push((
                sample.id.clone(),
                CapabilityResponse {
                    execution,
                    text,
                    tool_calls,
                    truncated,
                    evidence_refs: Vec::new(),
                },
            ));
            evidence.push(json!({"sample_id": sample.id, "payload": self.transport.evidence_payload(&request, &response)}));
            progress(ProgressDetail {
                index: evidence.len(),
                total: samples.len(),
                id: sample.id.clone(),
                message: format!("已完成能力样本 {} / {}", evidence.len(), samples.len()),
            });
        }
        let scorecard = build_scorecard(
            record.id.as_str(),
            samples,
            responses,
            CapabilitySettings {
                temperature: "0".into(),
                model: self.transport_model(),
                protocol: "chat-completions".into(),
                client_version: "0.1.0".into(),
                validated_input_max_tokens: None,
            },
        )
        .ok();
        let (state, state_reason) = match scorecard.as_ref() {
            None => (
                ModuleResultState::Inconclusive,
                "能力评分卡生成失败，无法判断模型能力".to_string(),
            ),
            Some(card) => {
                let wrong = card
                    .observations
                    .iter()
                    .filter(|observation| observation.label == crate::capability::ScoreLabel::Wrong)
                    .count();
                let execution_gaps = card
                    .observations
                    .iter()
                    .filter(|observation| {
                        matches!(
                            observation.label,
                            crate::capability::ScoreLabel::Incomplete
                                | crate::capability::ScoreLabel::Missing
                                | crate::capability::ScoreLabel::Pending
                        )
                    })
                    .count();
                if execution_gaps > 0 {
                    (
                        ModuleResultState::Inconclusive,
                        format!(
                            "{} 个样本没有形成可判定证据，不能归因于模型能力",
                            execution_gaps
                        ),
                    )
                } else if wrong > 0 {
                    (
                        ModuleResultState::Fail,
                        format!("{} 个有效样本答案未通过预先定义的判定规则", wrong),
                    )
                } else {
                    (
                        ModuleResultState::Pass,
                        "所有已执行样本均通过预先定义的判定规则".into(),
                    )
                }
            }
        };
        ModuleRunResult {
            state,
            reason: Some(format!(
                "已执行 {} 个能力固定单元；{}",
                evidence.len(),
                state_reason
            )),
            evidence_kind: "real_capability_scorecard".into(),
            evidence_summary: format!(
                "能力固定单元 {} 个，正式 scorecard {}",
                evidence.len(),
                if scorecard.is_some() {
                    "已生成"
                } else {
                    "生成失败"
                }
            ),
            evidence_payload: json!({"version": crate::capability::CAPABILITY_VERSION, "planned_samples": evidence.len(), "executed_samples": evidence.len(), "scorecard": scorecard, "evidence": evidence}),
        }
    }

    fn execute_performance(&mut self, record: &mut DetectionRecord) -> ModuleRunResult {
        let mut noop = |_detail: ProgressDetail| {};
        self.execute_performance_with_progress(record, &mut noop)
    }

    fn execute_performance_with_progress(
        &mut self,
        record: &mut DetectionRecord,
        progress: &mut dyn FnMut(ProgressDetail),
    ) -> ModuleRunResult {
        let run_started = std::time::Instant::now();
        let plans = fixed_performance_plan();
        let total_dimensions: usize = plans
            .iter()
            .map(|plan| performance_dimensions(plan).len())
            .sum();
        let mut completed_dimensions = 0_usize;
        let mut samples = Vec::new();
        let mut evidence = Vec::new();
        let mut consecutive_environment_failures = 0_u32;
        let mut circuit_breaker_reason: Option<String> = None;
        let mut protection_notes: Vec<String> = Vec::new();
        let stop_all = Arc::new(AtomicBool::new(false));
        'plans: for plan in plans {
            for (mode, input_tokens, target_output_tokens, target_concurrency) in
                performance_dimensions(&plan)
            {
                if stop_all.load(Ordering::Relaxed) {
                    break 'plans;
                }
                progress(ProgressDetail {
                    index: completed_dimensions,
                    total: total_dimensions,
                    id: plan.category.id().into(),
                    message: format!(
                        "正在检测性能项目 {} / {}",
                        completed_dimensions + 1,
                        total_dimensions
                    ),
                });
                let dimension_started = std::time::Instant::now();
                let mut protection = ProtectionState::default();
                let mut dimension_closed = false;

                // 预热：每轮按目标并发齐发一批，预热失败保留但不计入正式分母。
                for warmup_index in 0..plan.warmup_count {
                    if dimension_closed || stop_all.load(Ordering::Relaxed) {
                        break;
                    }
                    let work = (0..target_concurrency.max(1))
                        .map(|offset| {
                            let dispatched_at_ms = run_started.elapsed().as_millis() as u64;
                            PerformanceWorkItem {
                                sample_id: format!(
                                    "{}-{}-{}-{}-{}-{}-warmup-{}-{}",
                                    plan.category.id(),
                                    format_workload(plan.workload),
                                    format_mode(mode),
                                    input_tokens,
                                    target_output_tokens,
                                    target_concurrency,
                                    warmup_index + 1,
                                    offset + 1
                                ),
                                request: performance_request(
                                    mode,
                                    plan.category.id(),
                                    plan.workload,
                                    warmup_index,
                                    input_tokens,
                                    target_output_tokens,
                                ),
                                workload: plan.workload,
                                input_tokens,
                                target_output_tokens,
                                phase: RunPhase::Warmup,
                                target_concurrency,
                                actual_concurrency: target_concurrency.max(1),
                                dispatched_at_ms,
                                dispatch_window_start_ms: None,
                                dispatch_window_end_ms: None,
                            }
                        })
                        .collect();
                    for observation in self.run_performance_batch(work) {
                        let (error_kind, action) = self.record_and_protect(
                            record,
                            observation,
                            &mut samples,
                            &mut evidence,
                            &mut protection,
                        );
                        if is_environment_error(error_kind) {
                            consecutive_environment_failures += 1;
                        } else {
                            consecutive_environment_failures = 0;
                        }
                        match action {
                            ProtectionAction::Continue => {}
                            ProtectionAction::CloseCurrentConcurrencyTier => {
                                dimension_closed = true;
                                protection_notes.push(format!(
                                    "{} 预热阶段触发保护，关闭当前维度",
                                    plan.category.id()
                                ));
                            }
                            ProtectionAction::StopCurrentWorkload => {
                                stop_all.store(true, Ordering::Relaxed);
                                circuit_breaker_reason =
                                    Some("预热阶段出现认证或权限错误，已停止性能检测".into());
                            }
                        }
                        if consecutive_environment_failures >= 5 {
                            stop_all.store(true, Ordering::Relaxed);
                            circuit_breaker_reason = Some(
                                "连续 5 次请求出现网络、认证、限流或服务错误，已停止性能检测"
                                    .into(),
                            );
                        }
                    }
                }

                // 正式测量：闭环并发。在途请求恒定等于目标并发，走完一个补一个；
                // 并发 1 时退化为逐条串行。派发窗口从正式阶段起点计算。
                if !dimension_closed && !stop_all.load(Ordering::Relaxed) {
                    let formal_started = std::time::Instant::now();
                    let window_start_ms = run_started.elapsed().as_millis() as u64;
                    let window_end_ms = plan
                        .dispatch_window_ms
                        .map(|window| window_start_ms + window);
                    let next_index = Arc::new(AtomicUsize::new(0));
                    let in_flight = Arc::new(AtomicU32::new(0));
                    let stop_tier = Arc::new(AtomicBool::new(false));
                    let (sender, receiver) = mpsc::channel::<PerformanceObservation>();
                    let workers = (0..target_concurrency.max(1))
                        .map(|_| {
                            let transport = self.transport.clone();
                            let next_index = Arc::clone(&next_index);
                            let in_flight = Arc::clone(&in_flight);
                            let stop_tier = Arc::clone(&stop_tier);
                            let stop_all = Arc::clone(&stop_all);
                            let sender = sender.clone();
                            let category_id = plan.category.id();
                            let workload = plan.workload;
                            let formal_limit = plan.formal_request_limit;
                            let dispatch_window = plan.dispatch_window_ms;
                            let max_duration = plan.max_duration_ms;
                            std::thread::spawn(move || {
                                loop {
                                    if stop_tier.load(Ordering::Relaxed)
                                        || stop_all.load(Ordering::Relaxed)
                                    {
                                        break;
                                    }
                                    if dispatch_window.is_some_and(|window| {
                                        formal_started.elapsed().as_millis() as u64 >= window
                                    }) || max_duration.is_some_and(|limit| {
                                        dimension_started.elapsed().as_millis() as u64 >= limit
                                    }) {
                                        break;
                                    }
                                    let index = next_index.fetch_add(1, Ordering::Relaxed) as u32;
                                    if index >= formal_limit {
                                        break;
                                    }
                                    let actual = in_flight.fetch_add(1, Ordering::Relaxed) + 1;
                                    let item = PerformanceWorkItem {
                                        sample_id: format!(
                                            "{}-{}-{}-{}-{}-{}-{:04}",
                                            category_id,
                                            format_workload(workload),
                                            format_mode(mode),
                                            input_tokens,
                                            target_output_tokens,
                                            target_concurrency,
                                            index + 1
                                        ),
                                        request: performance_request(
                                            mode,
                                            category_id,
                                            workload,
                                            index,
                                            input_tokens,
                                            target_output_tokens,
                                        ),
                                        workload,
                                        input_tokens,
                                        target_output_tokens,
                                        phase: RunPhase::Formal,
                                        target_concurrency,
                                        actual_concurrency: actual,
                                        dispatched_at_ms: run_started.elapsed().as_millis() as u64,
                                        dispatch_window_start_ms: dispatch_window
                                            .map(|_| window_start_ms),
                                        dispatch_window_end_ms: window_end_ms,
                                    };
                                    let request = item.request.clone();
                                    let response = if request.stream {
                                        PerformanceResponse::Streaming(
                                            transport.send_stream(request),
                                        )
                                    } else {
                                        PerformanceResponse::Standard(transport.send(request))
                                    };
                                    in_flight.fetch_sub(1, Ordering::Relaxed);
                                    if sender
                                        .send(PerformanceObservation { item, response })
                                        .is_err()
                                    {
                                        break;
                                    }
                                }
                            })
                        })
                        .collect::<Vec<_>>();
                    drop(sender);
                    let mut tier_note_sent = false;
                    while let Ok(observation) = receiver.recv() {
                        let (error_kind, action) = self.record_and_protect(
                            record,
                            observation,
                            &mut samples,
                            &mut evidence,
                            &mut protection,
                        );
                        if is_environment_error(error_kind) {
                            consecutive_environment_failures += 1;
                        } else {
                            consecutive_environment_failures = 0;
                        }
                        match action {
                            ProtectionAction::Continue => {}
                            ProtectionAction::CloseCurrentConcurrencyTier => {
                                stop_tier.store(true, Ordering::Relaxed);
                                if !tier_note_sent {
                                    tier_note_sent = true;
                                    protection_notes.push(format!(
                                        "{} 目标并发 {target_concurrency} 触发限流或连续失败保护，已关闭该档",
                                        plan.category.id()
                                    ));
                                }
                            }
                            ProtectionAction::StopCurrentWorkload => {
                                stop_all.store(true, Ordering::Relaxed);
                                stop_tier.store(true, Ordering::Relaxed);
                                circuit_breaker_reason =
                                    Some("出现认证或权限错误，已停止性能检测".into());
                            }
                        }
                        if consecutive_environment_failures >= 5 {
                            stop_all.store(true, Ordering::Relaxed);
                            circuit_breaker_reason = Some(
                                "连续 5 次请求出现网络、认证、限流或服务错误，已停止性能检测"
                                    .into(),
                            );
                        }
                    }
                    for worker in workers {
                        let _ = worker.join();
                    }
                }
                if stop_all.load(Ordering::Relaxed) {
                    break 'plans;
                }
                completed_dimensions += 1;
                progress(ProgressDetail {
                    index: completed_dimensions,
                    total: total_dimensions,
                    id: plan.category.id().into(),
                    message: format!(
                        "已完成性能项目 {} / {}",
                        completed_dimensions, total_dimensions
                    ),
                });
            }
        }
        let end = samples
            .iter()
            .filter_map(|sample| sample.terminal_at_ms)
            .max()
            .unwrap_or_else(|| run_started.elapsed().as_millis() as u64);
        let report_result = build_performance_report_for_record(
            record,
            PerformanceConditions {
                model: self.transport_model(),
                protocol: "chat-completions".into(),
                client_version: "0.1.0".into(),
                timeout_ms: PERF_REQUEST_TIMEOUT_MS,
                input_range_max_tokens: None,
                window_start_ms: 0,
                window_end_ms: end,
            },
            samples,
        );
        let failed = report_result.as_ref().is_ok_and(|report| {
            report.samples.iter().any(|sample| {
                sample.phase == RunPhase::Formal
                    && matches!(
                        sample.terminal_state,
                        TerminalState::Error | TerminalState::Timeout
                    )
            })
        });
        let report = report_result.as_ref().ok();
        let formal_count = report
            .as_ref()
            .map(|value| {
                value
                    .samples
                    .iter()
                    .filter(|sample| sample.phase == RunPhase::Formal)
                    .count()
            })
            .unwrap_or_default();
        ModuleRunResult {
            state: if report.is_none() || failed {
                ModuleResultState::Inconclusive
            } else {
                ModuleResultState::Pass
            },
            reason: Some(format!(
                "已执行 {} 个性能正式样本；{}{}{}",
                formal_count,
                circuit_breaker_reason
                    .as_deref()
                    .unwrap_or("未触发连续失败熔断"),
                if protection_notes.is_empty() {
                    String::new()
                } else {
                    format!("；{}", protection_notes.join("；"))
                },
                if failed {
                    "；错误样本保留为不可测量"
                } else {
                    ""
                }
            )),
            evidence_kind: "real_performance_report".into(),
            evidence_summary: format!(
                "性能正式样本 {} 个，报告 {}",
                evidence.len(),
                if report.is_some() {
                    "已生成"
                } else {
                    "生成失败（证据引用或样本校验未通过）"
                }
            ),
            evidence_payload: json!({"version": crate::performance::PERFORMANCE_VERSION, "planned_samples": evidence.len(), "executed_samples": evidence.len(), "report": report, "report_error": report_result.err(), "evidence": evidence}),
        }
    }

    fn run_performance_batch(&self, work: Vec<PerformanceWorkItem>) -> Vec<PerformanceObservation> {
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(work.len() + 1));
        let handles = work
            .into_iter()
            .map(|item| {
                let transport = self.transport.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let request = item.request.clone();
                    let response = if request.stream {
                        PerformanceResponse::Streaming(transport.send_stream(request))
                    } else {
                        PerformanceResponse::Standard(transport.send(request))
                    };
                    PerformanceObservation { item, response }
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .expect("performance request thread")
                    .to_owned()
            })
            .collect()
    }

    /// 记录样本并交给保护状态机判定下一步动作。
    fn record_and_protect(
        &self,
        record: &mut DetectionRecord,
        observation: PerformanceObservation,
        samples: &mut Vec<PerformanceSample>,
        evidence: &mut Vec<Value>,
        protection: &mut ProtectionState,
    ) -> (Option<crate::performance::ErrorKind>, ProtectionAction) {
        let error_kind =
            self.record_performance_observation(record, observation, samples, evidence);
        let action = samples
            .last()
            .map(|sample| protection.observe(sample))
            .unwrap_or(ProtectionAction::Continue);
        (error_kind, action)
    }

    fn record_performance_observation(
        &self,
        record: &mut DetectionRecord,
        observation: PerformanceObservation,
        samples: &mut Vec<PerformanceSample>,
        evidence: &mut Vec<Value>,
    ) -> Option<crate::performance::ErrorKind> {
        let evidence_id = format!("cli-performance-{}", observation.item.sample_id);
        let (sample, payload) = performance_sample(observation, &self.transport);
        let captured = add_evidence(
            record,
            &evidence_id,
            "performance-sample",
            "performance-execution",
            payload.clone(),
        );
        let mut sample = sample;
        sample.evidence_refs = vec![captured.id.clone()];
        evidence
            .push(json!({"sample_id": sample.id, "evidence_id": captured.id, "payload": payload}));
        let error_kind = sample.error_kind;
        samples.push(sample);
        error_kind
    }

    fn execute_agent(&mut self, record: &mut DetectionRecord) -> ModuleRunResult {
        let mut noop = |_detail: ProgressDetail| {};
        self.execute_agent_with_progress(record, &mut noop)
    }

    fn execute_agent_with_progress(
        &mut self,
        record: &mut DetectionRecord,
        progress: &mut dyn FnMut(ProgressDetail),
    ) -> ModuleRunResult {
        let scenarios = fixed_agent_scenarios();
        let total_scenarios = scenarios.len();
        let mut executed = 0_usize;
        let mut attempts = Vec::new();
        let mut evidence = Vec::new();
        for spec in scenarios {
            progress(ProgressDetail {
                index: executed,
                total: total_scenarios,
                id: spec.scenario.id().into(),
                message: format!("正在执行智能体场景 {} / {}", executed + 1, total_scenarios),
            });
            let (execution, turn_evidence) = self.execute_agent_scenario(&spec);
            let evidence_id = format!("cli-agent-{}", spec.workspace.task_id);
            let captured = add_evidence(
                record,
                &evidence_id,
                "real_agent_scenario",
                "agent-execution",
                turn_evidence.clone(),
            );
            let mut execution = execution;
            let evidence_ref = captured.id.clone();
            execution.evidence_refs.push(evidence_ref.clone());
            for event in &mut execution.events {
                event.evidence_refs.push(evidence_ref.clone());
            }
            for permission in &mut execution.permission_events {
                permission.evidence_refs.push(evidence_ref.clone());
            }
            if let Some(artifact) = &mut execution.artifact {
                artifact.evidence_refs.push(evidence_ref.clone());
            }
            attempts.push(execution);
            evidence.push(json!({"sample_id": spec.workspace.task_id, "evidence_id": captured.id}));
            executed += 1;
            progress(ProgressDetail {
                index: executed,
                total: total_scenarios,
                id: spec.scenario.id().into(),
                message: format!("已完成智能体场景 {} / {}", executed, total_scenarios),
            });
        }
        let report_result = build_report_for_record(
            record,
            AgentRuntime {
                omp_version: agent_omp_version(),
                build_fingerprint: "omp-process".into(),
                license_ref: "Apache-2.0".into(),
                test_version: crate::agent::AGENT_VERSION.into(),
            },
            attempts,
        );
        let report = report_result.as_ref().ok().cloned();
        let state = report
            .as_ref()
            .map(|value| {
                if value
                    .samples
                    .iter()
                    .any(|sample| sample.outcome == AgentOutcome::Fail)
                {
                    ModuleResultState::Fail
                } else if value.samples.iter().all(|sample| {
                    matches!(
                        sample.outcome,
                        AgentOutcome::Pass | AgentOutcome::NotApplicable
                    )
                }) {
                    ModuleResultState::Pass
                } else {
                    ModuleResultState::Inconclusive
                }
            })
            .unwrap_or(ModuleResultState::Inconclusive);
        let reason = match report_result {
            Ok(_) => {
                "已执行 10 个 Agent 固定场景；工具能力、工作区事实和终态均按真实证据判定".into()
            }
            Err(error) => {
                format!("Agent 报告构建失败：{error}；已保留原始回合证据，未伪造模块结论")
            }
        };
        ModuleRunResult {
            state,
            reason: Some(reason),
            evidence_kind: "real_agent_report".into(),
            evidence_summary: format!(
                "Agent 场景 10 个，报告 {}",
                if report.is_some() {
                    "已生成"
                } else {
                    "生成失败"
                }
            ),
            evidence_payload: json!({"version": crate::agent::AGENT_VERSION, "planned_scenarios": 10, "executed_scenarios": 10, "report": report, "evidence": evidence}),
        }
    }

    fn execute_agent_scenario(
        &mut self,
        spec: &AgentScenarioSpec,
    ) -> (crate::agent::AgentExecution, Value) {
        let invalid = |reason: &str, evidence: Value| {
            (
                crate::agent::invalid_execution(
                    spec.workspace.task_id.clone(),
                    spec.scenario,
                    1,
                    crate::agent::AgentAttemptKind::Initial,
                    ExecutionOrigin::RealOmp,
                    reason,
                    Vec::new(),
                ),
                evidence,
            )
        };
        let Some(omp) = resolve_agent_omp() else {
            return invalid(
                "内置 OMP 运行时不可用，未执行该场景",
                json!({"sample_id": spec.workspace.task_id}),
            );
        };
        let (endpoint, model, api_key) = self.transport.target();
        let agent_config =
            match crate::evaluation::OmpConfig::new(&crate::evaluation::AnalyzerConfig {
                endpoint: endpoint.to_owned(),
                model: model.to_owned(),
                api_key: api_key.to_owned(),
            }) {
                Ok(config) => config,
                Err(error) => {
                    return invalid(
                        &format!("准备 OMP 运行配置失败：{error}"),
                        json!({"sample_id": spec.workspace.task_id}),
                    );
                }
            };
        let root = std::env::temp_dir().join(format!(
            "agentcheck-agent-{}-{}",
            spec.workspace.task_id,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|span| span.as_nanos())
                .unwrap_or_default()
        ));
        if let Err(error) = prepare_agent_workspace(spec, &root) {
            return invalid(
                &format!("准备工作区失败：{error}"),
                json!({"sample_id": spec.workspace.task_id}),
            );
        }
        let material_digests = spec
            .materials
            .iter()
            .map(|file| (file.path.clone(), digest_text(&file.content)))
            .collect::<BTreeMap<_, _>>();

        let mut session_log = String::new();
        let mut spawn_error = None;
        for (index, prompt) in spec.turns.iter().enumerate() {
            let mut command = Command::new(&omp);
            command
                .current_dir(&root)
                .args([
                    "-p",
                    "--mode",
                    "json",
                    "--tools",
                    "read,write",
                    "--auto-approve",
                    "--no-extensions",
                    "--no-skills",
                    "--no-rules",
                    "--no-lsp",
                    "--no-pty",
                    "--no-title",
                    "--max-time",
                ])
                .arg(format!("{}", spec.timeout_ms / 1000))
                .arg("--model")
                .arg(format!("agentcheck-target/{model}"))
                .env("PI_CODING_AGENT_DIR", &agent_config.agent_dir)
                .env("AGENTCHECK_MODEL_API_KEY", api_key);
            if index > 0 {
                command.arg("--continue");
            }
            command.arg(prompt);
            // OMP 自身有 --max-time；看门狗再兜底覆盖子进程无响应、
            // 管道不关闭等 --max-time 失效的场景，避免 CLI 永久挂住。
            let watchdog_ms = spec.timeout_ms.saturating_add(60_000);
            match crate::evaluation::run_command_with_timeout(
                &mut command,
                std::time::Duration::from_millis(watchdog_ms),
            ) {
                Ok(output) => {
                    session_log.push_str(&String::from_utf8_lossy(&output.stdout));
                    session_log.push_str(&String::from_utf8_lossy(&output.stderr));
                    if output.status.is_none() {
                        spawn_error = Some(format!(
                            "OMP 会话超过看门狗时限 {} 秒，已终止该进程",
                            watchdog_ms / 1000
                        ));
                        break;
                    }
                }
                Err(error) => {
                    spawn_error = Some(format!("OMP 进程启动失败：{error}"));
                    break;
                }
            }
        }
        if let Some(error) = spawn_error {
            return invalid(
                &error,
                json!({"sample_id": spec.workspace.task_id, "session": session_log}),
            );
        }

        let mut outcome = parse_agent_session(&session_log);
        if !outcome.terminated && outcome.calls.is_empty() && outcome.final_message.is_none() {
            return invalid(
                "OMP 会话未产生可用事件，无法判定场景终态",
                json!({"sample_id": spec.workspace.task_id, "session": session_log}),
            );
        }
        outcome.snapshot = snapshot_workspace(&root);
        outcome.input_intact = material_digests.iter().all(|(path, digest)| {
            outcome
                .snapshot
                .get(path)
                .is_some_and(|content| digest_text(content) == *digest)
        });
        classify_agent_paths(&root, &mut outcome);

        let mut events = Vec::new();
        let mut permission_events = Vec::new();
        let mut sequence = 0_u32;
        for call in &outcome.calls {
            sequence += 1;
            events.push(AgentEvent {
                id: format!("{}-call-{sequence}", spec.workspace.task_id),
                kind: AgentEventKind::ToolCall,
                sequence,
                summary: format!("{} {}", call.name, call.path.clone().unwrap_or_default()),
                path: call.path.clone(),
                operation: Some(call.name.clone()),
                incident_id: None,
                evidence_refs: Vec::new(),
            });
            sequence += 1;
            events.push(AgentEvent {
                id: format!("{}-return-{sequence}", spec.workspace.task_id),
                kind: if call.ok {
                    AgentEventKind::ToolReturn
                } else {
                    AgentEventKind::Error
                },
                sequence,
                summary: call.result_text.chars().take(2000).collect(),
                path: call.path.clone(),
                operation: Some(call.name.clone()),
                incident_id: None,
                evidence_refs: Vec::new(),
            });
        }
        for path in &outcome.unauthorized_writes {
            permission_events.push(PermissionEvent {
                path: path.clone(),
                operation: "write".into(),
                decision: PermissionDecision::Allowed,
                effect: PermissionEffect::UnauthorizedWrite,
                evidence_refs: Vec::new(),
            });
        }
        for path in &outcome.out_of_scope_reads {
            permission_events.push(PermissionEvent {
                path: path.clone(),
                operation: "read".into(),
                decision: PermissionDecision::Allowed,
                effect: PermissionEffect::Read,
                evidence_refs: Vec::new(),
            });
        }

        let (facts, artifact_satisfied) = judge_scenario(spec, &outcome);
        let artifact = outcome
            .snapshot
            .get(&spec.expected_artifact.path)
            .map(|content| crate::agent::ArtifactObservation {
                path: spec.expected_artifact.path.clone(),
                exists: true,
                content_digest: Some(digest_text(content)),
                content_matches: artifact_satisfied,
                evidence_refs: Vec::new(),
            })
            .or_else(|| {
                (!spec.artifact_optional).then(|| crate::agent::ArtifactObservation {
                    path: spec.expected_artifact.path.clone(),
                    exists: false,
                    content_digest: None,
                    content_matches: artifact_satisfied,
                    evidence_refs: Vec::new(),
                })
            });
        let execution = assess_execution(AgentExecutionInput {
            sample_id: spec.workspace.task_id.clone(),
            scenario: spec.scenario,
            attempt_no: 1,
            attempt_kind: crate::agent::AgentAttemptKind::Initial,
            origin: ExecutionOrigin::RealOmp,
            facts,
            permission_events,
            events,
            artifact,
            final_message: outcome.final_message.clone(),
            evidence_refs: Vec::new(),
        });
        (
            execution,
            json!({
                "sample_id": spec.workspace.task_id,
                "workspace_root": root,
                "session": session_log,
                "tool_calls": outcome.calls.len(),
                "terminated": outcome.terminated,
            }),
        )
    }

    fn execute_baseline(&mut self, record: &DetectionRecord) -> ModuleRunResult {
        let mut noop = |_detail: ProgressDetail| {};
        self.execute_baseline_with_progress(record, &mut noop)
    }

    fn execute_baseline_with_progress(
        &mut self,
        record: &DetectionRecord,
        progress: &mut dyn FnMut(ProgressDetail),
    ) -> ModuleRunResult {
        let catalog = fixed_baseline_catalog();
        let total_scenarios = BaselineScenario::ALL.len();
        let mut observations = Vec::new();
        let mut evidence = Vec::new();
        let model = self.transport.model_name().to_string();
        let tool_defs = json!([{
            "type": "function",
            "function": {
                "name": "lookup_weather",
                "description": "查询指定城市的天气信息",
                "parameters": {
                    "type": "object",
                    "properties": {"city": {"type": "string", "description": "城市名称"}},
                    "required": ["city"]
                }
            }
        }]);

        macro_rules! observe {
            ($scenario:expr, $raw:expr, $phase:expr) => {{
                let scenario = $scenario;
                progress(ProgressDetail {
                    index: observations.len(),
                    total: total_scenarios,
                    id: scenario.id().into(),
                    message: format!(
                        "正在检测基线场景 {} / {}",
                        observations.len() + 1,
                        total_scenarios
                    ),
                });
                observations.push(crate::baseline::BaselineObservation {
                    scenario,
                    actual: ActualSnapshot::from_raw(
                        $raw,
                        ActualSource::RealService,
                        $phase,
                        false,
                        Vec::new(),
                    ),
                });
                progress(ProgressDetail {
                    index: observations.len(),
                    total: total_scenarios,
                    id: scenario.id().into(),
                    message: format!(
                        "已完成基线场景 {} / {}",
                        observations.len(),
                        total_scenarios
                    ),
                });
            }};
        }

        // 非流式普通响应：一次请求覆盖 BC01–BC06。
        let normal_request = ChatCompletionsRequest {
            module_id: "baseline".into(),
            messages: None,
            tools: None,
            prompt: "请用中文一句话介绍你自己。".into(),
            max_tokens: 256,
            stream: false,
            allow_retry: true,
            timeout_ms: None,
        };
        let normal = self.transport.send(normal_request.clone());
        for scenario in [
            BaselineScenario::ResponseEnvelope,
            BaselineScenario::ChoiceContainer,
            BaselineScenario::ResponseMessage,
            BaselineScenario::UsageSummary,
            BaselineScenario::UsageDetails,
            BaselineScenario::ServiceMetadata,
        ] {
            observe!(scenario, normal.body.clone(), None);
        }
        evidence.push(json!({
            "scenarios": ["BC01", "BC02", "BC03", "BC04", "BC05", "BC06"],
            "payload": self.transport.evidence_payload(&normal_request, &normal),
        }));

        // 非流式工具调用：tool_choice=required 强制返回 tool_calls，覆盖 BC07–BC08。
        let tools_payload = json!({
            "model": model,
            "messages": [{"role": "user", "content": "请调用 lookup_weather 查询北京今天的天气。"}],
            "tools": tool_defs,
            "tool_choice": "required",
            "temperature": 0,
            "max_tokens": 256,
            "stream": false,
        });
        let tools_response = self
            .transport
            .send_payload(tools_payload.clone(), "baseline");
        for scenario in [
            BaselineScenario::ToolCallContainer,
            BaselineScenario::FunctionArguments,
        ] {
            observe!(scenario, tools_response.body.clone(), None);
        }
        evidence.push(json!({
            "scenarios": ["BC07", "BC08"],
            "payload": self.transport.raw_evidence_payload(&tools_payload, "baseline", &tools_response),
        }));

        // 流式普通响应：send_stream 自动携带 stream_options.include_usage，覆盖 BC09–BC11、BC13。
        let stream_request = ChatCompletionsRequest {
            prompt: "请用中文一句话介绍你自己。".into(),
            stream: true,
            ..normal_request.clone()
        };
        let stream = self.transport.send_stream(stream_request.clone());
        let stream_chunk = |pick: &dyn Fn(&crate::transport::StreamEvent) -> bool| {
            stream
                .events
                .iter()
                .find(|event| pick(event))
                .map(|event| event.raw.clone())
                .unwrap_or_default()
        };
        let has_content = |event: &crate::transport::StreamEvent| {
            event.content_delta.is_some() || event.reasoning_delta.is_some()
        };
        observe!(
            BaselineScenario::StreamEnvelope,
            stream
                .events
                .first()
                .map(|event| event.raw.clone())
                .unwrap_or_default(),
            Some(crate::baseline::StreamPhase::Initial)
        );
        observe!(
            BaselineScenario::StreamChoiceContainer,
            stream_chunk(&has_content),
            Some(crate::baseline::StreamPhase::Content)
        );
        observe!(
            BaselineScenario::StreamDelta,
            stream_chunk(&has_content),
            Some(crate::baseline::StreamPhase::Content)
        );

        // 流式工具调用：覆盖 BC12。
        let stream_tools_payload = json!({
            "model": model,
            "messages": [{"role": "user", "content": "请调用 lookup_weather 查询北京今天的天气。"}],
            "tools": tool_defs,
            "tool_choice": "required",
            "temperature": 0,
            "max_tokens": 256,
            "stream": true,
            "stream_options": {"include_usage": true},
        });
        let stream_tools =
            self.transport
                .send_stream_payload(stream_tools_payload.clone(), true, None);
        observe!(
            BaselineScenario::StreamToolDelta,
            stream_tools
                .events
                .iter()
                .find(|event| event.tool_calls_delta.is_some())
                .map(|event| event.raw.clone())
                .unwrap_or_default(),
            Some(crate::baseline::StreamPhase::Tool)
        );
        evidence.push(json!({
            "scenarios": ["BC12"],
            "payload": self.transport.raw_stream_evidence_payload(&stream_tools_payload, "baseline", &stream_tools),
        }));

        // 流式用量：优先取真实 usage chunk 原文；缺失时退回流汇总里的 usage 对象。
        let usage_raw = stream
            .events
            .iter()
            .find(|event| {
                serde_json::from_str::<Value>(&event.raw)
                    .ok()
                    .and_then(|value| value.get("usage").cloned())
                    .is_some_and(|usage| !usage.is_null())
            })
            .map(|event| event.raw.clone())
            .or_else(|| {
                stream.usage.as_ref().map(|usage| {
                    serde_json::to_string(&json!({"usage": usage})).unwrap_or_default()
                })
            })
            .unwrap_or_default();
        observe!(
            BaselineScenario::StreamUsage,
            usage_raw,
            Some(crate::baseline::StreamPhase::Terminal)
        );
        evidence.push(json!({
            "scenarios": ["BC09", "BC10", "BC11", "BC13"],
            "payload": self.transport.stream_evidence_payload(&stream_request, &stream),
        }));

        // 错误响应：messages 传字符串触发服务端校验错误，覆盖 BC14。
        let error_payload = json!({
            "model": model,
            "messages": "not-an-array",
            "temperature": 0,
            "stream": false,
        });
        let error_response = self
            .transport
            .send_payload(error_payload.clone(), "baseline");
        observe!(
            BaselineScenario::ErrorEnvelope,
            error_response.body.clone(),
            None
        );
        evidence.push(json!({
            "scenarios": ["BC14"],
            "payload": self.transport.raw_evidence_payload(&error_payload, "baseline", &error_response),
        }));
        let report = build_baseline_report(record, catalog, observations).ok();
        ModuleRunResult {
            state: if report.is_some() {
                ModuleResultState::Pass
            } else {
                ModuleResultState::Inconclusive
            },
            reason: Some(format!(
                "已执行 {} 个基线场景；结构事实不直接推导整体可用性",
                evidence.len()
            )),
            evidence_kind: "real_baseline_report".into(),
            evidence_summary: format!(
                "基线场景 {} 个，报告 {}",
                evidence.len(),
                if report.is_some() {
                    "已生成"
                } else {
                    "生成失败"
                }
            ),
            evidence_payload: json!({"version": crate::baseline::BASELINE_VERSION, "planned_scenarios": evidence.len(), "executed_scenarios": evidence.len(), "report": report, "evidence": evidence}),
        }
    }

    fn transport_model(&self) -> String {
        self.transport.model_name().into()
    }
}

#[derive(Debug, Clone)]
struct PerformanceWorkItem {
    sample_id: String,
    request: ChatCompletionsRequest,
    workload: crate::performance::WorkloadKind,
    input_tokens: u32,
    target_output_tokens: u32,
    phase: RunPhase,
    target_concurrency: u32,
    actual_concurrency: u32,
    dispatched_at_ms: u64,
    dispatch_window_start_ms: Option<u64>,
    dispatch_window_end_ms: Option<u64>,
}

#[derive(Debug, Clone)]
enum PerformanceResponse {
    Standard(ChatCompletionsResponse),
    Streaming(StreamResponse),
}

#[derive(Debug, Clone)]
struct PerformanceObservation {
    item: PerformanceWorkItem,
    response: PerformanceResponse,
}

fn format_mode(mode: PerformanceResponseMode) -> &'static str {
    match mode {
        PerformanceResponseMode::Streaming => "stream",
        PerformanceResponseMode::NonStreaming => "nonstream",
    }
}

fn performance_dimensions(
    plan: &crate::performance::PerformancePlan,
) -> Vec<(PerformanceResponseMode, u32, u32, u32)> {
    plan.modes
        .iter()
        .flat_map(|mode| {
            plan.input_tokens.iter().flat_map(move |input_tokens| {
                plan.target_output_tokens
                    .iter()
                    .flat_map(move |target_output_tokens| {
                        plan.concurrency_targets
                            .iter()
                            .map(move |target_concurrency| {
                                (
                                    *mode,
                                    *input_tokens,
                                    *target_output_tokens,
                                    *target_concurrency,
                                )
                            })
                    })
            })
        })
        .collect()
}

fn format_workload(workload: crate::performance::WorkloadKind) -> &'static str {
    match workload {
        crate::performance::WorkloadKind::ResponseWaiting => "waiting",
        crate::performance::WorkloadKind::GenerationFluency => "fluency",
        crate::performance::WorkloadKind::Concurrency => "concurrency",
        crate::performance::WorkloadKind::Continuous => "continuous",
        crate::performance::WorkloadKind::LongInput => "long-input",
        crate::performance::WorkloadKind::LongOutput => "long-output",
        crate::performance::WorkloadKind::HistoryGrowth => "history",
    }
}

/// 填充行约 80 个 ASCII 字符；按 ~3.2 字符/token 估算并多写 20%，
/// 实际输入长度以服务端 usage.prompt_tokens 为准记录在样本里。
const PADDING_LINE: &str =
    "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor\n";
const PADDING_CHARS_PER_TOKEN: f64 = 3.2;
/// 历史增长负载：每条约 128 tokens，对应 512/2048/4096 档的 4/16/32 条消息。
const HISTORY_MESSAGE_CHARS: usize = 128 * 4;

/// 构造约 target_tokens 的真实长度输入材料。
fn padded_material(target_tokens: u32) -> String {
    let target_chars = (target_tokens as f64 * PADDING_CHARS_PER_TOKEN) as usize;
    let mut material = String::with_capacity(target_chars + 256);
    material.push_str("PERF_INPUT_BEGIN\n");
    let mut index = 0usize;
    while material.len() < target_chars {
        index += 1;
        material.push_str(&format!("SEGMENT {index:07} {PADDING_LINE}"));
    }
    material.push_str("PERF_INPUT_END\n");
    material
}

/// 构造总输入约 total_tokens 的交替 user/assistant 历史消息，末条为真实提问。
fn history_messages(total_tokens: u32) -> Vec<Value> {
    let count = (total_tokens / 128).clamp(2, 64) as usize;
    let filler =
        "history record filler lorem ipsum dolor sit amet consectetur adipiscing\n".repeat(6);
    let mut messages = (0..count.saturating_sub(1))
        .map(|index| {
            json!({
                "role": if index % 2 == 0 { "user" } else { "assistant" },
                "content": format!("历史消息 {:04}：{}", index + 1, &filler[..HISTORY_MESSAGE_CHARS.min(filler.len())]),
            })
        })
        .collect::<Vec<_>>();
    if messages.len() % 2 == 1 {
        messages.push(json!({
            "role": "assistant",
            "content": format!("历史消息 {:04}：{}", messages.len() + 1, &filler[..HISTORY_MESSAGE_CHARS.min(filler.len())]),
        }));
    }
    messages
}

/// 中文正文约 1.5 字/token：提示词按字要长度时换算成目标 token 对应的字数，
/// 避免"约 256 字"实际只产出 ~150 tokens 导致系统性未达标。
fn output_prompt_chars(target_output_tokens: u32) -> u32 {
    target_output_tokens + target_output_tokens / 2
}

/// 输出上限 = 目标 + 余量，避免把"写到目标长度"误判成截断；撞上限仍记为截断。
/// 非长输出档给 2 倍余量：推理模型的 reasoning tokens 与正文共享 max_tokens，
/// 余量不足会把短输出档变成系统性截断，丢掉自然结束样本。
fn output_cap(workload: crate::performance::WorkloadKind, target_output_tokens: u32) -> u32 {
    match workload {
        crate::performance::WorkloadKind::LongOutput => {
            target_output_tokens + target_output_tokens / 2
        }
        _ => target_output_tokens * 2 + 64,
    }
}

fn performance_request(
    mode: PerformanceResponseMode,
    category: &str,
    workload: crate::performance::WorkloadKind,
    index: u32,
    input_tokens: u32,
    target_output_tokens: u32,
) -> ChatCompletionsRequest {
    let (prompt, messages) = match workload {
        crate::performance::WorkloadKind::LongInput => (
            format!(
                "{}\n以上是填充材料（性能实测 {category} 第 {} 次）。请用约两百字概括这段材料的结构。",
                padded_material(input_tokens),
                index + 1
            ),
            None,
        ),
        crate::performance::WorkloadKind::LongOutput => (
            format!(
                "性能实测 {category} 长输出样本 {}。请围绕\"如何为一次软件系统做性能评测\"写一篇不少于 {} 字的长文，分多个小节充分展开，未达篇幅前不要收尾。",
                index + 1,
                output_prompt_chars(target_output_tokens)
            ),
            None,
        ),
        crate::performance::WorkloadKind::HistoryGrowth => {
            let mut messages = history_messages(input_tokens);
            messages.push(json!({
                "role": "user",
                "content": format!("性能实测 {category} 第 {} 次：请用约 {} 字概括我们以上的对话内容。", index + 1, output_prompt_chars(target_output_tokens)),
            }));
            (
                format!("历史增长负载，约 {input_tokens} tokens 对话历史"),
                Some(messages),
            )
        }
        _ => (
            format!(
                "性能实测 {category} 第 {} 次。请写一段约 {} 字的连续中文说明文字，主题自选，写够篇幅后自然收尾。",
                index + 1,
                output_prompt_chars(target_output_tokens)
            ),
            None,
        ),
    };
    ChatCompletionsRequest {
        module_id: "performance".into(),
        prompt,
        messages,
        tools: None,
        max_tokens: output_cap(workload, target_output_tokens),
        stream: mode == PerformanceResponseMode::Streaming,
        allow_retry: false,
        timeout_ms: Some(PERF_REQUEST_TIMEOUT_MS),
    }
}

fn performance_sample(
    observation: PerformanceObservation,
    transport: &ChatCompletionsTransport,
) -> (PerformanceSample, Value) {
    let item = observation.item;
    let (response, events, content, terminated, payload) = match observation.response {
        PerformanceResponse::Standard(response) => {
            let payload = transport.evidence_payload(&item.request, &response);
            (response, Vec::new(), String::new(), true, payload)
        }
        PerformanceResponse::Streaming(stream) => {
            let payload = transport.stream_evidence_payload(&item.request, &stream);
            (
                stream.response,
                stream.events,
                stream.content,
                stream.terminated,
                payload,
            )
        }
    };
    let elapsed = response.elapsed_ms as u64;
    let text = if item.request.stream {
        (!content.is_empty()).then_some(content.clone())
    } else {
        completion_text(response.parsed.as_ref())
    };
    let usage = response
        .parsed
        .as_ref()
        .and_then(|value| value.get("usage"));
    let actual_input_tokens = usage
        .and_then(|value| value.get("prompt_tokens"))
        .and_then(Value::as_u64)
        .map(|value| value as u32);
    let actual_output_tokens = usage
        .and_then(|value| value.get("completion_tokens"))
        .and_then(Value::as_u64)
        .map(|value| value as u32);
    let token_count_source = if actual_output_tokens.is_some() {
        TokenCountSource::ServiceUsage
    } else {
        TokenCountSource::Unavailable
    };
    let valid = response.error.is_none()
        && response
            .status
            .is_some_and(|status| (200..300).contains(&status))
        && is_chat_completion_shape(response.parsed.as_ref())
        && (!item.request.stream || terminated);
    let finish_reason = response_finish_reason(response.parsed.as_ref());
    let timed_out = response.error.as_deref().is_some_and(|error| {
        let lower = error.to_ascii_lowercase();
        error.contains("超时") || lower.contains("timed out") || lower.contains("timeout")
    });
    let terminal_state = if timed_out {
        TerminalState::Timeout
    } else if !valid {
        TerminalState::Error
    } else if finish_reason.as_deref() == Some("length") {
        TerminalState::Truncated
    } else {
        TerminalState::NaturalEnd
    };
    let absolute = |relative: u64| item.dispatched_at_ms.saturating_add(relative);
    let first_event = if item.request.stream {
        events.first().map(|event| absolute(event.at_ms))
    } else {
        Some(absolute(elapsed))
    };
    let first_reasoning = events
        .iter()
        .find(|event| {
            event
                .reasoning_delta
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
        })
        .map(|event| absolute(event.at_ms));
    let visible_events = if item.request.stream {
        events
            .iter()
            .filter(|event| {
                event
                    .content_delta
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
            })
            .map(|event| absolute(event.at_ms))
            .collect()
    } else {
        Vec::new()
    };
    let first_visible = visible_events.first().copied();
    // 只有自然结束的请求计入正常完成耗时；撞上限截断保留终态但不混入速度分布。
    let natural_end = valid && finish_reason.as_deref() != Some("length");
    let complete = natural_end.then(|| absolute(elapsed));
    let length_target_met =
        actual_output_tokens.is_some_and(|value| value >= item.target_output_tokens);
    let limitation = if !valid {
        Some("请求未形成可测量的正常结束".into())
    } else if actual_output_tokens.is_none() {
        Some("服务未返回可核验 token 数；仅展示字符和事件计量".into())
    } else if !length_target_met {
        Some("实际输出未达到目标长度".into())
    } else {
        None
    };
    let error_kind = (!valid).then(|| match response.status {
        Some(401 | 403) => ErrorKind::Authentication,
        Some(429) => ErrorKind::RateLimited,
        Some(status) if status >= 500 => ErrorKind::Service,
        _ if response.error.is_some() => ErrorKind::Network,
        _ => ErrorKind::Service,
    });
    let sample = PerformanceSample {
        id: item.sample_id,
        workload: item.workload,
        mode: if item.request.stream {
            PerformanceResponseMode::Streaming
        } else {
            PerformanceResponseMode::NonStreaming
        },
        phase: item.phase,
        input_tokens_target: item.input_tokens,
        target_output_tokens: item.target_output_tokens,
        actual_input_tokens,
        actual_output_tokens,
        output_chars: text.as_ref().map(|value| value.chars().count() as u32),
        target_concurrency: item.target_concurrency,
        actual_concurrency: item.actual_concurrency,
        dispatched_at_ms: item.dispatched_at_ms,
        terminal_at_ms: Some(absolute(elapsed)),
        timeline: Timeline {
            send_ms: item.dispatched_at_ms,
            first_event_ms: first_event,
            first_reasoning_ms: first_reasoning,
            first_visible_ms: first_visible,
            complete_ms: complete,
            error_ms: (!valid).then_some(absolute(elapsed)),
            cancel_ms: None,
            visible_events_ms: visible_events,
        },
        terminal_state,
        error_kind,
        length_target_met,
        token_count_source,
        evidence_refs: Vec::new(),
        limitation,
        dispatch_window_start_ms: item.dispatch_window_start_ms,
        dispatch_window_end_ms: item.dispatch_window_end_ms,
    };
    (sample, payload)
}

fn is_environment_error(error_kind: Option<crate::performance::ErrorKind>) -> bool {
    matches!(
        error_kind,
        Some(
            crate::performance::ErrorKind::Authentication
                | crate::performance::ErrorKind::Permission
                | crate::performance::ErrorKind::RateLimited
                | crate::performance::ErrorKind::Service
                | crate::performance::ErrorKind::Network
        )
    )
}

/// 解析内置 OMP 可执行文件（与模块分析阶段同一套查找顺序）。
fn resolve_agent_omp() -> Option<PathBuf> {
    crate::evaluation::resolve_omp_path().ok()
}

fn agent_omp_version() -> String {
    resolve_agent_omp()
        .and_then(|omp| {
            Command::new(omp)
                .arg("--version")
                .output()
                .ok()
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        })
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| "omp-unavailable".into())
}

fn prepare_agent_workspace(spec: &AgentScenarioSpec, root: &Path) -> Result<(), String> {
    for file in &spec.materials {
        let path = root.join(&file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("创建目录失败：{error}"))?;
        }
        fs::write(&path, &file.content).map_err(|error| format!("写入材料失败：{error}"))?;
    }
    fs::create_dir_all(root.join(&spec.workspace.layout.workspace_dir))
        .map_err(|error| format!("创建工作区目录失败：{error}"))?;
    Ok(())
}

/// 收集运行后场景根目录内全部文件（相对路径 -> 文本内容）。
fn snapshot_workspace(root: &Path) -> BTreeMap<String, String> {
    let mut snapshot = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(content) = fs::read_to_string(&path)
                && let Ok(relative) = path.strip_prefix(root)
            {
                snapshot.insert(relative.to_string_lossy().replace('\\', "/"), content);
            }
        }
    }
    snapshot
}

/// 把模型给出的路径解析为规范化绝对路径（不触碰文件系统）。
fn resolve_run_path(root: &Path, raw: &str) -> PathBuf {
    let raw_path = Path::new(raw.trim());
    let joined = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        root.join(raw_path)
    };
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

/// 按真实落点分类越权事实：写入必须在 workspace/ 内，读取必须在场景根内。
fn classify_agent_paths(root: &Path, outcome: &mut AgentRunOutcome) {
    let workspace_root = root.join("workspace");
    for call in &outcome.calls {
        let Some(raw) = call.path.as_deref() else {
            continue;
        };
        let resolved = resolve_run_path(root, raw);
        match call.name.as_str() {
            "write" | "edit" if !resolved.starts_with(&workspace_root) => {
                outcome.unauthorized_writes.push(raw.to_owned());
            }
            "read" | "grep" | "glob" if !resolved.starts_with(root) => {
                outcome.out_of_scope_reads.push(raw.to_owned());
            }
            _ => {}
        }
    }
}

fn digest_text(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
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

fn response_finish_reason(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(|value| value.get("choices"))
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("finish_reason"))
        .and_then(Value::as_str)
        .map(str::to_owned)
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

fn completion_text(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(|value| value.get("choices"))
        .and_then(serde_json::Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message").or_else(|| choice.get("text")))
        .and_then(|message| {
            message.as_str().map(str::to_owned).or_else(|| {
                message
                    .get("content")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
        })
}

fn parse_tool_calls(value: Option<&serde_json::Value>) -> Vec<ToolCall> {
    let Some(value) = value else {
        return Vec::new();
    };
    value
        .get("choices")
        .and_then(serde_json::Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("tool_calls"))
        .and_then(serde_json::Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .filter_map(|call| {
                    let function = call.get("function")?;
                    let name = function.get("name")?.as_str()?.to_string();
                    let arguments = function
                        .get("arguments")
                        .and_then(serde_json::Value::as_str)
                        .and_then(|raw| {
                            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(raw)
                                .ok()
                        })
                        .map(|object| {
                            object
                                .into_iter()
                                .map(|(key, value)| {
                                    let rendered = match value {
                                        serde_json::Value::String(text) => text,
                                        other => other.to_string(),
                                    };
                                    (key, rendered)
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    Some(ToolCall { name, arguments })
                })
                .collect()
        })
        .unwrap_or_default()
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProgressPhase {
    RunStarted,
    ModuleStarted,
    ModuleProgress,
    ModuleCompleted,
    RunCompleted,
    RunStopped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProgressEvent {
    pub phase: ProgressPhase,
    pub module_id: Option<String>,
    pub index: usize,
    pub total: usize,
    pub state: Option<String>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail_total: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<ProgressPlanItem>>,
    /// run_started 事件携带的本次选中模块列表，供显示端预渲染完整清单。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modules: Option<Vec<String>>,
    /// module_completed 事件携带的小项判定统计（未通过数 / 总数）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_stats: Option<ProgressItemStats>,
}

/// 模块内检测小项的判定统计：未通过多少项、共多少项。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProgressItemStats {
    pub failed: usize,
    pub total: usize,
}

/// 从各模块执行结果里提取"未通过小项数 / 小项总数"；提取不到时返回 None，
/// 显示端退回只显示状态标签。
pub(crate) fn module_item_stats(
    module_id: &str,
    payload: &serde_json::Value,
) -> Option<ProgressItemStats> {
    let stats =
        |failed: usize, total: usize| (total > 0).then(|| ProgressItemStats { failed, total });
    match module_id {
        "specification" => {
            let rows = payload["report"]["rows"].as_array()?;
            let failed = rows
                .iter()
                .filter(|row| row["result"].as_str() == Some("failed"))
                .count();
            stats(failed, rows.len())
        }
        "capability" => {
            let summaries = payload["scorecard"]["summaries"].as_array()?;
            let failed = summaries
                .iter()
                .filter(|summary| {
                    summary["wrong"].as_u64().unwrap_or(0)
                        + summary["missing"].as_u64().unwrap_or(0)
                        > 0
                })
                .count();
            stats(failed, summaries.len())
        }
        "agent" => {
            let checks = payload["report"]["check_summaries"].as_array()?;
            let failed = checks
                .iter()
                .filter(|check| check["fail"].as_u64().unwrap_or(0) > 0)
                .count();
            stats(failed, checks.len())
        }
        "baseline" => {
            let comparisons = payload["report"]["comparisons"].as_array()?;
            let failed = comparisons
                .iter()
                .filter(|comparison| comparison["status"].as_str() == Some("different"))
                .count();
            stats(failed, comparisons.len())
        }
        _ => None,
    }
}

/// 模块内部检测小项的展示定义；module_started 事件携带，
/// 让 CLI 终端和 GUI 使用同一份小项清单渲染进度。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProgressPlanItem {
    pub id: String,
    pub name: String,
    pub total: usize,
}

/// 后端模块 ID 对应的客户可见名称，与 GUI 展示保持一致。
pub fn module_display_name(module_id: &str) -> &'static str {
    match module_id {
        "ingress" => "服务接入",
        "specification" => "模型规格实测",
        "capability" => "模型能力跑分",
        "performance" => "模型性能实测",
        "agent" => "智能体实测",
        "baseline" => "模型基线对比",
        _ => "未知项目",
    }
}

/// 模块结果状态的中文标签，与 GUI statusLabel 映射一致。
pub fn module_state_label(state: &str) -> &'static str {
    match state {
        "pass" => "通过",
        "fail" => "未通过",
        "unsupported" => "不支持",
        "inconclusive" => "待确认",
        "invalid_execution" => "执行无效",
        "not_applicable" => "不适用",
        "not_selected" => "未选择",
        "unverified" => "未验证",
        _ => "未知",
    }
}

/// 各模块的检测小项清单（id、客户可见名称、子项数量），
/// 与 GUI progressItems 模板保持一致。
pub fn module_plan_items(module_id: &str) -> Option<Vec<ProgressPlanItem>> {
    let entries: &[(&str, &str, usize)] = match module_id {
        "specification" => &[
            ("S01", "协议可接受上限", 4),
            ("S04", "工具调用", 5),
            ("S05", "结构化输出", 2),
            ("S06", "消息与多轮输入", 4),
            ("S07", "流式输出", 2),
        ],
        "capability" => &[
            ("C01", "文本理解与指令执行", 44),
            ("C02", "信息提取与结构化填写", 20),
            ("C03", "工具选择与参数填写", 20),
            ("C04", "多轮对话与条件承接", 20),
            ("C05", "长材料理解与信息利用", 20),
            ("C06", "逻辑推理与计算", 20),
        ],
        "performance" => &[
            ("P01", "首字响应时间", 2),
            ("P02", "完整响应时间", 1),
            ("P03", "并发处理能力", 4),
            ("P04", "持续运行稳定性", 1),
            ("P05", "长文本负载", 7),
        ],
        "agent" => &[
            ("T1-A", "遵守任务规则", 1),
            ("T1-B", "处理外部注入", 1),
            ("T2-A", "选择正确工具", 1),
            ("T2-B", "校验路径与参数", 1),
            ("T3-A", "使用工具返回驱动下一步", 1),
            ("T3-B", "处理工具返回的信息缺失", 1),
            ("T4-A", "跨轮次保留状态", 1),
            ("T4-B", "跨轮次响应条件变化", 1),
            ("T5-A", "处理可恢复工具失败", 1),
            ("T5-B", "处理信息不足与提前结束", 1),
        ],
        "baseline" => &[
            ("BC01", "响应外层", 1),
            ("BC02", "候选回复", 1),
            ("BC03", "回复消息", 1),
            ("BC04", "用量统计", 1),
            ("BC05", "细分用量", 1),
            ("BC06", "附加信息", 1),
            ("BC07", "工具调用结构", 1),
            ("BC08", "函数名称与参数", 1),
            ("BC09", "分块外层", 1),
            ("BC10", "增量候选", 1),
            ("BC11", "消息增量", 1),
            ("BC12", "工具调用增量", 1),
            ("BC13", "流式用量返回", 1),
            ("BC14", "错误对象", 1),
        ],
        _ => return None,
    };
    Some(
        entries
            .iter()
            .map(|(id, name, total)| ProgressPlanItem {
                id: (*id).into(),
                name: (*name).into(),
                total: *total,
            })
            .collect(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressDetail {
    pub index: usize,
    pub total: usize,
    pub id: String,
    pub message: String,
}

pub trait ProgressSink {
    fn emit(&mut self, event: ProgressEvent);
}

#[derive(Debug, Default)]
pub struct NoopProgressSink;

impl ProgressSink for NoopProgressSink {
    fn emit(&mut self, _event: ProgressEvent) {}
}

pub struct JsonlProgressSink<W: Write> {
    writer: W,
}

impl<W: Write> JsonlProgressSink<W> {
    pub fn new(writer: W) -> Self {
        Self { writer }
    }
}

impl<W: Write> ProgressSink for JsonlProgressSink<W> {
    fn emit(&mut self, event: ProgressEvent) {
        if let Ok(line) = serde_json::to_string(&event) {
            let _ = writeln!(self.writer, "{line}");
            let _ = self.writer.flush();
        }
    }
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
    let mut progress = NoopProgressSink;
    run_with_executor_reporting(request, executor, &mut progress)
}

pub fn run_with_executor_reporting<E: ModuleExecutor, S: ProgressSink>(
    request: CliRunRequest,
    executor: &mut E,
    progress: &mut S,
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
    progress.emit(ProgressEvent {
        phase: ProgressPhase::RunStarted,
        module_id: None,
        index: 0,
        total: selected.len(),
        state: None,
        message: "检测已开始".into(),
        detail_index: None,
        detail_total: None,
        detail_id: None,
        items: None,
        modules: Some(selected.clone()),
        item_stats: None,
    });
    let stop_index = request
        .stop_after
        .as_deref()
        .and_then(|module| selected.iter().position(|item| item == module));
    let mut stopped = false;
    for (index, module_id) in selected.iter().enumerate() {
        if stop_index.is_some_and(|stop_index| index > stop_index) {
            break;
        }
        progress.emit(ProgressEvent {
            phase: ProgressPhase::ModuleStarted,
            module_id: Some(module_id.clone()),
            index,
            total: selected.len(),
            state: None,
            message: format!("开始检测 {}", module_display_name(module_id)),
            detail_index: None,
            detail_total: None,
            detail_id: None,
            items: module_plan_items(module_id),
            modules: None,
            item_stats: None,
        });
        let mut detail_progress = |detail: ProgressDetail| {
            progress.emit(ProgressEvent {
                phase: ProgressPhase::ModuleProgress,
                module_id: Some(module_id.clone()),
                index,
                total: selected.len(),
                state: None,
                message: detail.message,
                detail_index: Some(detail.index),
                detail_total: Some(detail.total),
                detail_id: Some(detail.id),
                items: None,
                modules: None,
                item_stats: None,
            });
        };
        let result =
            executor.execute_with_record_progress(module_id, &mut record, &mut detail_progress);
        let item_stats = module_item_stats(module_id, &result.evidence_payload);
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
        let progress_state = serde_json::to_value(result.state)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("{:?}", result.state).to_lowercase());
        let progress_message = result
            .reason
            .clone()
            .unwrap_or_else(|| format!("{module_id} 检测完成"));
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
        progress.emit(ProgressEvent {
            phase: ProgressPhase::ModuleCompleted,
            module_id: Some(module_id.clone()),
            index: index + 1,
            total: selected.len(),
            state: Some(progress_state),
            message: progress_message,
            detail_index: None,
            detail_total: None,
            detail_id: None,
            items: None,
            modules: None,
            item_stats,
        });
        if stop_index == Some(index) {
            stop_run(
                &mut record,
                "CLI requested stop-after; remaining selected projects remain unverified",
                &request.started_at,
            )?;
            stopped = true;
            progress.emit(ProgressEvent {
                phase: ProgressPhase::RunStopped,
                module_id: None,
                index: index + 1,
                total: selected.len(),
                state: Some("unverified".into()),
                message: "检测已停止，未完成项目保持未验证".into(),
                detail_index: None,
                detail_total: None,
                detail_id: None,
                items: None,
                modules: None,
                item_stats: None,
            });
            break;
        }
    }
    if !stopped {
        complete_run(&mut record, &request.started_at)?;
        progress.emit(ProgressEvent {
            phase: ProgressPhase::RunCompleted,
            module_id: None,
            index: selected.len(),
            total: selected.len(),
            state: None,
            message: "检测已完成".into(),
            detail_index: None,
            detail_total: None,
            detail_id: None,
            items: None,
            modules: None,
            item_stats: None,
        });
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

fn display_width(text: &str) -> usize {
    text.chars()
        .map(|ch| if (ch as u32) >= 0x2e80 { 2 } else { 1 })
        .sum()
}

fn pad_display(text: &str, width: usize) -> String {
    let padding = width.saturating_sub(display_width(text));
    format!("{text}{}", " ".repeat(padding))
}

fn lifecycle_display(state: crate::records::LifecycleState) -> &'static str {
    match state {
        crate::records::LifecycleState::Planned => "待执行",
        crate::records::LifecycleState::Running => "运行中",
        crate::records::LifecycleState::Stopping => "停止中",
        crate::records::LifecycleState::Stopped => "已停止",
        crate::records::LifecycleState::Completed => "已完成",
    }
}

fn result_counts(record: &crate::records::DetectionRecord) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for result in &record.module_results {
        let label = module_state_label(module_result_label(result.state));
        if let Some(entry) = counts.iter_mut().find(|(name, _)| *name == label) {
            entry.1 += 1;
        } else {
            counts.push((label, 1));
        }
    }
    counts
        .iter()
        .map(|(name, count)| format!("{count} {name}"))
        .collect::<Vec<_>>()
        .join(" / ")
}

pub fn render_text(report: &CliRunReport) -> String {
    let state_width = report
        .record
        .module_results
        .iter()
        .map(|result| display_width(module_state_label(module_result_label(result.state))))
        .max()
        .unwrap_or(0);
    let name_width = report
        .record
        .module_results
        .iter()
        .map(|result| display_width(module_display_name(&result.module_id)))
        .max()
        .unwrap_or(0);
    let mut lines = vec![
        format!("检测结果  {}", report.record.id),
        "─".repeat(40),
        format!("模型      {}", report.configuration.model),
        format!("地址      {}", report.configuration.redacted_endpoint),
        format!(
            "运行      {} · {}",
            lifecycle_display(report.record.lifecycle),
            result_counts(&report.record)
        ),
        format!("结论      {}", report.customer_conclusion.text),
        String::new(),
        "检测项目".to_string(),
    ];
    lines.extend(report.record.module_results.iter().map(|result| {
        let state = module_state_label(module_result_label(result.state));
        let name = pad_display(module_display_name(&result.module_id), name_width);
        let reason = result
            .reason
            .as_deref()
            .map_or(String::new(), |reason| format!("  {reason}"));
        format!("  {}  {}{}", pad_display(state, state_width), name, reason)
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

    #[derive(Debug, Default)]
    struct RecordingProgress {
        events: Vec<ProgressEvent>,
    }

    impl ProgressSink for RecordingProgress {
        fn emit(&mut self, event: ProgressEvent) {
            self.events.push(event);
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

    #[test]
    fn performance_matrix_expands_every_declared_dimension() {
        let plans = fixed_performance_plan();
        let dimensions = plans
            .iter()
            .map(|plan| performance_dimensions(plan).len())
            .collect::<Vec<_>>();
        assert_eq!(dimensions, vec![2, 1, 3, 1, 3, 2, 1]);
        assert_eq!(dimensions.into_iter().sum::<usize>(), 13);
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

    #[test]
    fn agent_workspace_writes_materials_and_classifies_out_of_scope_paths() {
        let spec = fixed_agent_scenarios()
            .into_iter()
            .find(|spec| spec.scenario == crate::agent::AgentScenario::T2A)
            .unwrap();
        let root = std::env::temp_dir().join(format!(
            "agentcheck-cli-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        prepare_agent_workspace(&spec, &root).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("input/projects.json")).unwrap(),
            spec.materials[0].content
        );
        assert!(root.join("workspace").is_dir());

        let mut outcome = AgentRunOutcome {
            input_intact: true,
            ..AgentRunOutcome::default()
        };
        outcome.calls = vec![
            crate::agent::AgentToolCall {
                seq: 1,
                name: "write".into(),
                path: Some("workspace/active.txt".into()),
                has_required_args: true,
                ok: true,
                result_text: String::new(),
            },
            crate::agent::AgentToolCall {
                seq: 2,
                name: "write".into(),
                path: Some("../escape.txt".into()),
                has_required_args: true,
                ok: true,
                result_text: String::new(),
            },
            crate::agent::AgentToolCall {
                seq: 3,
                name: "read".into(),
                path: Some("/etc/passwd".into()),
                has_required_args: true,
                ok: true,
                result_text: String::new(),
            },
        ];
        classify_agent_paths(&root, &mut outcome);
        assert_eq!(outcome.unauthorized_writes, vec!["../escape.txt"]);
        assert_eq!(outcome.out_of_scope_reads, vec!["/etc/passwd"]);
        let _ = fs::remove_dir_all(&root);
    }

    fn mock_server(
        response_status: u16,
        response_body: &'static str,
        connections: usize,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        mock_server_with(response_status, connections, move |_| {
            response_body.to_owned()
        })
    }

    /// 按请求内容生成响应体的 mock：S04 工具请求回合法 tool_calls、
    /// S05 结构化请求回合规 JSON，其余回 "ok"。
    fn spec_mock_server(connections: usize) -> (String, thread::JoinHandle<Vec<String>>) {
        mock_server_with(200, connections, spec_response_body)
    }

    fn spec_response_body(request: &str) -> String {
        let weather = |arguments: &str| json!({"type": "function", "function": {"name": "lookup_weather", "arguments": arguments}});
        let air = |arguments: &str| json!({"type": "function", "function": {"name": "lookup_air_quality", "arguments": arguments}});
        let weather_args = |city: &str, days: u8, alert: bool, locations: &str| {
            format!(
                r#"{{"city":"{city}","days":{days},"unit":"celsius","include_alert":{alert},"locations":{locations},"options":{{"lang":"zh"}}}}"#
            )
        };
        let is_stream = request.contains("\"stream\":true");
        let message = if is_stream && request.contains("\"tools\"") {
            // 流式工具场景：单事件给出完整 tool_calls（parser 兼容 delta/message 两种形态）。
            json!({"tool_calls": [weather(&weather_args("北京", 1, false, "[\"海淀\"]"))]})
        } else if is_stream {
            json!({"content": "STREAM_BEGIN 北京明天晴 STREAM_END"})
        } else if request.contains("\"tool_choice\":\"none\"") {
            json!({"content": "北京明天晴，气温25度。"})
        } else if request.contains("\"tool_choice\":{\"type\":\"function\"") {
            json!({"tool_calls": [weather(&weather_args("上海", 1, false, "[\"浦东\"]"))]})
        } else if request.contains("分别发起两次") {
            json!({"tool_calls": [
                weather(&weather_args("北京", 1, false, "[\"海淀\"]")),
                weather(&weather_args("上海", 1, false, "[\"浦东\"]"))
            ]})
        } else if request.contains("空气质量工具") {
            json!({"tool_calls": [
                weather(&weather_args("北京", 1, false, "[\"海淀\"]")),
                air(r#"{"city":"上海","index":3}"#)
            ]})
        } else if request.contains("\"tools\"") {
            json!({"tool_calls": [weather(&weather_args("北京", 3, true, "[\"海淀\",\"朝阳\"]"))]})
        } else if request.contains("json_schema") || request.contains("json_object") {
            json!({"content": r#"{"city":"杭州","temperature":23,"raining":false,"tags":["沿海","春季"]}"#})
        } else {
            json!({"content": "ok"})
        };
        json!({"choices": [{"message": message, "finish_reason": "stop"}]}).to_string()
    }

    fn mock_server_with(
        response_status: u16,
        connections: usize,
        respond: impl Fn(&str) -> String + Send + 'static,
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
                let request_text = String::from_utf8_lossy(&request[..size]);
                let is_stream = request_text.contains("\"stream\":true");
                let response_body = respond(&request_text);
                let body = if is_stream {
                    format!("data: {response_body}\n\ndata: [DONE]\n\n")
                } else {
                    response_body
                };
                let content_type = if is_stream {
                    "text/event-stream"
                } else {
                    "application/json"
                };
                let response = format!(
                    "HTTP/1.1 {response_status} Test\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
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
    fn agent_report_is_serialized_instead_of_dropped_when_module_evidence_is_added_later() {
        // OMP 不可用时每个场景应记为无效执行而非伪造结果，报告仍须序列化。
        unsafe { std::env::set_var("OMP_BIN", "/nonexistent-agentcheck-omp") };
        let report = {
            let (endpoint, server) = mock_server(
                200,
                r#"{"choices":[{"message":{"role":"assistant","content":"已完成当前回合"},"finish_reason":"stop"}]}"#,
                0,
            );
            let mut executor = LiveExecutor::new_full(
                endpoint,
                "model-a",
                "secret-value",
                std::time::Duration::from_secs(5),
            )
            .unwrap();
            let report = run_with_executor(
                CliRunRequest {
                    selected_modules: Some(vec!["agent".into()]),
                    ..request(None)
                },
                &mut executor,
            )
            .unwrap();
            server.join().unwrap();
            report
        };
        unsafe { std::env::remove_var("OMP_BIN") };
        let evidence = report
            .record
            .evidence
            .iter()
            .find(|item| item.id == "cli-agent-0")
            .unwrap();
        let payload = &evidence.payload["payload"];
        assert!(payload["report"].is_object());
        assert_eq!(
            payload["report"]["scenarios"].as_array().map(Vec::len),
            Some(10)
        );
        assert_eq!(
            payload["report"]["samples"].as_array().map(Vec::len),
            Some(10)
        );
        assert_eq!(report.record.evidence.len(), 11);
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
    fn full_executor_runs_every_specification_sample_and_keeps_report_evidence() {
        let sample_count = crate::specification::specification_plan()
            .iter()
            .map(|plan| plan.samples.len())
            .sum::<usize>();
        // S01 除档位探针外多发一个密度校准请求
        let (endpoint, server) = spec_mock_server(sample_count + 1);
        let mut executor = LiveExecutor::new_full(
            endpoint,
            "model-a",
            "secret-value",
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        let result = executor.execute("specification", &executor_record());
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), sample_count + 1);
        assert_eq!(result.state, ModuleResultState::Pass);
        assert_eq!(result.evidence_payload["planned_samples"], sample_count);
        assert_eq!(result.evidence_payload["executed_samples"], sample_count);
        assert!(result.evidence_payload["report"]["rows"].is_array());
        assert!(!result.evidence_payload.to_string().contains("secret-value"));
    }

    #[test]
    fn full_specification_run_reports_detail_progress_for_every_sample() {
        let sample_count = crate::specification::specification_plan()
            .iter()
            .map(|plan| plan.samples.len())
            .sum::<usize>();
        // S01 除档位探针外多发一个密度校准请求
        let (endpoint, server) = spec_mock_server(sample_count + 1);
        let mut executor = LiveExecutor::new_full(
            endpoint,
            "model-a",
            "secret-value",
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        let mut progress = RecordingProgress::default();
        run_with_executor_reporting(
            CliRunRequest {
                selected_modules: Some(vec!["specification".into()]),
                ..request(None)
            },
            &mut executor,
            &mut progress,
        )
        .unwrap();
        server.join().unwrap();
        let details = progress
            .events
            .iter()
            .filter(|event| event.phase == ProgressPhase::ModuleProgress)
            .collect::<Vec<_>>();
        assert_eq!(
            details.len(),
            sample_count * 2,
            "每个样本上报开始与完成两次进度"
        );
        assert!(
            details
                .iter()
                .all(|event| event.detail_total == Some(sample_count))
        );
        assert_eq!(details[0].detail_index, Some(0));
        assert_eq!(details[0].detail_id.as_deref(), Some("S01"));
        assert!(details[0].message.contains("正在检测"));
        let last = details.last().expect("末尾存在完成事件");
        assert_eq!(last.detail_index, Some(sample_count));
        assert_eq!(last.detail_id.as_deref(), Some("S07"));
        assert!(last.message.contains("已完成"));
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
    fn progress_sink_emits_run_and_module_lifecycle() {
        let mut executor = TestExecutor {
            calls: Vec::new(),
            state: ModuleResultState::Pass,
        };
        let mut progress = RecordingProgress::default();
        run_with_executor_reporting(
            request(Some(vec!["capability".into(), "agent".into()])),
            &mut executor,
            &mut progress,
        )
        .unwrap();
        let phases = progress
            .events
            .iter()
            .map(|event| event.phase.clone())
            .collect::<Vec<_>>();
        assert_eq!(phases[0], ProgressPhase::RunStarted);
        assert_eq!(phases.last(), Some(&ProgressPhase::RunCompleted));
        assert_eq!(
            progress
                .events
                .iter()
                .filter(|event| event.phase == ProgressPhase::ModuleStarted)
                .count(),
            2
        );
        assert_eq!(
            progress
                .events
                .iter()
                .filter(|event| event.phase == ProgressPhase::ModuleCompleted)
                .count(),
            2
        );
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
        assert!(text.contains("待确认"));
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
