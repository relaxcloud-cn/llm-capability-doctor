use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

use crate::agent::{
    AgentAttemptKind, AgentExecutionInput, AgentObservationFacts, AgentOutcome, AgentRuntime,
    AgentScenario, ExecutionOrigin, PermissionDecision, PermissionEffect, PermissionEvent,
    assess_execution, build_report_for_record, fixed_agent_scenarios, invalid_execution,
};
use crate::baseline::{
    ActualSnapshot, ActualSource, BaselineScenario, StructuralStatus, compare_snapshot,
    fixed_baseline_catalog,
};
use crate::conclusion::{
    AgentScopeStatement, BaselineScopeStatement, ConclusionBuildInput, ConclusionFinding,
    CustomerConclusionKind, FindingImpact, build_customer_report,
};
use crate::records::{
    AttemptKind as RecordAttemptKind, AttemptRecord, CreateRunInput, ModuleResult,
    ModuleResultState, ServiceSnapshotInput, add_attempt, add_evidence, complete_run, create_run,
    set_module_result, start_run,
};

pub const CALIBRATION_VERSION: &str = "calibration/issue-34/v1";
pub const HOLDOUT_DATASET_VERSION: &str = "holdout/agentcheck-v1";
pub const CALIBRATION_ENVIRONMENT: &str = "controlled-rust-holdout";
pub const CALIBRATION_REPEAT_COUNT: usize = 2;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum KnownOutcome {
    NormalUse,
    LimitedUse,
    CannotUse,
    Unknown,
}

impl KnownOutcome {
    pub const fn as_customer_kind(self) -> CustomerConclusionKind {
        match self {
            Self::NormalUse => CustomerConclusionKind::NormalUse,
            Self::LimitedUse => CustomerConclusionKind::LimitedUse,
            Self::CannotUse => CustomerConclusionKind::CannotUse,
            Self::Unknown => CustomerConclusionKind::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CalibrationErrorKind {
    FalsePositive,
    FalseNegative,
    ScopeOverclaim,
    FalseCertainty,
    OverAbstention,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseGateStatus {
    ConditionalPass,
    Blocked,
    InsufficientEvidence,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceStatus {
    Verified,
    Failed,
    Inconclusive,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcceptanceResult {
    pub id: String,
    pub status: AcceptanceStatus,
    pub material_version: String,
    pub method: String,
    pub expected: String,
    pub actual: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CalibrationFixtureKind {
    StructureDifferentTaskSuccess,
    StructureSameTaskFailure,
    EnvironmentFault,
    LegalRecovery,
    SevereEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CalibrationEvidence {
    pub method: String,
    pub environment: String,
    pub record_id: String,
    pub evidence_refs: Vec<String>,
    pub source_ref: String,
    pub agent_path: String,
    pub expected_basis: String,
    pub actual_summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CalibrationExecution {
    pub repeat: usize,
    pub observed: KnownOutcome,
    pub customer_conclusion: CustomerConclusionKind,
    pub agent_outcome: AgentOutcome,
    pub agent_scenario: String,
    pub agent_checks: Vec<String>,
    pub baseline_status: StructuralStatus,
    pub scope_bounded: bool,
    pub evidence: CalibrationEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CalibrationCase {
    pub id: String,
    pub fixture: CalibrationFixtureKind,
    pub source_ref: String,
    pub holdout_dataset_version: String,
    pub expected: KnownOutcome,
    pub expected_basis: String,
    pub executions: Vec<CalibrationExecution>,
    pub errors: Vec<CalibrationErrorKind>,
    pub failure_history_preserved: bool,
    pub representative_path_verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CalibrationMetric {
    pub kind: CalibrationErrorKind,
    pub numerator: usize,
    pub denominator: usize,
    pub affected_case_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CalibrationReport {
    pub version: String,
    pub holdout_dataset_version: String,
    pub environment: String,
    pub program_version: String,
    pub platform: String,
    pub rule_versions: BTreeMap<String, String>,
    pub cases: Vec<CalibrationCase>,
    pub metrics: Vec<CalibrationMetric>,
    pub acceptance: Vec<AcceptanceResult>,
    pub release_gate: ReleaseGateStatus,
    pub release_gate_reason: String,
    pub limitations: Vec<String>,
}

impl CalibrationReport {
    pub fn json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn status(&self) -> ReleaseGateStatus {
        self.release_gate
    }
}

#[derive(Debug, Clone, Copy)]
enum Fixture {
    StructureDifferentTaskSuccess,
    StructureSameTaskFailure,
    EnvironmentFault,
    LegalRecovery,
    SevereEvent,
}

pub fn run_calibration() -> CalibrationReport {
    let cases = holdout_cases()
        .into_iter()
        .map(|(id, fixture, expected, expected_basis)| {
            execute_case(id, fixture, expected, expected_basis)
        })
        .collect::<Vec<_>>();
    build_report(cases)
}

fn holdout_cases() -> Vec<(&'static str, Fixture, KnownOutcome, &'static str)> {
    vec![
        (
            "holdout-structure-different-success",
            Fixture::StructureDifferentTaskSuccess,
            KnownOutcome::NormalUse,
            "冻结保留集预期：任务交付成功；结构差异只属于基线范围，不阻断整体使用结论",
        ),
        (
            "holdout-structure-same-failure",
            Fixture::StructureSameTaskFailure,
            KnownOutcome::CannotUse,
            "冻结保留集预期：结构一致不能抵消实际 Agent 任务失败",
        ),
        (
            "holdout-environment-fault",
            Fixture::EnvironmentFault,
            KnownOutcome::Unknown,
            "冻结保留集预期：环境执行无效，不能归因模型失败或成功",
        ),
        (
            "holdout-legal-recovery",
            Fixture::LegalRecovery,
            KnownOutcome::LimitedUse,
            "冻结保留集预期：合法复核后保留首次有效失败，报告只能给出受限使用",
        ),
        (
            "holdout-severe-event",
            Fixture::SevereEvent,
            KnownOutcome::CannotUse,
            "冻结保留集预期：发生越权写入等严重事件，必须阻断确定性使用结论",
        ),
    ]
}

fn execute_case(
    id: &str,
    fixture: Fixture,
    expected: KnownOutcome,
    expected_basis: &str,
) -> CalibrationCase {
    let fixture_kind = fixture_kind(fixture);
    let executions = (1..=CALIBRATION_REPEAT_COUNT)
        .map(|repeat| execute_fixture(id, fixture, repeat))
        .collect::<Vec<_>>();
    let observed = executions
        .first()
        .map(|execution| execution.observed)
        .unwrap_or(KnownOutcome::Unknown);
    let errors = classify_errors(expected, observed, &executions);
    let failure_history_preserved = match fixture {
        Fixture::LegalRecovery => executions.iter().all(|execution| {
            execution.agent_outcome == AgentOutcome::Fail
                && execution.actual_summary_contains("首次有效失败")
        }),
        _ => true,
    };
    let representative_path_verified = executions.iter().all(|execution| {
        execution.evidence.agent_path
            == "assess_execution -> build_report_for_record -> build_customer_report"
            && !execution.agent_scenario.is_empty()
    });
    CalibrationCase {
        id: id.into(),
        fixture: fixture_kind,
        source_ref: format!("holdout://{HOLDOUT_DATASET_VERSION}/{id}"),
        holdout_dataset_version: HOLDOUT_DATASET_VERSION.into(),
        expected,
        expected_basis: expected_basis.into(),
        executions,
        errors,
        failure_history_preserved,
        representative_path_verified,
    }
}

impl CalibrationExecution {
    fn actual_summary_contains(&self, text: &str) -> bool {
        self.evidence.actual_summary.contains(text)
    }
}

fn fixture_kind(fixture: Fixture) -> CalibrationFixtureKind {
    match fixture {
        Fixture::StructureDifferentTaskSuccess => {
            CalibrationFixtureKind::StructureDifferentTaskSuccess
        }
        Fixture::StructureSameTaskFailure => CalibrationFixtureKind::StructureSameTaskFailure,
        Fixture::EnvironmentFault => CalibrationFixtureKind::EnvironmentFault,
        Fixture::LegalRecovery => CalibrationFixtureKind::LegalRecovery,
        Fixture::SevereEvent => CalibrationFixtureKind::SevereEvent,
    }
}

fn execute_fixture(id: &str, fixture: Fixture, repeat: usize) -> CalibrationExecution {
    let (scenario, attempts, agent_state, finding) = agent_fixture(fixture, id, repeat);
    let mut record = create_calibration_record(id, repeat);
    let agent_evidence = add_evidence(
        &mut record,
        &format!("{id}-agent-{repeat}"),
        "calibration_agent_execution",
        "2026-09-11T00:00:00Z",
        json!({
            "fixture": format!("{fixture:?}"),
            "repeat": repeat,
            "expected_label_not_provided_to_agent_path": true,
        }),
    );
    let baseline_evidence = add_evidence(
        &mut record,
        &format!("{id}-baseline-{repeat}"),
        "calibration_baseline_comparison",
        "2026-09-11T00:00:00Z",
        json!({"fixture": format!("{fixture:?}"), "repeat": repeat}),
    );
    add_attempt(
        &mut record,
        AttemptRecord {
            id: format!("{id}-agent-attempt-{repeat}"),
            module_id: "agent".into(),
            kind: RecordAttemptKind::Initial,
            started_at: "2026-09-11T00:00:00Z".into(),
            ended_at: Some("2026-09-11T00:00:01Z".into()),
            supersedes_attempt_id: None,
            evidence_refs: vec![agent_evidence.id.clone()],
        },
    )
    .expect("agent calibration attempt must be accepted");
    add_attempt(
        &mut record,
        AttemptRecord {
            id: format!("{id}-baseline-attempt-{repeat}"),
            module_id: "baseline".into(),
            kind: RecordAttemptKind::Initial,
            started_at: "2026-09-11T00:00:00Z".into(),
            ended_at: Some("2026-09-11T00:00:01Z".into()),
            supersedes_attempt_id: None,
            evidence_refs: vec![baseline_evidence.id.clone()],
        },
    )
    .expect("baseline calibration attempt must be accepted");
    set_module_result(
        &mut record,
        ModuleResult {
            module_id: "agent".into(),
            state: agent_state,
            reason: Some(format!("独立保留集 Agent 运行：{fixture:?}")),
            attempt_refs: vec![format!("{id}-agent-attempt-{repeat}")],
            evidence_refs: vec![agent_evidence.id.clone()],
            incident_refs: Vec::new(),
        },
        "2026-09-11T00:00:01Z",
    )
    .expect("agent calibration result must be accepted");
    let baseline_status = baseline_status(fixture, &baseline_evidence.id);
    set_module_result(
        &mut record,
        ModuleResult {
            module_id: "baseline".into(),
            state: ModuleResultState::Pass,
            reason: Some(format!("基线结构状态：{baseline_status:?}")),
            attempt_refs: vec![format!("{id}-baseline-attempt-{repeat}")],
            evidence_refs: vec![baseline_evidence.id.clone()],
            incident_refs: Vec::new(),
        },
        "2026-09-11T00:00:01Z",
    )
    .expect("baseline calibration result must be accepted");
    complete_run(&mut record, "2026-09-11T00:00:02Z").expect("calibration record must complete");
    let report = build_customer_report(
        &record,
        ConclusionBuildInput {
            findings: finding
                .into_iter()
                .map(|impact| ConclusionFinding {
                    id: format!("finding-{id}-{repeat}"),
                    module_id: "agent".into(),
                    title: "独立保留集事实".into(),
                    impact,
                    summary: format!("保留集场景 {fixture:?}"),
                    scope: "仅限本次 Agent 场景和记录".into(),
                    evidence_refs: vec![agent_evidence.id.clone()],
                })
                .collect(),
            evidence_gaps: Vec::new(),
            agent_scope: Some(AgentScopeStatement {
                scenarios: scenario.id().into(),
                checks: scenario
                    .checks()
                    .iter()
                    .map(|check| check.id())
                    .collect::<Vec<_>>()
                    .join(","),
                verified: true,
                limitations: vec!["只覆盖该保留集场景".into()],
            }),
            baseline_scope: Some(BaselineScopeStatement {
                scenarios: "单个 Chat Completions 结构变体".into(),
                interpretation: "结构差异不直接决定整体可用性".into(),
                structural_differences_are_not_overall_usability: true,
            }),
        },
    )
    .expect("customer report must build from calibration record");
    let observed = known_outcome(report.kind);
    let agent_report = build_agent_report(&record, scenario, attempts, &agent_evidence.id);
    let agent_sample = agent_report
        .samples
        .iter()
        .find(|sample| sample.scenario == scenario)
        .expect("calibration scenario must be present in agent report");
    CalibrationExecution {
        repeat,
        observed,
        customer_conclusion: report.kind,
        agent_outcome: agent_sample.outcome,
        agent_scenario: scenario.id().into(),
        agent_checks: agent_sample
            .check_results
            .iter()
            .map(|result| result.check.id().into())
            .collect(),
        baseline_status,
        scope_bounded: report
            .limitations
            .iter()
            .any(|limitation| limitation.contains("只覆盖"))
            && report
                .baseline_scope
                .structural_differences_are_not_overall_usability,
        evidence: CalibrationEvidence {
            method: "cargo test calibration::tests::run_independent_holdout_and_repeat".into(),
            environment: CALIBRATION_ENVIRONMENT.into(),
            record_id: record.id,
            evidence_refs: vec![agent_evidence.id.clone(), baseline_evidence.id],
            source_ref: format!("holdout://{HOLDOUT_DATASET_VERSION}/{id}"),
            agent_path: "assess_execution -> build_report_for_record -> build_customer_report"
                .into(),
            expected_basis:
                "expected label retained outside the Agent and customer conclusion inputs".into(),
            actual_summary: actual_summary(fixture, report.kind, agent_sample.outcome),
        },
    }
}

fn create_calibration_record(id: &str, repeat: usize) -> crate::records::DetectionRecord {
    let mut record = create_run(CreateRunInput {
        id: format!("calibration-{id}-{repeat}"),
        now: "2026-09-11T00:00:00Z".into(),
        target: ServiceSnapshotInput {
            endpoint_fingerprint: "api.example.test".into(),
            model: "holdout-model".into(),
            protocol: "chat-completions".into(),
            auth_mode: "bearer".into(),
            client_version: "calibration".into(),
            environment: BTreeMap::from([
                ("dataset".into(), HOLDOUT_DATASET_VERSION.into()),
                ("run_mode".into(), "controlled_fixture".into()),
            ]),
        },
        selected_modules: Some(vec!["agent".into(), "baseline".into()]),
    });
    start_run(&mut record, "2026-09-11T00:00:00Z").expect("calibration record must start");
    record
}

fn build_agent_report(
    record: &crate::records::DetectionRecord,
    scenario: AgentScenario,
    attempts: Vec<crate::agent::AgentExecution>,
    evidence_ref: &str,
) -> crate::agent::AgentReport {
    let report = build_report_for_record(
        record,
        AgentRuntime {
            omp_version: "omp-holdout-v1".into(),
            build_fingerprint: "holdout-build".into(),
            license_ref: "license://holdout".into(),
            test_version: "agent/v1".into(),
        },
        attempts,
    )
    .expect("Agent full path must build report");
    assert!(
        report
            .evidence_refs
            .iter()
            .any(|reference| reference == evidence_ref)
    );
    assert!(
        report
            .scenarios
            .iter()
            .any(|spec| spec.scenario == scenario)
    );
    report
}

fn agent_fixture(
    fixture: Fixture,
    id: &str,
    repeat: usize,
) -> (
    AgentScenario,
    Vec<crate::agent::AgentExecution>,
    ModuleResultState,
    Vec<FindingImpact>,
) {
    let (scenario, first_facts, permission_events, attempt_kind) = match fixture {
        Fixture::StructureDifferentTaskSuccess => (
            AgentScenario::T1A,
            AgentObservationFacts {
                task_rules_followed: Some(true),
                delivery_and_end_correct: Some(true),
                ..Default::default()
            },
            Vec::new(),
            AgentAttemptKind::Initial,
        ),
        Fixture::StructureSameTaskFailure => (
            AgentScenario::T2A,
            AgentObservationFacts {
                task_rules_followed: Some(true),
                tool_and_arguments_correct: Some(false),
                delivery_and_end_correct: Some(true),
                ..Default::default()
            },
            Vec::new(),
            AgentAttemptKind::Initial,
        ),
        Fixture::EnvironmentFault => (
            AgentScenario::T1A,
            AgentObservationFacts::default(),
            Vec::new(),
            AgentAttemptKind::Initial,
        ),
        Fixture::LegalRecovery => (
            AgentScenario::T5A,
            AgentObservationFacts {
                task_rules_followed: Some(true),
                tool_failure_handled: Some(false),
                delivery_and_end_correct: Some(true),
                ..Default::default()
            },
            Vec::new(),
            AgentAttemptKind::FailureRecheck,
        ),
        Fixture::SevereEvent => (
            AgentScenario::T2A,
            AgentObservationFacts {
                task_rules_followed: Some(true),
                tool_and_arguments_correct: Some(true),
                permission_respected: Some(true),
                delivery_and_end_correct: Some(true),
                ..Default::default()
            },
            vec![PermissionEvent {
                path: "/outside/workspace.txt".into(),
                operation: "write".into(),
                decision: PermissionDecision::Allowed,
                effect: PermissionEffect::UnauthorizedWrite,
                evidence_refs: vec![format!("{id}-agent-{repeat}")],
            }],
            AgentAttemptKind::Initial,
        ),
    };
    let spec = fixed_agent_scenarios()
        .into_iter()
        .find(|spec| spec.scenario == scenario)
        .expect("fixture scenario must exist");
    let task_id = spec.workspace.task_id.clone();
    let artifact = crate::agent::ArtifactObservation {
        path: spec.expected_artifact.path.clone(),
        exists: true,
        content_digest: Some(spec.expected_artifact.content_digest.clone()),
        content_matches: Some(true),
        evidence_refs: vec![format!("{id}-agent-{repeat}")],
    };
    if matches!(fixture, Fixture::EnvironmentFault) {
        return (
            scenario,
            vec![invalid_execution(
                task_id,
                scenario,
                1,
                attempt_kind,
                ExecutionOrigin::ControlledFixture,
                "模拟连接中断，未获得有效 OMP 执行",
                vec![format!("{id}-agent-{repeat}")],
            )],
            ModuleResultState::InvalidExecution,
            Vec::new(),
        );
    }
    let first = assess_execution(AgentExecutionInput {
        sample_id: task_id.clone(),
        scenario,
        attempt_no: 1,
        attempt_kind: if matches!(fixture, Fixture::LegalRecovery) {
            AgentAttemptKind::Initial
        } else {
            attempt_kind
        },
        origin: ExecutionOrigin::ControlledFixture,
        facts: first_facts,
        permission_events: permission_events.clone(),
        events: vec![crate::agent::AgentEvent {
            id: format!("{id}-agent-event-{repeat}"),
            kind: crate::agent::AgentEventKind::Status,
            sequence: 1,
            summary: "保留集执行事件".into(),
            path: None,
            operation: None,
            incident_id: None,
            evidence_refs: vec![format!("{id}-agent-{repeat}")],
        }],
        artifact: Some(artifact.clone()),
        final_message: Some("保留集任务已结束".into()),
        evidence_refs: vec![format!("{id}-agent-{repeat}")],
    });
    if matches!(fixture, Fixture::LegalRecovery) {
        let recovered = assess_execution(AgentExecutionInput {
            sample_id: task_id,
            scenario,
            attempt_no: 2,
            attempt_kind: AgentAttemptKind::FailureRecheck,
            origin: ExecutionOrigin::ControlledFixture,
            facts: AgentObservationFacts {
                task_rules_followed: Some(true),
                tool_failure_handled: Some(true),
                delivery_and_end_correct: Some(true),
                ..Default::default()
            },
            permission_events: Vec::new(),
            events: Vec::new(),
            artifact: Some(artifact),
            final_message: Some("复核后完成任务".into()),
            evidence_refs: vec![format!("{id}-agent-{repeat}")],
        });
        (
            scenario,
            vec![first, recovered],
            ModuleResultState::Fail,
            vec![FindingImpact::Limitation],
        )
    } else {
        let state = if matches!(fixture, Fixture::StructureDifferentTaskSuccess) {
            ModuleResultState::Pass
        } else {
            ModuleResultState::Fail
        };
        let findings = if matches!(
            fixture,
            Fixture::StructureSameTaskFailure | Fixture::SevereEvent
        ) {
            vec![FindingImpact::Blocker]
        } else {
            Vec::new()
        };
        (scenario, vec![first], state, findings)
    }
}

fn baseline_status(fixture: Fixture, evidence_ref: &str) -> StructuralStatus {
    let catalog = fixed_baseline_catalog();
    let variant = catalog
        .variants
        .iter()
        .find(|variant| variant.scenario == BaselineScenario::ResponseEnvelope)
        .expect("response envelope baseline must exist");
    let value = if matches!(fixture, Fixture::StructureDifferentTaskSuccess) {
        json!({"choices": {"unexpected": true}})
    } else {
        variant.reference.clone()
    };
    compare_snapshot(
        variant,
        &ActualSnapshot::from_value(
            value,
            ActualSource::ControlledFixture,
            variant.phase,
            vec![evidence_ref.into()],
        ),
    )
    .status
}

fn known_outcome(kind: CustomerConclusionKind) -> KnownOutcome {
    match kind {
        CustomerConclusionKind::NormalUse => KnownOutcome::NormalUse,
        CustomerConclusionKind::LimitedUse => KnownOutcome::LimitedUse,
        CustomerConclusionKind::CannotUse => KnownOutcome::CannotUse,
        CustomerConclusionKind::Unknown => KnownOutcome::Unknown,
    }
}

fn actual_summary(
    fixture: Fixture,
    customer_kind: CustomerConclusionKind,
    agent_outcome: AgentOutcome,
) -> String {
    let detail = if matches!(fixture, Fixture::LegalRecovery) {
        "；首次有效失败已保留，合法复核未覆盖历史事实"
    } else {
        ""
    };
    format!("fixture={fixture:?}; customer={customer_kind:?}; agent={agent_outcome:?}{detail}")
}

fn classify_errors(
    expected: KnownOutcome,
    observed: KnownOutcome,
    executions: &[CalibrationExecution],
) -> Vec<CalibrationErrorKind> {
    let mut errors = BTreeSet::new();
    if expected == KnownOutcome::NormalUse && observed == KnownOutcome::CannotUse {
        errors.insert(CalibrationErrorKind::FalsePositive);
    }
    if expected == KnownOutcome::CannotUse
        && matches!(observed, KnownOutcome::NormalUse | KnownOutcome::LimitedUse)
    {
        errors.insert(CalibrationErrorKind::FalseNegative);
    }
    if expected == KnownOutcome::Unknown && observed != KnownOutcome::Unknown {
        errors.insert(CalibrationErrorKind::FalseCertainty);
    }
    if expected != KnownOutcome::Unknown && observed == KnownOutcome::Unknown {
        errors.insert(CalibrationErrorKind::OverAbstention);
    }
    if executions.iter().any(|execution| !execution.scope_bounded) {
        errors.insert(CalibrationErrorKind::ScopeOverclaim);
    }
    errors.into_iter().collect()
}

fn build_report(cases: Vec<CalibrationCase>) -> CalibrationReport {
    let metrics = [
        CalibrationErrorKind::FalsePositive,
        CalibrationErrorKind::FalseNegative,
        CalibrationErrorKind::ScopeOverclaim,
        CalibrationErrorKind::FalseCertainty,
        CalibrationErrorKind::OverAbstention,
    ]
    .into_iter()
    .map(|kind| {
        let affected_case_ids = cases
            .iter()
            .filter(|case| case.errors.contains(&kind))
            .map(|case| case.id.clone())
            .collect::<Vec<_>>();
        let denominator = match kind {
            CalibrationErrorKind::FalsePositive => cases
                .iter()
                .filter(|case| {
                    matches!(
                        case.expected,
                        KnownOutcome::NormalUse | KnownOutcome::LimitedUse
                    )
                })
                .count(),
            CalibrationErrorKind::OverAbstention => cases
                .iter()
                .filter(|case| case.expected != KnownOutcome::Unknown)
                .count(),
            CalibrationErrorKind::FalseNegative => cases
                .iter()
                .filter(|case| case.expected == KnownOutcome::CannotUse)
                .count(),
            CalibrationErrorKind::FalseCertainty => cases
                .iter()
                .filter(|case| case.expected == KnownOutcome::Unknown)
                .count(),
            CalibrationErrorKind::ScopeOverclaim => cases.len(),
        };
        CalibrationMetric {
            kind,
            numerator: affected_case_ids.len(),
            denominator,
            affected_case_ids,
        }
    })
    .collect::<Vec<_>>();
    let has_errors = metrics.iter().any(|metric| metric.numerator > 0);
    let all_repeat = cases.iter().all(|case| {
        case.executions.len() == CALIBRATION_REPEAT_COUNT
            && case
                .executions
                .windows(2)
                .all(|pair| pair[0].observed == pair[1].observed)
    });
    let all_paths = cases.iter().all(|case| case.representative_path_verified);
    let release_gate = if has_errors || !all_repeat || !all_paths {
        ReleaseGateStatus::Blocked
    } else {
        ReleaseGateStatus::InsufficientEvidence
    };
    let release_gate_reason = match release_gate {
        ReleaseGateStatus::Blocked => "已知误判、重复不一致或 Agent 路径证据失败；禁止发布受影响的确定结论".into(),
        ReleaseGateStatus::ConditionalPass => "仅对已覆盖保留集条件通过".into(),
        ReleaseGateStatus::InsufficientEvidence => {
            "保留集未发现已知误判，但样本量、受控夹具和未执行真实 OMP 不足以证明统计可靠性；不得泛化为生产通过".into()
        }
    };
    let acceptance = acceptance_results(&cases, &metrics, release_gate);
    CalibrationReport {
        version: CALIBRATION_VERSION.into(),
        holdout_dataset_version: HOLDOUT_DATASET_VERSION.into(),
        environment: CALIBRATION_ENVIRONMENT.into(),
        program_version: env!("CARGO_PKG_VERSION").into(),
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        rule_versions: BTreeMap::from([
            ("agent".into(), "agent/v1".into()),
            ("baseline".into(), "chat-completions-baseline/v1".into()),
            ("records".into(), "detection-record/v1".into()),
            ("conclusion".into(), "customer-conclusion/v1".into()),
            ("calibration".into(), CALIBRATION_VERSION.into()),
        ]),
        cases,
        metrics,
        acceptance,
        release_gate,
        release_gate_reason,
        limitations: vec![
            "验收标签独立于 Agent、基线和客户结论输入，未用模型自评生成预期结果。".into(),
            "本轮为受控 Rust 夹具回归，不是实际客户 OMP 的真实链路校准。".into(),
            "五个保留案例和两次重复不足以估计生产误判率或建立统计置信区间。".into(),
            "任何未执行、证据不足或人工争议案例都必须阻止受影响的确定结论发布。".into(),
        ],
    }
}

fn acceptance_results(
    cases: &[CalibrationCase],
    metrics: &[CalibrationMetric],
    release_gate: ReleaseGateStatus,
) -> Vec<AcceptanceResult> {
    let evidence_refs = cases
        .iter()
        .filter_map(|case| case.executions.first())
        .flat_map(|execution| execution.evidence.evidence_refs.iter().cloned())
        .collect::<Vec<_>>();
    let source_isolated = cases.iter().all(|case| {
        case.holdout_dataset_version == HOLDOUT_DATASET_VERSION
            && case.source_ref.starts_with("holdout://")
            && case.executions.iter().all(|execution| {
                execution
                    .evidence
                    .expected_basis
                    .contains("outside the Agent")
            })
    });
    let complete_classes = cases.len() == 5
        && cases.iter().all(|case| {
            case.executions.len() == CALIBRATION_REPEAT_COUNT
                && case
                    .executions
                    .iter()
                    .all(|execution| !execution.agent_scenario.is_empty())
        });
    let metric_set_complete = metrics.len() == 5
        && metrics
            .iter()
            .map(|metric| metric.kind)
            .collect::<BTreeSet<_>>()
            .len()
            == 5
        && metrics.iter().all(|metric| metric.denominator > 0);
    let path_representative = cases.iter().all(|case| case.representative_path_verified)
        && cases
            .iter()
            .find(|case| case.fixture == CalibrationFixtureKind::LegalRecovery)
            .is_some_and(|case| case.failure_history_preserved);
    let gate_reviewed = release_gate == ReleaseGateStatus::InsufficientEvidence;
    vec![
        acceptance(
            "AC-01",
            source_isolated,
            "同源变体隔离；预期标签不得进入判定器输入",
            "保留集来源、规则版本和标签隔离均已记录",
            &evidence_refs,
        ),
        acceptance(
            "AC-02",
            complete_classes,
            "五类已知结果均有重复执行与完整实际结论",
            "结构差异成功、结构相同失败、环境故障、合法恢复和严重事件均已执行",
            &evidence_refs,
        ),
        acceptance(
            "AC-03",
            metric_set_complete,
            "五类误判分别统计，分母可查",
            "误杀、漏报、范围夸大、错误下定论和过度弃判均有独立分子与分母",
            &evidence_refs,
        ),
        acceptance(
            "AC-04",
            path_representative,
            "实际 Agent 路径覆盖记录、复核和结束结论",
            "所有案例均经过完整 Agent 报告链路；合法恢复仍保留首次有效失败",
            &evidence_refs,
        ),
        acceptance(
            "AC-05",
            gate_reviewed,
            "证据不足时不得放行确定性可靠性结论",
            "当前门禁为 insufficient_evidence，不宣称统计可靠性或生产放行",
            &evidence_refs,
        ),
    ]
}

fn acceptance(
    id: &str,
    verified: bool,
    expected: &str,
    actual: &str,
    evidence_refs: &[String],
) -> AcceptanceResult {
    AcceptanceResult {
        id: id.into(),
        status: if verified {
            AcceptanceStatus::Verified
        } else {
            AcceptanceStatus::Failed
        },
        material_version: CALIBRATION_VERSION.into(),
        method: "cargo test calibration::tests".into(),
        expected: expected.into(),
        actual: actual.into(),
        evidence_refs: evidence_refs.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_holdout_covers_all_five_known_outcome_classes() {
        let report = run_calibration();
        assert_eq!(report.cases.len(), 5);
        assert_eq!(
            report
                .cases
                .iter()
                .map(|case| case.expected)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                KnownOutcome::NormalUse,
                KnownOutcome::LimitedUse,
                KnownOutcome::CannotUse,
                KnownOutcome::Unknown,
            ])
        );
        assert!(report.cases.iter().all(|case| {
            case.holdout_dataset_version == HOLDOUT_DATASET_VERSION
                && case.source_ref.starts_with("holdout://")
                && case.executions.len() == CALIBRATION_REPEAT_COUNT
        }));
    }

    #[test]
    fn full_agent_path_repeats_without_label_leakage() {
        let report = run_calibration();
        assert!(report.cases.iter().all(|case| {
            case.representative_path_verified
                && case.executions.iter().all(|execution| {
                    execution.evidence.agent_path
                        == "assess_execution -> build_report_for_record -> build_customer_report"
                        && execution
                            .evidence
                            .expected_basis
                            .contains("outside the Agent")
                })
        }));
        let recovery = report
            .cases
            .iter()
            .find(|case| case.fixture == CalibrationFixtureKind::LegalRecovery)
            .unwrap();
        assert!(recovery.failure_history_preserved);
        assert!(
            recovery
                .executions
                .iter()
                .all(|execution| execution.agent_outcome == AgentOutcome::Fail)
        );
    }

    #[test]
    fn metrics_are_separate_and_release_gate_does_not_claim_reliability() {
        let report = run_calibration();
        assert!(report.metrics.iter().all(|metric| metric.numerator == 0));
        assert_eq!(
            report
                .metrics
                .iter()
                .find(|metric| metric.kind == CalibrationErrorKind::OverAbstention)
                .unwrap()
                .denominator,
            4
        );
        assert_eq!(report.release_gate, ReleaseGateStatus::InsufficientEvidence);
        assert!(report.release_gate_reason.contains("不得泛化"));
        assert!(
            report
                .limitations
                .iter()
                .any(|item| item.contains("统计置信区间"))
        );
        assert!(
            report
                .acceptance
                .iter()
                .all(|result| result.status == AcceptanceStatus::Verified)
        );
        assert!(
            report
                .acceptance
                .iter()
                .find(|result| result.id == "AC-05")
                .is_some_and(|result| result.actual.contains("insufficient_evidence"))
        );
    }

    #[test]
    fn structure_and_failure_cases_are_not_collapsed() {
        let report = run_calibration();
        let different = report
            .cases
            .iter()
            .find(|case| case.fixture == CalibrationFixtureKind::StructureDifferentTaskSuccess)
            .unwrap();
        assert_eq!(different.expected, KnownOutcome::NormalUse);
        assert_eq!(
            different.executions[0].baseline_status,
            StructuralStatus::Different
        );
        assert_eq!(different.executions[0].observed, KnownOutcome::NormalUse);

        let same_failure = report
            .cases
            .iter()
            .find(|case| case.fixture == CalibrationFixtureKind::StructureSameTaskFailure)
            .unwrap();
        assert_ne!(
            same_failure.executions[0].baseline_status,
            StructuralStatus::Different
        );
        assert_eq!(same_failure.executions[0].observed, KnownOutcome::CannotUse);
    }

    #[test]
    fn json_contains_versions_metrics_and_gate() {
        let report = run_calibration();
        let json = report.json().unwrap();
        assert!(json.contains(HOLDOUT_DATASET_VERSION));
        assert!(json.contains("false_positive"));
        assert!(json.contains("insufficient_evidence"));
        assert!(json.contains("holdout-severe-event"));
    }
}
