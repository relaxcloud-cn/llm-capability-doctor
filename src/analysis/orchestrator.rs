use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::Local;
use serde::Serialize;
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::client::{AnalysisClient, AnalysisTarget, ClientError, ModelAnalysisClient};
use super::evidence_reader::{EvidenceError, EvidenceSource, read};
use super::packet::{EvidencePacket, PacketError, default_batches};
use super::prompt::{PROMPT_VERSION, build_prompt, build_repair_prompt};
use super::validator::{
    CandidateStatus, DecisionSource, ValidatedReview, ValidatedStatus, validate_candidates,
};
use crate::private_file::create_new_private_file;
use crate::protocol::{AuthMode, Protocol};
use crate::redaction::Redactor;

pub const SELF_ANALYSIS_SCHEMA_VERSION: &str = "llm-capability-doctor.self-analysis.v1";
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
    let batches = default_batches(&evidence)?;
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
            endpoint: redactor.redact_url(&settings.url),
            requested_model: redactor.redact_text(&settings.model),
            detected_protocol: settings.protocol.to_string(),
            authentication_mode: settings.auth_mode.to_string(),
        },
        prompt_version: PROMPT_VERSION,
        provenance: SELF_ANALYSIS_PROVENANCE,
        tests,
        batches: batch_results,
        counts,
    };
    let pass_count = artifact.counts.pass;
    let fail_count = artifact.counts.fail;
    let unavailable_count = artifact.counts.unavailable;
    let path = write_artifact(&settings.log_path, &artifact)?;
    Ok(AnalysisOutcome {
        path,
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
            Err(error) if attempts == 1 && matches!(error, ClientError::InvalidJson(_)) => {
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

pub(crate) fn collision_safe_output_path(log_path: &Path) -> PathBuf {
    let stem = log_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("model-doctor");
    let initial = log_path.with_file_name(format!("{stem}-self-analysis.json"));
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
        let candidate = log_path.with_file_name(format!("{stem}-self-analysis-{suffix}.json"));
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
                write_complete(&mut file, &bytes)?;
                break candidate;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    };

    loop {
        let destination = collision_safe_output_path(log_path);
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
        let (directory, settings, batches) = fixture();
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
        assert!(output.contains("llm-capability-doctor.self-analysis.v1"));
        assert!(!output.contains("secret-key"));
        drop(directory);
    }

    #[tokio::test]
    async fn failed_batch_is_explicit_and_later_batches_continue() {
        let (_directory, settings, batches) = fixture();
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
        let (_directory, settings, _batches) = fixture();
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
    }

    #[test]
    fn output_collision_never_overwrites_existing_analysis() {
        let directory = tempfile::tempdir().unwrap();
        let log = directory.path().join("doctor.log");
        let first = collision_safe_output_path(&log);
        std::fs::write(&first, "existing").unwrap();

        let second = collision_safe_output_path(&log);

        assert_ne!(first, second);
        assert_eq!(std::fs::read_to_string(first).unwrap(), "existing");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn output_file_is_private() {
        use std::os::unix::fs::PermissionsExt;

        let (_directory, settings, batches) = fixture();
        let responses = batches
            .iter()
            .map(|batch| Ok(valid_envelope_for(batch)))
            .collect();
        let client = FakeClient::new(responses);

        let outcome = analyze_with_client(settings, &client, CancellationToken::new())
            .await
            .unwrap();

        let mode = std::fs::metadata(outcome.path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[tokio::test]
    async fn source_path_is_redacted_in_output_metadata() {
        let (_directory, settings, batches) = fixture_named("secret-key-doctor.log");
        let responses = batches
            .iter()
            .map(|batch| Ok(valid_envelope_for(batch)))
            .collect();
        let client = FakeClient::new(responses);

        let outcome = analyze_with_client(settings, &client, CancellationToken::new())
            .await
            .unwrap();

        let output = std::fs::read_to_string(outcome.path).unwrap();
        assert!(!output.contains("secret-key"));
        assert!(output.contains("[REDACTED]-doctor.log"));
    }

    fn fixture() -> (
        tempfile::TempDir,
        AnalysisSettings,
        Vec<Vec<EvidencePacket>>,
    ) {
        fixture_named("doctor.log")
    }

    fn fixture_named(
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
        let batches = default_batches(&parsed).unwrap();
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
