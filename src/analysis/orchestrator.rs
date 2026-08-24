use std::fmt::Write as FmtWrite;
use std::fs::File;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::Local;
use serde::Serialize;
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::client::{AnalysisClient, AnalysisTarget, ClientError, ModelAnalysisClient};
use super::evidence_reader::{EvidenceError, EvidenceSource, ParsedEvidence, ParsedRequest, read};
use super::packet::{EvidencePacket, PacketError, bounded_excerpt, default_batches};
use super::prompt::{PROMPT_VERSION, build_prompt, build_repair_prompt};
use super::validator::{
    CandidateStatus, DecisionSource, ValidatedReview, ValidatedStatus, validate_candidates,
};
use crate::private_file::create_new_private_file;
use crate::protocol::{AuthMode, Protocol};
use crate::redaction::Redactor;

pub const SELF_ANALYSIS_SCHEMA_VERSION: &str = "llm-capability-doctor.self-analysis.v2";
pub const SELF_ANALYSIS_PROVENANCE: &str = "TARGET_MODEL_SELF_ANALYSIS";

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub struct AnalysisConnectionSettings {
    pub url: Url,
    pub model: String,
    pub api_key: String,
    pub timeout: Duration,
    pub insecure: bool,
}

impl AnalysisConnectionSettings {
    pub fn for_run(
        self,
        log_path: PathBuf,
        protocol: Protocol,
        auth_mode: AuthMode,
    ) -> AnalysisSettings {
        AnalysisSettings {
            log_path,
            url: self.url,
            model: self.model,
            api_key: self.api_key,
            timeout: self.timeout,
            insecure: self.insecure,
            protocol,
            auth_mode,
        }
    }
}

pub struct AnalysisSettings {
    pub log_path: PathBuf,
    pub url: Url,
    pub model: String,
    pub api_key: String,
    pub timeout: Duration,
    pub insecure: bool,
    pub protocol: Protocol,
    pub auth_mode: AuthMode,
}

pub struct AnalysisOutcome {
    pub path: PathBuf,
    pub markdown_path: PathBuf,
    pub available_count: usize,
    pub pass_count: usize,
    pub fail_count: usize,
    pub unavailable_count: usize,
    pub cancelled: bool,
}

#[derive(Debug, Error)]
pub enum AnalysisError {
    #[error(transparent)]
    Evidence(#[from] EvidenceError),
    #[error(transparent)]
    Packet(#[from] PacketError),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("self-analysis prompt could not be encoded: {0}")]
    Prompt(#[from] serde_json::Error),
    #[error("self-analysis output failed: {0}")]
    Output(#[from] std::io::Error),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SelfAnalysisArtifact {
    schema_version: &'static str,
    collector_version: String,
    evidence_schema_version: String,
    generated_at: String,
    source: EvidenceSource,
    target: AnalysisTargetMetadata,
    prompt_version: &'static str,
    provenance: &'static str,
    tests: Vec<AnalysisTestResult>,
    batches: Vec<BatchResult>,
    counts: AnalysisCounts,
    #[serde(skip)]
    concurrency_waves: Vec<ConcurrencyWave>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AnalysisTargetMetadata {
    endpoint: String,
    requested_model: String,
    detected_protocol: String,
    authentication_mode: String,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum AnalysisState {
    Available,
    AnalysisUnavailable,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AnalysisTestResult {
    test_id: String,
    analysis_state: AnalysisState,
    candidate_status: Option<CandidateStatus>,
    validated_status: Option<ValidatedStatus>,
    decision_source: Option<DecisionSource>,
    observations: Vec<String>,
    failure_cause: Option<String>,
    evidence_refs: Vec<String>,
    limitations: Vec<String>,
    validation_notes: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchResult {
    test_ids: Vec<String>,
    attempts: usize,
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AnalysisCounts {
    pass: usize,
    fail: usize,
    unavailable: usize,
}

#[derive(Clone, Debug)]
struct ConcurrencyWave {
    concurrency: usize,
    total_requests: usize,
    succeeded: usize,
    failed: usize,
    average_response_time_seconds: Option<f64>,
    failures: Vec<ConcurrencyFailure>,
}

#[derive(Clone, Debug)]
struct ConcurrencyFailure {
    request_id: String,
    error: String,
}

struct ProcessedBatch {
    attempts: usize,
    reviews: Option<Vec<ValidatedReview>>,
    error: Option<String>,
}

pub async fn analyze(
    settings: AnalysisSettings,
    cancellation: CancellationToken,
) -> Result<AnalysisOutcome, AnalysisError> {
    let client = ModelAnalysisClient::new(AnalysisTarget {
        url: settings.url.clone(),
        model: settings.model.clone(),
        api_key: settings.api_key.clone(),
        timeout: settings.timeout,
        insecure: settings.insecure,
        protocol: settings.protocol,
        auth_mode: settings.auth_mode,
    })?;
    analyze_with_client(settings, &client, cancellation).await
}

pub(crate) async fn analyze_with_client<C: AnalysisClient>(
    settings: AnalysisSettings,
    client: &C,
    cancellation: CancellationToken,
) -> Result<AnalysisOutcome, AnalysisError> {
    let redactor = Redactor::new(&settings.api_key, &settings.url);
    let evidence = read(&settings.log_path, &redactor)?;
    let batches = default_batches(&evidence).await?;
    let mut tests = Vec::with_capacity(crate::catalog::CATALOG.len());
    let mut batch_results = Vec::with_capacity(batches.len());
    let mut cancelled = false;

    for batch in batches {
        let test_ids: Vec<String> = batch.iter().map(|packet| packet.test_id.clone()).collect();
        if cancellation.is_cancelled() {
            cancelled = true;
            let error = "analysis cancelled before batch started".to_owned();
            tests.extend(unavailable_results(&batch, &error));
            batch_results.push(BatchResult {
                test_ids,
                attempts: 0,
                error: Some(error),
            });
            continue;
        }

        let processed = process_batch(client, &batch, cancellation.child_token()).await?;
        if cancellation.is_cancelled() {
            cancelled = true;
        }
        let safe_error = processed
            .error
            .as_deref()
            .map(|error| redactor.redact_text(error));
        match processed.reviews {
            Some(reviews) => tests.extend(
                reviews
                    .into_iter()
                    .map(|review| available_result(review, &redactor)),
            ),
            None => tests.extend(unavailable_results(
                &batch,
                safe_error.as_deref().unwrap_or("analysis unavailable"),
            )),
        }
        batch_results.push(BatchResult {
            test_ids,
            attempts: processed.attempts,
            error: safe_error,
        });
    }

    let counts = AnalysisCounts {
        pass: tests
            .iter()
            .filter(|result| result.validated_status == Some(ValidatedStatus::Pass))
            .count(),
        fail: tests
            .iter()
            .filter(|result| result.validated_status == Some(ValidatedStatus::Fail))
            .count(),
        unavailable: tests
            .iter()
            .filter(|result| matches!(result.analysis_state, AnalysisState::AnalysisUnavailable))
            .count(),
    };
    let collector_version = evidence.run["script_version"].clone();
    let evidence_schema_version = evidence.run["log_schema"].clone();
    let concurrency_waves = summarize_concurrency(&evidence);
    let mut source = evidence.source;
    source.path = PathBuf::from(redactor.redact_text(&source.path.to_string_lossy()));
    source.file_name = redactor.redact_text(&source.file_name);
    let artifact = SelfAnalysisArtifact {
        schema_version: SELF_ANALYSIS_SCHEMA_VERSION,
        collector_version,
        evidence_schema_version,
        generated_at: Local::now().to_rfc3339(),
        source,
        target: AnalysisTargetMetadata {
            endpoint: redact_endpoint_metadata(&settings.url, &redactor),
            requested_model: redactor.redact_text(&settings.model),
            detected_protocol: settings.protocol.to_string(),
            authentication_mode: settings.auth_mode.to_string(),
        },
        prompt_version: PROMPT_VERSION,
        provenance: SELF_ANALYSIS_PROVENANCE,
        tests,
        batches: batch_results,
        counts,
        concurrency_waves,
    };
    let pass_count = artifact.counts.pass;
    let fail_count = artifact.counts.fail;
    let unavailable_count = artifact.counts.unavailable;
    let path = write_artifact(&settings.log_path, &artifact)?;
    let markdown = render_markdown(&artifact);
    let markdown_path = write_markdown_artifact(&settings.log_path, markdown.as_bytes())?;
    Ok(AnalysisOutcome {
        path,
        markdown_path,
        available_count: pass_count + fail_count,
        pass_count,
        fail_count,
        unavailable_count,
        cancelled,
    })
}

async fn process_batch<C: AnalysisClient>(
    client: &C,
    batch: &[EvidencePacket],
    cancellation: CancellationToken,
) -> Result<ProcessedBatch, AnalysisError> {
    let original_prompt = build_prompt(batch)?;
    let mut prompt = original_prompt.clone();
    for attempts in 1..=2 {
        if cancellation.is_cancelled() {
            return Ok(ProcessedBatch {
                attempts: attempts - 1,
                reviews: None,
                error: Some("analysis cancelled".into()),
            });
        }
        match client.analyze(&prompt, cancellation.child_token()).await {
            Ok(envelope) => match validate_candidates(batch, envelope) {
                Ok(reviews) => {
                    return Ok(ProcessedBatch {
                        attempts,
                        reviews: Some(reviews),
                        error: None,
                    });
                }
                Err(errors) if attempts == 1 => {
                    prompt = build_repair_prompt(&original_prompt, &errors)?;
                }
                Err(errors) => {
                    return Ok(ProcessedBatch {
                        attempts,
                        reviews: None,
                        error: Some(errors.join("; ")),
                    });
                }
            },
            Err(error) if attempts == 1 && repairable_candidate_response(&error) => {
                prompt = build_repair_prompt(&original_prompt, &[error.to_string()])?;
            }
            Err(error) => {
                return Ok(ProcessedBatch {
                    attempts,
                    reviews: None,
                    error: Some(error.to_string()),
                });
            }
        }
    }
    unreachable!("batch loop always returns")
}

fn repairable_candidate_response(error: &ClientError) -> bool {
    matches!(
        error,
        ClientError::InvalidJson(_)
            | ClientError::InvalidResponseEnvelope(_)
            | ClientError::MissingAssistantContent
    )
}

fn available_result(review: ValidatedReview, redactor: &Redactor) -> AnalysisTestResult {
    AnalysisTestResult {
        test_id: review.test_id,
        analysis_state: AnalysisState::Available,
        candidate_status: Some(review.candidate_status),
        validated_status: Some(review.validated_status),
        decision_source: Some(review.decision_source),
        observations: redact_values(review.observations, redactor),
        failure_cause: review
            .failure_cause
            .map(|value| redactor.redact_text(&value)),
        evidence_refs: redact_values(review.evidence_refs, redactor),
        limitations: redact_values(review.limitations, redactor),
        validation_notes: redact_values(review.validation_notes, redactor),
    }
}

fn redact_values(values: Vec<String>, redactor: &Redactor) -> Vec<String> {
    values
        .into_iter()
        .map(|value| redactor.redact_text(&value))
        .collect()
}

fn redact_endpoint_metadata(url: &Url, redactor: &Redactor) -> String {
    let mut safe = url.clone();
    if !safe.username().is_empty() {
        safe.set_username("[REDACTED]")
            .expect("HTTP endpoint URLs support userinfo");
    }
    if safe.password().is_some() {
        safe.set_password(Some("[REDACTED]"))
            .expect("HTTP endpoint URLs support userinfo");
    }
    let query_keys: Vec<String> = safe
        .query_pairs()
        .map(|(key, _)| key.into_owned())
        .collect();
    safe.set_query(None);
    if !query_keys.is_empty() {
        let mut query = safe.query_pairs_mut();
        for key in query_keys {
            query.append_pair(&key, "[REDACTED]");
        }
    }
    redactor.redact_url(&safe)
}

fn unavailable_results(batch: &[EvidencePacket], error: &str) -> Vec<AnalysisTestResult> {
    batch
        .iter()
        .map(|packet| AnalysisTestResult {
            test_id: packet.test_id.clone(),
            analysis_state: AnalysisState::AnalysisUnavailable,
            candidate_status: None,
            validated_status: None,
            decision_source: None,
            observations: Vec::new(),
            failure_cause: None,
            evidence_refs: Vec::new(),
            limitations: vec![error.to_owned()],
            validation_notes: vec!["no validated self-analysis candidate was produced".into()],
        })
        .collect()
}

fn summarize_concurrency(evidence: &ParsedEvidence) -> Vec<ConcurrencyWave> {
    let mut requests_by_wave: std::collections::BTreeMap<usize, Vec<&ParsedRequest>> =
        std::collections::BTreeMap::new();
    for request in evidence.requests.values() {
        if let Some(concurrency) = concurrency_level(&request.request_id) {
            requests_by_wave
                .entry(concurrency)
                .or_default()
                .push(request);
        }
    }

    [4, 8, 16, 32]
        .into_iter()
        .map(|concurrency| {
            let requests = requests_by_wave.remove(&concurrency).unwrap_or_default();
            let succeeded = requests
                .iter()
                .filter(|request| concurrency_request_succeeded(request))
                .count();
            let times: Vec<f64> = requests
                .iter()
                .filter_map(|request| request.metrics.get("time_total"))
                .filter_map(|value| value.parse::<f64>().ok())
                .filter(|value| value.is_finite() && *value >= 0.0)
                .collect();
            let failures = requests
                .iter()
                .filter(|request| !concurrency_request_succeeded(request))
                .map(|request| ConcurrencyFailure {
                    request_id: request.request_id.clone(),
                    error: concurrency_failure_reason(request),
                })
                .collect();
            ConcurrencyWave {
                concurrency,
                total_requests: requests.len(),
                succeeded,
                failed: requests.len().saturating_sub(succeeded),
                average_response_time_seconds: (!times.is_empty())
                    .then(|| times.iter().sum::<f64>() / times.len() as f64),
                failures,
            }
        })
        .collect()
}

fn concurrency_level(request_id: &str) -> Option<usize> {
    let suffix = request_id.strip_prefix("test-057-c")?;
    let (concurrency, index) = suffix.split_once('-')?;
    let concurrency = concurrency.parse::<usize>().ok()?;
    let index = index.parse::<usize>().ok()?;
    (matches!(concurrency, 4 | 8 | 16 | 32) && index > 0).then_some(concurrency)
}

fn concurrency_request_succeeded(request: &ParsedRequest) -> bool {
    let http_success = request
        .metrics
        .get("http_status")
        .and_then(|value| value.parse::<u16>().ok())
        .is_some_and(|status| (200..300).contains(&status));
    let transport_success = request
        .metadata
        .get("transport_outcome")
        .is_some_and(|outcome| matches!(outcome.as_str(), "completed_eof" | "protocol_terminated"));
    http_success && transport_success && !request.response_body.trim().is_empty()
}

fn concurrency_failure_reason(request: &ParsedRequest) -> String {
    let mut reasons = Vec::new();
    let http_status = request
        .metrics
        .get("http_status")
        .and_then(|value| value.parse::<u16>().ok());
    if !http_status.is_some_and(|status| (200..300).contains(&status)) {
        reasons.push(match http_status {
            Some(status) => format!("HTTP {status}"),
            None => "HTTP status unavailable".into(),
        });
    }
    if let Some(outcome) = request.metadata.get("transport_outcome")
        && !matches!(outcome.as_str(), "completed_eof" | "protocol_terminated")
    {
        reasons.push(format!("transport_outcome={outcome}"));
    }
    if request.response_body.trim().is_empty() {
        reasons.push("response body empty".into());
    }
    if !request.stderr.trim().is_empty() {
        reasons.push(format!(
            "stderr: {}",
            bounded_excerpt(request.stderr.trim(), 512)
        ));
    }
    if reasons.is_empty() {
        "request did not meet success criteria".into()
    } else {
        reasons.join("; ")
    }
}

fn markdown_cell(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '|' => escaped.push_str("\\|"),
            '\r' | '\n' => escaped.push(' '),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn result_status(result: &AnalysisTestResult) -> &'static str {
    match result.validated_status {
        Some(ValidatedStatus::Pass) => "PASS",
        Some(ValidatedStatus::Fail) => "FAIL",
        None => "ANALYSIS_UNAVAILABLE",
    }
}

fn decision_source_label(source: Option<DecisionSource>) -> &'static str {
    match source {
        Some(DecisionSource::TargetModel) => "TARGET_MODEL",
        None => "-",
    }
}

fn result_reason(result: &AnalysisTestResult) -> String {
    match result_status(result) {
        "PASS" => "-".into(),
        "FAIL" => result
            .failure_cause
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("模型未提供 failureCause")
            .into(),
        _ => {
            let limitation = result
                .limitations
                .iter()
                .find(|value| !value.trim().is_empty())
                .map(String::as_str)
                .unwrap_or("未返回分析结果");
            format!("分析不可用：{limitation}")
        }
    }
}

fn result_for<'a>(
    artifact: &'a SelfAnalysisArtifact,
    test_id: &str,
) -> Option<&'a AnalysisTestResult> {
    artifact
        .tests
        .iter()
        .find(|result| result.test_id == test_id)
}

fn missing_result_reason(test: &crate::catalog::TestCase) -> String {
    format!("{} {}：分析不可用（结果缺失）", test.id, test.name)
}

fn non_pass_reason(test: &crate::catalog::TestCase, result: Option<&AnalysisTestResult>) -> String {
    match result {
        Some(result) => format!("{} {}：{}", test.id, test.name, result_reason(result)),
        None => missing_result_reason(test),
    }
}

fn verified_category_conclusion(category: &str) -> &'static str {
    match category {
        "接口与协议" => {
            "接口可正常访问，鉴权和模型名均被接受；同时支持同步、流式响应、完整流结束、Token usage 和结构化错误返回。"
        }
        "结构化结果" => {
            "能按要求输出严格 JSON，字段类型、嵌套对象、数组顺序、空值和额外字段控制均符合要求，并能正确返回调查阶段和证据引用。"
        }
        "上下文" => {
            "本轮 8K、16K、32K、64K、128K 字符级请求均返回非空响应，多轮修正状态也能保持；本轮最高验证到 128K 字符近似档位，不等同于真实 Token 上限。"
        }
        "指令与文本" => {
            "能执行精确文本、组合格式、多字段抽取和限长摘要要求，输出内容完整且没有额外干扰文本。"
        }
        "Thinking 与推理" => {
            "支持 low/high reasoning 档位，能够分离 reasoning 与最终答案，并完成流式思考事件和时间顺序推理测试。"
        }
        "工具调用" => {
            "支持单工具、工具选择、参数校验、嵌套参数、并行调用、串行调用、工具结果关联、失败重试和大工具目录。"
        }
        "性能与稳定性" => {
            "本轮首字节时间、完整响应耗时和并发响应指标均有有效观测；重复请求均成功，P50/P95 可计算，4-32 并发样本均返回有效响应，具体数值见检测明细。"
        }
        "护栏与词汇" => {
            "中文和英文安全业务词场景均能按要求返回指定业务字段和值；该结论只覆盖本轮词汇与业务字段测试，不代表完整安全能力。"
        }
        _ => "该分类本轮所有检测项通过。",
    }
}

fn render_markdown(artifact: &SelfAnalysisArtifact) -> String {
    let mut output = String::new();
    writeln!(output, "# 模型能力检测结果").unwrap();
    output.push('\n');
    output.push_str("## 运行信息\n\n");
    output.push_str("| 项目 | 值 |\n|---|---|\n");
    writeln!(
        output,
        "| 生成时间 | {} |",
        markdown_cell(&artifact.generated_at)
    )
    .unwrap();
    writeln!(
        output,
        "| 请求模型 | {} |",
        markdown_cell(&artifact.target.requested_model)
    )
    .unwrap();
    writeln!(
        output,
        "| 检测协议 | {} |",
        markdown_cell(&artifact.target.detected_protocol)
    )
    .unwrap();
    writeln!(
        output,
        "| 分析来源 | {} |",
        markdown_cell(artifact.provenance)
    )
    .unwrap();
    writeln!(
        output,
        "| 检测项总数 | {}（PASS {} / FAIL {} / 不可用 {}） |",
        artifact.tests.len(),
        artifact.counts.pass,
        artifact.counts.fail,
        artifact.counts.unavailable
    )
    .unwrap();
    writeln!(
        output,
        "| Endpoint | {} |",
        markdown_cell(&artifact.target.endpoint)
    )
    .unwrap();
    output.push('\n');

    render_category_summary(artifact, &mut output);
    render_concurrency_summary(artifact, &mut output);
    render_test_table(artifact, &mut output);
    render_non_pass_details(artifact, &mut output);
    output
}

fn render_category_summary(artifact: &SelfAnalysisArtifact, output: &mut String) {
    output.push_str("## 大分类结论\n\n");
    output.push_str("| 能力分类 | 结论 | 检测结论 |\n|---|---|---|\n");
    let mut categories = Vec::new();
    for test in crate::catalog::CATALOG {
        if !categories.contains(&test.category) {
            categories.push(test.category);
        }
    }
    for category in categories {
        let members: Vec<_> = crate::catalog::CATALOG
            .iter()
            .filter(|test| test.category == category)
            .collect();
        let non_pass: Vec<_> = members
            .iter()
            .filter(|test| {
                result_for(artifact, test.id)
                    .map(|result| result_status(result) != "PASS")
                    .unwrap_or(true)
            })
            .collect();
        let status = if non_pass.is_empty() {
            "满足"
        } else {
            "不满足"
        };
        let reason = if non_pass.is_empty() {
            verified_category_conclusion(category).into()
        } else {
            non_pass
                .iter()
                .map(|test| non_pass_reason(test, result_for(artifact, test.id)))
                .collect::<Vec<_>>()
                .join("；")
        };
        writeln!(
            output,
            "| {} | {} | {} |",
            markdown_cell(category),
            status,
            markdown_cell(&reason)
        )
        .unwrap();
    }
    output.push('\n');
}

fn render_concurrency_summary(artifact: &SelfAnalysisArtifact, output: &mut String) {
    if artifact.concurrency_waves.is_empty() {
        return;
    }
    output.push_str("## 并发响应明细\n\n");
    output.push_str(
        "| 并发档位 | 总请求 | 成功 | 失败 | 平均响应时间（全部请求） |\n|---:|---:|---:|---:|---:|\n",
    );
    for wave in &artifact.concurrency_waves {
        let average = wave
            .average_response_time_seconds
            .map(|seconds| format!("{:.1} ms", seconds * 1000.0))
            .unwrap_or_else(|| "-".into());
        writeln!(
            output,
            "| {} | {} | {} | {} | {} |",
            wave.concurrency, wave.total_requests, wave.succeeded, wave.failed, average
        )
        .unwrap();
    }
    output.push('\n');

    let failures: Vec<_> = artifact
        .concurrency_waves
        .iter()
        .flat_map(|wave| {
            wave.failures
                .iter()
                .map(move |failure| (wave.concurrency, failure))
        })
        .collect();
    if failures.is_empty() {
        output.push_str("失败请求：无。\n\n");
        return;
    }
    output.push_str("失败请求：\n\n");
    for (concurrency, failure) in failures {
        writeln!(
            output,
            "- {} 并发，request_id={}：{}",
            concurrency,
            markdown_cell(&failure.request_id),
            markdown_cell(&failure.error)
        )
        .unwrap();
    }
    output.push('\n');
}

fn render_test_table(artifact: &SelfAnalysisArtifact, output: &mut String) {
    output.push_str("## 46 项检测明细\n\n");
    output.push_str("| 编号 | 分类 | 检测项 | 结果 | 原因 |\n|---|---|---|---|---|\n");
    for test in crate::catalog::CATALOG {
        let result = result_for(artifact, test.id);
        let status = result.map(result_status).unwrap_or("ANALYSIS_UNAVAILABLE");
        let reason = result
            .map(result_reason)
            .unwrap_or_else(|| "分析不可用（结果缺失）".into());
        writeln!(
            output,
            "| {} | {} | {} | {} | {} |",
            markdown_cell(test.id),
            markdown_cell(test.category),
            markdown_cell(test.name),
            status,
            markdown_cell(&reason)
        )
        .unwrap();
    }
    output.push('\n');
}

fn render_non_pass_details(artifact: &SelfAnalysisArtifact, output: &mut String) {
    let details: Vec<_> = crate::catalog::CATALOG
        .iter()
        .filter_map(|test| {
            result_for(artifact, test.id)
                .filter(|result| result_status(result) != "PASS")
                .map(|result| (test, result))
        })
        .collect();
    if details.is_empty() {
        return;
    }
    output.push_str("## 非通过项详情\n\n");
    for (test, result) in details {
        writeln!(
            output,
            "### {} {}\n\n- 分类：{}\n- 结果：`{}`\n- 决策来源：`{}`\n- 原因：{}\n",
            test.id,
            markdown_cell(test.name),
            markdown_cell(test.category),
            result_status(result),
            decision_source_label(result.decision_source),
            markdown_cell(&result_reason(result))
        )
        .unwrap();
        if !result.observations.is_empty() {
            output.push_str("**模型观察**\n\n");
            for observation in &result.observations {
                writeln!(output, "- {}", markdown_cell(observation)).unwrap();
            }
            output.push('\n');
        }
        if !result.evidence_refs.is_empty() {
            writeln!(
                output,
                "**证据引用**：{}\n",
                result
                    .evidence_refs
                    .iter()
                    .map(|reference| format!("`{}`", markdown_cell(reference)))
                    .collect::<Vec<_>>()
                    .join("、")
            )
            .unwrap();
        }
        if !result.limitations.is_empty() {
            output.push_str("**限制说明**\n\n");
            for limitation in &result.limitations {
                writeln!(output, "- {}", markdown_cell(limitation)).unwrap();
            }
            output.push('\n');
        }
        if !result.validation_notes.is_empty() {
            output.push_str("**校验说明**\n\n");
            for note in &result.validation_notes {
                writeln!(output, "- {}", markdown_cell(note)).unwrap();
            }
            output.push('\n');
        }
    }
}

pub(crate) fn collision_safe_output_path(log_path: &Path, extension: &str) -> PathBuf {
    let stem = log_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("model-doctor");
    let initial = log_path.with_file_name(format!("{stem}-self-analysis.{extension}"));
    if !initial.exists() {
        return initial;
    }

    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    for sequence in 1.. {
        let suffix = if sequence == 1 {
            timestamp.to_string()
        } else {
            format!("{timestamp}-{sequence}")
        };
        let candidate =
            log_path.with_file_name(format!("{stem}-self-analysis-{suffix}.{extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

fn write_artifact(
    log_path: &Path,
    artifact: &SelfAnalysisArtifact,
) -> Result<PathBuf, AnalysisError> {
    let mut bytes = serde_json::to_vec_pretty(artifact)?;
    bytes.push(b'\n');
    write_output(log_path, "json", &bytes)
}

fn write_markdown_artifact(log_path: &Path, markdown: &[u8]) -> Result<PathBuf, AnalysisError> {
    write_output(log_path, "md", markdown)
}

fn write_output(log_path: &Path, extension: &str, bytes: &[u8]) -> Result<PathBuf, AnalysisError> {
    let directory = log_path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(directory)?;
    let temp_path = loop {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let candidate = directory.join(format!(
            ".model-doctor-self-analysis-{}-{sequence}.tmp",
            std::process::id()
        ));
        match create_new_private_file(&candidate) {
            Ok(mut file) => {
                write_complete(&mut file, bytes)?;
                break candidate;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    };

    loop {
        let destination = collision_safe_output_path(log_path, extension);
        match std::fs::hard_link(&temp_path, &destination) {
            Ok(()) => {
                std::fs::remove_file(&temp_path)?;
                return Ok(destination);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                let _ = std::fs::remove_file(&temp_path);
                return Err(error.into());
            }
        }
    }
}

fn write_complete(file: &mut File, bytes: &[u8]) -> std::io::Result<()> {
    file.write_all(bytes)?;
    file.flush()?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::time::Duration;

    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::analysis::client::{AnalysisClient, ClientError};
    use crate::analysis::evidence_reader::{complete_test_log, read};
    use crate::analysis::packet::{EvidencePacket, default_batches};
    use crate::analysis::validator::{CandidateEnvelope, CandidateReview, CandidateStatus};
    use crate::protocol::{AuthMode, Protocol};
    use crate::redaction::Redactor;

    struct FakeClient {
        responses: Mutex<VecDeque<Result<CandidateEnvelope, ClientError>>>,
        prompts: Mutex<Vec<String>>,
    }

    impl FakeClient {
        fn new(responses: Vec<Result<CandidateEnvelope, ClientError>>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                prompts: Mutex::new(Vec::new()),
            }
        }

        fn call_count(&self) -> usize {
            self.prompts.lock().unwrap().len()
        }

        fn prompts(&self) -> Vec<String> {
            self.prompts.lock().unwrap().clone()
        }
    }

    impl AnalysisClient for FakeClient {
        async fn analyze(
            &self,
            prompt: &str,
            _cancellation: CancellationToken,
        ) -> Result<CandidateEnvelope, ClientError> {
            self.prompts.lock().unwrap().push(prompt.to_owned());
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("queued fake response")
        }
    }

    #[tokio::test]
    async fn repairs_one_invalid_batch_then_succeeds() {
        let (directory, settings, batches) = fixture().await;
        let mut responses = vec![Err(ClientError::InvalidJson("missing reviews".into()))];
        responses.push(Ok(valid_envelope_for(&batches[0])));
        responses.extend(
            batches[1..]
                .iter()
                .map(|batch| Ok(valid_envelope_for(batch))),
        );
        let client = FakeClient::new(responses);

        let outcome = analyze_with_client(settings, &client, CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(outcome.unavailable_count, 0);
        assert_eq!(outcome.available_count, 46);
        assert_eq!(client.call_count(), batches.len() + 1);
        assert!(client.prompts()[1].contains("missing reviews"));
        let output = std::fs::read_to_string(&outcome.path).unwrap();
        assert!(output.contains("llm-capability-doctor.self-analysis.v2"));
        assert!(!output.contains("secret-key"));
        let markdown = std::fs::read_to_string(&outcome.markdown_path).unwrap();
        assert!(markdown.contains("## 大分类结论"));
        assert_eq!(markdown.matches("| PASS |").count(), 46);
        assert!(!markdown.contains("secret-key"));
        drop(directory);
    }

    #[tokio::test]
    async fn repairs_one_missing_candidate_response_then_succeeds() {
        let (_directory, settings, batches) = fixture().await;
        let mut responses = vec![Err(ClientError::MissingAssistantContent)];
        responses.push(Ok(valid_envelope_for(&batches[0])));
        responses.extend(
            batches[1..]
                .iter()
                .map(|batch| Ok(valid_envelope_for(batch))),
        );
        let client = FakeClient::new(responses);

        let outcome = analyze_with_client(settings, &client, CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(outcome.unavailable_count, 0);
        assert_eq!(client.call_count(), batches.len() + 1);
        assert!(client.prompts()[1].contains("no protocol-native assistant content"));
    }

    #[tokio::test]
    async fn failed_batch_is_explicit_and_later_batches_continue() {
        let (_directory, settings, batches) = fixture().await;
        let mut responses = vec![Err(ClientError::Transport("offline".into()))];
        responses.extend(
            batches[1..]
                .iter()
                .map(|batch| Ok(valid_envelope_for(batch))),
        );
        let client = FakeClient::new(responses);

        let outcome = analyze_with_client(settings, &client, CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(outcome.unavailable_count, batches[0].len());
        assert_eq!(outcome.available_count, 46 - batches[0].len());
        assert_eq!(client.call_count(), batches.len());
    }

    #[tokio::test]
    async fn cancellation_stops_calls_and_flushes_unavailable_results() {
        let (_directory, settings, _batches) = fixture().await;
        let client = FakeClient::new(Vec::new());
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let outcome = analyze_with_client(settings, &client, cancellation)
            .await
            .unwrap();

        assert!(outcome.cancelled);
        assert_eq!(outcome.available_count, 0);
        assert_eq!(outcome.unavailable_count, 46);
        assert_eq!(client.call_count(), 0);
        let output = std::fs::read_to_string(outcome.path).unwrap();
        assert!(output.contains("\"collectorVersion\": \"0.12.0\""));
        assert!(
            output.contains("\"evidenceSchemaVersion\": \"llm-capability-doctor.evidence.v4\"")
        );
        assert!(output.contains("\"analysisState\": \"ANALYSIS_UNAVAILABLE\""));
        let markdown = std::fs::read_to_string(outcome.markdown_path).unwrap();
        assert!(markdown.contains("| 接口与协议 | 不满足 |"));
        assert!(markdown.contains("分析不可用"));
    }

    #[test]
    fn output_collision_never_overwrites_existing_analysis() {
        let directory = tempfile::tempdir().unwrap();
        let log = directory.path().join("doctor.log");
        let first = collision_safe_output_path(&log, "json");
        std::fs::write(&first, "existing").unwrap();

        let second = collision_safe_output_path(&log, "json");

        assert_ne!(first, second);
        assert_eq!(std::fs::read_to_string(first).unwrap(), "existing");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn output_file_is_private() {
        use std::os::unix::fs::PermissionsExt;

        let (_directory, settings, batches) = fixture().await;
        let responses = batches
            .iter()
            .map(|batch| Ok(valid_envelope_for(batch)))
            .collect();
        let client = FakeClient::new(responses);

        let outcome = analyze_with_client(settings, &client, CancellationToken::new())
            .await
            .unwrap();

        for path in [outcome.path, outcome.markdown_path] {
            let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[tokio::test]
    async fn source_path_is_redacted_in_output_metadata() {
        let (_directory, settings, batches) = fixture_named("secret-key-doctor.log").await;
        let responses = batches
            .iter()
            .map(|batch| Ok(valid_envelope_for(batch)))
            .collect();
        let client = FakeClient::new(responses);

        let outcome = analyze_with_client(settings, &client, CancellationToken::new())
            .await
            .unwrap();

        let json = std::fs::read_to_string(&outcome.path).unwrap();
        let markdown = std::fs::read_to_string(&outcome.markdown_path).unwrap();
        assert!(!json.contains("secret-key"));
        assert!(!markdown.contains("secret-key"));
        assert!(json.contains("[REDACTED]-doctor.log"));
    }

    #[test]
    fn endpoint_metadata_masks_every_query_value_and_userinfo() {
        let url = Url::parse(
            "https://user:password@example.test/v1/chat?X-Amz-Signature=signed-secret&region=us-east-1",
        )
        .unwrap();
        let redactor = Redactor::new("api-key", &url);

        let endpoint = redact_endpoint_metadata(&url, &redactor);

        assert!(!endpoint.contains("user"));
        assert!(!endpoint.contains("password"));
        assert!(!endpoint.contains("signed-secret"));
        assert!(!endpoint.contains("us-east-1"));
        assert!(endpoint.contains("X-Amz-Signature=%5BREDACTED%5D"));
        assert!(endpoint.contains("region=%5BREDACTED%5D"));
    }

    #[test]
    fn markdown_all_pass_has_eight_satisfied_categories_and_46_rows() {
        let artifact = markdown_fixture(
            crate::catalog::CATALOG
                .iter()
                .map(|test| available_markdown_result(test.id, CandidateStatus::Pass, None))
                .collect(),
        );

        let markdown = render_markdown(&artifact);

        assert_eq!(markdown.matches("| 满足 |").count(), 8);
        assert_eq!(markdown.matches("| PASS |").count(), 46);
        assert!(!markdown.contains("## 非通过项详情"));
    }

    #[test]
    fn markdown_category_conclusions_describe_verified_capabilities() {
        let artifact = markdown_fixture(
            crate::catalog::CATALOG
                .iter()
                .map(|test| available_markdown_result(test.id, CandidateStatus::Pass, None))
                .collect(),
        );

        let markdown = render_markdown(&artifact);

        assert!(markdown.contains(
            "接口可正常访问，鉴权和模型名均被接受；同时支持同步、流式响应、完整流结束、Token usage 和结构化错误返回。"
        ));
        assert!(markdown.contains(
            "本轮 8K、16K、32K、64K、128K 字符级请求均返回非空响应，多轮修正状态也能保持；本轮最高验证到 128K 字符近似档位，不等同于真实 Token 上限。"
        ));
        assert!(markdown.contains(
            "支持单工具、工具选择、参数校验、嵌套参数、并行调用、串行调用、工具结果关联、失败重试和大工具目录。"
        ));
        assert!(markdown.contains(
            "中文和英文安全业务词场景均能按要求返回指定业务字段和值；该结论只覆盖本轮词汇与业务字段测试，不代表完整安全能力。"
        ));
        assert!(!markdown.contains("8/8 项检测通过"));
    }

    #[test]
    fn markdown_concurrency_summary_reports_each_wave_and_failures() {
        let mut artifact = markdown_fixture(
            crate::catalog::CATALOG
                .iter()
                .map(|test| available_markdown_result(test.id, CandidateStatus::Pass, None))
                .collect(),
        );
        artifact.concurrency_waves = vec![
            ConcurrencyWave {
                concurrency: 4,
                total_requests: 4,
                succeeded: 4,
                failed: 0,
                average_response_time_seconds: Some(0.42),
                failures: Vec::new(),
            },
            ConcurrencyWave {
                concurrency: 8,
                total_requests: 8,
                succeeded: 7,
                failed: 1,
                average_response_time_seconds: Some(0.88),
                failures: vec![ConcurrencyFailure {
                    request_id: "test-057-c8-3".into(),
                    error: "HTTP 500: upstream unavailable".into(),
                }],
            },
            ConcurrencyWave {
                concurrency: 16,
                total_requests: 16,
                succeeded: 16,
                failed: 0,
                average_response_time_seconds: Some(1.24),
                failures: Vec::new(),
            },
            ConcurrencyWave {
                concurrency: 32,
                total_requests: 32,
                succeeded: 31,
                failed: 1,
                average_response_time_seconds: None,
                failures: vec![ConcurrencyFailure {
                    request_id: "test-057-c32-9".into(),
                    error: "timeout".into(),
                }],
            },
        ];

        let markdown = render_markdown(&artifact);

        assert!(markdown.contains("## 并发响应明细"));
        assert!(markdown.contains("| 4 | 4 | 4 | 0 | 420.0 ms |"));
        assert!(markdown.contains("| 8 | 8 | 7 | 1 | 880.0 ms |"));
        assert!(markdown.contains("| 16 | 16 | 16 | 0 | 1240.0 ms |"));
        assert!(markdown.contains("| 32 | 32 | 31 | 1 | - |"));
        assert!(markdown.contains("request_id=test-057-c8-3：HTTP 500: upstream unavailable"));
        assert!(markdown.contains("request_id=test-057-c32-9：timeout"));
    }

    #[test]
    fn markdown_fail_explains_each_non_pass_item() {
        let mut results = crate::catalog::CATALOG
            .iter()
            .map(|test| available_markdown_result(test.id, CandidateStatus::Pass, None))
            .collect::<Vec<_>>();
        let failed = results
            .iter_mut()
            .find(|result| result.test_id == "046")
            .unwrap();
        failed.candidate_status = Some(CandidateStatus::Fail);
        failed.validated_status = Some(ValidatedStatus::Fail);
        failed.failure_cause = Some("tool envelope incomplete".into());

        let markdown = render_markdown(&markdown_fixture(results));

        assert!(markdown.contains("| 工具调用 | 不满足 |"));
        assert!(markdown.contains("046 官方工具协议结构合规：tool envelope incomplete"));
        assert!(markdown.contains("### 046 官方工具协议结构合规"));
        assert!(markdown.contains("tool envelope incomplete"));
    }

    #[test]
    fn markdown_unavailable_is_not_presented_as_model_fail() {
        let mut results = crate::catalog::CATALOG
            .iter()
            .map(|test| available_markdown_result(test.id, CandidateStatus::Pass, None))
            .collect::<Vec<_>>();
        let unavailable = results
            .iter_mut()
            .find(|result| result.test_id == "049")
            .unwrap();
        unavailable.analysis_state = AnalysisState::AnalysisUnavailable;
        unavailable.candidate_status = None;
        unavailable.validated_status = None;
        unavailable.decision_source = None;
        unavailable.limitations = vec!["offline".into()];

        let markdown = render_markdown(&markdown_fixture(results));

        assert!(markdown.contains("| 工具调用 | 不满足 |"));
        assert!(markdown.contains("049 工具失败恢复：分析不可用：offline"));
        assert!(markdown.contains("| ANALYSIS_UNAVAILABLE |"));
        assert!(markdown.contains("分析不可用：offline"));
        assert!(!markdown.contains("049 工具失败恢复：模型失败"));
    }

    #[test]
    fn markdown_escapes_table_content() {
        let mut result =
            available_markdown_result("046", CandidateStatus::Fail, Some("first | line\nsecond"));
        result.observations = vec!["left | right\nnext".into()];

        let markdown = render_markdown(&markdown_fixture(vec![result]));

        assert!(markdown.contains("first \\| line second"));
        assert!(markdown.contains("left \\| right next"));
        assert!(!markdown.contains("first | line\nsecond"));
    }

    #[test]
    fn markdown_and_json_use_distinct_collision_safe_paths() {
        let directory = tempfile::tempdir().unwrap();
        let log = directory.path().join("doctor.log");
        let json = collision_safe_output_path(&log, "json");
        let markdown = collision_safe_output_path(&log, "md");
        std::fs::write(&json, "existing json").unwrap();
        std::fs::write(&markdown, "existing markdown").unwrap();

        let next_json = collision_safe_output_path(&log, "json");
        let next_markdown = collision_safe_output_path(&log, "md");

        assert_ne!(json, next_json);
        assert_ne!(markdown, next_markdown);
        assert_eq!(std::fs::read_to_string(json).unwrap(), "existing json");
        assert_eq!(
            std::fs::read_to_string(markdown).unwrap(),
            "existing markdown"
        );
    }

    fn markdown_fixture(tests: Vec<AnalysisTestResult>) -> SelfAnalysisArtifact {
        SelfAnalysisArtifact {
            schema_version: SELF_ANALYSIS_SCHEMA_VERSION,
            collector_version: "0.12.0".into(),
            evidence_schema_version: "llm-capability-doctor.evidence.v4".into(),
            generated_at: "2026-08-24T00:00:00+08:00".into(),
            source: EvidenceSource {
                path: PathBuf::from("doctor.log"),
                file_name: "doctor.log".into(),
                size_bytes: 0,
                sha256: "fixture-sha256".into(),
            },
            target: AnalysisTargetMetadata {
                endpoint: "https://example.test/v1/chat/completions".into(),
                requested_model: "test-model".into(),
                detected_protocol: "openai-chat".into(),
                authentication_mode: "bearer".into(),
            },
            prompt_version: PROMPT_VERSION,
            provenance: SELF_ANALYSIS_PROVENANCE,
            tests,
            batches: Vec::new(),
            counts: AnalysisCounts {
                pass: 0,
                fail: 0,
                unavailable: 0,
            },
            concurrency_waves: Vec::new(),
        }
    }

    fn available_markdown_result(
        test_id: &str,
        status: CandidateStatus,
        failure_cause: Option<&str>,
    ) -> AnalysisTestResult {
        AnalysisTestResult {
            test_id: test_id.into(),
            analysis_state: AnalysisState::Available,
            candidate_status: Some(status),
            validated_status: Some(match status {
                CandidateStatus::Pass => ValidatedStatus::Pass,
                CandidateStatus::Fail => ValidatedStatus::Fail,
            }),
            decision_source: Some(DecisionSource::TargetModel),
            observations: vec!["observable fact".into()],
            failure_cause: failure_cause.map(str::to_owned),
            evidence_refs: vec![format!("request:test-{test_id}")],
            limitations: Vec::new(),
            validation_notes: Vec::new(),
        }
    }

    async fn fixture() -> (
        tempfile::TempDir,
        AnalysisSettings,
        Vec<Vec<EvidencePacket>>,
    ) {
        fixture_named("doctor.log").await
    }

    async fn fixture_named(
        log_file_name: &str,
    ) -> (
        tempfile::TempDir,
        AnalysisSettings,
        Vec<Vec<EvidencePacket>>,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let log_path = directory.path().join(log_file_name);
        std::fs::write(&log_path, complete_test_log()).unwrap();
        let url = "https://example.test/v1/chat/completions".parse().unwrap();
        let redactor = Redactor::new("secret-key", &url);
        let parsed = read(&log_path, &redactor).unwrap();
        let batches = default_batches(&parsed).await.unwrap();
        let settings = AnalysisSettings {
            log_path,
            url,
            model: "test-model".into(),
            api_key: "secret-key".into(),
            timeout: Duration::from_secs(2),
            insecure: false,
            protocol: Protocol::OpenAiChat,
            auth_mode: AuthMode::Bearer,
        };
        (directory, settings, batches)
    }

    fn valid_envelope_for(batch: &[EvidencePacket]) -> CandidateEnvelope {
        CandidateEnvelope {
            reviews: batch
                .iter()
                .map(|packet| CandidateReview {
                    test_id: packet.test_id.clone(),
                    candidate_status: CandidateStatus::Pass,
                    observations: vec!["observable secret-key evidence reviewed".into()],
                    failure_cause: None,
                    evidence_refs: vec![packet.allowed_evidence_refs[0].clone()],
                    limitations: Vec::new(),
                })
                .collect(),
        }
    }
}
