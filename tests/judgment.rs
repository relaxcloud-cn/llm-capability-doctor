use agent_check::{
    config::{ModelConfig, Protocol, Target},
    engine,
    judgment::{self, CheckGroup, CheckSpec, Claim, Outcome, RequirementSpec, Validity},
    model::{CheckId, CheckResult, FailureOrigin, RunResult, Status, Verdict},
    registry::{CheckContext, Registration, Registry},
};
use futures_util::future::BoxFuture;
use serde_json::json;
use std::path::Path;
use tokio_util::sync::CancellationToken;

fn spec(id: u32) -> CheckSpec {
    CheckSpec {
        id: CheckId::Custom(id),
        name: format!("扩展证据自检 {id}"),
        version: "1".into(),
        scope: "本地证据写入与规则判定自检，不请求模型".into(),
        group: CheckGroup::Interface,
        required: true,
        budget_seconds: 10,
        repetitions: 0,
        requirements: vec![RequirementSpec {
            id: "saved".into(),
            title: "实际保存独立证据".into(),
            required: true,
            subject: None,
        }],
        pass_summary: None,
    }
}
fn claim(outcome: Outcome) -> Claim {
    Claim {
        requirement: "saved".into(),
        validity: Validity::Verified,
        outcome,
        reason: "测试要求".into(),
        expected: json!(true),
        observed: json!(outcome == Outcome::Satisfied),
        evidence_ids: vec!["file:observations/proof.json".into()],
        origin: None,
        condition: None,
    }
}
fn proof(dir: &Path) {
    std::fs::create_dir_all(dir.join("observations")).unwrap();
    std::fs::write(dir.join("observations/proof.json"), "{\"recorded\":true}").unwrap();
}
#[test]
fn unverified_claims_and_detector_errors_cannot_pass_or_fail_the_model() {
    let dir = tempfile::tempdir().unwrap();
    proof(dir.path());
    let index = judgment::index_evidence(dir.path()).unwrap();
    let definition = spec(1);
    for origin in [
        FailureOrigin::Detector,
        FailureOrigin::Runtime,
        FailureOrigin::Configuration,
    ] {
        let mut c = claim(Outcome::Unsatisfied);
        c.origin = Some(origin);
        let assessment = judgment::assess(&definition, &[c], &index, dir.path());
        assert_eq!(assessment.status, Status::Incomplete);
    }
    let mut c = claim(Outcome::Satisfied);
    c.evidence_ids = vec!["missing".into()];
    assert_eq!(
        judgment::assess(&definition, &[c], &index, dir.path()).status,
        Status::Incomplete
    );
    std::fs::write(dir.path().join("observations/proof.json"), "changed").unwrap();
    assert_eq!(
        judgment::assess(
            &definition,
            &[claim(Outcome::Satisfied)],
            &index,
            dir.path()
        )
        .status,
        Status::Incomplete
    );
}
#[test]
fn confirmed_failure_survives_other_unknown_requirements_and_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    proof(dir.path());
    let index = judgment::index_evidence(dir.path()).unwrap();
    let a = spec(1);
    let b = spec(2);
    let mut check = CheckResult::pending(a.id);
    check.finished = true;
    let mut c = claim(Outcome::Unsatisfied);
    c.origin = Some(FailureOrigin::ModelOutput);
    judgment::seal(&a, &mut check, vec![c], dir.path(), &index).unwrap();
    let mut run = RunResult::with_plan(
        Target {
            model: "test".into(),
            endpoint: "http://localhost/v1".into(),
            protocol: Protocol::ChatCompletions,
        },
        vec![a, b],
    );
    run.update(check);
    run.cancelled = true;
    assert_eq!(run.compute_verdict(), Verdict::Failed);
    assert!(!run.decision.unwrap().evidence_ids.is_empty());
}
#[test]
fn duplicate_claims_are_conflicting_and_status_labels_are_not_proof() {
    let dir = tempfile::tempdir().unwrap();
    proof(dir.path());
    let index = judgment::index_evidence(dir.path()).unwrap();
    let s = spec(1);
    let c = claim(Outcome::Satisfied);
    let result = judgment::assess(&s, &[c.clone(), c], &index, dir.path());
    assert_eq!(result.validity, Validity::Conflicting);
    let mut check = CheckResult::pending(s.id);
    check.status = Status::Passed;
    check.finished = true;
    assert_eq!(
        judgment::aggregate(&[s], &[check]).verdict,
        Verdict::Incomplete
    );
}
#[test]
fn optional_compatibility_conditions_are_preserved_and_strict_policy_can_require_them() {
    let dir = tempfile::tempdir().unwrap();
    proof(dir.path());
    let index = judgment::index_evidence(dir.path()).unwrap();
    let mut definition = spec(9);
    definition.requirements.push(RequirementSpec {
        id: "native".into(),
        title: "原生格式".into(),
        required: false,
        subject: None,
    });
    let mut content = claim(Outcome::Satisfied);
    content.condition = Some("需要兼容解析".into());
    let mut native = claim(Outcome::Unsatisfied);
    native.requirement = "native".into();
    native.origin = Some(FailureOrigin::Service);
    let claims = vec![content, native];
    let mut check = CheckResult::pending(definition.id);
    check.finished = true;
    judgment::seal(&definition, &mut check, claims.clone(), dir.path(), &index).unwrap();
    let decision = judgment::aggregate(std::slice::from_ref(&definition), &[check]);
    assert_eq!(decision.verdict, Verdict::Passed);
    assert_eq!(decision.conditions, vec!["需要兼容解析"]);
    definition.requirements[1].required = true;
    assert_eq!(
        judgment::assess(&definition, &claims, &index, dir.path()).status,
        Status::Failed
    );
}

#[test]
fn evidence_paths_cannot_escape_the_run_directory() {
    let dir = tempfile::tempdir().unwrap();
    proof(dir.path());
    let mut index = judgment::index_evidence(dir.path()).unwrap();
    index.values_mut().next().unwrap().path = "../outside.json".into();
    assert_eq!(
        judgment::assess(&spec(1), &[claim(Outcome::Satisfied)], &index, dir.path()).status,
        Status::Incomplete
    );
}

#[test]
fn registry_rejects_duplicate_checks_and_duplicate_requirements() {
    let entry = Registration {
        spec: spec(1),
        execute: Some(execute),
        evaluate,
    };
    assert!(Registry::new(vec![entry.clone(), entry.clone()]).is_err());
    let mut repeated = entry;
    repeated
        .spec
        .requirements
        .push(repeated.spec.requirements[0].clone());
    assert!(Registry::new(vec![repeated]).is_err());
}

#[test]
fn a_failed_first_request_does_not_claim_unexecuted_streaming_also_failed() {
    let dir = tempfile::tempdir().unwrap();
    let definition = agent_check::registry::builtin_specs()
        .into_iter()
        .find(|s| s.id == CheckId::Response)
        .unwrap();
    let mut check = CheckResult::pending(CheckId::Response);
    check.status = Status::Failed;
    check.evidence_ids = vec!["http-first".into()];
    check.findings.push(agent_check::model::Finding {
        code: "request_failed".into(),
        title: "服务失败".into(),
        fact: "HTTP 503".into(),
        impact: String::new(),
        suggestion: String::new(),
        origin: FailureOrigin::Service,
        evidence_ids: check.evidence_ids.clone(),
    });
    // A label alone cannot prove failure; it becomes Unsatisfied only when the
    // preserved HTTP evidence independently shows the 503.
    let index = judgment::index_evidence(dir.path()).unwrap();
    let claims =
        agent_check::assessment::builtin_claims(&definition, &check, dir.path(), &index).unwrap();
    assert_eq!(
        claims
            .iter()
            .find(|c| c.requirement == "nonstream")
            .unwrap()
            .outcome,
        Outcome::Unknown
    );
    std::fs::create_dir_all(dir.path().join("http")).unwrap();
    agent_check::engine::atomic_json(
        &dir.path().join("http/http-first.json"),
        &agent_check::transport::HttpEvidence {
            id: "http-first".into(),
            endpoint: "http://fixture/v1/chat/completions".into(),
            protocol: Protocol::ChatCompletions,
            request: json!({"model":"fixture","stream":false}),
            stream: false,
            elapsed_ms: 1,
            attempts: vec![agent_check::transport::HttpAttempt {
                number: 1,
                elapsed_ms: 1,
                http_status: Some(503),
                headers: Default::default(),
                response_body: "{\"error\":{\"message\":\"unavailable\"}}".into(),
                error: None,
            }],
            parse_error: None,
            failure: None,
        },
    )
    .unwrap();
    let index = judgment::index_evidence(dir.path()).unwrap();
    let claims =
        agent_check::assessment::builtin_claims(&definition, &check, dir.path(), &index).unwrap();
    assert_eq!(
        claims
            .iter()
            .find(|c| c.requirement == "nonstream")
            .unwrap()
            .outcome,
        Outcome::Unsatisfied
    );
    assert_eq!(
        claims
            .iter()
            .find(|c| c.requirement == "stream")
            .unwrap()
            .outcome,
        Outcome::Unknown
    );
}

fn execute(ctx: CheckContext<'_>) -> BoxFuture<'_, anyhow::Result<CheckResult>> {
    Box::pin(async move {
        proof(ctx.directory);
        let mut check = CheckResult::pending(ctx.spec.id);
        check.finished = true;
        check.status = Status::Passed;
        check.title = "扩展项已经保存证据".into();
        Ok(check)
    })
}
fn evaluate(
    _: &CheckSpec,
    _: &CheckResult,
    dir: &Path,
    _: &judgment::EvidenceIndex,
) -> anyhow::Result<judgment::Evaluation> {
    let actual: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("observations/proof.json"))?)?;
    assert_eq!(actual["recorded"], true);
    Ok(judgment::Evaluation {
        claims: vec![claim(Outcome::Satisfied)],
        facts: Default::default(),
    })
}
#[tokio::test]
async fn registered_extensions_run_through_shared_engine_and_report_without_core_changes() {
    let entries = (1001..=1009)
        .map(|id| Registration {
            spec: spec(id),
            execute: Some(execute),
            evaluate,
        })
        .collect();
    let registry = Registry::new(entries).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let config = ModelConfig::new(
        "http://127.0.0.1:1/v1",
        "unused",
        "not-a-real-key".into(),
        Some(Protocol::ChatCompletions),
    )
    .unwrap();
    let mut output =
        engine::run_with_registry(config, dir.path(), CancellationToken::new(), None, registry)
            .await
            .unwrap();
    agent_check::report::write(&mut output.run, &output.directory).unwrap();
    assert_eq!(output.run.checks.len(), 9);
    assert_eq!(output.run.verdict, Verdict::Passed);
    assert_eq!(output.run.completed_count(), 9);
    let report = std::fs::read_to_string(output.directory.join("report.html")).unwrap();
    assert!(report.contains("扩展证据自检 1009"));
    assert!(report.contains("file:observations/proof.json"));
    assert!(
        output
            .run
            .checks
            .iter()
            .all(|c| c.assessment.as_ref().unwrap().status == Status::Passed)
    );
    for evidence in output.run.evidence.values() {
        judgment::verify_reference(&output.directory, evidence).unwrap();
    }
}
