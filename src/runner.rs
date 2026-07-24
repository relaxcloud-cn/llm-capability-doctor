use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Local;
use thiserror::Error;
use tokio::sync::Barrier;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::audit::{AuditError, AuditWriter, RequestEvidence, RunMetadata, TestManifest};
use crate::catalog::{CatalogError, TestCase, select, select_onsite};
use crate::checks::{
    Body, CheckError, ManifestRefs, PlanContext, PlannedRequest, RequestGroup, plan,
};
use crate::cli::{CollectionProfile, Config};
use crate::http::{HttpExecutor, RequestInput};
use crate::protocol::tools::build_follow_up;
use crate::protocol::{AuthMode, PROBE_CANDIDATES, Protocol, basic_request, matches_response};
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
    let selected = match config.profile {
        CollectionProfile::Onsite => select_onsite(),
        CollectionProfile::Full => select(None)?,
        CollectionProfile::Custom => select(config.only.as_deref())?,
    };
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
        collection_profile: config.profile,
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
}

impl Runner {
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
        let context = PlanContext {
            protocol: self.detected_protocol,
            auth_mode: self.detected_auth_mode,
            model: &self.config.model,
            onsite: self.config.profile == CollectionProfile::Onsite,
        };
        let check_plan = plan(test.id, &context)?;
        let initial_tool_body = if matches!(test.id, "047" | "048" | "049") {
            check_plan
                .groups
                .first()
                .and_then(|group| group.requests().first())
                .map(|request| request.body.json().clone())
        } else {
            None
        };

        let mut evidence = if check_plan.manifest_refs == ManifestRefs::SharedRepeatSamples
            && !self.repeat_refs.is_empty()
        {
            Vec::new()
        } else {
            self.execute_groups(check_plan.groups).await?
        };

        if let (Some(initial_body), Some(initial_evidence)) = (initial_tool_body, evidence.first())
            && let Ok(response) = serde_json::from_slice(&initial_evidence.response_body)
            && let Ok(spec) = build_follow_up(
                self.detected_protocol,
                &self.config.model,
                test.id,
                &initial_body,
                &response,
            )
        {
            let follow = PlannedRequest {
                id: format!("test-{}-follow", test.id),
                protocol: self.detected_protocol,
                auth_mode: self.detected_auth_mode,
                body: Body::Json(spec.body),
                stream: spec.stream,
            };
            evidence.push(self.execute_one(follow).await?);
        }

        let executed_refs: Vec<String> = evidence
            .iter()
            .map(|request| request.request_id.clone())
            .collect();
        let request_refs = match check_plan.manifest_refs {
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
        let evidence = self
            .http
            .execute(input, self.cancellation.child_token())
            .await;
        self.audit.append_request(&evidence)?;
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
        for (_, request) in completed {
            self.audit.append_request(&request)?;
            evidence.push(request);
        }
        Ok(evidence)
    }

    fn request_input(&self, request: PlannedRequest) -> RequestInput {
        RequestInput {
            request_id: request.id,
            url: self.config.url.clone(),
            protocol: request.protocol,
            auth_mode: request.auth_mode,
            body: request.body.to_bytes(),
            stream: request.stream,
            api_key: self.config.api_key.expose().to_owned(),
        }
    }
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
    Catalog(#[from] CatalogError),
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
