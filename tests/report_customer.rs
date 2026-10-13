use agent_check::{
    config::{Protocol, Target},
    model::{CheckId, FailureOrigin, Finding, RunResult, Status, Verdict},
    report,
};
use serde_json::json;

fn run() -> RunResult {
    let mut run = RunResult::new(Target {
        model: "customer-model".into(),
        endpoint: "https://example.org/v1/chat/completions".into(),
        protocol: Protocol::ChatCompletions,
    });
    run.id = "AC-fixture".into();
    run.elapsed_ms = 275_028;
    run.finished_at = Some(run.started_at);
    for check in &mut run.checks {
        check.finished = true;
        check.status = Status::Passed;
        check.title = format!("{}的本次样本已完成", check.id.name());
    }
    run.verdict = Verdict::Passed;
    run
}

#[test]
fn customer_report_is_static_three_chapters_without_demo_or_fake_reference() {
    let run = run();
    let page = report::render(&run, &[]).unwrap();
    for heading in ["检测结论", "检测结果与基线对照", "使用建议"] {
        assert!(page.contains(heading));
    }
    assert_eq!(page.matches("<section ").count(), 3);
    assert_eq!(page.matches("data-check=").count(), run.checks.len());
    assert!(page.contains("<tbody id=\"check-rows\">"));
    assert!(page.contains("data-baseline=\"missing\""));
    assert!(page.contains("尚无参考实测"));
    assert!(page.contains("id=\"record-appendix\""));
    assert!(!page.contains("id=\"record-appendix\" open"));
    assert!(page.contains("Chat Completions"));
    assert!(page.contains("<th scope=\"row\">上下文容量"));
    assert!(page.contains("<th scope=\"row\">并发能力"));
    assert!(page.contains("AC-fixture"));
    assert!(page.contains("4 分 35 秒"));
    for absent in [
        "设计预览",
        "示意",
        "示例报告",
        "OpenAI",
        "gpt-6-astra",
        "baseline-mode",
        "差异怎么理解",
        "beforeprint=",
        "el.open=true);});",
    ] {
        assert!(!page.contains(absent), "unexpected content: {absent}");
    }
    assert_eq!(page.matches("data-status=\"passed\"").count(), 8);
    assert!(!page.contains("建议 64"));
    assert!(!page.contains("256K"));
}

#[test]
fn failed_incomplete_and_unexecuted_checks_keep_distinct_statuses() {
    let mut run = run();
    run.verdict = Verdict::Failed;
    run.cancelled = true;
    run.checks[0].status = Status::Failed;
    run.checks[0].title = "明确的接口失败".into();
    run.checks[1].status = Status::Incomplete;
    run.checks[1].title = "证据不足，未确认".into();
    run.checks[2].status = Status::NotExecuted;
    run.checks[2].title = "尚未执行".into();
    run.checks[2].finished = false;
    let page = report::render(&run, &[]).unwrap();
    assert!(page.contains("暂不建议接入平台"));
    assert_eq!(page.matches("data-status=\"passed\"").count(), 5);
    assert_eq!(page.matches("data-status=\"failed\"").count(), 1);
    assert_eq!(page.matches("data-status=\"incomplete\"").count(), 1);
    assert_eq!(page.matches("data-status=\"not_executed\"").count(), 1);
    assert!(page.contains("明确的接口失败"));
    assert!(page.contains("证据不足，未确认"));
    assert!(page.contains("未执行"));
    assert!(page.contains("提前停止"));
    assert!(!page.contains("未发现阻止本次接入的问题"));
    assert!(!page.contains("不影响通过"));
}

#[test]
fn guidance_uses_actual_state_and_keeps_capacity_boundaries() {
    let mut run = run();
    let check = run
        .checks
        .iter_mut()
        .find(|c| c.id == CheckId::Performance)
        .unwrap();
    check
        .facts
        .insert("probe_mode".into(), json!("concurrent_requests"));
    check
        .facts
        .insert("workload_type".into(), json!("short_text"));
    check
        .facts
        .insert("recommended_concurrency".into(), json!(16));
    check
        .facts
        .insert("first_unsuccessful_concurrency".into(), json!(32));
    check
        .facts
        .insert("max_mean_elapsed_ms".into(), json!(10_000));
    check
        .facts
        .insert("max_tail_elapsed_ms".into(), json!(20_000));
    check
        .facts
        .insert("request_timeout_seconds".into(), json!(180));
    check.facts.insert("rounds".into(), json!([
        {"concurrency":16,"successful":16,"failed":0,"mean_elapsed_ms":2000,"longest_elapsed_ms":4000,"tail_metric":"maximum","performance_passed":true,"failure_reason":"passed"},
        {"concurrency":32,"successful":32,"failed":0,"mean_elapsed_ms":12000,"p95_elapsed_ms":21000,"tail_metric":"p95","performance_passed":false,"failure_reason":"mean_latency"}
    ]));
    check.findings.push(Finding {
        code: "concurrency_estimate".into(),
        title: "本次建议 16".into(),
        fact: "32 并发全部返回，但耗时超标".into(),
        impact: "仅为本次负载下的建议".into(),
        suggestion: "按建议档位试用".into(),
        origin: FailureOrigin::Service,
        evidence_ids: vec![],
    });
    let page = report::render(&run, &[]).unwrap();
    assert!(page.contains("建议从 16 并发档位试用"));
    assert!(page.contains("data-advice=\"true\""));
    assert!(page.contains("不是精确极限"));
    assert!(page.contains("最慢 4.00 秒"));
    assert!(page.contains("P95 21.00 秒"));
    assert!(page.contains("1 条使用建议"));
    assert!(!page.contains("需整改的问题"));
    assert!(!page.contains("94%"));
}

#[test]
fn report_escapes_untrusted_content_and_never_reinterprets_placeholder_text() {
    let mut run = run();
    run.target.model = "</script><img src=x onerror=alert(1)>{{ROWS}}".into();
    run.checks[0].title = "<script>bad()</script> & \"text\" {{MODEL}}".into();
    run.checks[0].status = Status::Failed;
    run.verdict = Verdict::Failed;
    let before = serde_json::to_value(&run).unwrap();
    let page = report::render(&run, &[]).unwrap();
    assert!(!page.contains("<img src=x onerror=alert(1)>"));
    assert!(!page.contains("<script>bad()</script>"));
    assert!(page.contains("&lt;script&gt;bad()&lt;/script&gt;"));
    assert!(page.contains("{{ROWS}}"));
    assert!(page.contains("{{MODEL}}"));
    assert_eq!(before, serde_json::to_value(&run).unwrap());
}

#[test]
fn approved_layout_styles_are_used_verbatim() {
    let reference = include_str!("../docs/report-reference/agentcheck-customer-report.html");
    let styles = reference
        .split_once("<style>")
        .unwrap()
        .1
        .split_once("</style>")
        .unwrap()
        .0;
    let page = report::render(&run(), &[]).unwrap();
    assert!(page.contains(styles));
    assert!(page.contains("@media print"));
    assert!(!page.contains("document.querySelectorAll('main details').forEach"));
}

#[test]
fn missing_old_producer_version_is_not_replaced_with_the_renderer_version() {
    let mut value = serde_json::to_value(run()).unwrap();
    value.as_object_mut().unwrap().remove("app_version");
    let historical: RunResult = serde_json::from_value(value).unwrap();
    assert_eq!(historical.app_version, None);
    let page = report::render(&historical, &[]).unwrap();
    assert!(page.contains("检测版本 <strong>未记录</strong>"));
    assert!(page.contains("报告生成器版本"));
    assert!(page.contains("历史记录未保存"));
}

#[test]
fn no_results_or_missing_measurements_do_not_become_successes_or_zeroes() {
    let mut run = run();
    run.checks.clear();
    run.plan.clear();
    run.verdict = Verdict::Incomplete;
    let page = report::render(&run, &[]).unwrap();
    assert!(page.contains("本次没有检测项目"));
    assert_eq!(page.matches("data-check=").count(), 0);
    assert!(!page.contains("本次接入检测通过"));
    let mut check = agent_check::model::CheckResult::pending(CheckId::Performance);
    check.facts.insert(
        "rounds".into(),
        json!([{"concurrency":4,"successful":2,"failure_reason":"network"}]),
    );
    run.checks.push(check);
    let page = report::render(&run, &[]).unwrap();
    assert!(page.contains("2 / 4"));
    assert!(page.contains("未记录"));
    assert!(!page.contains("0.00 秒"));
    assert!(!page.contains("平均 ≤ 10 秒"));
}

#[test]
fn unsafe_evidence_links_are_not_rendered_as_navigable_paths() {
    let mut run = run();
    let check = &mut run.checks[0];
    check.evidence_ids.push("unsafe".into());
    run.evidence.insert(
        "unsafe".into(),
        agent_check::judgment::EvidenceRef {
            id: "unsafe".into(),
            path: "../private.json".into(),
            kind: "http".into(),
            sha256: "test".into(),
            bytes: 1,
        },
    );
    let page = report::render(&run, &[]).unwrap();
    assert!(!page.contains("href=\"./../private.json\""));
    assert!(page.contains("unsafe"));
    run.evidence.get_mut("unsafe").unwrap().path = "http/文件 #1?.json".into();
    let page = report::render(&run, &[]).unwrap();
    assert!(page.contains("href=\"./http/%E6%96%87%E4%BB%B6%20%231%3F.json\""));
}
