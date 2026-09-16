use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::agent::{
    AgentEvent, AgentEventKind, AgentExecutionInput, AgentObservationFacts, AgentOutcome,
    AgentRuntime, AgentScenarioSpec, AgentTool, ExecutionOrigin, PermissionDecision,
    PermissionEffect, PermissionEvent, assess_execution, build_report_for_record,
    fixed_agent_scenarios,
};
use crate::baseline::{
    ActualSnapshot, ActualSource, BaselineScenario,
    build_report_for_record as build_baseline_report, fixed_baseline_catalog,
};
use crate::capability::{
    CapabilityResponse, CapabilitySettings, ExecutionState, build_scorecard,
    fixed_capability_catalog,
};
use crate::conclusion::{CustomerConclusionReport, build_default_customer_report};
use crate::ingress::{ConnectionConfig, redact_endpoint};
use crate::performance::{
    ErrorKind, PerformanceConditions, PerformanceSample, ResponseMode as PerformanceResponseMode,
    RunPhase, TerminalState, Timeline, TokenCountSource,
    build_report_for_record as build_performance_report_for_record, fixed_performance_plan,
};
use crate::records::{
    AttemptKind, EventInput, EventKind, ModuleResult, ModuleResultState, OverallConclusion,
    add_attempt, add_event, add_evidence, complete_run, create_run, set_module_result,
    set_overall_conclusion, start_run, stop_run,
};
use crate::records::{AttemptRecord, CreateRunInput, DetectionRecord};
use crate::specification::{
    AttemptKind as SpecificationAttemptKind, EvidenceOrigin, SpecStatus, SpecificationObservation,
    build_report, seven_category_plan,
};
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
        let plans = seven_category_plan();
        let total_samples: usize = plans.iter().map(|plan| plan.samples.len()).sum();
        let mut observations = Vec::new();
        let mut evidence = Vec::new();
        for plan in plans {
            for sample_id in plan.samples {
                progress(ProgressDetail {
                    index: evidence.len(),
                    total: total_samples,
                    id: plan.category.id().into(),
                    message: format!(
                        "正在检测规格样本 {} / {}",
                        evidence.len() + 1,
                        total_samples
                    ),
                });
                let request = ChatCompletionsRequest {
                    module_id: "specification".into(),
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
                progress(ProgressDetail {
                    index: evidence.len(),
                    total: total_samples,
                    id: plan.category.id().into(),
                    message: format!("已完成规格样本 {} / {}", evidence.len(), total_samples),
                });
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
            }
        }
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
            evidence_payload: json!({"version": crate::specification::SPECIFICATION_VERSION, "planned_samples": evidence.len(), "executed_samples": evidence.len(), "report": report, "evidence": evidence}),
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
                message: format!("正在检测能力样本 {} / {}", evidence.len() + 1, samples.len()),
            });
            let request = ChatCompletionsRequest {
                module_id: "capability".into(),
                prompt: sample.prompt.clone(),
                max_tokens: 256,
                stream: false,
            };
            let response = self.transport.send(request.clone());
            let text = completion_text(response.parsed.as_ref());
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
                    tool_calls: Vec::new(),
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
        'plans: for plan in plans {
            for (mode, input_tokens, target_output_tokens, target_concurrency) in
                performance_dimensions(&plan)
            {
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
                for warmup_index in 0..plan.warmup_count {
                    let work = PerformanceWorkItem {
                        sample_id: format!(
                            "{}-{}-{}-{}-{}-warmup-{}-{}",
                            plan.category.id(),
                            format_workload(plan.workload),
                            format_mode(mode),
                            input_tokens,
                            target_output_tokens,
                            target_concurrency,
                            warmup_index + 1
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
                        actual_concurrency: 1,
                        dispatched_at_ms: run_started.elapsed().as_millis() as u64,
                    };
                    let mut observations = self.run_performance_batch(vec![work]);
                    let observation = observations.pop().expect("one warmup request");
                    let error_kind = self.record_performance_observation(
                        record,
                        observation,
                        &mut samples,
                        &mut evidence,
                    );
                    if is_environment_error(error_kind) {
                        consecutive_environment_failures += 1;
                    } else {
                        consecutive_environment_failures = 0;
                    }
                    if consecutive_environment_failures >= 3 {
                        circuit_breaker_reason = Some(
                            "连续 3 次请求出现网络、认证、限流或服务错误，已停止性能检测".into(),
                        );
                        break 'plans;
                    }
                }

                let mut remaining = plan.formal_request_limit;
                let mut formal_index = 0_u32;
                while remaining > 0 {
                    if plan.max_duration_ms.is_some_and(|limit| {
                        dimension_started.elapsed().as_millis() as u64 >= limit
                    }) {
                        break;
                    }
                    if plan.dispatch_window_ms.is_some_and(|limit| {
                        formal_index > 0 && dimension_started.elapsed().as_millis() as u64 >= limit
                    }) {
                        break;
                    }
                    let batch_size = target_concurrency.min(remaining).max(1);
                    let dispatched_at_ms = run_started.elapsed().as_millis() as u64;
                    let work = (0..batch_size)
                        .map(|batch_index| {
                            let index = formal_index + batch_index;
                            PerformanceWorkItem {
                                sample_id: format!(
                                    "{}-{}-{}-{}-{}-{}-{:04}",
                                    plan.category.id(),
                                    format_workload(plan.workload),
                                    format_mode(mode),
                                    input_tokens,
                                    target_output_tokens,
                                    target_concurrency,
                                    index + 1
                                ),
                                request: performance_request(
                                    mode,
                                    plan.category.id(),
                                    plan.workload,
                                    index,
                                    input_tokens,
                                    target_output_tokens,
                                ),
                                workload: plan.workload,
                                input_tokens,
                                target_output_tokens,
                                phase: RunPhase::Formal,
                                target_concurrency,
                                actual_concurrency: batch_size,
                                dispatched_at_ms,
                            }
                        })
                        .collect();
                    for observation in self.run_performance_batch(work) {
                        let error_kind = self.record_performance_observation(
                            record,
                            observation,
                            &mut samples,
                            &mut evidence,
                        );
                        if is_environment_error(error_kind) {
                            consecutive_environment_failures += 1;
                        } else {
                            consecutive_environment_failures = 0;
                        }
                        if consecutive_environment_failures >= 3 {
                            circuit_breaker_reason = Some(
                                "连续 3 次请求出现网络、认证、限流或服务错误，已停止性能检测"
                                    .into(),
                            );
                            break 'plans;
                        }
                    }
                    formal_index += batch_size;
                    remaining -= batch_size;
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
                timeout_ms: 300_000,
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
                "已执行 {} 个性能正式样本；{}{}",
                formal_count,
                circuit_breaker_reason
                    .as_deref()
                    .unwrap_or("未触发连续失败熔断"),
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
                message: format!(
                    "正在执行智能体场景 {} / {}",
                    executed + 1,
                    total_scenarios
                ),
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
                omp_version: "chat-completions-agent-adapter/v1".into(),
                build_fingerprint: "runtime-recorded".into(),
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
        let mut messages = vec![json!({
            "role": "user",
            "content": format!(
                "{}\n工作区：{}\n请使用声明工具完成任务，目标产物路径为 {}，完成后用最终消息说明结果。",
                spec.prompt, spec.workspace.root, spec.expected_artifact.path
            )
        })];
        let tools = agent_tool_definitions(spec);
        let mut files = spec
            .workspace
            .initial_files
            .iter()
            .map(|file| (file.path.clone(), file.content.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut events = Vec::new();
        let mut permission_events = Vec::new();
        let mut evidence = Vec::new();
        let mut final_message = None;
        let mut saw_tool = false;
        let mut saw_tool_return = false;
        let mut valid_arguments = true;
        let mut permission_respected = true;
        let mut tool_failure = false;
        let mut tool_failure_recovered = false;
        let mut multi_turn = false;
        let mut sequence = 0;

        for _ in 0..6 {
            let turn = self
                .transport
                .send_agent_turn(messages.clone(), tools.clone());
            evidence.push(json!({
                "messages": messages,
                "tools": tools,
                "response": turn.response.parsed.clone().unwrap_or_else(|| Value::String(turn.response.body.clone())),
                "status": turn.response.status,
                "elapsed_ms": turn.response.elapsed_ms,
                "attempts": turn.response.attempts,
            }));
            if turn.response.error.is_some()
                || !turn
                    .response
                    .status
                    .is_some_and(|status| (200..300).contains(&status))
            {
                let execution = crate::agent::invalid_execution(
                    spec.workspace.task_id.clone(),
                    spec.scenario,
                    1,
                    crate::agent::AgentAttemptKind::Initial,
                    ExecutionOrigin::RealOmp,
                    "真实 Agent 工具回合无效；服务未提供可用的闭环响应",
                    Vec::new(),
                );
                return (
                    execution,
                    json!({"sample_id": spec.workspace.task_id, "turns": evidence}),
                );
            }
            let Some(message) = turn.message else {
                let execution = crate::agent::invalid_execution(
                    spec.workspace.task_id.clone(),
                    spec.scenario,
                    1,
                    crate::agent::AgentAttemptKind::Initial,
                    ExecutionOrigin::RealOmp,
                    "响应缺少 assistant message，无法判定 Agent 终态",
                    Vec::new(),
                );
                return (
                    execution,
                    json!({"sample_id": spec.workspace.task_id, "turns": evidence}),
                );
            };
            messages.push(message.clone());
            if turn.tool_calls.is_empty() {
                final_message = turn.text.filter(|text| !text.trim().is_empty());
                break;
            }
            if tool_failure {
                tool_failure_recovered = true;
            }
            saw_tool = true;
            multi_turn = true;
            for call in turn.tool_calls {
                sequence += 1;
                let call_id = call
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("tool-call")
                    .to_owned();
                let function = call.get("function").cloned().unwrap_or_default();
                let name = function
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let arguments = function
                    .get("arguments")
                    .and_then(Value::as_str)
                    .and_then(|value| serde_json::from_str::<Value>(value).ok())
                    .unwrap_or(Value::Null);
                let (result, valid, allowed) =
                    apply_agent_tool(spec, name, &arguments, &mut files, &mut permission_events);
                valid_arguments &= valid;
                permission_respected &= allowed;
                tool_failure |= result.get("ok") == Some(&Value::Bool(false));
                events.push(AgentEvent {
                    id: format!("{}-call-{sequence}", spec.workspace.task_id),
                    kind: AgentEventKind::ToolCall,
                    sequence,
                    summary: format!("{name} {arguments}"),
                    path: arguments
                        .get("path")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    operation: Some(name.to_owned()),
                    incident_id: None,
                    evidence_refs: Vec::new(),
                });
                sequence += 1;
                events.push(AgentEvent {
                    id: format!("{}-return-{sequence}", spec.workspace.task_id),
                    kind: AgentEventKind::ToolReturn,
                    sequence,
                    summary: result.to_string(),
                    path: arguments
                        .get("path")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    operation: Some(name.to_owned()),
                    incident_id: None,
                    evidence_refs: Vec::new(),
                });
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": result.to_string(),
                }));
                saw_tool_return = true;
            }
        }

        let actual_content = files.get(&spec.expected_artifact.path);
        let artifact = Some(crate::agent::ArtifactObservation {
            path: spec.expected_artifact.path.clone(),
            exists: actual_content.is_some(),
            content_digest: actual_content.map(|content| digest_text(content)),
            content_matches: actual_content
                .map(|content| content == &spec.expected_artifact.content),
            evidence_refs: Vec::new(),
        });
        let facts = AgentObservationFacts {
            task_rules_followed: saw_tool.then_some(permission_respected && valid_arguments),
            tool_and_arguments_correct: saw_tool.then_some(valid_arguments),
            tool_return_used: saw_tool.then_some(saw_tool_return),
            multi_turn_state_preserved: multi_turn
                .then_some(saw_tool_return && final_message.is_some()),
            tool_failure_handled: tool_failure.then_some(tool_failure_recovered),
            missing_information_handled: None,
            permission_respected: saw_tool.then_some(permission_respected),
            delivery_and_end_correct: Some(
                artifact
                    .as_ref()
                    .is_some_and(|value| value.content_matches == Some(true))
                    && final_message.is_some(),
            ),
        };
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
            final_message,
            evidence_refs: Vec::new(),
        });
        (
            execution,
            json!({"sample_id": spec.workspace.task_id, "turns": evidence}),
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
        for scenario in BaselineScenario::ALL {
            progress(ProgressDetail {
                index: evidence.len(),
                total: total_scenarios,
                id: scenario.id().into(),
                message: format!(
                    "正在检测基线场景 {} / {}",
                    evidence.len() + 1,
                    total_scenarios
                ),
            });
            let request = ChatCompletionsRequest {
                module_id: "baseline".into(),
                prompt: format!("基线场景 {}：{}", scenario.id(), scenario.title()),
                max_tokens: 256,
                stream: matches!(
                    scenario,
                    BaselineScenario::StreamEnvelope
                        | BaselineScenario::StreamChoiceContainer
                        | BaselineScenario::StreamDelta
                        | BaselineScenario::StreamToolDelta
                        | BaselineScenario::StreamUsage
                ),
            };
            let response = self.transport.send(request.clone());
            let source = ActualSnapshot::from_raw(
                response.body.clone(),
                ActualSource::RealService,
                None,
                false,
                Vec::new(),
            );
            observations.push(crate::baseline::BaselineObservation {
                scenario,
                actual: source,
            });
            evidence.push(json!({"scenario": scenario.id(), "payload": self.transport.evidence_payload(&request, &response)}));
            progress(ProgressDetail {
                index: evidence.len(),
                total: total_scenarios,
                id: scenario.id().into(),
                message: format!("已完成基线场景 {} / {}", evidence.len(), total_scenarios),
            });
        }
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

fn performance_request(
    mode: PerformanceResponseMode,
    category: &str,
    workload: crate::performance::WorkloadKind,
    index: u32,
    input_tokens: u32,
    target_output_tokens: u32,
) -> ChatCompletionsRequest {
    ChatCompletionsRequest {
        module_id: "performance".into(),
        prompt: format!(
            "性能计划 {category} {:?} 第 {} 次；输入目标 {input_tokens}；输出目标 {target_output_tokens}；返回短文本并保持正常结束。",
            workload,
            index + 1
        ),
        max_tokens: target_output_tokens,
        stream: mode == PerformanceResponseMode::Streaming,
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
    let terminal_state = if valid {
        TerminalState::NaturalEnd
    } else if response
        .error
        .as_deref()
        .is_some_and(|error| error.to_ascii_lowercase().contains("timeout"))
    {
        TerminalState::Timeout
    } else {
        TerminalState::Error
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
    let complete = valid.then(|| absolute(elapsed));
    let length_target_met =
        actual_output_tokens.is_some_and(|value| value >= item.request.max_tokens);
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

fn agent_tool_definitions(spec: &AgentScenarioSpec) -> Vec<Value> {
    spec.tools
        .iter()
        .map(|permission| {
            let (name, description, properties) = match permission.tool {
                AgentTool::ReadFile => (
                    "read_file",
                    "读取声明工作区内的文件",
                    json!({"path": {"type": "string"}}),
                ),
                AgentTool::WriteFile => (
                    "write_file",
                    "写入声明工作区内的结果文件",
                    json!({"path": {"type": "string"}, "content": {"type": "string"}}),
                ),
                AgentTool::ListDirectory => (
                    "list_directory",
                    "列出声明目录下的文件",
                    json!({"path": {"type": "string"}}),
                ),
                AgentTool::MoveFile | AgentTool::CopyFile => (
                    "unsupported",
                    "当前版本不开放该工具",
                    json!({"path": {"type": "string"}}),
                ),
            };
            json!({
                "type": "function",
                "function": {
                    "name": name,
                    "description": description,
                    "parameters": {
                        "type": "object",
                        "properties": properties,
                        "required": ["path"],
                        "additionalProperties": false
                    }
                }
            })
        })
        .collect()
}

fn apply_agent_tool(
    spec: &AgentScenarioSpec,
    name: &str,
    arguments: &Value,
    files: &mut BTreeMap<String, String>,
    permission_events: &mut Vec<PermissionEvent>,
) -> (Value, bool, bool) {
    let raw_path = arguments
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let path = agent_path(&spec.workspace.root, raw_path);
    let tool = match name {
        "read_file" => AgentTool::ReadFile,
        "write_file" => AgentTool::WriteFile,
        "list_directory" => AgentTool::ListDirectory,
        _ => return (json!({"ok": false, "error": "unknown_tool"}), false, false),
    };
    let safe = path.starts_with(&format!("{}/", spec.workspace.root))
        && !path.split('/').any(|part| part == "..")
        && !path.starts_with(&format!("{}/", spec.workspace.layout.expected_dir));
    let permitted = spec.tools.iter().any(|permission| {
        permission.tool == tool && path.starts_with(&format!("{}/", permission.root))
    });
    let allowed = safe && permitted;
    permission_events.push(PermissionEvent {
        path: path.clone(),
        operation: name.to_owned(),
        decision: if allowed {
            PermissionDecision::Allowed
        } else {
            PermissionDecision::Denied
        },
        effect: if !allowed {
            PermissionEffect::None
        } else if tool == AgentTool::WriteFile {
            PermissionEffect::Write
        } else {
            PermissionEffect::Read
        },
        evidence_refs: Vec::new(),
    });
    if !allowed {
        return (
            json!({"ok": false, "error": "permission_denied", "path": path}),
            true,
            false,
        );
    }
    match tool {
        AgentTool::ReadFile => match files.get(&path) {
            Some(content) => (
                json!({"ok": true, "path": path, "content": content}),
                true,
                true,
            ),
            None => (
                json!({"ok": false, "error": "not_found", "path": path}),
                true,
                true,
            ),
        },
        AgentTool::WriteFile => {
            let Some(content) = arguments.get("content").and_then(Value::as_str) else {
                return (
                    json!({"ok": false, "error": "missing_content", "path": path}),
                    false,
                    true,
                );
            };
            files.insert(path.clone(), content.to_owned());
            (json!({"ok": true, "path": path}), true, true)
        }
        AgentTool::ListDirectory => {
            let prefix = format!("{}/", path.trim_end_matches('/'));
            let entries = files
                .keys()
                .filter(|file| file.starts_with(&prefix))
                .cloned()
                .collect::<Vec<_>>();
            (
                json!({"ok": true, "path": path, "entries": entries}),
                true,
                true,
            )
        }
        AgentTool::MoveFile | AgentTool::CopyFile => unreachable!(),
    }
}

fn agent_path(root: &str, raw_path: &str) -> String {
    let raw_path = raw_path.trim_start_matches('/');
    if raw_path.starts_with("runs/") {
        raw_path.to_owned()
    } else {
        format!("{root}/{raw_path}")
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
            message: format!("开始检测 {module_id}"),
            detail_index: None,
            detail_total: None,
            detail_id: None,
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
            });
        };
        let result =
            executor.execute_with_record_progress(module_id, &mut record, &mut detail_progress);
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
        assert_eq!(dimensions, vec![2, 1, 4, 1, 3, 2, 3]);
        assert_eq!(dimensions.into_iter().sum::<usize>(), 16);
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
    fn controlled_agent_tools_write_expected_artifact_and_deny_hidden_paths() {
        let spec = fixed_agent_scenarios()
            .into_iter()
            .find(|spec| spec.scenario == crate::agent::AgentScenario::T2A)
            .unwrap();
        let mut files = spec
            .workspace
            .initial_files
            .iter()
            .map(|file| (file.path.clone(), file.content.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut permissions = Vec::new();
        let (written, valid, allowed) = apply_agent_tool(
            &spec,
            "write_file",
            &json!({
                "path": spec.expected_artifact.path,
                "content": spec.expected_artifact.content,
            }),
            &mut files,
            &mut permissions,
        );
        assert_eq!(written["ok"], true);
        assert!(valid);
        assert!(allowed);
        assert_eq!(
            files.get(&spec.expected_artifact.path),
            Some(&spec.expected_artifact.content)
        );

        let (denied, valid, allowed) = apply_agent_tool(
            &spec,
            "read_file",
            &json!({"path": format!("{}/secret.txt", spec.workspace.layout.expected_dir)}),
            &mut files,
            &mut permissions,
        );
        assert_eq!(denied["error"], "permission_denied");
        assert!(valid);
        assert!(!allowed);
        assert_eq!(permissions.last().unwrap().effect, PermissionEffect::None);

        let _ = apply_agent_tool(
            &spec,
            "write_file",
            &json!({
                "path": format!("{}/secret.txt", spec.workspace.layout.expected_dir),
                "content": "should not be written",
            }),
            &mut files,
            &mut permissions,
        );
        assert_eq!(permissions.last().unwrap().effect, PermissionEffect::None);
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
                let request_text = String::from_utf8_lossy(&request[..size]);
                let is_stream = request_text.contains("\"stream\":true");
                let body = if is_stream {
                    format!("data: {response_body}\n\ndata: [DONE]\n\n")
                } else {
                    response_body.to_owned()
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
        let (endpoint, server) = mock_server(
            200,
            r#"{"choices":[{"message":{"role":"assistant","content":"已完成当前回合"},"finish_reason":"stop"}]}"#,
            10,
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
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 10);
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
        let sample_count = crate::specification::seven_category_plan()
            .iter()
            .map(|plan| plan.samples.len())
            .sum::<usize>();
        let (endpoint, server) = mock_server(
            200,
            r#"{"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}]}"#,
            sample_count,
        );
        let mut executor = LiveExecutor::new_full(
            endpoint,
            "model-a",
            "secret-value",
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        let result = executor.execute("specification", &executor_record());
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), sample_count);
        assert_eq!(result.state, ModuleResultState::Pass);
        assert_eq!(result.evidence_payload["planned_samples"], sample_count);
        assert_eq!(result.evidence_payload["executed_samples"], sample_count);
        assert!(result.evidence_payload["report"]["rows"].is_array());
        assert!(!result.evidence_payload.to_string().contains("secret-value"));
    }

    #[test]
    fn full_specification_run_reports_detail_progress_for_every_sample() {
        let sample_count = crate::specification::seven_category_plan()
            .iter()
            .map(|plan| plan.samples.len())
            .sum::<usize>();
        let (endpoint, server) = mock_server(
            200,
            r#"{"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}]}"#,
            sample_count,
        );
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
        assert!(details.iter().all(|event| event.detail_total == Some(sample_count)));
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
