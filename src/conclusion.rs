use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use crate::cli::module_display_name;
use crate::records::{
    DetectionRecord, EventKind, LifecycleState, ModuleResultState, ModuleSelectionState,
};

pub const CONCLUSION_VERSION: &str = "customer-conclusion/v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CustomerConclusionKind {
    NormalUse,
    LimitedUse,
    CannotUse,
    Unknown,
}

impl CustomerConclusionKind {
    pub const fn text(self) -> &'static str {
        match self {
            Self::NormalUse => "这套模型服务可以正常使用",
            Self::LimitedUse => "这套模型服务可以使用，但存在限制",
            Self::CannotUse => "这套模型服务目前不能正常使用",
            Self::Unknown => "目前无法判断这套模型服务是否可用",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FindingImpact {
    None,
    Limitation,
    Blocker,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConclusionFinding {
    pub id: String,
    pub module_id: String,
    pub title: String,
    pub impact: FindingImpact,
    pub summary: String,
    pub scope: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScopeItem {
    pub module_id: String,
    pub state: ModuleResultState,
    pub verified: bool,
    pub description: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvidenceTraceStep {
    pub kind: String,
    pub id: String,
    pub summary: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvidenceTrace {
    pub evidence_ref: String,
    pub steps: Vec<EvidenceTraceStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentScopeStatement {
    pub scenarios: String,
    pub checks: String,
    pub verified: bool,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineScopeStatement {
    pub scenarios: String,
    pub interpretation: String,
    pub structural_differences_are_not_overall_usability: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomerConclusionReport {
    pub version: String,
    pub record_id: String,
    pub model: String,
    pub endpoint_fingerprint: String,
    pub lifecycle: LifecycleState,
    pub kind: CustomerConclusionKind,
    pub text: String,
    pub scope: Vec<ScopeItem>,
    pub findings: Vec<ConclusionFinding>,
    pub evidence_gaps: Vec<String>,
    pub evidence_traces: Vec<EvidenceTrace>,
    pub agent_scope: AgentScopeStatement,
    pub baseline_scope: BaselineScopeStatement,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ConclusionBuildInput {
    pub findings: Vec<ConclusionFinding>,
    pub evidence_gaps: Vec<String>,
    pub agent_scope: Option<AgentScopeStatement>,
    pub baseline_scope: Option<BaselineScopeStatement>,
}

pub fn build_default_customer_report(
    record: &DetectionRecord,
) -> Result<CustomerConclusionReport, String> {
    build_customer_report(record, ConclusionBuildInput::default())
}

/// 普通运行路径的客户结论：把失败模块汇成 findings，让"关键发现"如实呈现，
/// 也让整体结论把模块失败纳入考量（否则 choose_conclusion 对 Fail 视而不见）。
/// 失败按「受限」级呈现；Blocker 级（如密钥泄漏、越权写入）仍由校准流程细化。
pub fn build_run_customer_report(
    record: &DetectionRecord,
) -> Result<CustomerConclusionReport, String> {
    let findings: Vec<ConclusionFinding> = record
        .module_results
        .iter()
        .filter(|result| result.state == ModuleResultState::Fail)
        .map(|result| ConclusionFinding {
            id: format!("module-{}-fail", result.module_id),
            module_id: result.module_id.clone(),
            title: format!("{}未通过", module_display_name(&result.module_id)),
            impact: FindingImpact::Limitation,
            summary: result.reason.clone().unwrap_or_else(|| {
                "该模块存在未通过的检测项，详见模块明细".to_string()
            }),
            scope: "本次实测覆盖的项目".into(),
            evidence_refs: result.evidence_refs.clone(),
        })
        .collect();
    build_customer_report(
        record,
        ConclusionBuildInput {
            findings,
            ..ConclusionBuildInput::default()
        },
    )
}

pub fn build_customer_report(
    record: &DetectionRecord,
    input: ConclusionBuildInput,
) -> Result<CustomerConclusionReport, String> {
    validate_input_evidence(record, &input)?;
    let scope = build_scope(record);
    let mut evidence_gaps = input.evidence_gaps;
    evidence_gaps.extend(automatic_gaps(&scope));
    deduplicate(&mut evidence_gaps);
    let kind = choose_conclusion(&input.findings, &evidence_gaps, &scope);
    let evidence_traces = build_evidence_traces(record, &input.findings, &scope);
    let agent_scope = input.agent_scope.unwrap_or_else(default_agent_scope);
    let baseline_scope = input.baseline_scope.unwrap_or_else(default_baseline_scope);
    Ok(CustomerConclusionReport {
        version: CONCLUSION_VERSION.into(),
        record_id: record.id.clone(),
        model: record.target.model.clone(),
        endpoint_fingerprint: record.target.endpoint_fingerprint.clone(),
        lifecycle: record.lifecycle,
        kind,
        text: kind.text().into(),
        scope,
        findings: input.findings,
        evidence_gaps,
        evidence_traces,
        agent_scope,
        baseline_scope,
        limitations: vec![
            "结论只覆盖本次记录中实际选择并有证据的范围，不扩展到未测业务或长期生产表现。".into(),
            "模块事实、基线结构差异和 Agent 场景结果分别展示，不相加生成总分。".into(),
            "未观测、复核未完成或归因有争议的必要范围保持无法判断。".into(),
        ],
    })
}

pub fn report_json(report: &CustomerConclusionReport) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(report)
}

fn build_scope(record: &DetectionRecord) -> Vec<ScopeItem> {
    record
        .plan
        .iter()
        .filter(|entry| entry.state == ModuleSelectionState::Selected)
        .map(|entry| {
            let result = record
                .module_results
                .iter()
                .find(|result| result.module_id == entry.module_id);
            let state = result.map_or(ModuleResultState::Unverified, |result| result.state);
            ScopeItem {
                module_id: entry.module_id.clone(),
                state,
                verified: matches!(
                    state,
                    ModuleResultState::Pass
                        | ModuleResultState::Fail
                        | ModuleResultState::NotApplicable
                ),
                description: result
                    .and_then(|result| result.reason.clone())
                    .unwrap_or_else(|| "该项目未完成实测，无可核对结果".into()),
                evidence_refs: result
                    .map(|result| result.evidence_refs.clone())
                    .unwrap_or_default(),
            }
        })
        .collect()
}

fn automatic_gaps(scope: &[ScopeItem]) -> Vec<String> {
    if scope.is_empty() {
        return vec!["本次运行没有选择任何正式检测项目".into()];
    }
    scope
        .iter()
        .filter_map(|item| {
            let name = module_display_name(&item.module_id);
            match item.state {
                ModuleResultState::Inconclusive => {
                    Some(format!("「{name}」未取得足够检测证据，暂不能判断其可用性"))
                }
                ModuleResultState::InvalidExecution => Some(format!(
                    "「{name}」检测执行受环境影响无效，结果不计入模型能力判断"
                )),
                ModuleResultState::Unverified => {
                    Some(format!("「{name}」未完成检测，不计入已测范围"))
                }
                ModuleResultState::Unsupported => {
                    Some(format!("「{name}」当前服务不支持，未取得有效能力证据"))
                }
                ModuleResultState::Fail if item.evidence_refs.is_empty() => {
                    Some(format!("「{name}」判定未通过，但缺少可核对的检测证据"))
                }
                _ => None,
            }
        })
        .collect()
}

fn choose_conclusion(
    findings: &[ConclusionFinding],
    evidence_gaps: &[String],
    scope: &[ScopeItem],
) -> CustomerConclusionKind {
    if findings
        .iter()
        .any(|finding| finding.impact == FindingImpact::Blocker)
    {
        CustomerConclusionKind::CannotUse
    } else if !evidence_gaps.is_empty()
        || scope.iter().any(|item| {
            matches!(
                item.state,
                ModuleResultState::Inconclusive
                    | ModuleResultState::InvalidExecution
                    | ModuleResultState::Unverified
                    | ModuleResultState::Unsupported
            )
        })
    {
        CustomerConclusionKind::Unknown
    } else if findings
        .iter()
        .any(|finding| finding.impact == FindingImpact::Limitation)
    {
        CustomerConclusionKind::LimitedUse
    } else {
        CustomerConclusionKind::NormalUse
    }
}

fn build_evidence_traces(
    record: &DetectionRecord,
    findings: &[ConclusionFinding],
    scope: &[ScopeItem],
) -> Vec<EvidenceTrace> {
    let refs = findings
        .iter()
        .flat_map(|finding| finding.evidence_refs.iter().cloned())
        .chain(
            scope
                .iter()
                .flat_map(|item| item.evidence_refs.iter().cloned()),
        )
        .collect::<BTreeSet<_>>();
    refs.into_iter()
        .map(|evidence_ref| {
            let mut steps = Vec::new();
            if let Some(evidence) = record.evidence.iter().find(|item| item.id == evidence_ref) {
                steps.push(EvidenceTraceStep {
                    kind: "evidence".into(),
                    id: evidence.id.clone(),
                    summary: format!("{} at {}", evidence.kind, evidence.captured_at),
                    evidence_refs: vec![evidence.id.clone()],
                });
            }
            steps.extend(
                record
                    .attempts
                    .iter()
                    .filter(|attempt| attempt.evidence_refs.contains(&evidence_ref))
                    .map(|attempt| EvidenceTraceStep {
                        kind: "attempt".into(),
                        id: attempt.id.clone(),
                        summary: format!("{} {:?}", attempt.module_id, attempt.kind),
                        evidence_refs: attempt.evidence_refs.clone(),
                    }),
            );
            steps.extend(
                record
                    .events
                    .iter()
                    .filter(|event| event.evidence_refs.contains(&evidence_ref))
                    .map(|event| EvidenceTraceStep {
                        kind: match event.kind {
                            EventKind::ToolFailure => "failure_event",
                            EventKind::Permission => "permission_event",
                            _ => "event",
                        }
                        .into(),
                        id: event.id.clone(),
                        summary: event.summary.clone(),
                        evidence_refs: event.evidence_refs.clone(),
                    }),
            );
            steps.extend(
                record
                    .module_results
                    .iter()
                    .filter(|result| result.evidence_refs.contains(&evidence_ref))
                    .map(|result| EvidenceTraceStep {
                        kind: "module_result".into(),
                        id: result.module_id.clone(),
                        summary: format!("module state {:?}", result.state),
                        evidence_refs: result.evidence_refs.clone(),
                    }),
            );
            EvidenceTrace {
                evidence_ref,
                steps,
            }
        })
        .collect()
}

fn validate_input_evidence(
    record: &DetectionRecord,
    input: &ConclusionBuildInput,
) -> Result<(), String> {
    let known = record
        .evidence
        .iter()
        .map(|evidence| evidence.id.as_str())
        .collect::<BTreeSet<_>>();
    for evidence_ref in input
        .findings
        .iter()
        .flat_map(|finding| finding.evidence_refs.iter())
    {
        if !known.contains(evidence_ref.as_str()) {
            return Err(format!(
                "Unknown conclusion evidence reference: {evidence_ref}"
            ));
        }
    }
    Ok(())
}

fn default_agent_scope() -> AgentScopeStatement {
    AgentScopeStatement {
        scenarios: "T1-A 至 T5-B 固定 OMP 场景".into(),
        checks: "A1 至 A8".into(),
        verified: false,
        limitations: vec!["未测业务工作流、长上下文、并发和生产持续运行不在该声明内".into()],
    }
}

fn default_baseline_scope() -> BaselineScopeStatement {
    BaselineScopeStatement {
        scenarios: "Chat Completions BC01 至 BC14".into(),
        interpretation: "只说明结构路径、类型和适用变体；动态值和结构差异不直接决定整体可用性"
            .into(),
        structural_differences_are_not_overall_usability: true,
    }
}

fn deduplicate(values: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::{
        AttemptKind, AttemptRecord, CreateRunInput, ModuleResult, ServiceSnapshotInput,
        add_attempt, add_event, add_evidence, create_run, set_module_result, start_run,
    };
    use std::collections::BTreeMap;

    fn record(selected: Vec<&str>) -> DetectionRecord {
        create_run(CreateRunInput {
            id: "run-conclusion".into(),
            now: "2026-09-11T00:00:00Z".into(),
            target: ServiceSnapshotInput {
                endpoint_fingerprint: "api.example.test".into(),
                model: "model-a".into(),
                protocol: "chat-completions".into(),
                auth_mode: "bearer".into(),
                client_version: "test".into(),
                environment: BTreeMap::new(),
            },
            selected_modules: Some(selected.into_iter().map(Into::into).collect()),
        })
    }

    fn pass_record() -> DetectionRecord {
        let mut record = record(vec!["capability"]);
        start_run(&mut record, "2026-09-11T00:01:00Z").unwrap();
        let evidence = add_evidence(
            &mut record,
            "capability-evidence",
            "capability_result",
            "2026-09-11T00:01:01Z",
            serde_json::json!({"status":"pass"}),
        );
        add_attempt(
            &mut record,
            AttemptRecord {
                id: "attempt-capability".into(),
                module_id: "capability".into(),
                kind: AttemptKind::Initial,
                started_at: "2026-09-11T00:01:00Z".into(),
                ended_at: Some("2026-09-11T00:01:01Z".into()),
                supersedes_attempt_id: None,
                evidence_refs: vec![evidence.id.clone()],
            },
        )
        .unwrap();
        add_event(
            &mut record,
            crate::records::EventInput {
                id: "event-capability".into(),
                kind: EventKind::Response,
                occurred_at: "2026-09-11T00:01:01Z".into(),
                summary: "能力结果已记录".into(),
                incident_id: None,
                attempt_id: Some("attempt-capability".into()),
                evidence_refs: vec![evidence.id.clone()],
            },
        )
        .unwrap();
        set_module_result(
            &mut record,
            ModuleResult {
                module_id: "capability".into(),
                state: ModuleResultState::Pass,
                reason: Some("fixed capability checks passed".into()),
                attempt_refs: vec!["attempt-capability".into()],
                evidence_refs: vec![evidence.id],
                incident_refs: Vec::new(),
            },
            "2026-09-11T00:01:01Z",
        )
        .unwrap();
        record
    }

    #[test]
    fn emits_exact_normal_use_text_when_scope_is_verified() {
        let report = build_default_customer_report(&pass_record()).unwrap();
        assert_eq!(report.kind, CustomerConclusionKind::NormalUse);
        assert_eq!(report.text, "这套模型服务可以正常使用");
        assert_eq!(report.scope[0].module_id, "capability");
        assert_eq!(report.evidence_traces.len(), 1);
    }

    #[test]
    fn limitation_is_lower_priority_than_a_confirmed_blocker() {
        let record = pass_record();
        let evidence = record.evidence[0].id.clone();
        let limitation = ConclusionFinding {
            id: "limited-output".into(),
            module_id: "capability".into(),
            title: "范围限制".into(),
            impact: FindingImpact::Limitation,
            summary: "部分长材料不在验证范围".into(),
            scope: "已测短输入".into(),
            evidence_refs: vec![evidence.clone()],
        };
        let report = build_customer_report(
            &record,
            ConclusionBuildInput {
                findings: vec![limitation],
                ..ConclusionBuildInput::default()
            },
        )
        .unwrap();
        assert_eq!(report.kind, CustomerConclusionKind::LimitedUse);
        let blocker = ConclusionFinding {
            id: "blocked-operation".into(),
            module_id: "agent".into(),
            title: "未授权写入".into(),
            impact: FindingImpact::Blocker,
            summary: "实际状态出现未授权写入".into(),
            scope: "A7".into(),
            evidence_refs: vec![evidence],
        };
        let report = build_customer_report(
            &record,
            ConclusionBuildInput {
                findings: vec![blocker],
                ..ConclusionBuildInput::default()
            },
        )
        .unwrap();
        assert_eq!(report.kind, CustomerConclusionKind::CannotUse);
        assert_eq!(report.text, "这套模型服务目前不能正常使用");
    }

    #[test]
    fn missing_scope_is_unknown_and_not_limited_or_normal() {
        let mut record = record(vec!["capability"]);
        start_run(&mut record, "2026-09-11T00:01:00Z").unwrap();
        let report = build_default_customer_report(&record).unwrap();
        assert_eq!(report.kind, CustomerConclusionKind::Unknown);
        assert_eq!(report.text, "目前无法判断这套模型服务是否可用");
        assert!(!report.evidence_gaps.is_empty());
    }

    #[test]
    fn failed_modules_enter_findings_and_limit_conclusion() {
        // 回归护栏：choose_conclusion 原本无视模块 Fail，失败模块会被当成"正常使用"。
        let mut record = record(vec!["capability", "agent"]);
        start_run(&mut record, "2026-09-11T00:01:00Z").unwrap();
        let mut finish = |record: &mut DetectionRecord,
                          module: &str,
                          state,
                          reason: &str,
                          evidence_id: &str| {
            let evidence = add_evidence(
                record,
                evidence_id,
                "module_result",
                "2026-09-11T00:01:01Z",
                serde_json::json!({"status": "recorded"}),
            );
            set_module_result(
                record,
                ModuleResult {
                    module_id: module.into(),
                    state,
                    reason: Some(reason.into()),
                    attempt_refs: Vec::new(),
                    evidence_refs: vec![evidence.id],
                    incident_refs: Vec::new(),
                },
                "2026-09-11T00:01:02Z",
            )
            .unwrap();
        };
        finish(
            &mut record,
            "capability",
            ModuleResultState::Fail,
            "5 个有效样本答案未通过预先定义的判定规则",
            "ev-capability",
        );
        finish(&mut record, "agent", ModuleResultState::Pass, "10 个场景全部通过", "ev-agent");
        let report = build_run_customer_report(&record).unwrap();
        assert_eq!(report.kind, CustomerConclusionKind::LimitedUse);
        assert_eq!(report.text, "这套模型服务可以使用，但存在限制");
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].module_id, "capability");
        assert_eq!(report.findings[0].title, "模型能力跑分未通过");
        assert!(report.findings[0].summary.contains("5 个有效样本"));
    }

    #[test]
    fn structural_baseline_note_does_not_change_customer_conclusion() {
        let report = build_customer_report(
            &pass_record(),
            ConclusionBuildInput {
                baseline_scope: Some(BaselineScopeStatement {
                    scenarios: "BC01".into(),
                    interpretation: "观察到结构差异，但没有使用影响".into(),
                    structural_differences_are_not_overall_usability: true,
                }),
                ..ConclusionBuildInput::default()
            },
        )
        .unwrap();
        assert_eq!(report.kind, CustomerConclusionKind::NormalUse);
        assert!(
            report
                .baseline_scope
                .structural_differences_are_not_overall_usability
        );
    }

    #[test]
    fn unknown_finding_evidence_cannot_enter_the_report() {
        let finding = ConclusionFinding {
            id: "unbound".into(),
            module_id: "capability".into(),
            title: "未知".into(),
            impact: FindingImpact::Blocker,
            summary: "没有当前记录证据".into(),
            scope: "test".into(),
            evidence_refs: vec!["evidence://other-run".into()],
        };
        assert!(
            build_customer_report(
                &pass_record(),
                ConclusionBuildInput {
                    findings: vec![finding],
                    ..ConclusionBuildInput::default()
                },
            )
            .is_err()
        );
    }

    #[test]
    fn serializes_customer_report_with_scope_and_trace() {
        let report = build_default_customer_report(&pass_record()).unwrap();
        let json = report_json(&report).unwrap();
        let restored: CustomerConclusionReport = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.record_id, "run-conclusion");
        assert!(json.contains("evidence_traces"));
        assert!(json.contains("目前无法判断") || json.contains("可以正常使用"));
    }
}
