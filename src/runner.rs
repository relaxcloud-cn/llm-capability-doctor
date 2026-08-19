use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Local;
use thiserror::Error;
use tokio::sync::Barrier;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::audit::{AuditError, AuditWriter, RequestEvidence, RunMetadata, TestManifest};
use crate::catalog::TestCase;
use crate::checks::{
    Body, CheckError, ManifestRefs, PlanContext, PlannedRequest, RequestGroup, plan,
};
use crate::cli::Config;
use crate::evidence::{StreamTermination, ToolContractStatus, ToolLoopOutcome, TransportOutcome};
use crate::http::{HttpExecutor, RequestInput};
use crate::protocol::stream::{StreamParseResult, parse_stream};
use crate::protocol::tool_loop::{LoopDecision, ToolLoopState};
use crate::protocol::tools::{
    ToolConversation, ToolProtocolError, ToolRequestPhase, validate_tool_request,
};
use crate::protocol::{
    AuthMode, PROBE_CANDIDATES, Protocol, basic_request, matches_response, normalize_request_url,
};
use crate::redaction::{Redactor, mask_api_key};

pub struct RunOutcome {
    pub duration: Duration,
    pub request_count: usize,
    pub manifest_count: usize,
    pub log_path: PathBuf,
}

pub async fn run(
    config: Config,
    cancellation: CancellationToken,
) -> Result<RunOutcome, RunnerError> {
    let selected = crate::catalog::all();
    let started_at = Local::now();
    let started = Instant::now();
    let log_path = resolve_log_path(config.log_file.as_deref(), started_at)?;
    let redactor = Redactor::new(config.api_key.expose(), &config.url);
    let metadata = RunMetadata {
        run_id: format!(
            "MD-{}-{}",
            started_at.format("%Y%m%d-%H%M%S"),
            std::process::id()
        ),
        started_at,
        url: config.url.clone(),
        model: config.model.clone(),
        masked_api_key: mask_api_key(config.api_key.expose()),
        selected_test_count: selected.len(),
        insecure: config.insecure,
    };
    let audit = AuditWriter::create(&log_path, metadata, redactor)?;
    let http = HttpExecutor::new(config.timeout, config.insecure)?;
    let mut runner = Runner {
        config,
        selected,
        http,
        audit,
        cancellation,
        protocol_checked: false,
        detected_protocol: Protocol::Unknown,
        detected_auth_mode: AuthMode::Bearer,
        protocol_probe_refs: Vec::new(),
        selected_probe_ref: None,
        repeat_refs: Vec::new(),
        #[cfg(test)]
        turn_hook: None,
        #[cfg(test)]
        forced_follow_up_errors: None,
        #[cfg(test)]
        force_invalid_follow_up: false,
    };
    runner.execute().await?;
    let duration = started.elapsed();
    runner.audit.finish(duration, Local::now())?;
    Ok(RunOutcome {
        duration,
        request_count: runner.audit.request_count(),
        manifest_count: runner.audit.manifest_count(),
        log_path,
    })
}

struct Runner {
    config: Config,
    selected: Vec<&'static TestCase>,
    http: HttpExecutor,
    audit: AuditWriter,
    cancellation: CancellationToken,
    protocol_checked: bool,
    detected_protocol: Protocol,
    detected_auth_mode: AuthMode,
    protocol_probe_refs: Vec<String>,
    selected_probe_ref: Option<String>,
    repeat_refs: Vec<String>,
    #[cfg(test)]
    turn_hook: Option<TurnHook>,
    #[cfg(test)]
    forced_follow_up_errors: Option<Vec<String>>,
    #[cfg(test)]
    force_invalid_follow_up: bool,
}

#[cfg(test)]
#[derive(Clone)]
struct TurnHook {
    turn_committed: Arc<tokio::sync::Notify>,
    resume: Arc<tokio::sync::Notify>,
}

impl Runner {
    async fn execute_tool_loop(
        &mut self,
        test: &'static TestCase,
        initial: PlannedRequest,
    ) -> Result<Vec<RequestEvidence>, RunnerError> {
        let initial_body = initial.body.json().clone();
        let mut outgoing_errors = validate_tool_request(
            initial.protocol,
            &normalize_request_url(initial.protocol, &self.config.url, initial.stream),
            initial.stream,
            &initial_body,
            ToolRequestPhase::Initial,
        );
        let mut conversation = match ToolConversation::from_initial(initial.protocol, initial_body)
        {
            Ok(conversation) => Some(conversation),
            Err(error) => {
                merge_errors(&mut outgoing_errors, tool_protocol_errors(&error));
                None
            }
        };
        let mut state = ToolLoopState::new(test.id).expect("tool-loop checks are allowlisted");
        let loop_protocol = initial.protocol;
        let loop_auth_mode = initial.auth_mode;
        let mut current = initial;
        let mut collected = Vec::new();

        for turn_number in 1..=crate::protocol::tool_loop::MAX_ASSISTANT_TURNS {
            self.ensure_not_cancelled()?;
            current.id = format!("test-{}-turn-{turn_number}", test.id);
            let input = self.request_input(current);
            let mut evidence = self
                .http
                .execute(input, self.cancellation.child_token())
                .await;
            evidence.tool_loop_turn = turn_number;

            let parsed = parse_stream_evidence(&mut evidence).await;
            let mut contract_errors = std::mem::take(&mut outgoing_errors);
            let transport_failure = !evidence.transport_outcome.is_success()
                || evidence.stream_termination == StreamTermination::HttpError;
            if let Some(parsed) = parsed.as_ref() {
                merge_errors(&mut contract_errors, parsed.contract_errors.clone());
            } else {
                merge_errors(&mut contract_errors, vec!["response.unavailable:/".into()]);
            }
            if !transport_failure && evidence.stream_termination != StreamTermination::Completed {
                merge_errors(
                    &mut contract_errors,
                    vec![terminal_contract_error(&evidence.stream_termination)],
                );
            }

            let assistant_turn = parsed.and_then(|parsed| parsed.assistant_turn);
            if !transport_failure && assistant_turn.is_none() {
                merge_errors(&mut contract_errors, vec!["response.unavailable:/".into()]);
            }

            let mut next = None;
            if transport_failure {
                evidence.tool_contract_status = ToolContractStatus::NonConformant;
                evidence.tool_loop_outcome = ToolLoopOutcome::TransportFailure;
            } else if evidence.stream_termination != StreamTermination::Completed
                || !contract_errors.is_empty()
                || assistant_turn.is_none()
            {
                evidence.tool_contract_status = ToolContractStatus::NonConformant;
                evidence.tool_loop_outcome = ToolLoopOutcome::InvalidTurn;
            } else {
                evidence.tool_contract_status = ToolContractStatus::Conformant;
                let assistant_turn = assistant_turn.expect("checked above");
                match state.advance(&assistant_turn) {
                    LoopDecision::Complete => {
                        evidence.tool_loop_outcome = ToolLoopOutcome::Completed;
                    }
                    LoopDecision::Continue(results) => {
                        let built = match self.take_forced_follow_up_error() {
                            Some(error) => Err(error),
                            None => conversation
                                .as_mut()
                                .ok_or(ToolProtocolError::UnsupportedProtocol)
                                .and_then(|conversation| {
                                    conversation.append_follow_up(&assistant_turn, &results)
                                }),
                        };
                        match built {
                            Ok(mut spec) => {
                                self.maybe_invalidate_follow_up(&mut spec);
                                let endpoint = normalize_request_url(
                                    loop_protocol,
                                    &self.config.url,
                                    spec.stream,
                                );
                                let validation_errors = validate_tool_request(
                                    loop_protocol,
                                    &endpoint,
                                    spec.stream,
                                    &spec.body,
                                    ToolRequestPhase::FollowUp {
                                        previous: &assistant_turn,
                                        results: &results,
                                    },
                                );
                                if validation_errors.is_empty() {
                                    evidence.tool_loop_outcome = ToolLoopOutcome::Continued;
                                    next = Some(PlannedRequest {
                                        id: String::new(),
                                        protocol: loop_protocol,
                                        auth_mode: loop_auth_mode,
                                        body: Body::Json(spec.body),
                                        stream: spec.stream,
                                    });
                                } else {
                                    merge_errors(&mut contract_errors, validation_errors);
                                    evidence.tool_contract_status =
                                        ToolContractStatus::NonConformant;
                                    evidence.tool_loop_outcome = ToolLoopOutcome::InvalidTurn;
                                }
                            }
                            Err(error) => {
                                merge_errors(&mut contract_errors, tool_protocol_errors(&error));
                                evidence.tool_contract_status = ToolContractStatus::NonConformant;
                                evidence.tool_loop_outcome = ToolLoopOutcome::InvalidTurn;
                            }
                        }
                    }
                    LoopDecision::Stop {
                        outcome,
                        contract_errors: loop_errors,
                    } => {
                        merge_errors(&mut contract_errors, loop_errors);
                        if !contract_errors.is_empty() {
                            evidence.tool_contract_status = ToolContractStatus::NonConformant;
                        }
                        evidence.tool_loop_outcome = outcome;
                    }
                }
            }
            evidence.tool_contract_errors = contract_errors;
            self.audit.append_request(&evidence)?;
            let cancelled = evidence.transport_outcome == TransportOutcome::ClientCancelled;
            collected.push(evidence);
            if cancelled {
                return Err(RunnerError::Interrupted);
            }
            let Some(next_request) = next else {
                break;
            };
            self.wait_between_turns().await;
            self.ensure_not_cancelled()?;
            current = next_request;
        }

        Ok(collected)
    }

    async fn execute(&mut self) -> Result<(), RunnerError> {
        for test in self.selected.clone() {
            self.ensure_not_cancelled()?;
            println!("正在执行检测项 {}：{}", test.id, test.name);
            if test.id != "001" && !self.protocol_checked {
                self.protocol_checked = true;
                self.detect_protocol().await?;
            }
            self.execute_check(test).await?;
        }
        Ok(())
    }

    async fn detect_protocol(&mut self) -> Result<(), RunnerError> {
        for (index, candidate) in PROBE_CANDIDATES.iter().enumerate() {
            let spec = basic_request(
                candidate.protocol,
                &self.config.model,
                "Reply only MODEL_DOCTOR_PROTOCOL_OK",
                false,
            );
            let request = PlannedRequest {
                id: format!("protocol-{}", index + 1),
                protocol: candidate.protocol,
                auth_mode: candidate.auth,
                body: Body::Json(spec.body),
                stream: false,
            };
            let evidence = self.execute_one(request).await?;
            self.protocol_probe_refs.push(evidence.request_id.clone());
            self.ensure_not_cancelled()?;
            if evidence.metrics.transport_exit_code == 0
                && evidence
                    .metrics
                    .http_status
                    .is_some_and(|status| (200..300).contains(&status))
                && matches_response(candidate.protocol, &evidence.response_body)
            {
                self.detected_protocol = candidate.protocol;
                self.detected_auth_mode = candidate.auth;
                self.selected_probe_ref = Some(evidence.request_id);
                break;
            }
        }
        Ok(())
    }

    async fn execute_check(&mut self, test: &'static TestCase) -> Result<(), RunnerError> {
        let is_tool_loop = matches!(test.id, "046" | "047" | "048" | "049");
        if is_tool_loop && self.detected_protocol == Protocol::Unknown {
            self.audit.append_manifest(&TestManifest {
                id: test.id.to_owned(),
                name: test.name.to_owned(),
                category: test.category.to_owned(),
                completed_at: Local::now(),
                request_refs: self.protocol_probe_refs.clone(),
            })?;
            return Ok(());
        }

        let context = PlanContext {
            protocol: self.detected_protocol,
            auth_mode: self.detected_auth_mode,
            model: &self.config.model,
        };
        let check_plan = plan(test.id, &context)?;
        let manifest_refs = check_plan.manifest_refs;
        let evidence = if is_tool_loop {
            let initial = check_plan
                .groups
                .into_iter()
                .flat_map(|group| match group {
                    RequestGroup::Sequential(requests) | RequestGroup::Concurrent(requests) => {
                        requests
                    }
                })
                .next()
                .expect("tool-loop plan has one initial request");
            self.execute_tool_loop(test, initial).await?
        } else if manifest_refs == ManifestRefs::SharedRepeatSamples && !self.repeat_refs.is_empty()
        {
            Vec::new()
        } else {
            self.execute_groups(check_plan.groups).await?
        };

        let executed_refs: Vec<String> = evidence
            .iter()
            .map(|request| request.request_id.clone())
            .collect();
        let request_refs = match manifest_refs {
            ManifestRefs::Executed => executed_refs,
            ManifestRefs::AllProtocolProbes => self.protocol_probe_refs.clone(),
            ManifestRefs::SelectedProtocolProbe => self
                .selected_probe_ref
                .clone()
                .map_or_else(|| self.protocol_probe_refs.clone(), |id| vec![id]),
            ManifestRefs::SharedRepeatSamples => {
                if self.repeat_refs.is_empty() {
                    self.repeat_refs = executed_refs;
                }
                self.repeat_refs.clone()
            }
        };
        self.audit.append_manifest(&TestManifest {
            id: test.id.to_owned(),
            name: test.name.to_owned(),
            category: test.category.to_owned(),
            completed_at: Local::now(),
            request_refs,
        })?;
        Ok(())
    }

    async fn execute_groups(
        &mut self,
        groups: Vec<RequestGroup>,
    ) -> Result<Vec<RequestEvidence>, RunnerError> {
        let mut evidence = Vec::new();
        for group in groups {
            match group {
                RequestGroup::Sequential(requests) => {
                    for request in requests {
                        evidence.push(self.execute_one(request).await?);
                        self.ensure_not_cancelled()?;
                    }
                }
                RequestGroup::Concurrent(requests) => {
                    evidence.extend(self.execute_concurrent(requests).await?);
                    self.ensure_not_cancelled()?;
                }
            }
        }
        Ok(evidence)
    }

    fn ensure_not_cancelled(&self) -> Result<(), RunnerError> {
        if self.cancellation.is_cancelled() {
            Err(RunnerError::Interrupted)
        } else {
            Ok(())
        }
    }

    async fn execute_one(
        &mut self,
        request: PlannedRequest,
    ) -> Result<RequestEvidence, RunnerError> {
        let input = self.request_input(request);
        let mut evidence = self
            .http
            .execute(input, self.cancellation.child_token())
            .await;
        enrich_stream_evidence(&mut evidence).await;
        self.audit.append_request(&evidence)?;
        if evidence.transport_outcome == TransportOutcome::ClientCancelled {
            return Err(RunnerError::Interrupted);
        }
        Ok(evidence)
    }

    async fn execute_concurrent(
        &mut self,
        requests: Vec<PlannedRequest>,
    ) -> Result<Vec<RequestEvidence>, RunnerError> {
        let barrier = Arc::new(Barrier::new(requests.len() + 1));
        let mut tasks = JoinSet::new();
        for (index, request) in requests.into_iter().enumerate() {
            let input = self.request_input(request);
            let executor = self.http.clone();
            let cancellation = self.cancellation.child_token();
            let barrier = barrier.clone();
            tasks.spawn(async move {
                barrier.wait().await;
                (index, executor.execute(input, cancellation).await)
            });
        }
        barrier.wait().await;

        let mut completed = Vec::new();
        while let Some(result) = tasks.join_next().await {
            completed.push(result.map_err(RunnerError::Join)?);
        }
        completed.sort_by_key(|(index, _)| *index);
        let mut evidence = Vec::with_capacity(completed.len());
        for (_, mut request) in completed {
            enrich_stream_evidence(&mut request).await;
            self.audit.append_request(&request)?;
            evidence.push(request);
        }
        if evidence
            .iter()
            .any(|request| request.transport_outcome == TransportOutcome::ClientCancelled)
        {
            return Err(RunnerError::Interrupted);
        }
        Ok(evidence)
    }

    fn request_input(&self, request: PlannedRequest) -> RequestInput {
        RequestInput {
            request_id: request.id,
            url: normalize_request_url(request.protocol, &self.config.url, request.stream),
            protocol: request.protocol,
            auth_mode: request.auth_mode,
            body: request.body.to_bytes(),
            stream: request.stream,
            api_key: self.config.api_key.expose().to_owned(),
        }
    }

    #[cfg(test)]
    async fn wait_between_turns(&self) {
        if let Some(hook) = &self.turn_hook {
            hook.turn_committed.notify_one();
            hook.resume.notified().await;
        }
    }

    #[cfg(not(test))]
    async fn wait_between_turns(&self) {}

    #[cfg(test)]
    fn take_forced_follow_up_error(&mut self) -> Option<ToolProtocolError> {
        self.forced_follow_up_errors
            .take()
            .map(ToolProtocolError::InvalidRequest)
    }

    #[cfg(not(test))]
    fn take_forced_follow_up_error(&mut self) -> Option<ToolProtocolError> {
        None
    }

    #[cfg(test)]
    fn maybe_invalidate_follow_up(&mut self, spec: &mut crate::protocol::RequestSpec) {
        if self.force_invalid_follow_up {
            self.force_invalid_follow_up = false;
            spec.stream = false;
        }
    }

    #[cfg(not(test))]
    fn maybe_invalidate_follow_up(&mut self, _spec: &mut crate::protocol::RequestSpec) {}
}

async fn parse_stream_evidence(evidence: &mut RequestEvidence) -> Option<StreamParseResult> {
    let is_success = evidence
        .metrics
        .http_status
        .is_some_and(|status| (200..300).contains(&status));
    if !evidence.stream || !evidence.transport_outcome.is_success() || !is_success {
        return None;
    }

    let parsed = parse_stream(evidence.protocol, &evidence.response_body).await;
    evidence.stream_termination = parsed.stream_termination.clone();
    evidence.stream_end_signal = parsed.stream_end_signal.clone();
    evidence.model_stop_reason = parsed.model_stop_reason.clone();
    evidence.stream_event_count = parsed.event_count;
    Some(parsed)
}

fn terminal_contract_error(termination: &StreamTermination) -> String {
    format!("response.{termination}:/")
}

fn tool_protocol_errors(error: &ToolProtocolError) -> Vec<String> {
    let errors = error.codes().to_vec();
    if errors.is_empty() {
        vec!["request.build_failed:/".into()]
    } else {
        errors
    }
}

fn merge_errors(target: &mut Vec<String>, incoming: Vec<String>) {
    for error in incoming {
        if !target.contains(&error) {
            target.push(error);
        }
    }
}

async fn enrich_stream_evidence(evidence: &mut RequestEvidence) {
    let _ = parse_stream_evidence(evidence).await;
}

fn resolve_log_path(
    configured: Option<&Path>,
    started_at: chrono::DateTime<Local>,
) -> Result<PathBuf, std::io::Error> {
    let path = configured.map_or_else(
        || {
            PathBuf::from(format!(
                "model-doctor-{}.log",
                started_at.format("%Y%m%d-%H%M%S")
            ))
        },
        Path::to_path_buf,
    );
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("collection interrupted")]
    Interrupted,
    #[error(transparent)]
    Check(#[from] CheckError),
    #[error(transparent)]
    Audit(#[from] AuditError),
    #[error("unable to initialize HTTP client: {0}")]
    Http(#[from] reqwest::Error),
    #[error("concurrent request task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests;
