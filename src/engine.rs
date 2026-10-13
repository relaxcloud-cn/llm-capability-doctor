use crate::{
    config::ModelConfig,
    judgment,
    model::{
        CheckId, CheckResult, EventEnvelope, FailureOrigin, Finding, RunEvent, RunResult, Status,
        TaskResult,
    },
    omp,
    registry::{CheckContext, Registration, Registry},
    transport::Transport,
};
use anyhow::Result;
use futures_util::{StreamExt, stream::FuturesUnordered};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct EventBus {
    started: Instant,
    sender: Option<UnboundedSender<EventEnvelope>>,
}
impl EventBus {
    pub fn emit(&self, event: RunEvent) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(EventEnvelope {
                elapsed_ms: self.started.elapsed().as_millis() as u64,
                event,
            });
        }
    }
}
pub fn atomic_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("结果路径缺少目录"))?;
    std::fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(&serde_json::to_vec_pretty(value)?)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
fn aggregate(kind: CheckId, tasks: Vec<TaskResult>, repetitions: u8) -> CheckResult {
    let failed = tasks.iter().any(|t| t.status == Status::Failed);
    let all = tasks.len() == repetitions as usize
        && (1..=repetitions).all(|run| {
            tasks
                .iter()
                .any(|t| t.repetition == run && t.status == Status::Passed)
        });
    let status = if failed {
        Status::Failed
    } else if all {
        Status::Passed
    } else {
        Status::Incomplete
    };
    let finished = tasks.len() == repetitions as usize && tasks.iter().all(|t| t.finished);
    let title = if all {
        match kind {
            CheckId::Continuous => "连续执行多个工具，任务结果正确",
            CheckId::Recovery => "工具出错后恢复，任务结果正确",
            _ => "新增和修改要求均已落实",
        }
        .into()
    } else {
        tasks
            .iter()
            .find(|t| t.status == Status::Failed)
            .or_else(|| tasks.iter().find(|t| t.status != Status::Passed))
            .map(|t| t.title.clone())
            .unwrap_or_else(|| {
                format!(
                    "等待下次执行，已完成 {} / {repetitions} 次",
                    tasks.iter().filter(|t| t.finished).count()
                )
            })
    };
    CheckResult {
        id: kind,
        finished,
        status,
        title,
        elapsed_ms: tasks.iter().map(|t| t.elapsed_ms).sum(),
        evidence_ids: tasks.iter().flat_map(|t| t.evidence_ids.clone()).collect(),
        facts: BTreeMap::new(),
        findings: tasks.iter().flat_map(|t| t.findings.clone()).collect(),
        tasks,
        assessment: None,
    }
}
fn blocked_check(id: CheckId, message: String) -> CheckResult {
    let mut check = CheckResult::pending(id);
    check.status = Status::Incomplete;
    check.title = "检测器运行受阻，尚未确认能力".into();
    check.findings.push(Finding {
        code: "detector_error".into(),
        title: check.title.clone(),
        fact: message,
        impact: "本项没有取得完整且可靠的检测结果。".into(),
        suggestion: "请根据检测器日志排查原因，修复后重新检测。".into(),
        origin: FailureOrigin::Detector,
        evidence_ids: vec![],
    });
    check
}
fn blocked_task(kind: CheckId, repetition: u8, message: String) -> TaskResult {
    TaskResult {
        id: format!("{kind:?}-{repetition}").to_lowercase(),
        finished: false,
        kind,
        repetition,
        status: Status::Incomplete,
        title: "智能体运行环境受阻，任务未完成".into(),
        elapsed_ms: 0,
        normal_end: false,
        fault_delivered: false,
        recovery_observed: false,
        tool_events: vec![],
        evidence_ids: vec![],
        findings: blocked_check(kind, message).findings,
    }
}
#[derive(Clone, Debug)]
pub struct EngineOutput {
    pub run: RunResult,
    pub directory: PathBuf,
}
pub const DIRECT_BUDGET: Duration = Duration::from_secs(6240);
pub fn interface_deadline(id: CheckId, stage_deadline: Instant) -> Instant {
    let seconds = crate::registry::builtin_specs()
        .into_iter()
        .find(|s| s.id == id)
        .map(|s| s.budget_seconds)
        .unwrap_or(180);
    stage_deadline.min(Instant::now() + Duration::from_secs(seconds))
}
pub async fn run(
    config: ModelConfig,
    root: &Path,
    cancel: CancellationToken,
    sender: Option<UnboundedSender<EventEnvelope>>,
) -> Result<EngineOutput> {
    run_with_registry(config, root, cancel, sender, Registry::builtin()).await
}
fn adjudicate(
    registration: &Registration,
    check: &mut CheckResult,
    run: &mut RunResult,
    directory: &Path,
    config: &ModelConfig,
) -> Result<()> {
    atomic_json(
        &directory
            .join("observations")
            .join(format!("{}.json", check.id.key())),
        check,
    )?;
    run.evidence = judgment::index_evidence(directory)?;
    let evaluation =
        match (registration.evaluate)(&registration.spec, check, directory, &run.evidence) {
            Ok(evaluation) => evaluation,
            Err(error) => judgment::Evaluation {
                claims: registration
                    .spec
                    .requirements
                    .iter()
                    .filter(|r| r.required)
                    .map(|r| judgment::Claim {
                        requirement: r.id.clone(),
                        validity: judgment::Validity::Insufficient,
                        outcome: judgment::Outcome::Unknown,
                        reason: format!("检测证据核对受阻：{}", config.redact(&error.to_string())),
                        expected: serde_json::Value::Null,
                        observed: serde_json::Value::Null,
                        evidence_ids: vec![format!("file:observations/{}.json", check.id.key())],
                        origin: Some(FailureOrigin::Detector),
                        condition: None,
                    })
                    .collect(),
                facts: Default::default(),
            },
        };
    check.facts = evaluation.facts;
    judgment::seal(
        &registration.spec,
        check,
        evaluation.claims,
        directory,
        &run.evidence,
    )?;
    // task-result.json stays the executor's raw record; adjudicated fields live
    // in the check assessment, so indexed evidence must not be rewritten here.
    Ok(())
}
pub async fn run_with_registry(
    config: ModelConfig,
    root: &Path,
    cancel: CancellationToken,
    sender: Option<UnboundedSender<EventEnvelope>>,
    registry: Registry,
) -> Result<EngineOutput> {
    let registry = Registry::new(registry.entries)?;
    std::fs::create_dir_all(root)?;
    let lease = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(".active.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lease)
        .map_err(|_| anyhow::anyhow!("本机已有检测正在执行，请等待当前检测结束"))?;
    let _lease = lease;
    let started = Instant::now();
    let overall_deadline = started + DIRECT_BUDGET + Duration::from_secs(780);
    let bus = EventBus { started, sender };
    let mut run = RunResult::with_plan(config.public_target(), registry.plan());
    let directory = std::path::absolute(root.join(&run.id))?;
    std::fs::create_dir_all(&directory)?;
    atomic_json(&directory.join("result.json"), &run)?;
    bus.emit(RunEvent::Started { run: run.clone() });
    let transport = Transport::new(config.clone(), &directory.join("http"), cancel.clone())?;
    let direct_deadline = started + DIRECT_BUDGET;
    for registration in registry.entries.iter().filter(|e| e.execute.is_some()) {
        let id = registration.spec.id;
        if cancel.is_cancelled() || Instant::now() >= direct_deadline {
            break;
        }
        bus.emit(RunEvent::CheckStarted { id });
        bus.emit(RunEvent::Step {
            id,
            message: registration.spec.scope.clone(),
        });
        let deadline = direct_deadline
            .min(Instant::now() + Duration::from_secs(registration.spec.budget_seconds));
        let progress = |message| bus.emit(RunEvent::Step { id, message });
        let execution = (registration.execute.unwrap())(CheckContext {
            spec: &registration.spec,
            directory: &directory,
            transport: &transport,
            deadline,
            progress: Some(&progress),
        });
        let outcome = tokio::select! {
            biased;
            result=execution=>result,
            _=cancel.cancelled()=>Err(anyhow::anyhow!("检测已取消，保留已有证据")),
            _=tokio::time::sleep_until(tokio::time::Instant::from_std(deadline))=>Err(anyhow::anyhow!("检测执行预算已耗尽")),
        };
        let mut check =
            outcome.unwrap_or_else(|error| blocked_check(id, config.redact(&error.to_string())));
        adjudicate(registration, &mut check, &mut run, &directory, &config)?;
        run.update(check.clone());
        atomic_json(&directory.join("result.json"), &run)?;
        bus.emit(RunEvent::CheckFinished { check });
    }
    let stage_deadline =
        (Instant::now() + Duration::from_secs(750)).min(overall_deadline - Duration::from_secs(30));
    let stage_cancel = cancel.child_token();
    let timeout_cancel = stage_cancel.clone();
    let timer = tokio::spawn(async move {
        tokio::time::sleep_until(tokio::time::Instant::from_std(stage_deadline)).await;
        timeout_cancel.cancel();
    });
    let mut tasks: BTreeMap<CheckId, Vec<TaskResult>> = BTreeMap::new();
    let agent_definitions: Vec<_> = registry
        .entries
        .iter()
        .filter(|e| e.execute.is_none())
        .collect();
    let repetitions = agent_definitions
        .iter()
        .map(|e| e.spec.repetitions)
        .max()
        .unwrap_or(0);
    let jobs: Vec<_> = (1..=repetitions)
        .flat_map(|repetition| {
            agent_definitions
                .iter()
                .filter(move |e| e.spec.repetitions >= repetition)
                .map(move |e| (*e, repetition))
        })
        .collect();
    for pair in jobs.chunks(2) {
        if stage_cancel.is_cancelled() || Instant::now() >= stage_deadline {
            break;
        }
        let mut futures = FuturesUnordered::new();
        for &(registration, repetition) in pair {
            let kind = registration.spec.id;
            if !tasks.contains_key(&kind) {
                bus.emit(RunEvent::CheckStarted { id: kind });
            }
            let task_id = format!("{kind:?}-{repetition}").to_lowercase();
            bus.emit(RunEvent::TaskStarted {
                id: task_id.clone(),
                kind,
                repetition,
            });
            let step_bus = bus.clone();
            let step_id = task_id.clone();
            let step: Arc<dyn Fn(String) + Send + Sync> = Arc::new(move |message| {
                step_bus.emit(RunEvent::TaskStep {
                    id: step_id.clone(),
                    kind,
                    repetition,
                    message,
                })
            });
            let cfg = config.clone();
            let task_dir = directory.join("agent").join(task_id);
            let token = stage_cancel.clone();
            futures.push(async move {
                let result =
                    omp::run_task(cfg, kind, repetition, &task_dir, token, Some(step)).await;
                (kind, repetition, result)
            });
        }
        while let Some((kind, repetition, result)) = futures.next().await {
            let mut task = result
                .unwrap_or_else(|e| blocked_task(kind, repetition, config.redact(&e.to_string())));
            if stage_cancel.is_cancelled()
                && !cancel.is_cancelled()
                && task.status == Status::Incomplete
            {
                task.title = "OMP 阶段预算已耗尽，任务未完成".into();
                task.finished = false;
                for finding in &mut task.findings {
                    finding.origin = FailureOrigin::Detector;
                    finding.title = task.title.clone();
                }
            }
            tasks.entry(kind).or_default().push(task);
            let registration = registry.entries.iter().find(|e| e.spec.id == kind).unwrap();
            let mut check = aggregate(kind, tasks[&kind].clone(), registration.spec.repetitions);
            adjudicate(registration, &mut check, &mut run, &directory, &config)?;
            if let Some(task) = check.tasks.iter().find(|t| t.repetition == repetition) {
                bus.emit(RunEvent::TaskFinished { task: task.clone() });
            }
            run.update(check.clone());
            atomic_json(&directory.join("result.json"), &run)?;
            if check.tasks.len() == registration.spec.repetitions as usize {
                bus.emit(RunEvent::CheckFinished { check });
            } else {
                bus.emit(RunEvent::CheckProgress { check });
            }
        }
    }
    timer.abort();
    for check in &mut run.checks {
        if check.id.is_agent() && !check.finished && !check.tasks.is_empty() {
            check.title = if check.status == Status::Failed {
                check.title.clone()
            } else {
                "任务未全部完成".into()
            };
            if check.status != Status::Failed {
                check.status = Status::Incomplete;
            }
        }
    }
    run.cancelled = cancel.is_cancelled();
    run.elapsed_ms = started.elapsed().as_millis() as u64;
    run.finished_at = Some(chrono::Utc::now());
    run.refresh_decision();
    atomic_json(&directory.join("result.json"), &run)?;
    bus.emit(RunEvent::ReportStarted);
    let mut report = tokio::process::Command::new(std::env::current_exe()?);
    report
        .arg("--render-result")
        .arg(directory.join("result.json"))
        .env_remove("MODEL_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = report.spawn()?;
    let rendered = tokio::time::timeout(Duration::from_secs(30), child.wait()).await;
    match rendered {
        Ok(Ok(status)) if status.success() => {
            run = serde_json::from_slice(&std::fs::read(directory.join("result.json"))?)?;
            if !directory.join("report.html").is_file() || !directory.join("records.zip").is_file()
            {
                run.report_path = None;
                bus.emit(RunEvent::Error {
                    message: "报告生成未产生完整文件".into(),
                });
            }
        }
        _ => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            run.report_path = None;
            bus.emit(RunEvent::Error {
                message: "报告生成失败或超过 30 秒，已有结果已保留".into(),
            });
        }
    }
    atomic_json(&directory.join("result.json"), &run)?;
    bus.emit(RunEvent::Finished { run: run.clone() });
    Ok(EngineOutput { run, directory })
}
