use std::collections::BTreeMap;
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
use super::evidence_reader::{
    DegradedEntry, EvidenceError, EvidenceSource, ParsedEvidence, ParsedRequest, read,
};
use super::packet::{EvidencePacket, PacketError, bounded_excerpt, default_batches};
use super::prompt::{PROMPT_VERSION, build_prompt, build_repair_prompt};
use super::validator::{
    CandidateEnvelope, CandidateStatus, DecisionSource, ValidatedReview, ValidatedStatus,
    validate_candidates,
};
use crate::context_capacity::{
    ContextConclusion, ContextStatus, ProbeOutcome, classify_probe, conclusion_sentence,
    is_calibration_request, parse_probe_target, protocol_from_wire_name,
};
use crate::private_file::create_new_private_file;
use crate::protocol::{AuthMode, Protocol};
use crate::redaction::Redactor;
use crate::terminal;

pub const SELF_ANALYSIS_SCHEMA_VERSION: &str = "llm-capability-doctor.self-analysis.v4";
pub const SELF_ANALYSIS_PROVENANCE: &str = "TARGET_MODEL_SELF_ANALYSIS_WITH_EVIDENCE_RULES";
const MAX_ANALYSIS_TRANSPORT_ATTEMPTS: usize = 3;
const MAX_ANALYSIS_SCHEMA_REPAIRS: usize = 5;
const GATEWAY_COMPATIBILITY_TEST_IDS: [&str; 8] =
    ["002", "004", "005", "006", "040", "041", "043", "047"];
const MINIMUM_CONCURRENCY: usize = 4;
const MINIMUM_CONCURRENCY_MAX_AVERAGE_SECONDS: f64 = 30.0;

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
    pub failed_curl_log_path: PathBuf,
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
    requirement_levels: BTreeMap<&'static str, crate::catalog::RequirementLevel>,
    batches: Vec<BatchResult>,
    counts: AnalysisCounts,
    gateway_compatibility: GatewayCompatibility,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    evidence_degradations: Vec<EvidenceDegradation>,
    #[serde(skip)]
    concurrency_waves: Vec<ConcurrencyWave>,
    #[serde(skip)]
    context_conclusion: ContextConclusion,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceDegradation {
    pub kind: &'static str,
    pub id: String,
    pub reason: String,
}

/// 汇总读取端跳过的损坏块，写入产物与报告，供用户核对证据完整性。
fn collect_evidence_degradations(evidence: &ParsedEvidence) -> Vec<EvidenceDegradation> {
    evidence
        .degraded_requests
        .iter()
        .map(|entry| EvidenceDegradation {
            kind: "request",
            id: entry.id.clone(),
            reason: entry.reason.clone(),
        })
        .chain(
            evidence
                .degraded_tests
                .iter()
                .map(|entry| EvidenceDegradation {
                    kind: "test",
                    id: entry.id.clone(),
                    reason: entry.reason.clone(),
                }),
        )
        .collect()
}

/// 证据损坏的检测项以“分析不可用”进入报告，而不是让整个自分析失败。
fn degraded_test_results(degraded_tests: &[DegradedEntry]) -> Vec<AnalysisTestResult> {
    degraded_tests
        .iter()
        .map(|entry| AnalysisTestResult {
            test_id: entry.id.clone(),
            report_test_id: entry.id.clone(),
            analysis_state: AnalysisState::AnalysisUnavailable,
            candidate_status: None,
            validated_status: None,
            decision_source: None,
            observations: Vec::new(),
            failure_cause: None,
            evidence_refs: Vec::new(),
            limitations: vec![entry.reason.clone()],
            validation_notes: vec!["no validated self-analysis candidate was produced".into()],
        })
        .collect()
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
    #[serde(skip)]
    report_test_id: String,
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

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum GatewayCompatibilityStatus {
    Pass,
    Fail,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GatewayCompatibility {
    status: GatewayCompatibilityStatus,
    statement: String,
    reasons: Vec<String>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct ConcurrencyWave {
    concurrency: usize,
    total_requests: usize,
    succeeded: usize,
    failed: usize,
    average_response_time_seconds: Option<f64>,
    failures: Vec<ConcurrencyFailure>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct ConcurrencyFailure {
    request_id: String,
    error: String,
}

struct ProcessedBatch {
    attempts: usize,
    reviews: Option<Vec<ValidatedReview>>,
    error: Option<String>,
}

#[derive(Clone, Debug)]
enum AnalysisProgressEvent {
    Started {
        total_batches: usize,
    },
    BatchStarted {
        index: usize,
        total: usize,
        categories: Vec<String>,
        test_ids: Vec<String>,
    },
    BatchRetry {
        index: usize,
        total: usize,
        categories: Vec<String>,
        retry: usize,
    },
    BatchFinished {
        index: usize,
        total: usize,
        categories: Vec<String>,
        unavailable: bool,
    },
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
    let mut print_progress = |event| println!("{}", analysis_progress_line(&event));
    analyze_with_client_and_progress(settings, &client, cancellation, &mut print_progress).await
}

#[cfg(test)]
pub(crate) async fn analyze_with_client<C: AnalysisClient>(
    settings: AnalysisSettings,
    client: &C,
    cancellation: CancellationToken,
) -> Result<AnalysisOutcome, AnalysisError> {
    let mut ignore_progress = |_| {};
    analyze_with_client_and_progress(settings, client, cancellation, &mut ignore_progress).await
}

async fn analyze_with_client_and_progress<C: AnalysisClient>(
    settings: AnalysisSettings,
    client: &C,
    cancellation: CancellationToken,
    progress: &mut dyn FnMut(AnalysisProgressEvent),
) -> Result<AnalysisOutcome, AnalysisError> {
    let redactor = Redactor::new(&settings.api_key, &settings.url);
    let evidence = read(&settings.log_path, &redactor)?;
    let batches = default_batches(&evidence).await?;
    let mut tests = Vec::with_capacity(crate::catalog::CATALOG.len());
    let mut batch_results = Vec::with_capacity(batches.len());
    let mut cancelled = false;
    let total_batches = batches.len();
    progress(AnalysisProgressEvent::Started { total_batches });

    for (batch_index, batch) in batches.into_iter().enumerate() {
        let index = batch_index + 1;
        let test_ids: Vec<String> = batch.iter().map(|packet| packet.test_id.clone()).collect();
        let categories = batch_categories(&batch);
        if cancellation.is_cancelled() {
            cancelled = true;
            let error = "analysis cancelled before batch started".to_owned();
            tests.extend(unavailable_results(&batch, &error));
            batch_results.push(BatchResult {
                test_ids,
                attempts: 0,
                error: Some(error),
            });
            progress(AnalysisProgressEvent::BatchFinished {
                index,
                total: total_batches,
                categories,
                unavailable: true,
            });
            continue;
        }

        progress(AnalysisProgressEvent::BatchStarted {
            index,
            total: total_batches,
            categories: categories.clone(),
            test_ids: test_ids.clone(),
        });
        let processed = {
            let mut report_retry = |retry| {
                progress(AnalysisProgressEvent::BatchRetry {
                    index,
                    total: total_batches,
                    categories: categories.clone(),
                    retry,
                });
            };
            process_batch(
                client,
                &batch,
                cancellation.child_token(),
                &mut report_retry,
            )
            .await?
        };
        if cancellation.is_cancelled() {
            cancelled = true;
        }
        let safe_error = processed
            .error
            .as_deref()
            .map(|error| redactor.redact_text(error));
        let unavailable = processed.reviews.is_none();
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
        progress(AnalysisProgressEvent::BatchFinished {
            index,
            total: total_batches,
            categories,
            unavailable,
        });
    }

    let tests = consolidate_report_results(tests);
    let evidence_degradations = collect_evidence_degradations(&evidence);
    let mut tests = tests
        .into_iter()
        .chain(degraded_test_results(&evidence.degraded_tests))
        .collect::<Vec<_>>();
    let concurrency_waves = summarize_concurrency(&evidence);
    let context_conclusion = summarize_context(&evidence);
    apply_hard_evidence_rules(
        &mut tests,
        &evidence,
        &context_conclusion,
        &concurrency_waves,
    );
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
    let detected_protocol = settings.protocol.to_string();
    let gateway_compatibility = derive_gateway_compatibility(&detected_protocol, &tests);
    let mut source = evidence.source.clone();
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
            detected_protocol,
            authentication_mode: settings.auth_mode.to_string(),
        },
        prompt_version: PROMPT_VERSION,
        provenance: SELF_ANALYSIS_PROVENANCE,
        tests,
        requirement_levels: requirement_levels(),
        batches: batch_results,
        counts,
        gateway_compatibility,
        evidence_degradations,
        concurrency_waves,
        context_conclusion,
    };
    let pass_count = artifact.counts.pass;
    let fail_count = artifact.counts.fail;
    let unavailable_count = artifact.counts.unavailable;
    let failed_curl_log_path = write_failed_curl_log(&settings.log_path, &artifact, &evidence)?;
    let path = write_artifact(&settings.log_path, &artifact)?;
    let markdown = render_markdown(&artifact);
    let markdown_path = write_markdown_artifact(&settings.log_path, markdown.as_bytes())?;
    Ok(AnalysisOutcome {
        path,
        markdown_path,
        failed_curl_log_path,
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
    on_retry: &mut dyn FnMut(usize),
) -> Result<ProcessedBatch, AnalysisError> {
    let original_prompt = build_prompt(batch)?;
    let mut prompt = original_prompt.clone();
    let mut attempts = 0;
    let mut schema_repair_attempts = 0;
    loop {
        if cancellation.is_cancelled() {
            return Ok(ProcessedBatch {
                attempts,
                reviews: None,
                error: Some("analysis cancelled".into()),
            });
        }
        let (response, transport_attempts) =
            analyze_with_transport_retries(client, &prompt, cancellation.child_token(), on_retry)
                .await;
        attempts += transport_attempts;
        match response {
            Ok(envelope) => match validate_candidates(batch, envelope) {
                Ok(reviews) => {
                    return Ok(ProcessedBatch {
                        attempts,
                        reviews: Some(reviews),
                        error: None,
                    });
                }
                Err(errors) if schema_repair_attempts < MAX_ANALYSIS_SCHEMA_REPAIRS => {
                    prompt = build_repair_prompt(&original_prompt, &errors)?;
                    schema_repair_attempts += 1;
                }
                Err(errors) => {
                    return Ok(ProcessedBatch {
                        attempts,
                        reviews: None,
                        error: Some(errors.join("; ")),
                    });
                }
            },
            Err(error)
                if schema_repair_attempts < MAX_ANALYSIS_SCHEMA_REPAIRS
                    && repairable_candidate_response(&error) =>
            {
                prompt = build_repair_prompt(&original_prompt, &[error.to_string()])?;
                schema_repair_attempts += 1;
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
}

async fn analyze_with_transport_retries<C: AnalysisClient>(
    client: &C,
    prompt: &str,
    cancellation: CancellationToken,
    on_retry: &mut dyn FnMut(usize),
) -> (Result<CandidateEnvelope, ClientError>, usize) {
    for attempt in 1..=MAX_ANALYSIS_TRANSPORT_ATTEMPTS {
        if cancellation.is_cancelled() {
            return (
                Err(ClientError::Transport("analysis cancelled".into())),
                attempt - 1,
            );
        }
        let response = client.analyze(prompt, cancellation.child_token()).await;
        if !retryable_transport_error(&response) || attempt == MAX_ANALYSIS_TRANSPORT_ATTEMPTS {
            return (response, attempt);
        }
        on_retry(attempt);
        let delay = Duration::from_secs(attempt as u64);
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = cancellation.cancelled() => {
                return (Err(ClientError::Transport("analysis cancelled".into())), attempt);
            }
        }
    }
    unreachable!("bounded retry loop always returns")
}

fn batch_categories(batch: &[EvidencePacket]) -> Vec<String> {
    let mut categories = Vec::new();
    for packet in batch {
        let category = terminal::display_category(&packet.category).to_owned();
        if !categories.contains(&category) {
            categories.push(category);
        }
    }
    categories
}

fn analysis_progress_line(event: &AnalysisProgressEvent) -> String {
    match event {
        AnalysisProgressEvent::Started { total_batches } => {
            format!("========== 阶段 2/2 被测模型分析本地证据 | 共 {total_batches} 批 ==========")
        }
        AnalysisProgressEvent::BatchStarted {
            index,
            total,
            categories,
            test_ids,
        } => terminal::analysis_start_line(*index, *total, categories, test_ids),
        AnalysisProgressEvent::BatchRetry {
            index,
            total,
            categories,
            retry,
        } => terminal::analysis_retry_line(
            *index,
            *total,
            categories,
            *retry,
            MAX_ANALYSIS_TRANSPORT_ATTEMPTS,
        ),
        AnalysisProgressEvent::BatchFinished {
            index,
            total,
            categories,
            unavailable,
        } => terminal::analysis_finish_line(*index, *total, categories, *unavailable),
    }
}

fn retryable_transport_error(response: &Result<CandidateEnvelope, ClientError>) -> bool {
    matches!(
        response,
        Err(ClientError::Transport(_))
            | Err(ClientError::HttpStatus(429))
            | Err(ClientError::HttpStatus(500..=599))
    )
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
        report_test_id: review.report_test_id,
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
            report_test_id: packet.report_test_id.clone(),
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

fn consolidate_report_results(results: Vec<AnalysisTestResult>) -> Vec<AnalysisTestResult> {
    let mut grouped: BTreeMap<String, Vec<AnalysisTestResult>> = BTreeMap::new();
    for result in results {
        grouped
            .entry(result.report_test_id.clone())
            .or_default()
            .push(result);
    }
    crate::catalog::CATALOG
        .iter()
        .filter_map(|test| grouped.remove(test.id).map(merge_report_results))
        .collect()
}

fn merge_report_results(mut results: Vec<AnalysisTestResult>) -> AnalysisTestResult {
    if results
        .first()
        .is_some_and(|result| result.report_test_id == "057")
        && let Some(index) = results
            .iter()
            .position(|result| result.test_id == "057-wave-4")
    {
        let mut primary = results.remove(index);
        primary.test_id = "057".into();
        for reference in results {
            primary.limitations.push(format!(
                "{}（仅性能参考）：{}，{}",
                wave_label(&reference.test_id),
                result_status(&reference),
                result_reason(&reference)
            ));
            primary.observations.extend(reference.observations);
            primary.evidence_refs.extend(reference.evidence_refs);
        }
        return primary;
    }
    if results.len() == 1 {
        let mut result = results.pop().expect("one result exists");
        result.test_id = result.report_test_id.clone();
        return result;
    }

    let report_test_id = results[0].report_test_id.clone();
    let failed: Vec<_> = results
        .iter()
        .filter(|result| result.validated_status == Some(ValidatedStatus::Fail))
        .collect();
    let unavailable: Vec<_> = results
        .iter()
        .filter(|result| matches!(result.analysis_state, AnalysisState::AnalysisUnavailable))
        .collect();
    let (analysis_state, candidate_status, validated_status, decision_source, failure_cause) =
        if !failed.is_empty() {
            (
                AnalysisState::Available,
                Some(CandidateStatus::Fail),
                Some(ValidatedStatus::Fail),
                Some(DecisionSource::TargetModel),
                Some(join_wave_values(&failed, |result| {
                    result
                        .failure_cause
                        .clone()
                        .unwrap_or_else(|| "模型未提供失败原因".into())
                })),
            )
        } else if !unavailable.is_empty() {
            (AnalysisState::AnalysisUnavailable, None, None, None, None)
        } else {
            (
                AnalysisState::Available,
                Some(CandidateStatus::Pass),
                Some(ValidatedStatus::Pass),
                Some(DecisionSource::TargetModel),
                None,
            )
        };
    AnalysisTestResult {
        test_id: report_test_id.clone(),
        report_test_id,
        analysis_state,
        candidate_status,
        validated_status,
        decision_source,
        observations: results
            .iter()
            .flat_map(|result| result.observations.clone())
            .collect(),
        failure_cause,
        evidence_refs: results
            .iter()
            .flat_map(|result| result.evidence_refs.clone())
            .collect(),
        limitations: results
            .iter()
            .flat_map(|result| result.limitations.clone())
            .collect(),
        validation_notes: results
            .iter()
            .flat_map(|result| result.validation_notes.clone())
            .collect(),
    }
}

fn join_wave_values(
    results: &[&AnalysisTestResult],
    value: impl Fn(&AnalysisTestResult) -> String,
) -> String {
    results
        .iter()
        .map(|result| format!("{}：{}", wave_label(&result.test_id), value(result)))
        .collect::<Vec<_>>()
        .join("；")
}

fn wave_label(test_id: &str) -> String {
    test_id.strip_prefix("057-wave-").map_or_else(
        || test_id.to_owned(),
        |concurrency| format!("{concurrency} 并发"),
    )
}

fn summarize_concurrency(evidence: &ParsedEvidence) -> Vec<ConcurrencyWave> {
    let mut requests_by_wave: std::collections::BTreeMap<usize, Vec<&ParsedRequest>> =
        std::collections::BTreeMap::new();
    for request in evidence.requests.values() {
        if !evidence
            .tests
            .get("057")
            .is_some_and(|test| test.request_refs.contains(&request.request_id))
        {
            continue;
        }
        if let Some(concurrency) = concurrency_level(&request.request_id) {
            requests_by_wave
                .entry(concurrency)
                .or_default()
                .push(request);
        }
    }

    [4, 8, 16]
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
                average_response_time_seconds: (!times.is_empty() && times.len() == requests.len())
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
    (matches!(concurrency, 4 | 8 | 16) && index > 0 && index <= concurrency).then_some(concurrency)
}

/// 从证据日志还原上下文容量实测结论（与采集端共用同一套判定函数）。
fn summarize_context(evidence: &ParsedEvidence) -> ContextConclusion {
    // 同一目标可能因超时重试出现多条记录（-a1/-a2），保留最后一次尝试。
    let mut outcomes: BTreeMap<u64, ProbeOutcome> = BTreeMap::new();
    for request in evidence.requests.values() {
        if !evidence
            .tests
            .get("018")
            .is_some_and(|test| test.request_refs.contains(&request.request_id))
        {
            continue;
        }
        if is_calibration_request(&request.request_id) {
            continue;
        }
        let Some(target) = parse_probe_target(&request.request_id) else {
            continue;
        };
        let protocol = protocol_from_wire_name(
            request
                .metadata
                .get("protocol")
                .map(String::as_str)
                .unwrap_or(""),
        );
        let http_status = request
            .metrics
            .get("http_status")
            .and_then(|value| value.parse::<u16>().ok());
        let transport_outcome = request
            .metadata
            .get("transport_outcome")
            .map(String::as_str)
            .unwrap_or("unknown");
        outcomes.insert(
            target,
            classify_probe(
                protocol,
                http_status,
                transport_outcome,
                request.response_body.as_bytes(),
            ),
        );
    }
    crate::context_capacity::conclude(&outcomes.into_iter().collect::<Vec<_>>())
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
    let protocol = protocol_from_wire_name(
        request
            .metadata
            .get("protocol")
            .map(String::as_str)
            .unwrap_or(""),
    );
    http_success
        && transport_success
        && crate::protocol::matches_response(protocol, request.response_body.as_bytes())
}

fn requirement_levels() -> BTreeMap<&'static str, crate::catalog::RequirementLevel> {
    crate::catalog::CATALOG
        .iter()
        .map(|test| (test.id, test.requirement_level()))
        .collect()
}

fn apply_hard_evidence_rules(
    tests: &mut [AnalysisTestResult],
    evidence: &ParsedEvidence,
    context: &ContextConclusion,
    waves: &[ConcurrencyWave],
) {
    for result in tests
        .iter_mut()
        .filter(|result| matches!(result.test_id.as_str(), "018" | "057"))
    {
        if evidence
            .degraded_tests
            .iter()
            .any(|test| test.id == result.test_id)
        {
            continue;
        }
        let (status, detail) = if result.test_id == "018" {
            let status = match context.status {
                ContextStatus::Satisfied => Some(ValidatedStatus::Pass),
                ContextStatus::NotSatisfied => Some(ValidatedStatus::Fail),
                ContextStatus::Indeterminate => None,
            };
            (status, conclusion_sentence(context))
        } else {
            evaluate_minimum_concurrency(waves)
        };
        result.validated_status = status;
        result.analysis_state = if status.is_some() {
            AnalysisState::Available
        } else {
            AnalysisState::AnalysisUnavailable
        };
        result.decision_source = Some(DecisionSource::EvidenceRule);
        result.failure_cause = (status == Some(ValidatedStatus::Fail)).then(|| detail.clone());
        result.validation_notes.push(detail.clone());
        if status.is_none() {
            result.limitations.push(detail);
        }
        if let Some(test) = evidence.tests.get(&result.test_id) {
            result.evidence_refs = test
                .request_refs
                .iter()
                .map(|id| format!("request:{id}"))
                .collect();
        }
    }
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
    } else {
        let protocol = protocol_from_wire_name(
            request
                .metadata
                .get("protocol")
                .map(String::as_str)
                .unwrap_or(""),
        );
        if !crate::protocol::matches_response(protocol, request.response_body.as_bytes()) {
            reasons.push("响应协议无效或模型回复为空".into());
        }
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
        Some(DecisionSource::EvidenceRule) => "EVIDENCE_RULE",
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

fn normalized_category_failure(reason: &str) -> String {
    let lower = reason.to_ascii_lowercase();
    if lower.contains("transport_error")
        || lower.contains("tls handshake eof")
        || lower.contains("timeout")
        || reason.contains("超时")
        || reason.contains("连接中断")
        || reason.contains("空响应")
        || reason.contains("未获得任何响应")
    {
        return "请求连接中断或未返回有效响应".into();
    }
    if lower.contains("auto tool choice") || reason.contains("自动工具选择") {
        return "接口不支持当前自动工具选择配置".into();
    }
    if lower.contains("tool loop")
        || lower.contains("tool envelope")
        || reason.contains("工具闭环")
        || reason.contains("结果关联")
    {
        return "工具调用流程未完成".into();
    }
    if lower.contains("analysis unavailable")
        || reason.contains("分析不可用")
        || reason.contains("结果缺失")
    {
        return "本轮分析未完成，无法判断该项能力".into();
    }
    if lower.contains("passcriteria")
        || reason.contains("期望")
        || reason.contains("字段")
        || reason.contains("参数")
        || reason.contains("类型")
        || reason.contains("格式")
    {
        return "模型返回结果与检测要求不一致".into();
    }
    clean_category_failure(reason)
}

fn clean_category_failure(reason: &str) -> String {
    let mut cleaned = reason.replace("stderrExcerpt", "");
    cleaned = cleaned.replace("responseBodyExcerpt", "");
    cleaned = cleaned.replace("request_id", "请求");
    if let Some((prefix, _)) = cleaned.split_once("url (") {
        cleaned = prefix.trim_end_matches([' ', ':', '(', '"']).to_owned();
    }
    let first_sentence = cleaned
        .split(['。', '.', ';', '；'])
        .next()
        .unwrap_or(cleaned.as_str())
        .trim();
    let bounded: String = first_sentence.chars().take(96).collect();
    if bounded.is_empty() {
        "存在未通过项".into()
    } else {
        bounded
    }
}

fn category_failure_summary(
    members: &[&crate::catalog::TestCase],
    non_pass: &[&crate::catalog::TestCase],
    artifact: &SelfAnalysisArtifact,
) -> String {
    let passed = members.len().saturating_sub(non_pass.len());
    let mut causes = BTreeMap::<String, usize>::new();
    for test in non_pass {
        let reason = result_for(artifact, test.id)
            .map(result_reason)
            .unwrap_or_else(|| missing_result_reason(test));
        *causes
            .entry(normalized_category_failure(&reason))
            .or_default() += 1;
    }
    let mut causes: Vec<_> = causes.into_iter().collect();
    causes.sort_by(|(left_reason, left_count), (right_reason, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| left_reason.cmp(right_reason))
    });
    let causes = causes
        .into_iter()
        .map(|(reason, count)| format!("{reason}（{count} 项）"))
        .collect::<Vec<_>>()
        .join("；");
    format!(
        "{} 项，{} 项通过、{} 项未通过。主要问题包括：{}。",
        members.len(),
        passed,
        non_pass.len(),
        causes
    )
}

fn derive_gateway_compatibility(
    detected_protocol: &str,
    tests: &[AnalysisTestResult],
) -> GatewayCompatibility {
    let mut reasons = Vec::new();
    if !matches!(
        detected_protocol,
        "openai_chat" | "openai_responses" | "anthropic_messages" | "gemini_generate_content"
    ) {
        reasons.push(format!(
            "当前接口识别为 {detected_protocol}，不属于 AI模型网关层支持的数据结构协议。"
        ));
    }
    for test_id in GATEWAY_COMPATIBILITY_TEST_IDS {
        let Some(result) = result_for_tests(tests, test_id) else {
            reasons.push(format!(
                "{}未能完成兼容性判断：本轮未生成该项分析结果。",
                gateway_check_name(test_id)
            ));
            continue;
        };
        match result_status(result) {
            "PASS" => {}
            "FAIL" => reasons.push(format!(
                "{}：{}",
                gateway_check_name(test_id),
                concrete_failure_reason(result)
            )),
            _ => reasons.push(format!(
                "{}未能完成兼容性判断：{}",
                gateway_check_name(test_id),
                unavailable_reason(result)
            )),
        }
    }
    if reasons.is_empty() {
        GatewayCompatibility {
            status: GatewayCompatibilityStatus::Pass,
            statement: "本轮检测表明，该模型接口的同步响应、流式响应、流结束信号、工具调用参数及连续工具调用的结果关联，均符合 AI模型网关层所需的数据结构。".into(),
            reasons,
        }
    } else {
        GatewayCompatibility {
            status: GatewayCompatibilityStatus::Fail,
            statement: "本轮检测发现 AI模型网关层所需的数据结构存在未通过项。".into(),
            reasons,
        }
    }
}

fn result_for_tests<'a>(
    tests: &'a [AnalysisTestResult],
    test_id: &str,
) -> Option<&'a AnalysisTestResult> {
    tests.iter().find(|result| result.test_id == test_id)
}

fn gateway_check_name(test_id: &str) -> &'static str {
    match test_id {
        "002" => "接口响应结构",
        "004" => "同步响应",
        "005" => "流式响应",
        "006" => "流结束信号",
        "040" => "单工具调用",
        "041" => "工具选择",
        "043" => "工具调用参数校验",
        "047" => "连续工具调用",
        _ => "数据结构检查",
    }
}

fn concrete_failure_reason(result: &AnalysisTestResult) -> String {
    result
        .failure_cause
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| {
            (!result.observations.is_empty()).then(|| {
                format!(
                    "模型未提供具体失败原因；可观察到{}。",
                    result.observations.join("；")
                )
            })
        })
        .unwrap_or_else(|| "模型未提供可用于定位的失败原因。".into())
}

fn unavailable_reason(result: &AnalysisTestResult) -> String {
    result
        .limitations
        .iter()
        .find(|value| !value.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| "未返回分析结果。".into())
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
            "上下文容量按实测 Token 判定（先校准字符/Token 密度，再按实测密度构造目标 Token 数的探针，以服务端统计为准）；多轮修正状态也能保持。"
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
            "本轮首字节时间、完整响应耗时均有有效观测；重复请求均成功，P50/P95 可计算，4 并发全部成功且平均响应时间不超过 30 秒；8、16 并发仅作参考，具体失败数与平均时间见检测明细。"
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

    render_overall_conclusion(artifact, &mut output);
    render_category_summary(artifact, &mut output);
    render_concurrency_summary(artifact, &mut output);
    render_test_table(artifact, &mut output);
    render_non_pass_details(artifact, &mut output);
    render_evidence_degradations(artifact, &mut output);
    output
}

fn render_evidence_degradations(artifact: &SelfAnalysisArtifact, output: &mut String) {
    if artifact.evidence_degradations.is_empty() {
        return;
    }
    output.push_str("## 证据降级说明\n\n");
    output.push_str(
        "以下证据块在解析时损坏，已被跳过；相关检测项标记为分析不可用，其余检测不受影响。\n\n",
    );
    for entry in &artifact.evidence_degradations {
        let kind = if entry.kind == "request" {
            "请求"
        } else {
            "检测"
        };
        writeln!(
            output,
            "- {kind} {}：{}",
            entry.id,
            markdown_cell(&entry.reason)
        )
        .unwrap();
    }
    output.push('\n');
}

fn render_overall_conclusion(artifact: &SelfAnalysisArtifact, output: &mut String) {
    output.push_str("## 关键检测项结论\n\n");
    output.push_str("| 检测项 | 检测结果 | 说明 |\n|---|---|---|\n");
    render_minimum_model_requirements(artifact, output);
    output.push('\n');
    output.push_str("刚性 11 项，柔性 31 项。刚性项用于接入及部署门槛；柔性项影响质量与体验，不单独阻断使用。上下文沿用 124000 Token 的通过阈值，为 128K 要求保留余量；4 并发必须全部成功且平均响应时间不超过 30 秒，8、16 并发仅作参考。\n\n");
}

fn render_minimum_model_requirements(artifact: &SelfAnalysisArtifact, output: &mut String) {
    let (concurrency_status, concurrency_detail) = minimum_concurrency_requirement(artifact);
    writeln!(
        output,
        "| 模型最低并发要求（4 并发） | {concurrency_status} | {concurrency_detail} |"
    )
    .unwrap();

    let (context_status, context_detail) = minimum_context_requirement(artifact);
    writeln!(
        output,
        "| 模型最低上下文要求（128K） | {context_status} | {context_detail} |"
    )
    .unwrap();
}

fn minimum_concurrency_requirement(artifact: &SelfAnalysisArtifact) -> (&'static str, String) {
    let (status, detail) = evaluate_minimum_concurrency(&artifact.concurrency_waves);
    let label = match status {
        Some(ValidatedStatus::Pass) => "通过",
        Some(ValidatedStatus::Fail) => "不通过",
        None => "无法判断",
    };
    (label, detail)
}

fn evaluate_minimum_concurrency(waves: &[ConcurrencyWave]) -> (Option<ValidatedStatus>, String) {
    let Some(wave) = waves
        .iter()
        .find(|wave| wave.concurrency == MINIMUM_CONCURRENCY)
    else {
        return (None, "未采集到 4 并发结果，无法验证最低并发要求。".into());
    };
    let average = wave
        .average_response_time_seconds
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0);
    let average_detail = average.map_or_else(
        || "平均响应时间缺失或无效，无法验证 30 秒门槛".into(),
        |seconds| {
            format!(
                "平均响应时间 {:.1} ms，{} 30000 ms（30 秒）",
                seconds * 1000.0,
                if seconds <= MINIMUM_CONCURRENCY_MAX_AVERAGE_SECONDS {
                    "不超过"
                } else {
                    "超过"
                }
            )
        },
    );
    let detail = format!(
        "4 并发结果为 {}/{} 成功、{} 失败，{}。",
        wave.succeeded, wave.total_requests, wave.failed, average_detail
    );
    let status = if wave.failed > 0 {
        Some(ValidatedStatus::Fail)
    } else if wave.total_requests != MINIMUM_CONCURRENCY || wave.succeeded != MINIMUM_CONCURRENCY {
        None
    } else {
        average.map(|seconds| {
            if seconds <= MINIMUM_CONCURRENCY_MAX_AVERAGE_SECONDS {
                ValidatedStatus::Pass
            } else {
                ValidatedStatus::Fail
            }
        })
    };
    (status, detail)
}

fn minimum_context_requirement(artifact: &SelfAnalysisArtifact) -> (&'static str, String) {
    match artifact.context_conclusion.status {
        ContextStatus::Satisfied => ("通过", conclusion_sentence(&artifact.context_conclusion)),
        ContextStatus::NotSatisfied => {
            ("不通过", conclusion_sentence(&artifact.context_conclusion))
        }
        ContextStatus::Indeterminate => (
            "无法判断",
            conclusion_sentence(&artifact.context_conclusion),
        ),
    }
}

fn render_category_summary(artifact: &SelfAnalysisArtifact, output: &mut String) {
    output.push_str("## 能力分类检测结论\n\n");
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
            .copied()
            .filter(|test| {
                result_for(artifact, test.id)
                    .map(|result| result_status(result) != "PASS")
                    .unwrap_or(true)
            })
            .collect();
        let status = if non_pass.is_empty() {
            "通过"
        } else {
            "不通过"
        };
        let reason = if non_pass.is_empty() {
            verified_category_conclusion(category).into()
        } else {
            category_failure_summary(&members, &non_pass, artifact)
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
}

fn render_test_table(artifact: &SelfAnalysisArtifact, output: &mut String) {
    output.push_str(&format!(
        "## {} 项检测明细\n\n",
        crate::catalog::CATALOG.len()
    ));
    output
        .push_str("| 编号 | 分类 | 检测项 | 刚柔属性 | 结果 | 原因 |\n|---|---|---|---|---|---|\n");
    for test in crate::catalog::CATALOG {
        let result = result_for(artifact, test.id);
        let status = result.map(result_status).unwrap_or("ANALYSIS_UNAVAILABLE");
        let reason = result
            .map(result_reason)
            .unwrap_or_else(|| "分析不可用（结果缺失）".into());
        writeln!(
            output,
            "| {} | {} | {} | {} | {} | {} |",
            markdown_cell(test.id),
            markdown_cell(test.category),
            markdown_cell(test.name),
            test.requirement_level().label(),
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
    write_private_output(log_path, bytes, "self-analysis", |path| {
        Ok(collision_safe_output_path(path, extension))
    })
}

fn failed_curl_log_path(log_path: &Path) -> std::io::Result<PathBuf> {
    let stem = log_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("model-doctor");
    let initial = log_path.with_file_name(format!("{stem}-failed-curls.log"));
    if output_path_is_available(&initial)? {
        return Ok(initial);
    }

    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    for sequence in 1.. {
        let suffix = if sequence == 1 {
            timestamp.to_string()
        } else {
            format!("{timestamp}-{sequence}")
        };
        let candidate = log_path.with_file_name(format!("{stem}-failed-curls-{suffix}.log"));
        if output_path_is_available(&candidate)? {
            return Ok(candidate);
        }
    }
    unreachable!()
}

fn output_path_is_available(path: &Path) -> std::io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
    }
}

fn render_failed_curl_log(
    artifact: &SelfAnalysisArtifact,
    evidence: &ParsedEvidence,
) -> Result<String, AnalysisError> {
    let failed: Vec<_> = artifact
        .tests
        .iter()
        .filter(|result| result.validated_status == Some(ValidatedStatus::Fail))
        .collect();
    let mut output = String::new();
    output.push_str("MODEL DOCTOR FAILED CURL COMMANDS\n");
    writeln!(output, "failed_test_count: {}", failed.len()).unwrap();

    for result in failed {
        let test_id = &result.report_test_id;
        if !evidence.tests.contains_key(test_id) {
            return Err(EvidenceError::Malformed(format!(
                "failed test {test_id} is missing from evidence"
            ))
            .into());
        }
        let catalog_test = crate::catalog::CATALOG
            .iter()
            .find(|test| test.id == test_id)
            .ok_or_else(|| {
                EvidenceError::Malformed(format!("failed test {test_id} is missing from catalog"))
            })?;
        if result.evidence_refs.is_empty() {
            return Err(EvidenceError::Malformed(format!(
                "failed test {test_id} has no request evidence references"
            ))
            .into());
        }

        output.push('\n');
        writeln!(output, "========== FAILED TEST {test_id} BEGIN ==========").unwrap();
        writeln!(output, "test_id: {test_id}").unwrap();
        writeln!(output, "name: {}", catalog_test.name).unwrap();
        writeln!(output, "category: {}", catalog_test.category).unwrap();
        writeln!(
            output,
            "failure_cause: {}",
            result
                .failure_cause
                .as_deref()
                .unwrap_or("模型未提供失败原因")
        )
        .unwrap();

        for reference in &result.evidence_refs {
            let request_id = reference
                .strip_prefix("request:")
                .filter(|id| !id.is_empty())
                .ok_or_else(|| {
                    EvidenceError::Malformed(format!(
                        "failed test {test_id} has malformed request evidence reference {reference}"
                    ))
                })?;
            let request = evidence.requests.get(request_id).ok_or_else(|| {
                EvidenceError::Malformed(format!(
                    "failed test {test_id} references missing request {request_id}"
                ))
            })?;

            writeln!(output, "evidence_ref: {reference}").unwrap();
            output.push_str("----- CURL COMMAND BEGIN -----\n");
            output.push_str(&request.curl_command);
            if !request.curl_command.ends_with('\n') {
                output.push('\n');
            }
            output.push_str("----- CURL COMMAND END -----\n");
        }
        writeln!(output, "========== FAILED TEST {test_id} END ==========").unwrap();
    }

    Ok(output)
}

fn write_failed_curl_log(
    log_path: &Path,
    artifact: &SelfAnalysisArtifact,
    evidence: &ParsedEvidence,
) -> Result<PathBuf, AnalysisError> {
    let rendered = render_failed_curl_log(artifact, evidence)?;
    write_private_output(
        log_path,
        rendered.as_bytes(),
        "failed-curls",
        failed_curl_log_path,
    )
}

fn write_private_output(
    log_path: &Path,
    bytes: &[u8],
    temporary_name: &str,
    output_path: impl Fn(&Path) -> std::io::Result<PathBuf>,
) -> Result<PathBuf, AnalysisError> {
    let directory = log_path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(directory)?;
    let temp_path = loop {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let candidate = directory.join(format!(
            ".model-doctor-{temporary_name}-{}-{sequence}.tmp",
            std::process::id(),
        ));
        match create_new_private_file(&candidate) {
            Ok(mut file) => {
                if let Err(error) = write_complete(&mut file, bytes) {
                    drop(file);
                    let _ = std::fs::remove_file(&candidate);
                    return Err(error.into());
                }
                break candidate;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    };

    let result = publish_private_output(
        &temp_path,
        log_path,
        output_path,
        |source, destination| std::fs::hard_link(source, destination),
        |path| std::fs::remove_file(path),
    );
    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result
}

fn publish_private_output(
    temp_path: &Path,
    log_path: &Path,
    output_path: impl Fn(&Path) -> std::io::Result<PathBuf>,
    hard_link: impl Fn(&Path, &Path) -> std::io::Result<()>,
    remove_file: impl Fn(&Path) -> std::io::Result<()>,
) -> Result<PathBuf, AnalysisError> {
    loop {
        let destination = output_path(log_path)?;
        match hard_link(temp_path, &destination) {
            Ok(()) => match remove_file(temp_path) {
                Ok(()) => return Ok(destination),
                Err(error) => {
                    let _ = remove_file(&destination);
                    let _ = remove_file(temp_path);
                    return Err(error.into());
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                let _ = remove_file(temp_path);
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
    use crate::analysis::evidence_reader::{complete_test_log, read, two_request_log};
    use crate::analysis::packet::{EvidencePacket, default_batches};
    use crate::analysis::validator::{CandidateEnvelope, CandidateReview, CandidateStatus};
    use crate::catalog::CATALOG;
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

        // 通用 fixture 没有容量/并发探针，模型声称 PASS 不能补足证据。
        assert_eq!(outcome.unavailable_count, 2);
        assert_eq!(outcome.available_count, 40);
        assert_eq!(client.call_count(), batches.len() + 1);
        assert!(client.prompts()[1].contains("missing reviews"));
        let output = std::fs::read_to_string(&outcome.path).unwrap();
        assert!(output.contains(SELF_ANALYSIS_SCHEMA_VERSION));
        let artifact: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(artifact["requirementLevels"]["004"], "hard");
        assert_eq!(artifact["requirementLevels"]["018"], "hard");
        assert_eq!(artifact["requirementLevels"]["041"], "hard");
        assert_eq!(artifact["requirementLevels"]["057"], "hard");
        assert_eq!(artifact["requirementLevels"]["043"], "soft");
        assert!(output.contains(PROMPT_VERSION));
        assert!(output.contains("\"gatewayCompatibility\""));
        assert!(!output.contains("openCodexCompatibility"));
        assert!(!output.contains("secret-key"));
        let failed_curls = std::fs::read_to_string(&outcome.failed_curl_log_path).unwrap();
        assert!(failed_curls.contains("failed_test_count: 0"));
        assert!(!failed_curls.contains("========== FAILED TEST"));
        let markdown = std::fs::read_to_string(&outcome.markdown_path).unwrap();
        assert!(markdown.contains("## 能力分类检测结论"));
        assert_eq!(markdown.matches("| PASS |").count(), 40);
        assert!(!markdown.contains("secret-key"));
        drop(directory);
    }

    #[tokio::test]
    async fn repairs_invalid_batch_up_to_five_times_then_succeeds() {
        let (_directory, settings, batches) = fixture().await;
        let mut responses = (0..5)
            .map(|_| Err(ClientError::InvalidJson("missing testId".into())))
            .collect::<Vec<_>>();
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

        assert_eq!(outcome.unavailable_count, 2);
        assert_eq!(outcome.available_count, 40);
        assert_eq!(client.call_count(), batches.len() + 5);
        assert!(client.prompts()[5].contains("missing testId"));
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

        assert_eq!(outcome.unavailable_count, 2);
        assert_eq!(client.call_count(), batches.len() + 1);
        assert!(client.prompts()[1].contains("no protocol-native assistant content"));
    }

    #[tokio::test]
    async fn retries_a_transient_analysis_transport_error_then_succeeds() {
        let (_directory, settings, batches) = fixture().await;
        let mut responses = vec![Err(ClientError::Transport("timeout".into()))];
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

        assert_eq!(outcome.unavailable_count, 2);
        assert_eq!(outcome.available_count, 40);
        assert_eq!(client.call_count(), batches.len() + 1);
    }

    #[tokio::test]
    async fn analysis_progress_reports_batch_start_retry_and_completion() {
        let (_directory, settings, batches) = fixture().await;
        let mut responses = vec![Err(ClientError::Transport("timeout".into()))];
        responses.push(Ok(valid_envelope_for(&batches[0])));
        responses.extend(
            batches[1..]
                .iter()
                .map(|batch| Ok(valid_envelope_for(batch))),
        );
        let client = FakeClient::new(responses);
        let mut lines = Vec::new();

        let outcome = analyze_with_client_and_progress(
            settings,
            &client,
            CancellationToken::new(),
            &mut |event| lines.push(analysis_progress_line(&event)),
        )
        .await
        .unwrap();

        assert_eq!(outcome.unavailable_count, 2);
        assert_eq!(
            lines[0],
            format!(
                "========== 阶段 2/2 被测模型分析本地证据 | 共 {} 批 ==========",
                batches.len()
            )
        );
        assert_eq!(lines[1], "[自分析 01/11] 接口与协议 | 001-004 | 正在分析");
        assert_eq!(lines[2], "[自分析 01/11] 接口与协议 | 重试 1/3");
        assert_eq!(lines[3], "[自分析 01/11] 接口与协议 | 已完成");
    }

    #[tokio::test]
    async fn analysis_progress_marks_unavailable_batch() {
        let (_directory, settings, batches) = fixture().await;
        let mut responses = (0..3)
            .map(|_| Err(ClientError::Transport("offline".into())))
            .collect::<Vec<_>>();
        responses.extend(
            batches[1..]
                .iter()
                .map(|batch| Ok(valid_envelope_for(batch))),
        );
        let client = FakeClient::new(responses);
        let mut lines = Vec::new();

        let outcome = analyze_with_client_and_progress(
            settings,
            &client,
            CancellationToken::new(),
            &mut |event| lines.push(analysis_progress_line(&event)),
        )
        .await
        .unwrap();

        assert_eq!(outcome.unavailable_count, batches[0].len() + 2);
        assert!(lines.contains(&"[自分析 01/11] 接口与协议 | 分析不可用".into()));
    }

    #[tokio::test]
    async fn degraded_evidence_still_produces_full_report() {
        let directory = tempfile::tempdir().unwrap();
        let log_path = directory.path().join("doctor.log");
        let mut log = two_request_log();
        let target = "----- RESPONSE BODY BEGIN -----\ncmVzcG9uc2U=";
        let index = log.find(target).unwrap();
        log.replace_range(
            index..index + target.len(),
            "----- RESPONSE BODY BEGIN -----\n!!!not-base64!!!",
        );
        std::fs::write(&log_path, log).unwrap();
        let url = "https://example.test/v1/chat/completions".parse().unwrap();
        let redactor = Redactor::new("secret-key", &url);
        let batches = default_batches(&read(&log_path, &redactor).unwrap())
            .await
            .unwrap();
        let responses = batches
            .iter()
            .map(|batch| Ok(valid_envelope_for(batch)))
            .collect();
        let client = FakeClient::new(responses);
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

        let outcome = analyze_with_client(settings, &client, CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(outcome.available_count, 1);
        assert_eq!(outcome.unavailable_count, CATALOG.len() - 1);
        let markdown = std::fs::read_to_string(&outcome.markdown_path).unwrap();
        assert!(markdown.contains("证据降级说明"));
        assert!(markdown.contains("test-shared"));
        assert!(markdown.contains("引用的请求 test-shared 证据损坏"));
    }

    #[test]
    fn analysis_retries_only_transient_transport_and_http_errors() {
        let transient = [
            Err(ClientError::Transport("timeout".into())),
            Err(ClientError::HttpStatus(429)),
            Err(ClientError::HttpStatus(500)),
        ];

        assert!(transient.iter().all(retryable_transport_error));
        assert!(!retryable_transport_error(&Err(ClientError::HttpStatus(
            400
        ))));
    }

    #[tokio::test]
    async fn failed_batch_is_explicit_and_later_batches_continue() {
        let (_directory, settings, batches) = fixture().await;
        let mut responses = (0..3)
            .map(|_| Err(ClientError::Transport("offline".into())))
            .collect::<Vec<_>>();
        responses.extend(
            batches[1..]
                .iter()
                .map(|batch| Ok(valid_envelope_for(batch))),
        );
        let client = FakeClient::new(responses);

        let outcome = analyze_with_client(settings, &client, CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(outcome.unavailable_count, batches[0].len() + 2);
        assert_eq!(outcome.available_count, 40 - batches[0].len());
        assert_eq!(client.call_count(), batches.len() + 2);
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
        assert_eq!(outcome.unavailable_count, 42);
        assert_eq!(client.call_count(), 0);
        let output = std::fs::read_to_string(outcome.path).unwrap();
        assert!(output.contains("\"collectorVersion\": \"0.13.0\""));
        assert!(
            output.contains("\"evidenceSchemaVersion\": \"llm-capability-doctor.evidence.v4\"")
        );
        assert!(output.contains("\"analysisState\": \"ANALYSIS_UNAVAILABLE\""));
        let markdown = std::fs::read_to_string(outcome.markdown_path).unwrap();
        assert!(markdown.contains("| 接口与协议 | 不通过 |"));
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

    #[test]
    fn failed_curl_log_path_uses_source_stem() {
        let directory = tempfile::tempdir().unwrap();
        let log = directory.path().join("doctor.log");

        assert_eq!(
            failed_curl_log_path(&log).unwrap(),
            directory.path().join("doctor-failed-curls.log")
        );
    }

    #[test]
    fn failed_curl_log_path_avoids_collisions_without_overwriting() {
        let directory = tempfile::tempdir().unwrap();
        let log = directory.path().join("doctor.log");
        let first = failed_curl_log_path(&log).unwrap();
        std::fs::write(&first, "existing failed curls").unwrap();

        let second = failed_curl_log_path(&log).unwrap();

        assert_ne!(first, second);
        assert_eq!(
            std::fs::read_to_string(first).unwrap(),
            "existing failed curls"
        );
    }

    #[cfg(unix)]
    #[test]
    fn failed_curl_log_path_skips_dangling_symlink() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let log = directory.path().join("doctor.log");
        let occupied = directory.path().join("doctor-failed-curls.log");
        symlink(directory.path().join("missing-target"), &occupied).unwrap();

        let candidate = failed_curl_log_path(&log).unwrap();

        assert_ne!(candidate, occupied);
    }

    #[test]
    fn published_output_is_removed_when_temp_cleanup_fails() {
        use std::cell::Cell;
        use std::io::ErrorKind;

        let directory = tempfile::tempdir().unwrap();
        let log_path = directory.path().join("doctor.log");
        let temp_path = directory.path().join(".temporary-output");
        let destination = directory.path().join("doctor-failed-curls.log");
        std::fs::write(&temp_path, "output").unwrap();
        let fail_first_temp_removal = Cell::new(true);

        let error = publish_private_output(
            &temp_path,
            &log_path,
            |_| Ok(destination.clone()),
            |source, destination| std::fs::hard_link(source, destination),
            |path| {
                if path == temp_path && fail_first_temp_removal.replace(false) {
                    return Err(std::io::Error::new(
                        ErrorKind::PermissionDenied,
                        "injected temporary cleanup failure",
                    ));
                }
                std::fs::remove_file(path)
            },
        )
        .unwrap_err();

        assert!(matches!(
            error,
            AnalysisError::Output(error)
                if error.kind() == ErrorKind::PermissionDenied
                    && error.to_string() == "injected temporary cleanup failure"
        ));
        assert!(!destination.exists());
        assert!(!temp_path.exists());
    }

    #[test]
    fn failed_curl_log_writes_only_validated_failures() {
        let directory = tempfile::tempdir().unwrap();
        let log_path = directory.path().join("doctor.log");
        std::fs::write(&log_path, complete_test_log()).unwrap();
        let url = Url::parse("https://example.test/v1/chat/completions").unwrap();
        let evidence = read(&log_path, &Redactor::new("secret-key", &url)).unwrap();
        let mut failed =
            available_markdown_result("001", CandidateStatus::Fail, Some("服务端拒绝了请求。"));
        failed.evidence_refs = vec!["request:test-shared".into()];
        let artifact = markdown_fixture(vec![
            failed,
            available_markdown_result("002", CandidateStatus::Pass, None),
            AnalysisTestResult {
                test_id: "003".into(),
                report_test_id: "003".into(),
                analysis_state: AnalysisState::AnalysisUnavailable,
                candidate_status: None,
                validated_status: None,
                decision_source: None,
                observations: Vec::new(),
                failure_cause: None,
                evidence_refs: Vec::new(),
                limitations: vec!["offline".into()],
                validation_notes: Vec::new(),
            },
        ]);

        let output_path = write_failed_curl_log(&log_path, &artifact, &evidence).unwrap();
        let output = std::fs::read_to_string(output_path).unwrap();

        assert!(output.contains("failed_test_count: 1"));
        assert!(output.contains("test_id: 001"));
        assert!(output.contains("name: URL 可达性"));
        assert!(output.contains("category: 接口与协议"));
        assert!(output.contains("failure_cause: 服务端拒绝了请求。"));
        assert!(output.contains("evidence_ref: request:test-shared"));
        assert!(output.contains("curl 'https://example.test/v1/chat/completions'"));
        assert!(!output.contains("test_id: 002"));
        assert!(!output.contains("name: 协议识别"));
        assert!(!output.contains("test_id: 003"));
        assert!(!output.contains("name: 鉴权与模型接受"));
    }

    #[test]
    fn failed_curl_log_uses_catalog_metadata_instead_of_evidence_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let log_path = directory.path().join("doctor.log");
        std::fs::write(&log_path, complete_test_log()).unwrap();
        let url = Url::parse("https://example.test/v1/chat/completions").unwrap();
        let mut evidence = read(&log_path, &Redactor::new("secret-key", &url)).unwrap();
        let test = evidence.tests.get_mut("001").unwrap();
        test.name = "secret-forged-name".into();
        test.category = "secret-forged-category".into();
        let mut failed =
            available_markdown_result("001", CandidateStatus::Fail, Some("服务端拒绝了请求。"));
        failed.evidence_refs = vec!["request:test-shared".into()];
        let artifact = markdown_fixture(vec![failed]);

        let output = render_failed_curl_log(&artifact, &evidence).unwrap();

        assert!(output.contains("name: URL 可达性"));
        assert!(output.contains("category: 接口与协议"));
        assert!(!output.contains("secret-forged-name"));
        assert!(!output.contains("secret-forged-category"));
    }

    #[test]
    fn failed_curl_log_repeats_shared_request_for_each_failed_test() {
        let directory = tempfile::tempdir().unwrap();
        let log_path = directory.path().join("doctor.log");
        std::fs::write(&log_path, complete_test_log()).unwrap();
        let url = Url::parse("https://example.test/v1/chat/completions").unwrap();
        let evidence = read(&log_path, &Redactor::new("secret-key", &url)).unwrap();
        let mut first =
            available_markdown_result("001", CandidateStatus::Fail, Some("首次检测失败。"));
        first.evidence_refs = vec!["request:test-shared".into()];
        let mut second =
            available_markdown_result("002", CandidateStatus::Fail, Some("第二次检测失败。"));
        second.evidence_refs = vec!["request:test-shared".into()];
        let artifact = markdown_fixture(vec![first, second]);

        let output_path = write_failed_curl_log(&log_path, &artifact, &evidence).unwrap();
        let output = std::fs::read_to_string(output_path).unwrap();

        for test_id in ["001", "002"] {
            let start = output
                .find(&format!(
                    "========== FAILED TEST {test_id} BEGIN =========="
                ))
                .unwrap();
            let end = output[start..]
                .find(&format!("========== FAILED TEST {test_id} END =========="))
                .unwrap();
            let block = &output[start..start + end];
            assert!(block.contains("evidence_ref: request:test-shared"));
            assert!(block.contains("curl 'https://example.test/v1/chat/completions'"));
        }
        assert_eq!(
            output
                .matches("curl 'https://example.test/v1/chat/completions'")
                .count(),
            2
        );
    }

    #[test]
    fn failed_curl_log_writes_empty_file_for_all_pass() {
        let directory = tempfile::tempdir().unwrap();
        let log_path = directory.path().join("doctor.log");
        std::fs::write(&log_path, complete_test_log()).unwrap();
        let url = Url::parse("https://example.test/v1/chat/completions").unwrap();
        let evidence = read(&log_path, &Redactor::new("secret-key", &url)).unwrap();
        let artifact = markdown_fixture(vec![available_markdown_result(
            "001",
            CandidateStatus::Pass,
            None,
        )]);

        let output_path = write_failed_curl_log(&log_path, &artifact, &evidence).unwrap();
        let output = std::fs::read_to_string(output_path).unwrap();

        assert!(output.contains("failed_test_count: 0"));
        assert!(!output.contains("========== FAILED TEST"));
    }

    #[test]
    fn failed_curl_log_rejects_malformed_request_reference_without_writing() {
        let directory = tempfile::tempdir().unwrap();
        let log_path = directory.path().join("doctor.log");
        std::fs::write(&log_path, complete_test_log()).unwrap();
        let url = Url::parse("https://example.test/v1/chat/completions").unwrap();
        let evidence = read(&log_path, &Redactor::new("secret-key", &url)).unwrap();
        let mut failed =
            available_markdown_result("001", CandidateStatus::Fail, Some("服务端拒绝了请求。"));
        failed.evidence_refs = vec!["response:test-shared".into()];
        let artifact = markdown_fixture(vec![failed]);

        let error = write_failed_curl_log(&log_path, &artifact, &evidence).unwrap_err();

        assert!(matches!(
            error,
            AnalysisError::Evidence(EvidenceError::Malformed(message))
                if message == "failed test 001 has malformed request evidence reference response:test-shared"
        ));
        assert!(!failed_curl_log_path(&log_path).unwrap().exists());
    }

    #[test]
    fn failed_curl_log_rejects_missing_request_reference_without_writing() {
        let directory = tempfile::tempdir().unwrap();
        let log_path = directory.path().join("doctor.log");
        std::fs::write(&log_path, complete_test_log()).unwrap();
        let url = Url::parse("https://example.test/v1/chat/completions").unwrap();
        let evidence = read(&log_path, &Redactor::new("secret-key", &url)).unwrap();
        let mut failed =
            available_markdown_result("001", CandidateStatus::Fail, Some("服务端拒绝了请求。"));
        failed.evidence_refs = vec!["request:not-recorded".into()];
        let artifact = markdown_fixture(vec![failed]);

        let error = write_failed_curl_log(&log_path, &artifact, &evidence).unwrap_err();

        assert!(matches!(
            error,
            AnalysisError::Evidence(EvidenceError::Malformed(message))
                if message == "failed test 001 references missing request not-recorded"
        ));
        assert!(!failed_curl_log_path(&log_path).unwrap().exists());
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

        for path in [
            outcome.path,
            outcome.markdown_path,
            outcome.failed_curl_log_path,
        ] {
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
    fn markdown_all_pass_has_eight_satisfied_categories_and_42_rows() {
        let artifact = markdown_fixture(
            crate::catalog::CATALOG
                .iter()
                .map(|test| available_markdown_result(test.id, CandidateStatus::Pass, None))
                .collect(),
        );

        let markdown = render_markdown(&artifact);

        assert!(markdown.contains("## 关键检测项结论"));
        assert!(markdown.contains("## 能力分类检测结论"));
        assert!(markdown.contains("## 42 项检测明细"));
        assert!(!markdown.contains("## 总体结论"));
        assert!(!markdown.contains("## 大分类结论"));
        assert!(markdown.contains("| 接口与协议 | 通过 |"));
        assert!(markdown.contains("| 护栏与词汇 | 通过 |"));
        assert_eq!(markdown.matches("| PASS |").count(), 42);
        assert!(!markdown.contains("## 非通过项详情"));
    }

    #[test]
    fn markdown_gateway_compatibility_passes_without_internal_test_ids() {
        let artifact = markdown_fixture(
            crate::catalog::CATALOG
                .iter()
                .map(|test| available_markdown_result(test.id, CandidateStatus::Pass, None))
                .collect(),
        );

        let markdown = render_markdown(&artifact);

        assert!(markdown.contains("## 关键检测项结论"));
        assert!(markdown.contains("| 检测项 | 检测结果 | 说明 |"));
        assert!(!markdown.contains("AI模型网关层数据结构兼容性"));
    }

    #[test]
    fn markdown_overall_conclusion_reports_satisfied_minimum_concurrency_and_context() {
        let mut artifact = markdown_fixture(
            crate::catalog::CATALOG
                .iter()
                .map(|test| available_markdown_result(test.id, CandidateStatus::Pass, None))
                .collect(),
        );
        artifact.concurrency_waves = vec![ConcurrencyWave {
            concurrency: 4,
            total_requests: 4,
            succeeded: 4,
            failed: 0,
            average_response_time_seconds: Some(1.2),
            failures: Vec::new(),
        }];
        artifact.context_conclusion =
            crate::context_capacity::ContextConclusion::satisfied_with(128_000, true, None, false);

        let markdown = render_markdown(&artifact);
        let overall = markdown.split("## 能力分类检测结论").next().unwrap();

        assert!(overall.contains("| 模型最低并发要求（4 并发） | 通过 |"));
        assert!(
            overall.contains("4/4 成功、0 失败，平均响应时间 1200.0 ms，不超过 30000 ms（30 秒）")
        );
        assert!(overall.contains("| 模型最低上下文要求（128K） | 通过 |"));
        assert!(overall.contains("经检测，上下文不低于128K，满足智能体部署所需要的128K的要求。"));
    }

    #[test]
    fn markdown_overall_conclusion_reports_measured_context_brackets() {
        let mut artifact = markdown_fixture(
            crate::catalog::CATALOG
                .iter()
                .map(|test| available_markdown_result(test.id, CandidateStatus::Pass, None))
                .collect(),
        );
        artifact.context_conclusion = crate::context_capacity::ContextConclusion::satisfied_with(
            248_000,
            true,
            Some(496_000),
            false,
        );

        let markdown = render_markdown(&artifact);
        let overall = markdown.split("## 能力分类检测结论").next().unwrap();

        assert!(overall.contains("| 模型最低上下文要求（128K） | 通过 |"));
        assert!(
            overall.contains("经检测，上下文在248K-496K左右，满足智能体部署所需要的128K的要求。")
        );
    }

    #[test]
    fn markdown_overall_conclusion_reports_indeterminate_context() {
        let mut artifact = markdown_fixture(
            crate::catalog::CATALOG
                .iter()
                .map(|test| available_markdown_result(test.id, CandidateStatus::Pass, None))
                .collect(),
        );
        artifact.context_conclusion =
            crate::context_capacity::ContextConclusion::indeterminate("124K的请求连续超时");

        let markdown = render_markdown(&artifact);
        let overall = markdown.split("## 能力分类检测结论").next().unwrap();

        assert!(overall.contains("| 模型最低上下文要求（128K） | 无法判断 |"));
        assert!(overall.contains("无法判断：124K的请求连续超时。本轮不计通过或不通过。"));
    }

    #[test]
    fn slow_successful_concurrency_fails_the_thirty_second_requirement() {
        let mut artifact = markdown_fixture(
            crate::catalog::CATALOG
                .iter()
                .map(|test| available_markdown_result(test.id, CandidateStatus::Pass, None))
                .collect(),
        );
        artifact.concurrency_waves = vec![ConcurrencyWave {
            concurrency: 4,
            total_requests: 4,
            succeeded: 4,
            failed: 0,
            average_response_time_seconds: Some(30.1),
            failures: Vec::new(),
        }];
        artifact.context_conclusion =
            crate::context_capacity::ContextConclusion::not_satisfied(Some(62_100), 93_000, false);

        let markdown = render_markdown(&artifact);
        let overall = markdown.split("## 能力分类检测结论").next().unwrap();

        assert!(overall.contains("| 模型最低并发要求（4 并发） | 不通过 |"));
        assert!(overall.contains("平均响应时间 30100.0 ms，超过 30000 ms（30 秒）"));
        assert!(overall.contains("| 模型最低上下文要求（128K） | 不通过 |"));
        assert!(
            overall.contains("经检测，上下文在62.1K-93K左右，不满足智能体部署所需要的128K的要求。")
        );
    }

    #[test]
    fn markdown_gateway_compatibility_explains_actual_failure_without_test_id() {
        let mut tests = crate::catalog::CATALOG
            .iter()
            .map(|test| available_markdown_result(test.id, CandidateStatus::Pass, None))
            .collect::<Vec<_>>();
        let result = tests
            .iter_mut()
            .find(|result| result.test_id == "043")
            .unwrap();
        result.candidate_status = Some(CandidateStatus::Fail);
        result.validated_status = Some(ValidatedStatus::Fail);
        result.failure_cause = Some(
            "期望 get_weather 同时包含 city、unit、days 三个字段；实际调用缺少 days 字段。".into(),
        );

        let markdown = render_markdown(&markdown_fixture(tests));
        let overall = markdown.split("## 能力分类检测结论").next().unwrap();

        assert!(overall.contains("| 检测项 | 检测结果 | 说明 |"));
        assert!(!overall.contains("AI模型网关层数据结构兼容性"));
        assert!(!overall.contains("043 必填参数与类型枚举"));
    }

    #[test]
    fn markdown_gateway_compatibility_explains_analysis_unavailable() {
        let mut tests = crate::catalog::CATALOG
            .iter()
            .map(|test| available_markdown_result(test.id, CandidateStatus::Pass, None))
            .collect::<Vec<_>>();
        let result = tests
            .iter_mut()
            .find(|result| result.test_id == "041")
            .unwrap();
        result.analysis_state = AnalysisState::AnalysisUnavailable;
        result.candidate_status = None;
        result.validated_status = None;
        result.decision_source = None;
        result.limitations = vec!["自分析请求在 300 秒内超时，重试后未返回结果。".into()];

        let markdown = render_markdown(&markdown_fixture(tests));
        let overall = markdown.split("## 能力分类检测结论").next().unwrap();

        assert!(overall.contains("| 检测项 | 检测结果 | 说明 |"));
        assert!(!overall.contains("AI模型网关层数据结构兼容性"));
        assert!(!overall.contains("工具选择未能完成兼容性判断"));
        assert!(!overall.contains("041 工具选择"));
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
            "上下文容量按实测 Token 判定（先校准字符/Token 密度，再按实测密度构造目标 Token 数的探针，以服务端统计为准）；多轮修正状态也能保持。"
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
                concurrency: 16,
                total_requests: 16,
                succeeded: 15,
                failed: 1,
                average_response_time_seconds: None,
                failures: vec![ConcurrencyFailure {
                    request_id: "test-057-c16-9".into(),
                    error: "timeout".into(),
                }],
            },
        ];

        let markdown = render_markdown(&artifact);

        assert!(markdown.contains("## 并发响应明细"));
        assert!(markdown.contains("| 4 | 4 | 4 | 0 | 420.0 ms |"));
        assert!(markdown.contains("| 8 | 8 | 7 | 1 | 880.0 ms |"));
        assert!(markdown.contains("| 16 | 16 | 16 | 0 | 1240.0 ms |"));
        assert!(markdown.contains("| 16 | 16 | 15 | 1 | - |"));
        assert!(!markdown.contains("失败请求："));
        assert!(!markdown.contains("request_id=test-057-c8-3"));
        assert!(!markdown.contains("request_id=test-057-c16-9"));
        assert!(!markdown.contains("HTTP 500: upstream unavailable"));
        assert!(!markdown.contains("timeout"));
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

        assert!(markdown.contains("| 工具调用 | 不通过 |"));
        assert!(markdown.contains("11 项，10 项通过、1 项未通过"));
        assert!(markdown.contains("工具调用流程未完成（1 项）"));
        assert!(markdown.contains("### 046 官方工具协议结构合规"));
        assert!(markdown.contains("tool envelope incomplete"));
    }

    #[test]
    fn markdown_category_failure_summary_aggregates_distinct_causes() {
        let mut results = crate::catalog::CATALOG
            .iter()
            .map(|test| available_markdown_result(test.id, CandidateStatus::Pass, None))
            .collect::<Vec<_>>();
        for (id, cause) in [
            (
                "040",
                "请求发生 transport_error：tls handshake eof，url=https://example.test",
            ),
            (
                "041",
                "stderrExcerpt=transport_error，request_id=test-041，tls handshake eof",
            ),
            (
                "043",
                "BadRequestError: auto tool choice requires --enable-auto-tool-choice",
            ),
        ] {
            let result = results
                .iter_mut()
                .find(|result| result.test_id == id)
                .unwrap();
            result.candidate_status = Some(CandidateStatus::Fail);
            result.validated_status = Some(ValidatedStatus::Fail);
            result.failure_cause = Some(cause.into());
        }

        let markdown = render_markdown(&markdown_fixture(results));
        let category_row = markdown
            .lines()
            .find(|line| line.starts_with("| 工具调用 |"))
            .unwrap();

        assert!(category_row.contains("11 项，8 项通过、3 项未通过"));
        assert!(category_row.contains("请求连接中断或未返回有效响应（2 项）"));
        assert!(category_row.contains("接口不支持当前自动工具选择配置（1 项）"));
        assert!(!category_row.contains("https://example.test"));
        assert!(!category_row.contains("request_id"));
        assert!(!category_row.contains("transportOutcome"));
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

        assert!(markdown.contains("| 工具调用 | 不通过 |"));
        assert!(markdown.contains("11 项，10 项通过、1 项未通过"));
        assert!(markdown.contains("本轮分析未完成，无法判断该项能力（1 项）"));
        assert!(markdown.contains("049 工具失败恢复"));
        assert!(markdown.contains("- 原因：分析不可用：offline"));
        assert!(markdown.contains("| ANALYSIS_UNAVAILABLE |"));
        assert!(markdown.contains("分析不可用：offline"));
        assert!(!markdown.contains("049 工具失败恢复：模型失败"));
    }

    #[test]
    fn markdown_non_pass_details_omit_observations_references_and_limitations() {
        let mut result = available_markdown_result(
            "046",
            CandidateStatus::Fail,
            Some("tool envelope incomplete"),
        );
        result.observations = vec!["model observation".into()];
        result.evidence_refs = vec!["request:test-046".into()];
        result.limitations = vec!["limited evidence".into()];

        let markdown = render_markdown(&markdown_fixture(vec![result]));

        assert!(markdown.contains("- 原因：tool envelope incomplete"));
        assert!(!markdown.contains("**模型观察**"));
        assert!(!markdown.contains("**证据引用**"));
        assert!(!markdown.contains("**限制说明**"));
        assert!(!markdown.contains("model observation"));
        assert!(!markdown.contains("request:test-046"));
        assert!(!markdown.contains("limited evidence"));
    }

    #[test]
    fn concurrency_wave_results_merge_into_one_report_item() {
        let mut wave_results = [4, 8, 16]
            .into_iter()
            .map(|concurrency| {
                let mut result = available_markdown_result(
                    &format!("057-wave-{concurrency}"),
                    CandidateStatus::Pass,
                    None,
                );
                result.report_test_id = "057".into();
                result
            })
            .collect::<Vec<_>>();
        let failed = wave_results
            .iter_mut()
            .find(|result| result.test_id == "057-wave-16")
            .unwrap();
        failed.candidate_status = Some(CandidateStatus::Fail);
        failed.validated_status = Some(ValidatedStatus::Fail);
        failed.failure_cause = Some("16 个并发请求中有 1 个请求超时。".into());

        let merged = consolidate_report_results(wave_results);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].test_id, "057");
        assert_eq!(merged[0].validated_status, Some(ValidatedStatus::Pass));
        assert!(merged[0].failure_cause.is_none());
        assert!(
            merged[0]
                .limitations
                .iter()
                .any(|detail| detail.contains("16 并发（仅性能参考）") && detail.contains("FAIL"))
        );
    }

    #[tokio::test]
    async fn four_concurrent_requests_require_four_valid_replies_despite_model_pass() {
        let (_directory, settings, _) = fixture().await;
        let mut evidence = read(
            &settings.log_path,
            &Redactor::new("secret-key", &settings.url),
        )
        .unwrap();
        let template = evidence.requests["test-shared"].clone();
        let refs = (1..=4)
            .map(|index| format!("test-057-c4-{index}"))
            .collect::<Vec<_>>();
        evidence.tests.get_mut("057").unwrap().request_refs = refs.clone();
        for (id, seconds) in refs.iter().zip([10, 20, 30, 60]) {
            let mut request = template.clone();
            request.request_id = id.clone();
            request.response_body = r#"{"choices":[{"message":{"content":"ok"}}]}"#.into();
            request
                .metrics
                .insert("time_total".into(), seconds.to_string());
            evidence.requests.insert(id.clone(), request);
        }
        let context = ContextConclusion::indeterminate("no context probes");
        let waves = summarize_concurrency(&evidence);
        assert_eq!(waves.len(), 3);
        assert_eq!(waves[0].average_response_time_seconds, Some(30.0));
        let mut results = vec![available_markdown_result(
            "057",
            CandidateStatus::Fail,
            Some("模型误判"),
        )];
        apply_hard_evidence_rules(&mut results, &evidence, &context, &waves);
        assert_eq!(results[0].validated_status, Some(ValidatedStatus::Pass));
        assert_eq!(
            results[0].decision_source,
            Some(DecisionSource::EvidenceRule)
        );

        for failed_index in 0..4 {
            for body in [
                "not JSON",
                r#"{"choices":[{"message":{"content":""}}]}"#,
                r#"{"error":{"message":"failed"}}"#,
            ] {
                evidence
                    .requests
                    .get_mut(&refs[failed_index])
                    .unwrap()
                    .response_body = body.into();
                let waves = summarize_concurrency(&evidence);
                assert_eq!(waves[0].succeeded, 3);
                assert_eq!(waves[0].failed, 1);
                let mut results = vec![available_markdown_result(
                    "057",
                    CandidateStatus::Pass,
                    None,
                )];
                apply_hard_evidence_rules(&mut results, &evidence, &context, &waves);
                assert_eq!(results[0].validated_status, Some(ValidatedStatus::Fail));
                assert_eq!(results[0].candidate_status, Some(CandidateStatus::Pass));
                let mut artifact = markdown_fixture(results);
                artifact.concurrency_waves = waves;
                assert_eq!(minimum_concurrency_requirement(&artifact).0, "不通过");
            }
            evidence
                .requests
                .get_mut(&refs[failed_index])
                .unwrap()
                .response_body = r#"{"choices":[{"message":{"content":"ok"}}]}"#.into();
        }

        evidence
            .requests
            .get_mut(&refs[0])
            .unwrap()
            .metadata
            .insert("transport_outcome".into(), "timeout".into());
        assert_eq!(summarize_concurrency(&evidence)[0].failed, 1);
        evidence
            .requests
            .get_mut(&refs[0])
            .unwrap()
            .metadata
            .insert("transport_outcome".into(), "completed_eof".into());
        evidence
            .requests
            .get_mut(&refs[0])
            .unwrap()
            .metrics
            .insert("http_status".into(), "500".into());
        assert_eq!(summarize_concurrency(&evidence)[0].failed, 1);

        evidence.requests.remove(&refs[0]);
        let mut results = vec![available_markdown_result(
            "057",
            CandidateStatus::Pass,
            None,
        )];
        apply_hard_evidence_rules(
            &mut results,
            &evidence,
            &context,
            &summarize_concurrency(&evidence),
        );
        assert_eq!(results[0].validated_status, None);

        let mut restored = template;
        restored.request_id = refs[0].clone();
        restored.response_body = r#"{"choices":[{"message":{"content":"ok"}}]}"#.into();
        restored.metrics.remove("time_total");
        evidence.requests.insert(refs[0].clone(), restored);
        let waves = summarize_concurrency(&evidence);
        assert_eq!(waves[0].succeeded, 4);
        assert_eq!(waves[0].average_response_time_seconds, None);
        assert_eq!(evaluate_minimum_concurrency(&waves).0, None);
    }

    #[tokio::test]
    async fn context_evidence_rule_preserves_124k_threshold() {
        let (_directory, settings, _) = fixture().await;
        let evidence = read(
            &settings.log_path,
            &Redactor::new("secret-key", &settings.url),
        )
        .unwrap();
        for tokens in [123_999, 124_000, 127_999, 128_000] {
            let context = crate::context_capacity::conclude(&[
                (
                    tokens,
                    ProbeOutcome::Accepted {
                        measured_tokens: Some(tokens),
                    },
                ),
                (
                    256_000,
                    ProbeOutcome::RejectedTooLong {
                        declared_limit: Some(tokens),
                    },
                ),
            ]);
            let mut results = vec![available_markdown_result(
                "018",
                CandidateStatus::Pass,
                None,
            )];
            apply_hard_evidence_rules(&mut results, &evidence, &context, &[]);
            assert_eq!(
                results[0].validated_status,
                Some(if tokens < 124_000 {
                    ValidatedStatus::Fail
                } else {
                    ValidatedStatus::Pass
                })
            );
            assert_eq!(
                results[0].decision_source,
                Some(DecisionSource::EvidenceRule)
            );
        }
    }

    #[tokio::test]
    async fn concurrency_gate_and_markdown_share_the_thirty_second_boundary() {
        let (_directory, settings, _) = fixture().await;
        let evidence = read(
            &settings.log_path,
            &Redactor::new("secret-key", &settings.url),
        )
        .unwrap();
        let context = ContextConclusion::indeterminate("no context probes");
        for (average, expected) in [
            (Some(29.999), Some(ValidatedStatus::Pass)),
            (Some(30.0), Some(ValidatedStatus::Pass)),
            (Some(30.001), Some(ValidatedStatus::Fail)),
            (Some(45.0), Some(ValidatedStatus::Fail)),
            (None, None),
            (Some(f64::NAN), None),
            (Some(f64::INFINITY), None),
        ] {
            let waves = vec![
                ConcurrencyWave {
                    concurrency: 4,
                    total_requests: 4,
                    succeeded: 4,
                    failed: 0,
                    average_response_time_seconds: average,
                    failures: Vec::new(),
                },
                ConcurrencyWave {
                    concurrency: 8,
                    total_requests: 8,
                    succeeded: 0,
                    failed: 8,
                    average_response_time_seconds: Some(60.0),
                    failures: Vec::new(),
                },
                ConcurrencyWave {
                    concurrency: 16,
                    total_requests: 16,
                    succeeded: 0,
                    failed: 16,
                    average_response_time_seconds: Some(90.0),
                    failures: Vec::new(),
                },
            ];
            let mut results = vec![available_markdown_result(
                "057",
                CandidateStatus::Pass,
                None,
            )];
            apply_hard_evidence_rules(&mut results, &evidence, &context, &waves);
            assert_eq!(results[0].validated_status, expected);
            let mut artifact = markdown_fixture(results);
            artifact.concurrency_waves = waves;
            assert_eq!(
                minimum_concurrency_requirement(&artifact).0,
                match expected {
                    Some(ValidatedStatus::Pass) => "通过",
                    Some(ValidatedStatus::Fail) => "不通过",
                    None => "无法判断",
                }
            );
        }
    }

    #[test]
    fn markdown_escapes_table_content() {
        let result =
            available_markdown_result("046", CandidateStatus::Fail, Some("first | line\nsecond"));
        let markdown = render_markdown(&markdown_fixture(vec![result]));

        assert!(markdown.contains("first \\| line second"));
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
        let gateway_compatibility = derive_gateway_compatibility("openai_chat", &tests);
        SelfAnalysisArtifact {
            schema_version: SELF_ANALYSIS_SCHEMA_VERSION,
            collector_version: "0.13.0".into(),
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
            requirement_levels: requirement_levels(),
            batches: Vec::new(),
            counts: AnalysisCounts {
                pass: 0,
                fail: 0,
                unavailable: 0,
            },
            gateway_compatibility,
            evidence_degradations: Vec::new(),
            concurrency_waves: Vec::new(),
            context_conclusion: crate::context_capacity::ContextConclusion::satisfied_with(
                128_000,
                true,
                Some(496_000),
                false,
            ),
        }
    }

    fn available_markdown_result(
        test_id: &str,
        status: CandidateStatus,
        failure_cause: Option<&str>,
    ) -> AnalysisTestResult {
        AnalysisTestResult {
            test_id: test_id.into(),
            report_test_id: test_id.into(),
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
