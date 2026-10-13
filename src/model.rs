use crate::config::Target;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use uuid::Uuid;

pub const CHECK_VERSION: &str = "agentcheck/10";
pub const RESULT_SCHEMA_VERSION: u32 = 2;
pub const MIN_CONTEXT_TOKENS: u64 = 131_072;
pub const OMP_VERSION: &str = env!("AGENTCHECK_OMP_VERSION");

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CheckId {
    Response,
    Tools,
    Schema,
    Context,
    Performance,
    Continuous,
    Recovery,
    Multiturn,
    Custom(u32),
}
impl Serialize for CheckId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.key())
    }
}
impl<'de> Deserialize<'de> for CheckId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let key = String::deserialize(deserializer)?;
        Self::ALL
            .into_iter()
            .find(|id| id.key() == key)
            .or_else(|| {
                key.strip_prefix("custom_")
                    .and_then(|s| s.parse().ok())
                    .map(Self::Custom)
            })
            .ok_or_else(|| serde::de::Error::custom("未知检测编号"))
    }
}
impl CheckId {
    pub const ALL: [Self; 8] = [
        Self::Response,
        Self::Tools,
        Self::Schema,
        Self::Context,
        Self::Performance,
        Self::Continuous,
        Self::Recovery,
        Self::Multiturn,
    ];
    pub fn key(self) -> String {
        match self {
            Self::Response => "response".into(),
            Self::Tools => "tools".into(),
            Self::Schema => "schema".into(),
            Self::Context => "context".into(),
            Self::Performance => "performance".into(),
            Self::Continuous => "continuous".into(),
            Self::Recovery => "recovery".into(),
            Self::Multiturn => "multiturn".into(),
            Self::Custom(id) => format!("custom_{id}"),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Response => "接口与响应",
            Self::Tools => "工具调用",
            Self::Schema => "结构化输出",
            Self::Context => "长上下文",
            Self::Performance => "并发能力探测",
            Self::Continuous => "连续工具执行",
            Self::Recovery => "工具故障恢复",
            Self::Multiturn => "多轮继续执行",
            Self::Custom(_) => "扩展检测",
        }
    }
    pub fn is_agent(self) -> bool {
        matches!(self, Self::Continuous | Self::Recovery | Self::Multiturn)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Passed,
    Failed,
    Incomplete,
    NotExecuted,
}
impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Passed => "通过",
            Self::Failed => "未通过",
            Self::Incomplete => "未确认",
            Self::NotExecuted => "未执行",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureOrigin {
    ModelOutput,
    Service,
    Configuration,
    Runtime,
    Detector,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Finding {
    pub code: String,
    pub title: String,
    pub fact: String,
    pub impact: String,
    pub suggestion: String,
    pub origin: FailureOrigin,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckResult {
    pub id: CheckId,
    #[serde(default)]
    pub finished: bool,
    pub status: Status,
    pub title: String,
    pub elapsed_ms: u64,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
    #[serde(default)]
    pub facts: BTreeMap<String, Value>,
    #[serde(default)]
    pub findings: Vec<Finding>,
    #[serde(default)]
    pub tasks: Vec<TaskResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assessment: Option<crate::judgment::Assessment>,
}
impl CheckResult {
    pub fn pending(id: CheckId) -> Self {
        Self {
            id,
            finished: false,
            status: Status::NotExecuted,
            title: "未执行".into(),
            elapsed_ms: 0,
            evidence_ids: vec![],
            facts: BTreeMap::new(),
            findings: vec![],
            tasks: vec![],
            assessment: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskResult {
    pub id: String,
    #[serde(default)]
    pub finished: bool,
    pub kind: CheckId,
    pub repetition: u8,
    pub status: Status,
    pub title: String,
    pub elapsed_ms: u64,
    pub normal_end: bool,
    pub fault_delivered: bool,
    pub recovery_observed: bool,
    pub tool_events: Vec<ToolEvent>,
    pub evidence_ids: Vec<String>,
    pub findings: Vec<Finding>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolEvent {
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
    pub result: Value,
    pub is_error: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Passed,
    Failed,
    Incomplete,
}
impl Verdict {
    pub fn title(self) -> &'static str {
        match self {
            Self::Passed => "本次通过接入验收",
            Self::Failed => "暂不建议接入平台",
            Self::Incomplete => "本次检测未完成",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunResult {
    pub schema_version: u32,
    pub check_version: String,
    pub omp_version: String,
    pub id: String,
    pub target: Target,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub elapsed_ms: u64,
    pub cancelled: bool,
    pub verdict: Verdict,
    pub checks: Vec<CheckResult>,
    pub report_path: Option<String>,
    pub baseline_version: Option<String>,
    #[serde(default)]
    pub plan: Vec<crate::judgment::CheckSpec>,
    #[serde(default)]
    pub evidence: crate::judgment::EvidenceIndex,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<crate::judgment::Decision>,
}
impl RunResult {
    pub fn new(target: Target) -> Self {
        Self::with_plan(target, crate::registry::builtin_specs())
    }
    pub fn with_plan(target: Target, plan: Vec<crate::judgment::CheckSpec>) -> Self {
        Self {
            schema_version: RESULT_SCHEMA_VERSION,
            check_version: CHECK_VERSION.into(),
            omp_version: OMP_VERSION.into(),
            id: format!(
                "AC-{}-{}",
                Utc::now().format("%Y%m%dT%H%M%SZ"),
                Uuid::new_v4().simple()
            ),
            target,
            started_at: Utc::now(),
            finished_at: None,
            elapsed_ms: 0,
            cancelled: false,
            verdict: Verdict::Incomplete,
            checks: plan
                .iter()
                .map(|spec| CheckResult::pending(spec.id))
                .collect(),
            report_path: None,
            baseline_version: None,
            plan,
            evidence: Default::default(),
            decision: None,
        }
    }
    pub fn update(&mut self, result: CheckResult) {
        if let Some(slot) = self.checks.iter_mut().find(|c| c.id == result.id) {
            *slot = result;
        }
        self.refresh_decision();
    }
    pub fn refresh_decision(&mut self) {
        self.verdict = self.compute_verdict();
        if self.schema_version >= 2 {
            self.decision = Some(crate::judgment::aggregate(&self.plan, &self.checks));
        }
    }
    pub fn check_name(&self, id: CheckId) -> &str {
        self.plan
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.name.as_str())
            .unwrap_or_else(|| id.name())
    }
    pub fn compute_verdict(&self) -> Verdict {
        if self.schema_version >= 2 {
            return crate::judgment::aggregate(&self.plan, &self.checks).verdict;
        }
        self.legacy_verdict()
    }
    fn legacy_verdict(&self) -> Verdict {
        if self.checks.iter().any(|c| {
            c.status == Status::Failed || c.tasks.iter().any(|t| t.status == Status::Failed)
        }) {
            return Verdict::Failed;
        }
        // All eight distinct groups, the 128K floor, and all six independent OMP runs are required.
        let complete = CheckId::ALL.iter().all(|id| {
            self.checks.iter().any(|c| {
                if c.id != *id || c.status != Status::Passed {
                    return false;
                }
                if *id == CheckId::Context {
                    return c
                        .facts
                        .get("verified_lower_bound_tokens")
                        .and_then(Value::as_u64)
                        .is_some_and(|n| n >= MIN_CONTEXT_TOKENS);
                }
                if id.is_agent() {
                    return [1, 2].iter().all(|run| {
                        c.tasks.iter().any(|t| {
                            t.kind == *id
                                && t.repetition == *run
                                && t.status == Status::Passed
                                && t.normal_end
                                && t.finished
                                && (*id != CheckId::Recovery
                                    || (t.fault_delivered && t.recovery_observed))
                        })
                    });
                }
                true
            })
        });
        if complete {
            Verdict::Passed
        } else {
            Verdict::Incomplete
        }
    }
    pub fn completed_count(&self) -> usize {
        self.checks.iter().filter(|c| c.finished).count()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunEvent {
    Started {
        run: RunResult,
    },
    CheckStarted {
        id: CheckId,
    },
    Step {
        id: CheckId,
        message: String,
    },
    TaskStarted {
        id: String,
        kind: CheckId,
        repetition: u8,
    },
    TaskStep {
        id: String,
        kind: CheckId,
        repetition: u8,
        message: String,
    },
    TaskFinished {
        task: TaskResult,
    },
    CheckFinished {
        check: CheckResult,
    },
    CheckProgress {
        check: CheckResult,
    },
    ReportStarted,
    Finished {
        run: RunResult,
    },
    Error {
        message: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub elapsed_ms: u64,
    pub event: RunEvent,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Protocol;
    fn run() -> RunResult {
        let mut run = RunResult::new(Target {
            model: "m".into(),
            endpoint: "http://localhost/v1/chat/completions".into(),
            protocol: Protocol::ChatCompletions,
        });
        run.schema_version = 1;
        run.check_version = "agentcheck/5".into();
        run.plan.clear();
        run
    }
    #[test]
    fn failure_survives_cancellation_and_missing_results() {
        let mut run = run();
        let mut result = CheckResult::pending(CheckId::Tools);
        result.status = Status::Failed;
        result.finished = true;
        run.update(result);
        run.cancelled = true;
        assert_eq!(run.compute_verdict(), Verdict::Failed);
        assert_eq!(run.completed_count(), 1);
    }
    #[test]
    fn complete_evidence_passes_without_maximum_or_baseline_and_task_failure_wins() {
        let mut run = run();
        for check in &mut run.checks {
            check.status = Status::Passed;
            check.finished = true;
            if check.id == CheckId::Context {
                check.facts.insert(
                    "verified_lower_bound_tokens".into(),
                    Value::from(MIN_CONTEXT_TOKENS),
                );
            }
            if check.id.is_agent() {
                for repetition in [1, 2] {
                    check.tasks.push(TaskResult {
                        id: format!("{}-{repetition}", check.id.name()),
                        finished: true,
                        kind: check.id,
                        repetition,
                        status: Status::Passed,
                        title: "validated".into(),
                        elapsed_ms: 1,
                        normal_end: true,
                        fault_delivered: check.id == CheckId::Recovery,
                        recovery_observed: check.id == CheckId::Recovery,
                        tool_events: vec![],
                        evidence_ids: vec![],
                        findings: vec![],
                    });
                }
            }
        }
        assert_eq!(run.compute_verdict(), Verdict::Passed);
        run.checks
            .iter_mut()
            .find(|c| c.id == CheckId::Recovery)
            .unwrap()
            .tasks[1]
            .status = Status::Failed;
        assert_eq!(run.compute_verdict(), Verdict::Failed);
    }

    #[test]
    fn labels_cannot_substitute_for_capacity_and_six_task_evidence() {
        let mut run = run();
        for check in &mut run.checks {
            check.status = Status::Passed;
        }
        assert_eq!(run.compute_verdict(), Verdict::Incomplete);
        run.checks
            .iter_mut()
            .find(|c| c.id == CheckId::Context)
            .unwrap()
            .facts
            .insert("verified_lower_bound_tokens".into(), Value::from(131072));
        assert_eq!(run.compute_verdict(), Verdict::Incomplete);
    }
}
