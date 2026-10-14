use agent_check::{
    assessment,
    config::Protocol,
    engine::atomic_json,
    judgment::{self, Evaluation, Outcome},
    model::{CheckId, CheckResult, Status},
    registry::builtin_specs,
    transport::{HttpAttempt, HttpEvidence, RequestFailure},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

// These fixtures deliberately do not use the probe or its request/wave classifications.
struct Experiment {
    directory: tempfile::TempDir,
    check: CheckResult,
    protocol: Protocol,
    next_start: u64,
}
impl Experiment {
    fn new(protocol: Protocol) -> Self {
        let mut check = CheckResult::pending(CheckId::Performance);
        check.finished = true;
        check.facts = BTreeMap::from([
            ("search_method".into(), json!("doubling_network_recheck")),
            ("rounds".into(), json!([])),
            ("stop_reason".into(), json!("first_unsuccessful_wave")),
            ("recommended_concurrency".into(), json!(999)),
            ("maximum_confirmed".into(), json!(true)),
            ("maximum_concurrency".into(), json!(999)),
        ]);
        Self {
            directory: tempfile::tempdir().unwrap(),
            check,
            protocol,
            next_start: 0,
        }
    }

    fn wave(&mut self, phase: &str, outcomes: &[&str]) {
        let start = self.next_start + if phase == "recheck" { 1000 } else { 0 };
        let mut records = vec![];
        let mut elapsed = 0;
        for outcome in outcomes {
            let id = format!("http-{}", self.check.evidence_ids.len());
            let text = if *outcome == "empty" { "" } else { "你好！" };
            let finish = if *outcome == "truncated" {
                "length"
            } else {
                "stop"
            };
            let wire = match self.protocol {
                Protocol::ChatCompletions => json!({"choices":[{"index":0,"message":{
                    "role":"assistant","content":text,
                    "refusal":if *outcome=="refused" {json!("拒绝")} else {Value::Null},
                    "tool_calls":if *outcome=="tool" {json!([{"id":"call-one","type":"function",
                        "function":{"name":"tool","arguments":"{}"}}])} else {json!([])}
                },"finish_reason":finish}]}),
                Protocol::Responses => {
                    json!({"status":if *outcome=="truncated" {"incomplete"} else {"completed"},
                    "output":if *outcome=="tool" {
                        json!([{"type":"function_call","id":"fc-one","call_id":"call-one",
                            "name":"tool","arguments":"{}","status":"completed"}])
                    } else {
                        json!([{"type":"message","id":"message-one","role":"assistant",
                            "status":"completed","content":[if *outcome=="refused" {
                                json!({"type":"refusal","refusal":"拒绝"})
                            } else {json!({"type":"output_text","text":text})}]}])
                    }})
                }
            };
            let failure = match *outcome {
                "network" | "contradictory_network" => Some(RequestFailure::Network),
                "timeout" => Some(RequestFailure::Timeout),
                "cancelled" | "unsent_cancelled" => Some(RequestFailure::Cancelled),
                "stage_budget" | "unsent_stage_budget" => Some(RequestFailure::StageBudget),
                "429" => Some(RequestFailure::Http(429)),
                "401" => Some(RequestFailure::Http(401)),
                "500" => Some(RequestFailure::Http(500)),
                "conflict" => Some(RequestFailure::EvidenceConflict("conflict".into())),
                "malformed" => Some(RequestFailure::InvalidResponse("invalid JSON".into())),
                _ => None,
            };
            let status = match *outcome {
                "network" | "timeout" | "cancelled" | "stage_budget" => None,
                "429" => Some(429),
                "401" => Some(401),
                "500" => Some(500),
                _ => Some(200),
            };
            let duration = if *outcome == "slow" { 20_001 } else { 10 };
            elapsed = elapsed.max(duration);
            let request = match self.protocol {
                Protocol::ChatCompletions => json!({"model":"fixture","messages":[
                    {"role":"user","content":"请用一句简短的中文打个招呼。"}],
                    "max_tokens":2048,"stream":false}),
                Protocol::Responses => json!({"model":"fixture","input":[
                    {"role":"user","content":"请用一句简短的中文打个招呼。"}],
                    "max_output_tokens":2048,"stream":false}),
            };
            let attempts = if outcome.starts_with("unsent_") {
                vec![]
            } else {
                vec![HttpAttempt {
                    number: 1,
                    elapsed_ms: duration,
                    http_status: status,
                    headers: BTreeMap::new(),
                    response_body: if status == Some(200) {
                        if *outcome == "malformed" {
                            "{".into()
                        } else {
                            wire.to_string()
                        }
                    } else {
                        String::new()
                    },
                    error: status.is_none().then(|| "transport stopped".into()),
                }]
            };
            let row = HttpEvidence {
                id: id.clone(),
                endpoint: format!("http://fixture/v1{}", self.protocol.suffix()),
                protocol: self.protocol,
                request,
                stream: false,
                elapsed_ms: duration,
                attempts,
                parse_error: None,
                failure,
                request_timeout_seconds: Some(180),
            };
            atomic_json(
                &self
                    .directory
                    .path()
                    .join("http")
                    .join(format!("{id}.json")),
                &row,
            )
            .unwrap();
            self.check.evidence_ids.push(id.clone());
            // False success/reason/aggregate labels must not influence raw-evidence decisions.
            records.push(json!({"evidence_id":id,"success":true,"reason":"passed",
                "elapsed_ms":0,"dispatch_offset_ms":0,"attempts":1}));
        }
        self.rounds()
            .push(json!({"phase":phase,"concurrency":outcomes.len(),
            "started_offset_ms":start,"elapsed_ms":elapsed,
            "successful":outcomes.len(),"failed":0,"performance_passed":true,
            "latency_passed":true,"failure_reason":"passed","requests":records}));
        self.next_start = start + elapsed;
        self.check.elapsed_ms = self.next_start;
    }

    fn rounds(&mut self) -> &mut Vec<Value> {
        self.check
            .facts
            .get_mut("rounds")
            .unwrap()
            .as_array_mut()
            .unwrap()
    }

    fn evaluate(&self) -> anyhow::Result<Evaluation> {
        atomic_json(
            &self.directory.path().join("observations/performance.json"),
            &self.check,
        )
        .unwrap();
        let index = judgment::index_evidence(self.directory.path()).unwrap();
        let spec = builtin_specs()
            .into_iter()
            .find(|spec| spec.id == CheckId::Performance)
            .unwrap();
        assessment::builtin_evaluation(&spec, &self.check, self.directory.path(), &index)
    }

    fn status(&self, evaluation: &Evaluation) -> Status {
        let index = judgment::index_evidence(self.directory.path()).unwrap();
        let spec = builtin_specs()
            .into_iter()
            .find(|spec| spec.id == CheckId::Performance)
            .unwrap();
        judgment::assess(&spec, &evaluation.claims, &index, self.directory.path()).status
    }

    fn lower_two(protocol: Protocol) -> Self {
        let mut experiment = Self::new(protocol);
        experiment.wave("expand", &["ok"]);
        experiment.wave("expand", &["ok", "ok"]);
        experiment
    }
}

#[test]
fn recovered_wave_keeps_original_errors_and_later_capacity_stop() {
    for protocol in [Protocol::ChatCompletions, Protocol::Responses] {
        let mut experiment = Experiment::lower_two(protocol);
        experiment.wave("expand", &["ok", "network", "ok", "ok"]);
        experiment.wave("recheck", &["ok"; 4]);
        experiment.wave("expand", &["429"; 8]);
        let evaluation = experiment.evaluate().unwrap();
        assert_eq!(experiment.status(&evaluation), Status::Passed);
        let facts = &evaluation.facts;
        assert_eq!(facts["recommended_concurrency"], 4);
        assert_eq!(facts["recommendation_basis"], "capacity_stop");
        assert_eq!(facts["estimate_complete"], true);
        assert_eq!(facts["first_unsuccessful_concurrency"], 8);
        assert_eq!(facts["first_unsuccessful_reason"], "request_failure");
        assert_eq!(facts["maximum_confirmed"], false);
        assert!(facts["maximum_concurrency"].is_null());
        assert_eq!(facts["request_count"], 19);
        assert_eq!(facts["network_rechecks"], 1);
        assert_eq!(facts["connection_errors"], 1);
        assert_eq!(facts["recovered_levels"], json!([4]));
        assert_eq!(facts["connection_stability"], "recovered");
        assert_eq!(facts["probe_elapsed_ms"], experiment.check.elapsed_ms);
        assert_eq!(facts["rounds"][2]["requests"][1]["reason"], "network");
        assert_eq!(facts["rounds"][2]["requests"][1]["elapsed_ms"], 10);
        assert_eq!(facts["rounds"][3]["phase"], "recheck");
        let recommendation = &evaluation.claims[0];
        assert!(
            recommendation
                .condition
                .as_ref()
                .unwrap()
                .contains("网络错误")
        );
        let completion = &evaluation.claims[1];
        assert_eq!(completion.expected, 19);
        assert_eq!(completion.observed, 10);
        assert_eq!(completion.outcome, Outcome::Unsatisfied);
    }
}

#[test]
fn repeated_network_errors_keep_only_conditional_trial() {
    for protocol in [Protocol::ChatCompletions, Protocol::Responses] {
        let mut experiment = Experiment::lower_two(protocol);
        experiment.wave("expand", &["network"; 4]);
        experiment.wave("recheck", &["network", "ok", "ok", "ok"]);
        let evaluation = experiment.evaluate().unwrap();
        assert_eq!(experiment.status(&evaluation), Status::Passed);
        assert_eq!(evaluation.facts["recommended_concurrency"], 2);
        assert_eq!(evaluation.facts["recommendation_basis"], "verified_trial");
        assert_eq!(evaluation.facts["estimate_complete"], false);
        assert_eq!(evaluation.facts["connection_stability"], "unresolved");
        assert_eq!(evaluation.facts["connection_errors"], 5);
        assert_eq!(evaluation.facts["request_count"], 11);
        assert_eq!(evaluation.facts["recovered_levels"], json!([]));
        assert_eq!(evaluation.facts["first_unsuccessful_concurrency"], 4);
        assert!(evaluation.claims[0].reason.contains("试跑起点"));
        assert!(
            evaluation.claims[0]
                .condition
                .as_ref()
                .unwrap()
                .contains("容量边界未知")
        );
        assert_eq!(evaluation.claims[1].observed, 6);
    }
}

#[test]
fn content_errors_and_mixed_failures_never_allow_recheck_or_recommendation() {
    for protocol in [Protocol::ChatCompletions, Protocol::Responses] {
        for failure in [
            "empty",
            "truncated",
            "refused",
            "tool",
            "malformed",
            "401",
            "500",
            "conflict",
            "cancelled",
            "stage_budget",
            "contradictory_network",
        ] {
            let mut experiment = Experiment::lower_two(protocol);
            experiment.wave("expand", &["ok", failure, "ok", "ok"]);
            let evaluation = experiment.evaluate().unwrap();
            assert_eq!(
                experiment.status(&evaluation),
                Status::Incomplete,
                "{failure}"
            );
            assert!(
                evaluation.facts["recommended_concurrency"].is_null(),
                "{failure}"
            );
            assert_eq!(evaluation.facts["estimate_complete"], false, "{failure}");
            experiment.wave("recheck", &["ok"; 4]);
            assert!(experiment.evaluate().is_err(), "retried {failure}");
        }
        for failure in ["429", "timeout", "empty", "cancelled", "500"] {
            let mut experiment = Experiment::lower_two(protocol);
            experiment.wave("expand", &["network", failure, "ok", "ok"]);
            let evaluation = experiment.evaluate().unwrap();
            assert!(evaluation.facts["recommended_concurrency"].is_null());
            assert_eq!(evaluation.facts["connection_stability"], "unresolved");
            experiment.wave("recheck", &["ok"; 4]);
            assert!(experiment.evaluate().is_err(), "mixed retry {failure}");
        }
    }
}

#[test]
fn duplicate_third_skipped_and_nonnetwork_retry_sequences_are_rejected() {
    for protocol in [Protocol::ChatCompletions, Protocol::Responses] {
        let mut duplicate = Experiment::lower_two(protocol);
        let id = duplicate.rounds()[0]["requests"][0]["evidence_id"].clone();
        duplicate.rounds()[1]["requests"][0]["evidence_id"] = id;
        assert!(
            duplicate
                .evaluate()
                .unwrap_err()
                .to_string()
                .contains("重复")
        );

        let mut third = Experiment::lower_two(protocol);
        third.wave("expand", &["network"; 4]);
        third.wave("recheck", &["network"; 4]);
        third.wave("recheck", &["ok"; 4]);
        assert!(third.evaluate().is_err());

        let mut skipped = Experiment::lower_two(protocol);
        skipped.wave("expand", &["network"; 4]);
        skipped.wave("expand", &["ok"; 8]);
        assert!(skipped.evaluate().is_err());

        let mut relabeled = Experiment::lower_two(protocol);
        relabeled.wave("expand", &["network"; 4]);
        relabeled.wave("expand", &["ok"; 4]);
        assert!(relabeled.evaluate().is_err());

        for failure in ["ok", "429", "timeout", "slow"] {
            let mut experiment = Experiment::lower_two(protocol);
            experiment.wave("expand", &[failure; 4]);
            experiment.wave("recheck", &["ok"; 4]);
            assert!(
                experiment.evaluate().is_err(),
                "unexpected retry of {failure}"
            );
        }
    }
}

#[test]
fn recheck_requires_completed_prior_wave_full_delay_and_single_attempts() {
    for corruption in [
        "delay", "overlap", "dispatch", "elapsed", "total", "attempts", "number",
    ] {
        let mut experiment = Experiment::lower_two(Protocol::ChatCompletions);
        experiment.wave("expand", &["network"; 4]);
        experiment.wave("recheck", &["ok"; 4]);
        match corruption {
            "delay" => experiment.rounds()[3]["started_offset_ms"] = json!(1029),
            "overlap" => experiment.rounds()[2]["started_offset_ms"] = json!(19),
            "dispatch" => experiment.rounds()[3]["requests"][0]["dispatch_offset_ms"] = json!(11),
            "elapsed" => experiment.rounds()[3]["elapsed_ms"] = json!(8),
            "total" => experiment.check.elapsed_ms = 1000,
            "attempts" | "number" => {
                let path = experiment.directory.path().join("http/http-7.json");
                let mut row: HttpEvidence =
                    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                if corruption == "attempts" {
                    row.attempts.push(row.attempts[0].clone());
                } else {
                    row.attempts[0].number = 2;
                }
                atomic_json(&path, &row).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            experiment.evaluate().is_err(),
            "accepted forged {corruption}"
        );
    }
}

#[test]
fn guard_stops_are_not_capacity_but_unlaunched_network_recheck_keeps_trial() {
    for stop in [
        "request_budget",
        "round_budget",
        "stage_budget",
        "local_resource_limit",
        "cancelled",
        "configuration",
    ] {
        let mut experiment = Experiment::lower_two(Protocol::ChatCompletions);
        experiment
            .check
            .facts
            .insert("stop_reason".into(), json!(stop));
        let evaluation = experiment.evaluate().unwrap();
        assert_eq!(experiment.status(&evaluation), Status::Incomplete);
        assert!(evaluation.facts["recommended_concurrency"].is_null());
        assert_eq!(evaluation.facts["estimate_complete"], false);

        experiment.wave("expand", &["network"; 4]);
        let evaluation = experiment.evaluate().unwrap();
        assert_eq!(evaluation.facts["estimate_complete"], false);
        assert_eq!(evaluation.facts["network_rechecks"], 0);
        assert_eq!(evaluation.facts["connection_errors"], 4);
        assert_eq!(evaluation.facts["request_count"], 7);
        if matches!(stop, "cancelled" | "configuration") {
            assert!(evaluation.facts["recommended_concurrency"].is_null());
        } else {
            assert_eq!(evaluation.facts["recommended_concurrency"], 2);
            assert_eq!(evaluation.facts["recommendation_basis"], "verified_trial");
            assert_eq!(experiment.status(&evaluation), Status::Passed);
        }
    }
    for guard in [
        "cancelled",
        "stage_budget",
        "unsent_cancelled",
        "unsent_stage_budget",
    ] {
        let mut experiment = Experiment::lower_two(Protocol::ChatCompletions);
        experiment.wave("expand", &[guard; 4]);
        let evaluation = experiment.evaluate().unwrap();
        assert_eq!(experiment.status(&evaluation), Status::Incomplete);
        assert_eq!(evaluation.facts["estimate_complete"], false);
        assert!(evaluation.facts["recommended_concurrency"].is_null());
        assert!(evaluation.facts["first_unsuccessful_concurrency"].is_null());
    }
}

#[test]
fn first_level_recovery_doubles_but_zero_or_one_recommendation_is_incomplete() {
    let mut recovered = Experiment::new(Protocol::ChatCompletions);
    recovered.wave("expand", &["network"]);
    recovered.wave("recheck", &["ok"]);
    recovered.wave("expand", &["ok"; 2]);
    recovered.wave("expand", &["429"; 4]);
    let evaluation = recovered.evaluate().unwrap();
    assert_eq!(recovered.status(&evaluation), Status::Passed);
    assert_eq!(evaluation.facts["recovered_levels"], json!([1]));
    assert_eq!(evaluation.facts["first_unsuccessful_concurrency"], 4);
    for lower in [0, 1] {
        let mut experiment = Experiment::new(Protocol::ChatCompletions);
        if lower == 1 {
            experiment.wave("expand", &["ok"]);
        }
        experiment.wave("expand", &vec!["network"; if lower == 1 { 2 } else { 1 }]);
        experiment.wave("recheck", &vec!["network"; if lower == 1 { 2 } else { 1 }]);
        let evaluation = experiment.evaluate().unwrap();
        assert_eq!(experiment.status(&evaluation), Status::Incomplete);
        if lower == 0 {
            assert!(evaluation.facts["recommended_concurrency"].is_null());
        } else {
            assert_eq!(evaluation.facts["recommended_concurrency"], 1);
        }
    }
}

#[test]
fn old_policy_is_not_upgraded_and_old_workload_keeps_meaningful_rejection() {
    for marker in [None, Some("doubling_first_failure")] {
        let mut experiment = Experiment::lower_two(Protocol::ChatCompletions);
        experiment.wave("expand", &["network"; 4]);
        if let Some(marker) = marker {
            experiment
                .check
                .facts
                .insert("search_method".into(), json!(marker));
        } else {
            experiment.check.facts.remove("search_method");
        }
        assert!(
            experiment
                .evaluate()
                .unwrap_err()
                .to_string()
                .contains("协议标记")
        );
        let path = experiment.directory.path().join("http/http-0.json");
        let mut row: HttpEvidence = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        row.request["messages"][0]["content"] = json!("Return exact JSON for the old workload");
        atomic_json(&path, &row).unwrap();
        assert!(
            experiment
                .evaluate()
                .unwrap_err()
                .to_string()
                .contains("旧负载")
        );
    }
}

#[test]
fn unsuccessful_recheck_never_clears_connection_risk() {
    for failure in ["429", "timeout", "slow", "empty", "cancelled"] {
        let mut experiment = Experiment::lower_two(Protocol::ChatCompletions);
        experiment.wave("expand", &["network"; 4]);
        experiment.wave("recheck", &[failure; 4]);
        let evaluation = experiment.evaluate().unwrap();
        assert_eq!(
            evaluation.facts["connection_stability"], "unresolved",
            "{failure}"
        );
        assert_eq!(evaluation.facts["recovered_levels"], json!([]));
        assert_eq!(evaluation.facts["connection_errors"], 4);
        assert!(
            evaluation.claims[0]
                .condition
                .as_ref()
                .unwrap()
                .contains("风险仍未解除")
        );
        if matches!(failure, "429" | "timeout" | "slow") {
            assert_eq!(evaluation.facts["recommendation_basis"], "capacity_stop");
            assert_eq!(evaluation.facts["recommended_concurrency"], 2);
            assert_eq!(evaluation.facts["estimate_complete"], true);
        } else {
            assert!(evaluation.facts["recommended_concurrency"].is_null());
            assert_eq!(evaluation.facts["estimate_complete"], false);
        }
    }
}

#[test]
fn independent_levels_each_get_one_recheck_and_guards_still_prevent_capacity() {
    let mut experiment = Experiment::new(Protocol::ChatCompletions);
    experiment.wave("expand", &["network"]);
    experiment.wave("recheck", &["ok"]);
    experiment.wave("expand", &["network", "ok"]);
    experiment.wave("recheck", &["ok"; 2]);
    experiment.wave("expand", &["429"; 4]);
    let evaluation = experiment.evaluate().unwrap();
    assert_eq!(evaluation.facts["network_rechecks"], 2);
    assert_eq!(evaluation.facts["recovered_levels"], json!([1, 2]));
    assert_eq!(evaluation.facts["connection_stability"], "recovered");
    assert_eq!(evaluation.facts["recommended_concurrency"], 2);
    experiment
        .check
        .facts
        .insert("stop_reason".into(), json!("cancelled"));
    let evaluation = experiment.evaluate().unwrap();
    assert_eq!(experiment.status(&evaluation), Status::Incomplete);
    assert!(evaluation.facts["recommended_concurrency"].is_null());
    assert_eq!(evaluation.facts["estimate_complete"], false);
}
