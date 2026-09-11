use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

use crate::records::DetectionRecord;

pub const SPECIFICATION_VERSION: &str = "specification/v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum SpecCategory {
    #[serde(rename = "S01")]
    ContextCapacity,
    #[serde(rename = "S02")]
    OutputLength,
    #[serde(rename = "S03")]
    CommonParameters,
    #[serde(rename = "S04")]
    ToolCalls,
    #[serde(rename = "S05")]
    StructuredOutput,
    #[serde(rename = "S06")]
    MessagesAndTurns,
    #[serde(rename = "S07")]
    Streaming,
}

impl SpecCategory {
    pub const ALL: [Self; 7] = [
        Self::ContextCapacity,
        Self::OutputLength,
        Self::CommonParameters,
        Self::ToolCalls,
        Self::StructuredOutput,
        Self::MessagesAndTurns,
        Self::Streaming,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::ContextCapacity => "S01",
            Self::OutputLength => "S02",
            Self::CommonParameters => "S03",
            Self::ToolCalls => "S04",
            Self::StructuredOutput => "S05",
            Self::MessagesAndTurns => "S06",
            Self::Streaming => "S07",
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::ContextCapacity => "上下文容量",
            Self::OutputLength => "输出长度",
            Self::CommonParameters => "常用参数",
            Self::ToolCalls => "工具调用",
            Self::StructuredOutput => "结构化输出",
            Self::MessagesAndTurns => "消息与多轮输入",
            Self::Streaming => "流式输出",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpecStatus {
    Accepted,
    Effective,
    VerifiedRange,
    Unsupported,
    Failed,
    Inconclusive,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceOrigin {
    RealExecution,
    ControlledFixture,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttemptKind {
    Initial,
    Recheck,
    Retry,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpecificationPlan {
    pub category: SpecCategory,
    pub samples: Vec<String>,
    pub fixed_settings: BTreeMap<String, String>,
    pub max_rechecks: u8,
    pub max_retries: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpecificationObservation {
    pub category: SpecCategory,
    pub sample_id: String,
    pub attempt: AttemptKind,
    pub conditions: BTreeMap<String, String>,
    pub actual_output: Option<Value>,
    pub finish_reason: Option<String>,
    pub natural_end: bool,
    pub status: SpecStatus,
    pub evidence_refs: Vec<String>,
    pub evidence_origin: EvidenceOrigin,
    pub limitation: Option<String>,
    pub verified_scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerifiedRange {
    pub category: SpecCategory,
    pub scope: String,
    pub conditions_fingerprint: String,
    pub evidence_refs: Vec<String>,
    pub record_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpecificationRow {
    pub category: SpecCategory,
    pub result: SpecStatus,
    pub verified_scope: Option<String>,
    pub conditions: BTreeMap<String, String>,
    pub limitations: Vec<String>,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpecificationReport {
    pub version: String,
    pub record_id: String,
    pub rows: Vec<SpecificationRow>,
    pub observations: Vec<SpecificationObservation>,
    pub verified_ranges: Vec<VerifiedRange>,
}

pub fn seven_category_plan() -> Vec<SpecificationPlan> {
    vec![
        plan(
            SpecCategory::ContextCapacity,
            vec!["context-1024", "context-2048", "context-boundary"],
            [("system", "fixed"), ("output_limit", "256")],
        ),
        plan(
            SpecCategory::OutputLength,
            vec!["output-small", "output-medium", "output-natural"],
            [
                ("prompt", "numbered-sequence"),
                ("one_output_limit_field", "true"),
            ],
        ),
        plan(
            SpecCategory::CommonParameters,
            vec!["P02", "P03", "P04", "P05", "P06"],
            [
                ("control_group", "one_factor_at_a_time"),
                ("baseline_group", "required"),
            ],
        ),
        plan(
            SpecCategory::ToolCalls,
            vec!["T01", "T02", "T03", "T04", "T05", "T06", "T07"],
            [("stream", "false"), ("tool_execution", "never")],
        ),
        plan(
            SpecCategory::StructuredOutput,
            vec!["plain-text", "json", "schema"],
            [("same_short_input", "true")],
        ),
        plan(
            SpecCategory::MessagesAndTurns,
            vec!["M01", "M02", "M03", "M04"],
            [("cross_request_memory", "false")],
        ),
        plan(
            SpecCategory::Streaming,
            vec!["stream-text", "stream-tool"],
            [("stream", "true"), ("normal_termination", "required")],
        ),
    ]
}

pub fn build_report(
    record_id: &str,
    observations: Vec<SpecificationObservation>,
) -> Result<SpecificationReport, String> {
    validate_observations(&observations)?;
    let mut rows = Vec::new();
    let mut ranges = Vec::new();
    for category in SpecCategory::ALL {
        let category_observations = observations
            .iter()
            .filter(|observation| observation.category == category)
            .collect::<Vec<_>>();
        if category_observations.is_empty() {
            rows.push(SpecificationRow {
                category,
                result: SpecStatus::Inconclusive,
                verified_scope: None,
                conditions: BTreeMap::new(),
                limitations: vec!["No observation was executed".into()],
                evidence_refs: Vec::new(),
            });
            continue;
        }
        let result = category_status(&category_observations);
        let verified_scope = category_observations
            .iter()
            .filter(|observation| observation.status == SpecStatus::VerifiedRange)
            .filter(|observation| observation.evidence_origin == EvidenceOrigin::RealExecution)
            .filter_map(|observation| observation.verified_scope.clone())
            .max_by_key(|scope| scope.len());
        let evidence_refs = unique_refs(&category_observations);
        let conditions = merge_conditions(&category_observations);
        let limitations = category_observations
            .iter()
            .filter_map(|observation| observation.limitation.clone())
            .collect();
        if let Some(scope) = verified_scope.clone() {
            ranges.push(VerifiedRange {
                category,
                scope,
                conditions_fingerprint: fingerprint(&conditions),
                evidence_refs: evidence_refs.clone(),
                record_id: record_id.into(),
            });
        }
        rows.push(SpecificationRow {
            category,
            result,
            verified_scope,
            conditions,
            limitations,
            evidence_refs,
        });
    }
    Ok(SpecificationReport {
        version: SPECIFICATION_VERSION.into(),
        record_id: record_id.into(),
        rows,
        observations,
        verified_ranges: ranges,
    })
}

pub fn build_report_for_record(
    record: &DetectionRecord,
    observations: Vec<SpecificationObservation>,
) -> Result<SpecificationReport, String> {
    let known_evidence = record
        .evidence
        .iter()
        .map(|evidence| evidence.id.as_str())
        .collect::<BTreeSet<_>>();
    for observation in &observations {
        for evidence_ref in &observation.evidence_refs {
            if !known_evidence.contains(evidence_ref.as_str()) {
                return Err(format!(
                    "Unknown evidence reference for {}: {evidence_ref}",
                    observation.sample_id
                ));
            }
        }
    }
    build_report(&record.id, observations)
}

pub fn reusable_verified_ranges(report: &SpecificationReport) -> Vec<VerifiedRange> {
    report.verified_ranges.clone()
}

fn plan(
    category: SpecCategory,
    samples: Vec<&str>,
    fixed_settings: impl IntoIterator<Item = (&'static str, &'static str)>,
) -> SpecificationPlan {
    SpecificationPlan {
        category,
        samples: samples.into_iter().map(str::to_owned).collect(),
        fixed_settings: fixed_settings
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect(),
        max_rechecks: 2,
        max_retries: 2,
    }
}

fn validate_observations(observations: &[SpecificationObservation]) -> Result<(), String> {
    for observation in observations {
        if observation.sample_id.trim().is_empty() {
            return Err("Specification sample ID cannot be empty".into());
        }
        if observation.evidence_refs.is_empty() && observation.status != SpecStatus::NotApplicable {
            return Err(format!(
                "Observation {} requires evidence references",
                observation.sample_id
            ));
        }
        if observation.status == SpecStatus::VerifiedRange
            && (observation.evidence_origin != EvidenceOrigin::RealExecution
                || observation.verified_scope.is_none())
        {
            return Err(format!(
                "Verified range {} requires real evidence and an explicit scope",
                observation.sample_id
            ));
        }
    }
    Ok(())
}

fn category_status(observations: &[&SpecificationObservation]) -> SpecStatus {
    if observations
        .iter()
        .any(|observation| observation.status == SpecStatus::VerifiedRange)
    {
        return SpecStatus::VerifiedRange;
    }
    if observations
        .iter()
        .any(|observation| observation.status == SpecStatus::Effective)
    {
        return SpecStatus::Effective;
    }
    if observations
        .iter()
        .any(|observation| observation.status == SpecStatus::Unsupported)
    {
        return SpecStatus::Unsupported;
    }
    if observations
        .iter()
        .any(|observation| observation.status == SpecStatus::Failed)
    {
        return SpecStatus::Failed;
    }
    if observations
        .iter()
        .any(|observation| observation.status == SpecStatus::Accepted)
    {
        return SpecStatus::Accepted;
    }
    if observations
        .iter()
        .all(|observation| observation.status == SpecStatus::NotApplicable)
    {
        return SpecStatus::NotApplicable;
    }
    SpecStatus::Inconclusive
}

fn unique_refs(observations: &[&SpecificationObservation]) -> Vec<String> {
    observations
        .iter()
        .flat_map(|observation| observation.evidence_refs.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn merge_conditions(observations: &[&SpecificationObservation]) -> BTreeMap<String, String> {
    observations
        .iter()
        .flat_map(|observation| observation.conditions.iter())
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn fingerprint(value: &BTreeMap<String, String>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(value).expect("conditions must serialize"));
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn observation(
        category: SpecCategory,
        sample_id: &str,
        status: SpecStatus,
        origin: EvidenceOrigin,
    ) -> SpecificationObservation {
        SpecificationObservation {
            category,
            sample_id: sample_id.into(),
            attempt: AttemptKind::Initial,
            conditions: BTreeMap::from([(String::from("test_version"), String::from("1.0"))]),
            actual_output: Some(json!({ "text": "redacted result" })),
            finish_reason: Some("stop".into()),
            natural_end: false,
            status,
            evidence_refs: vec![format!("evidence://{sample_id}")],
            evidence_origin: origin,
            limitation: None,
            verified_scope: None,
        }
    }

    #[test]
    fn freezes_the_seven_category_plan_and_retry_limits() {
        let plans = seven_category_plan();
        assert_eq!(plans.len(), 7);
        assert_eq!(
            plans.iter().map(|plan| plan.category).collect::<Vec<_>>(),
            SpecCategory::ALL
        );
        assert!(
            plans
                .iter()
                .all(|plan| plan.max_rechecks == 2 && plan.max_retries == 2)
        );
        assert_eq!(
            plans[3].samples,
            ["T01", "T02", "T03", "T04", "T05", "T06", "T07"]
        );
    }

    #[test]
    fn accepted_does_not_become_effective_or_verified() {
        let report = build_report(
            "run-a",
            vec![observation(
                SpecCategory::CommonParameters,
                "P02",
                SpecStatus::Accepted,
                EvidenceOrigin::RealExecution,
            )],
        )
        .unwrap();
        assert_eq!(report.rows[2].result, SpecStatus::Accepted);
        assert!(report.verified_ranges.is_empty());
    }

    #[test]
    fn real_evidence_can_create_a_scoped_range_but_fixture_cannot() {
        let mut real = observation(
            SpecCategory::ContextCapacity,
            "context-2048",
            SpecStatus::VerifiedRange,
            EvidenceOrigin::RealExecution,
        );
        real.verified_scope = Some("input <= 2048 tokens; output余量=256".into());
        let mut fixture = observation(
            SpecCategory::OutputLength,
            "output-medium",
            SpecStatus::VerifiedRange,
            EvidenceOrigin::ControlledFixture,
        );
        fixture.verified_scope = Some("output <= 256 tokens".into());
        assert!(build_report("run-a", vec![fixture]).is_err());
        let report = build_report("run-a", vec![real]).unwrap();
        assert_eq!(report.verified_ranges.len(), 1);
        assert_eq!(reusable_verified_ranges(&report)[0].record_id, "run-a");
    }

    #[test]
    fn natural_end_and_unknown_boundary_are_reported_without_maximum_claims() {
        let mut natural = observation(
            SpecCategory::OutputLength,
            "output-natural",
            SpecStatus::Accepted,
            EvidenceOrigin::RealExecution,
        );
        natural.natural_end = true;
        natural.finish_reason = Some("stop".into());
        natural.limitation = Some("自然结束，未证明输出上限".into());
        let mut unknown = observation(
            SpecCategory::ContextCapacity,
            "context-boundary",
            SpecStatus::Inconclusive,
            EvidenceOrigin::RealExecution,
        );
        unknown.limitation = Some("计数或拒绝原因不足，边界未确认".into());
        let report = build_report("run-a", vec![natural, unknown]).unwrap();
        assert_eq!(report.rows[1].result, SpecStatus::Accepted);
        assert_eq!(report.rows[0].result, SpecStatus::Inconclusive);
        assert!(report.verified_ranges.is_empty());
        assert!(report.rows[1].limitations[0].contains("未证明"));
    }

    #[test]
    fn keeps_not_applicable_and_failure_separate_from_global_usability() {
        let report = build_report(
            "run-a",
            vec![
                observation(
                    SpecCategory::MessagesAndTurns,
                    "M04",
                    SpecStatus::NotApplicable,
                    EvidenceOrigin::RealExecution,
                ),
                observation(
                    SpecCategory::ToolCalls,
                    "T05",
                    SpecStatus::Failed,
                    EvidenceOrigin::RealExecution,
                ),
            ],
        )
        .unwrap();
        assert_eq!(report.rows[3].result, SpecStatus::Failed);
        assert_eq!(report.rows[5].result, SpecStatus::NotApplicable);
        assert!(
            !report
                .rows
                .iter()
                .any(|row| row.result == SpecStatus::Unsupported
                    && row.category == SpecCategory::ToolCalls)
        );
    }

    #[test]
    fn report_serialization_is_shared_by_cli_and_gui_consumers() {
        let report = build_report(
            "run-a",
            vec![observation(
                SpecCategory::StructuredOutput,
                "json",
                SpecStatus::Effective,
                EvidenceOrigin::RealExecution,
            )],
        )
        .unwrap();
        let serialized = serde_json::to_string(&report).unwrap();
        let restored: SpecificationReport = serde_json::from_str(&serialized).unwrap();
        assert_eq!(restored, report);
        assert_eq!(restored.version, SPECIFICATION_VERSION);
    }

    #[test]
    fn binds_observations_to_the_current_detection_record() {
        let record = crate::records::create_run(crate::records::CreateRunInput {
            id: "run-a".into(),
            now: "2026-09-11T00:00:00Z".into(),
            target: crate::records::ServiceSnapshotInput {
                endpoint_fingerprint: "endpoint-a".into(),
                model: "model-a".into(),
                protocol: "chat-completions".into(),
                auth_mode: "bearer".into(),
                client_version: "0.1.0".into(),
                environment: BTreeMap::new(),
            },
            selected_modules: None,
        });
        let observation = observation(
            SpecCategory::StructuredOutput,
            "json",
            SpecStatus::Effective,
            EvidenceOrigin::RealExecution,
        );
        assert!(build_report_for_record(&record, vec![observation]).is_err());
    }
}
