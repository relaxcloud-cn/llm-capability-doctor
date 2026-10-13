use crate::model::{CheckId, CheckResult, FailureOrigin, Status, Verdict};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckGroup {
    Interface,
    Agent,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequirementSpec {
    pub id: String,
    pub title: String,
    pub required: bool,
    #[serde(default)]
    pub subject: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckSpec {
    pub id: CheckId,
    pub name: String,
    pub version: String,
    pub scope: String,
    pub group: CheckGroup,
    pub required: bool,
    pub budget_seconds: u64,
    pub repetitions: u8,
    pub requirements: Vec<RequirementSpec>,
    #[serde(default)]
    pub pass_summary: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Satisfied,
    Unsatisfied,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Validity {
    Verified,
    #[default]
    Insufficient,
    Conflicting,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Claim {
    pub requirement: String,
    #[serde(default)]
    pub validity: Validity,
    pub outcome: Outcome,
    pub reason: String,
    pub expected: Value,
    pub observed: Value,
    pub evidence_ids: Vec<String>,
    pub origin: Option<FailureOrigin>,
    pub condition: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Evaluation {
    pub claims: Vec<Claim>,
    pub facts: BTreeMap<String, Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequirementResult {
    pub id: String,
    pub title: String,
    pub required: bool,
    #[serde(default)]
    pub subject: Option<String>,
    pub validity: Validity,
    pub outcome: Outcome,
    pub reason: String,
    pub expected: Value,
    pub observed: Value,
    pub evidence_ids: Vec<String>,
    pub origin: Option<FailureOrigin>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Assessment {
    pub validity: Validity,
    pub status: Status,
    pub requirements: Vec<RequirementResult>,
    pub conditions: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub id: String,
    pub path: String,
    pub kind: String,
    pub sha256: String,
    pub bytes: u64,
}
pub type EvidenceIndex = BTreeMap<String, EvidenceRef>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Decision {
    pub verdict: Verdict,
    pub reason: String,
    pub evidence_ids: Vec<String>,
    pub conditions: Vec<String>,
}

pub fn index_evidence(directory: &Path) -> Result<EvidenceIndex> {
    fn visit(base: &Path, path: &Path, index: &mut EvidenceIndex) -> Result<()> {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path)?;
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                if entry.file_name() != "config" {
                    visit(base, &path, index)?;
                }
                continue;
            }
            let relative = path
                .strip_prefix(base)?
                .to_string_lossy()
                .replace('\\', "/");
            let name = entry.file_name().to_string_lossy().into_owned();
            let http = name.starts_with("http-")
                && name.ends_with(".json")
                && path.parent().is_some_and(|p| p.ends_with("http"));
            let artifact = matches!(
                name.as_str(),
                "rpc.json"
                    | "source-snapshot.json"
                    | "validation.json"
                    | "stage1-result.json"
                    | "task-result.json"
                    | "result.json"
            ) || relative.starts_with("observations/");
            if !http && !artifact {
                continue;
            }
            let id = if http {
                name.trim_end_matches(".json").to_owned()
            } else {
                format!("file:{relative}")
            };
            let data = std::fs::read(&path)?;
            let kind = if http {
                "http"
            } else if name == "rpc.json" {
                "execution_trace"
            } else if name == "source-snapshot.json" {
                "source_snapshot"
            } else {
                "artifact"
            };
            let item = EvidenceRef {
                id: id.clone(),
                path: relative,
                kind: kind.into(),
                sha256: format!("{:x}", Sha256::digest(&data)),
                bytes: data.len() as u64,
            };
            if index.insert(id, item).is_some() {
                bail!("检测证据编号重复");
            }
        }
        Ok(())
    }
    let mut index = EvidenceIndex::new();
    for name in ["http", "agent", "observations"] {
        let path = directory.join(name);
        if path.is_dir() && !std::fs::symlink_metadata(&path)?.file_type().is_symlink() {
            visit(directory, &path, &mut index)?;
        }
    }
    Ok(index)
}

pub fn verify_reference(directory: &Path, reference: &EvidenceRef) -> Result<()> {
    let relative = Path::new(&reference.path);
    if relative.as_os_str().is_empty()
        || !relative
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    {
        bail!("证据路径不安全");
    }
    let mut path = directory.to_path_buf();
    for part in relative.components() {
        path.push(part);
        if std::fs::symlink_metadata(&path)?.file_type().is_symlink() {
            bail!("证据不能引用符号链接");
        }
    }
    let data = std::fs::read(path)?;
    if data.len() as u64 != reference.bytes
        || format!("{:x}", Sha256::digest(&data)) != reference.sha256
    {
        bail!("证据内容与保存时不一致");
    }
    Ok(())
}

pub fn assess(
    spec: &CheckSpec,
    claims: &[Claim],
    index: &EvidenceIndex,
    directory: &Path,
) -> Assessment {
    let mut conditions = BTreeSet::new();
    let requirements = spec
        .requirements
        .iter()
        .map(|rule| {
            let matches: Vec<_> = claims.iter().filter(|c| c.requirement == rule.id).collect();
            let mut result = RequirementResult {
                id: rule.id.clone(),
                title: rule.title.clone(),
                required: rule.required,
                subject: rule.subject.clone(),
                validity: Validity::Insufficient,
                outcome: Outcome::Unknown,
                reason: "尚未取得该要求的完整证据".into(),
                expected: Value::Null,
                observed: Value::Null,
                evidence_ids: vec![],
                origin: Some(FailureOrigin::Detector),
            };
            if matches.len() > 1 {
                result.validity = Validity::Conflicting;
                result.reason = "同一要求存在重复裁决，不能确认结论".into();
                return result;
            }
            let Some(claim) = matches.first() else {
                return result;
            };
            result.expected = claim.expected.clone();
            result.observed = claim.observed.clone();
            result.evidence_ids = claim.evidence_ids.clone();
            result.origin = claim.origin;
            let proof = !claim.evidence_ids.is_empty()
                && claim.evidence_ids.iter().all(|id| {
                    index
                        .get(id)
                        .is_some_and(|r| verify_reference(directory, r).is_ok())
                });
            if !proof {
                result.reason = "裁决引用的证据缺失或校验不一致，结论未确认".into();
                result.origin = Some(FailureOrigin::Detector);
                return result;
            }
            result.reason = claim.reason.clone();
            let blocked = matches!(
                claim.origin,
                Some(
                    FailureOrigin::Detector | FailureOrigin::Runtime | FailureOrigin::Configuration
                )
            );
            result.validity = claim.validity;
            if claim.validity != Validity::Verified || blocked {
                if result.validity != Validity::Conflicting {
                    result.validity = Validity::Insufficient;
                }
                return result;
            }
            result.outcome = claim.outcome;
            if let Some(condition) = &claim.condition {
                conditions.insert(condition.clone());
            }
            result
        })
        .collect::<Vec<_>>();
    let required: Vec<_> = requirements.iter().filter(|r| r.required).collect();
    let status = if required
        .iter()
        .any(|r| r.validity == Validity::Verified && r.outcome == Outcome::Unsatisfied)
    {
        Status::Failed
    } else if !required.is_empty()
        && required
            .iter()
            .all(|r| r.validity == Validity::Verified && r.outcome == Outcome::Satisfied)
    {
        Status::Passed
    } else {
        Status::Incomplete
    };
    let validity = if required.iter().any(|r| r.validity == Validity::Conflicting) {
        Validity::Conflicting
    } else if required.iter().all(|r| r.validity == Validity::Verified) && !required.is_empty() {
        Validity::Verified
    } else {
        Validity::Insufficient
    };
    Assessment {
        validity,
        status,
        requirements,
        conditions: conditions.into_iter().collect(),
    }
}

pub fn aggregate(plan: &[CheckSpec], checks: &[CheckResult]) -> Decision {
    let mut evidence = BTreeSet::new();
    let mut conditions = BTreeSet::new();
    let mut failed = vec![];
    let mut pending = vec![];
    let mut seen = BTreeSet::new();
    for spec in plan.iter().filter(|s| s.required) {
        if !seen.insert(spec.id) {
            pending.push(format!("{}：计划重复", spec.name));
            continue;
        }
        let matches: Vec<_> = checks.iter().filter(|c| c.id == spec.id).collect();
        if matches.len() != 1 {
            pending.push(format!("{}：缺少唯一检测结果", spec.name));
            continue;
        }
        let check = matches[0];
        let Some(assessment) = &check.assessment else {
            pending.push(format!("{}：尚未完成证据裁决", spec.name));
            continue;
        };
        let mut all = true;
        for required in spec.requirements.iter().filter(|r| r.required) {
            let matches: Vec<_> = assessment
                .requirements
                .iter()
                .filter(|r| r.id == required.id)
                .collect();
            let Some(result) = matches.first().filter(|_| matches.len() == 1) else {
                all = false;
                continue;
            };
            if result.validity != Validity::Verified
                || result.evidence_ids.is_empty()
                || matches!(
                    result.origin,
                    Some(
                        FailureOrigin::Detector
                            | FailureOrigin::Runtime
                            | FailureOrigin::Configuration
                    )
                )
            {
                all = false;
                continue;
            }
            evidence.extend(result.evidence_ids.iter().cloned());
            if result.outcome == Outcome::Unsatisfied {
                failed.push(format!("{}：{}", spec.name, result.reason));
            }
            if result.outcome != Outcome::Satisfied {
                all = false;
            }
        }
        if !all || !check.finished || !spec.requirements.iter().any(|r| r.required) {
            pending.push(spec.name.clone());
        }
        conditions.extend(assessment.conditions.iter().cloned());
    }
    let (verdict, reason) = if let Some(reason) = failed.first() {
        (Verdict::Failed, reason.clone())
    } else if seen.is_empty() || !pending.is_empty() {
        (
            Verdict::Incomplete,
            format!("尚未确认全部必需要求：{}", pending.join("、")),
        )
    } else {
        (
            Verdict::Passed,
            "本次选定的必需要求均有经核验的满足证据".into(),
        )
    };
    Decision {
        verdict,
        reason,
        evidence_ids: evidence.into_iter().collect(),
        conditions: conditions.into_iter().collect(),
    }
}

pub fn seal(
    spec: &CheckSpec,
    check: &mut CheckResult,
    claims: Vec<Claim>,
    directory: &Path,
    index: &EvidenceIndex,
) -> Result<()> {
    if spec.id != check.id {
        bail!("执行结果与注册的检测项不匹配");
    }
    let assessment = assess(spec, &claims, index, directory);
    check.title = if assessment.status == Status::Passed {
        spec.pass_summary.clone().unwrap_or_else(|| {
            assessment
                .requirements
                .iter()
                .filter(|r| r.required)
                .map(|r| r.reason.as_str())
                .collect::<Vec<_>>()
                .join("；")
        })
    } else {
        assessment
            .requirements
            .iter()
            .find(|r| {
                r.required && r.validity == Validity::Verified && r.outcome == Outcome::Unsatisfied
            })
            .or_else(|| {
                assessment.requirements.iter().find(|r| {
                    r.required
                        && (r.outcome != Outcome::Satisfied || r.validity != Validity::Verified)
                })
            })
            .map(|r| r.reason.clone())
            .context("裁决状态与要求不一致")?
    };
    check.status = assessment.status;
    let mut seen = BTreeSet::new();
    check.evidence_ids = assessment
        .requirements
        .iter()
        .flat_map(|r| r.evidence_ids.iter().cloned())
        .filter(|id| seen.insert(id.clone()))
        .collect();
    for task in &mut check.tasks {
        let requirements: Vec<_> = assessment
            .requirements
            .iter()
            .filter(|r| r.required && r.subject.as_deref() == Some(task.id.as_str()))
            .collect();
        task.status = if requirements
            .iter()
            .any(|r| r.validity == Validity::Verified && r.outcome == Outcome::Unsatisfied)
        {
            Status::Failed
        } else if !requirements.is_empty()
            && requirements
                .iter()
                .all(|r| r.validity == Validity::Verified && r.outcome == Outcome::Satisfied)
        {
            Status::Passed
        } else {
            Status::Incomplete
        };
        task.title = requirements
            .iter()
            .find(|r| r.outcome == Outcome::Unsatisfied && r.validity == Validity::Verified)
            .or_else(|| {
                requirements
                    .iter()
                    .find(|r| r.outcome != Outcome::Satisfied)
            })
            .or_else(|| requirements.first())
            .map(|r| r.reason.clone())
            .unwrap_or_else(|| "尚未取得本任务的独立裁决".into());
        task.evidence_ids = requirements
            .iter()
            .flat_map(|r| r.evidence_ids.iter().cloned())
            .collect();
        task.findings=requirements.iter().filter(|r|r.outcome!=Outcome::Satisfied).map(|r|crate::model::Finding {
            code:"task_judgment".into(),title:r.reason.clone(),fact:serde_json::json!({"expected":r.expected,"observed":r.observed,"validity":r.validity}).to_string(),
            impact:"本任务的规定要求尚未全部确认满足".into(),suggestion:"依据本次证据核对具体原因".into(),origin:r.origin.unwrap_or(FailureOrigin::Detector),evidence_ids:r.evidence_ids.clone(),
        }).collect();
        let verified_flag = |key: &str| {
            requirements
                .iter()
                .any(|r| r.validity == Validity::Verified && r.observed["execution"][key] == true)
        };
        task.normal_end = verified_flag("normal_end");
        task.fault_delivered = verified_flag("fault_delivered");
        task.recovery_observed = verified_flag("recovery_observed");
    }
    check.assessment = Some(assessment);
    Ok(())
}
