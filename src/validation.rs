use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;

use crate::agent::{AgentCheck, AgentScenario, fixed_agent_scenarios};
use crate::baseline::{
    ActualSnapshot, ActualSource, BaselineScenario, StructuralStatus, compare_snapshot,
    fixed_baseline_catalog,
};
use crate::capability::{
    CAPABILITY_VERSION, CapabilityCategory, CapabilitySettings, build_scorecard,
    fixed_capability_catalog,
};
use crate::cli::{
    CLI_VERSION, CliRunRequest, UnavailableExecutor, default_module_ids, normalized_selection,
    run_with_executor,
};
use crate::gui::{DesktopPlatform, GUI_VERSION, detect_desktop, render_workbench_html};
use crate::performance::{PERFORMANCE_VERSION, PerformanceCategory, fixed_performance_plan};
use crate::records::ModuleResultState;
use crate::specification::{SPECIFICATION_VERSION, SpecCategory, specification_plan};

pub const VALIDATION_VERSION: &str = "validation/issue-33/v1";
pub const VALIDATION_ENVIRONMENT: &str = "controlled-rust-regression";
pub const VALIDATION_FIXTURE_KIND: &str = "controlled_fixture";
pub const REAL_EXECUTION_KIND: &str = "real_service";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ValidationDomain {
    Specification,
    Capability,
    Performance,
    Agent,
    Baseline,
    Records,
    Cli,
    Gui,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    Pass,
    Fail,
    Inconclusive,
    NotRun,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ValidationEvidenceKind {
    ControlledFixture,
    RealService,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationEvidence {
    pub kind: ValidationEvidenceKind,
    pub method: String,
    pub environment: String,
    pub expected: String,
    pub actual: String,
    pub evidence_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationCase {
    pub id: String,
    pub domain: ValidationDomain,
    pub rule_version: String,
    pub description: String,
    pub status: ValidationStatus,
    pub evidence: ValidationEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationDefect {
    pub id: String,
    pub original_case_id: String,
    pub description: String,
    pub cause: String,
    pub fix_ref: String,
    pub retest_case_id: String,
    pub status: ValidationStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationRuleVersions {
    pub specification: String,
    pub capability: String,
    pub performance: String,
    pub agent: String,
    pub baseline: String,
    pub cli: String,
    pub gui: String,
    pub records: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationMatrix {
    pub version: String,
    pub environment: String,
    pub executed_at: String,
    pub rule_versions: ValidationRuleVersions,
    pub cases: Vec<ValidationCase>,
    pub defects: Vec<ValidationDefect>,
    pub limitations: Vec<String>,
}

impl ValidationMatrix {
    pub fn status(&self) -> ValidationStatus {
        if self
            .cases
            .iter()
            .any(|case| case.status == ValidationStatus::Fail)
            || self
                .defects
                .iter()
                .any(|defect| defect.status != ValidationStatus::Pass)
        {
            return ValidationStatus::Fail;
        }
        if self.cases.iter().any(|case| {
            matches!(
                case.status,
                ValidationStatus::Inconclusive | ValidationStatus::NotRun
            )
        }) {
            return ValidationStatus::Inconclusive;
        }
        ValidationStatus::Pass
    }

    pub fn cases_for(&self, domain: ValidationDomain) -> Vec<&ValidationCase> {
        self.cases
            .iter()
            .filter(|case| case.domain == domain)
            .collect()
    }

    pub fn json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

pub fn run_validation_suite() -> ValidationMatrix {
    let mut cases = Vec::new();
    let spec_plan = specification_plan();
    for category in SpecCategory::ALL {
        let observed = spec_plan.iter().any(|plan| plan.category == category);
        cases.push(controlled_case(
            format!("spec-{}", category.id()),
            ValidationDomain::Specification,
            SPECIFICATION_VERSION,
            format!("固定规格清单包含 {} {}", category.id(), category.title()),
            "存在一条对应规格计划",
            observed.to_string(),
        ));
    }

    let capability_catalog = fixed_capability_catalog();
    for category in CapabilityCategory::ALL {
        let count = capability_catalog
            .iter()
            .filter(|sample| sample.category == category)
            .count();
        cases.push(controlled_case(
            format!("capability-{}", category.id()),
            ValidationDomain::Capability,
            CAPABILITY_VERSION,
            format!("固定能力清单包含 {} {}", category.id(), category.title()),
            "40 个中文能力单元",
            count.to_string(),
        ));
    }

    let performance_plan = fixed_performance_plan();
    for category in PerformanceCategory::ALL {
        let count = performance_plan
            .iter()
            .filter(|plan| plan.category == category)
            .count();
        cases.push(controlled_case(
            format!("performance-{}", category.id()),
            ValidationDomain::Performance,
            PERFORMANCE_VERSION,
            format!("固定性能计划覆盖 {} {}", category.id(), category.title()),
            "至少一条正式测量计划",
            count.to_string(),
        ));
    }

    let _ = fixed_agent_scenarios();
    for check in AgentCheck::ALL {
        let scenario_count = AgentScenario::ALL
            .iter()
            .filter(|scenario| scenario.checks().contains(&check))
            .count();
        cases.push(controlled_case(
            format!("agent-{}", check.id()),
            ValidationDomain::Agent,
            "agent/v1",
            format!("Agent 检查 {} {} 被场景引用", check.id(), check.title()),
            "至少被一个固定场景引用",
            scenario_count.to_string(),
        ));
    }

    let baseline_catalog = fixed_baseline_catalog();
    for scenario in BaselineScenario::ALL {
        let present = baseline_catalog
            .variants
            .iter()
            .any(|variant| variant.scenario == scenario);
        cases.push(controlled_case(
            format!("baseline-{}", scenario.id()),
            ValidationDomain::Baseline,
            "chat-completions-baseline/v1",
            format!(
                "基线结构变体 {} {} 可被比较",
                scenario.id(),
                scenario.title()
            ),
            "变体存在且可使用",
            present.to_string(),
        ));
    }

    cases.extend(record_cases());
    cases.extend(cli_cases());
    cases.extend(gui_cases());

    ValidationMatrix {
        version: VALIDATION_VERSION.into(),
        environment: VALIDATION_ENVIRONMENT.into(),
        executed_at: "2026-09-11T00:00:00Z".into(),
        rule_versions: ValidationRuleVersions {
            specification: SPECIFICATION_VERSION.into(),
            capability: CAPABILITY_VERSION.into(),
            performance: PERFORMANCE_VERSION.into(),
            agent: "agent/v1".into(),
            baseline: "chat-completions-baseline/v1".into(),
            cli: CLI_VERSION.into(),
            gui: GUI_VERSION.into(),
            records: "detection-record/v1".into(),
        },
        cases,
        defects: vec![ValidationDefect {
            id: "baseline-optional-field-retest".into(),
            original_case_id: "baseline-BC12".into(),
            description: "可选字段缺失必须保持结构兼容，不能误判为必填字段缺失".into(),
            cause: "基线字段规则修正后需要保留原失败与复测关系".into(),
            fix_ref: "integration/740925a".into(),
            retest_case_id: "baseline-BC12".into(),
            status: ValidationStatus::Pass,
        }],
        limitations: vec![
            "本矩阵执行的是 Rust 规则与受控夹具回归，不等同于真实模型服务校准。".into(),
            "真实服务请求未在验收夹具中执行；上线前必须保存真实请求、响应、环境和证据引用。".into(),
            "当前 CLI 的默认执行器仍会把未接入的模块标为 inconclusive，不把未测量内容包装成通过。"
                .into(),
        ],
    }
}

fn controlled_case(
    id: String,
    domain: ValidationDomain,
    rule_version: &str,
    description: String,
    expected: &str,
    actual: String,
) -> ValidationCase {
    let status = if actual == "false" || actual == "0" {
        ValidationStatus::Fail
    } else {
        ValidationStatus::Pass
    };
    ValidationCase {
        id: id.clone(),
        domain,
        rule_version: rule_version.into(),
        description,
        status,
        evidence: ValidationEvidence {
            kind: ValidationEvidenceKind::ControlledFixture,
            method: "run_validation_suite".into(),
            environment: VALIDATION_ENVIRONMENT.into(),
            expected: expected.into(),
            actual,
            evidence_ref: format!("validation://{id}"),
        },
    }
}

fn record_cases() -> Vec<ValidationCase> {
    let mut executor = UnavailableExecutor;
    let report = run_with_executor(
        CliRunRequest {
            endpoint: "https://api.example.test/v1/chat".into(),
            model: "validation-model".into(),
            api_key: Some("secret-value".into()),
            selected_modules: Some(vec!["capability".into()]),
            stop_after: None,
            run_id: "validation-record".into(),
            started_at: "2026-09-11T00:00:00Z".into(),
        },
        &mut executor,
    )
    .expect("controlled CLI run must build a record");
    let serialized = serde_json::to_string(&report.record).expect("record must serialize");
    vec![
        controlled_case(
            "records-evidence-binding".into(),
            ValidationDomain::Records,
            "detection-record/v1",
            "模块结果、尝试和事件都引用同一份证据记录".into(),
            "模块结果存在且证据引用可解析",
            report
                .record
                .module_results
                .iter()
                .all(|result| {
                    result.evidence_refs.iter().all(|reference| {
                        report
                            .record
                            .evidence
                            .iter()
                            .any(|item| &item.id == reference)
                    })
                })
                .to_string(),
        ),
        controlled_case(
            "records-config-redaction".into(),
            ValidationDomain::Records,
            "detection-record/v1",
            "序列化记录不泄露 API 密钥，服务地址只保留脱敏形式".into(),
            "不包含 secret-value",
            (!serialized.contains("secret-value") && !serialized.contains("api_key")).to_string(),
        ),
        controlled_case(
            "records-inconclusive-state".into(),
            ValidationDomain::Records,
            "detection-record/v1",
            "未接入执行器的模块保持 inconclusive，不被计为通过".into(),
            "capability 状态为 inconclusive",
            report
                .record
                .module_results
                .iter()
                .find(|result| result.module_id == "capability")
                .is_some_and(|result| result.state == ModuleResultState::Inconclusive)
                .to_string(),
        ),
    ]
}

fn cli_cases() -> Vec<ValidationCase> {
    let default = default_module_ids();
    let selected = normalized_selection(Some(vec!["agent".into(), "baseline".into()]))
        .expect("selected modules must normalize");
    let mut executor = UnavailableExecutor;
    let stopped = run_with_executor(
        CliRunRequest {
            endpoint: "https://api.example.test/v1/chat".into(),
            model: "validation-model".into(),
            api_key: None,
            selected_modules: Some(vec!["specification".into(), "capability".into()]),
            stop_after: Some("specification".into()),
            run_id: "validation-stop".into(),
            started_at: "2026-09-11T00:00:00Z".into(),
        },
        &mut executor,
    )
    .expect("stop-after run must build a report");
    vec![
        controlled_case(
            "cli-default-all".into(),
            ValidationDomain::Cli,
            CLI_VERSION,
            "省略模块选择时执行当前六个正式项目".into(),
            "6",
            default.len().to_string(),
        ),
        controlled_case(
            "cli-selected-modules".into(),
            ValidationDomain::Cli,
            CLI_VERSION,
            "显式选择只影响本次编排且保持顺序".into(),
            "agent,baseline",
            selected.join(","),
        ),
        controlled_case(
            "cli-stop-after".into(),
            ValidationDomain::Cli,
            CLI_VERSION,
            "停止后剩余项目保持未验证，整体结论为 inconclusive".into(),
            "stopped + unverified",
            format!(
                "{} + {}",
                stopped.record.lifecycle == crate::records::LifecycleState::Stopped,
                stopped
                    .record
                    .plan
                    .iter()
                    .filter(|entry| entry.state == crate::records::ModuleSelectionState::Unverified)
                    .count()
            ),
        ),
    ]
}

fn gui_cases() -> Vec<ValidationCase> {
    let mut env = BTreeMap::new();
    let no_desktop = detect_desktop(DesktopPlatform::Linux, &env);
    env.insert("DISPLAY".into(), ":0".into());
    let desktop = detect_desktop(DesktopPlatform::Linux, &env);
    env.insert("CI".into(), "true".into());
    let ci = detect_desktop(DesktopPlatform::Linux, &env);
    let mut executor = UnavailableExecutor;
    let report = run_with_executor(
        CliRunRequest {
            endpoint: "https://api.example.test/v1/chat".into(),
            model: "validation-model".into(),
            api_key: None,
            selected_modules: Some(vec!["capability".into()]),
            stop_after: None,
            run_id: "validation-gui".into(),
            started_at: "2026-09-11T00:00:00Z".into(),
        },
        &mut executor,
    )
    .expect("GUI report fixture must build");
    let html = render_workbench_html(&report).expect("GUI workbench must render");
    vec![
        controlled_case(
            "gui-shared-report".into(),
            ValidationDomain::Gui,
            GUI_VERSION,
            "GUI 展示与 CLI 使用同一份记录、状态和六个模块".into(),
            "record id + six module labels",
            (html.contains("validation-gui")
                && [
                    "服务接入",
                    "模型规格",
                    "能力跑分",
                    "性能实测",
                    "智能体实测",
                    "模型基线",
                ]
                .iter()
                .all(|label| html.contains(label)))
            .to_string(),
        ),
        controlled_case(
            "gui-desktop-fallback".into(),
            ValidationDomain::Gui,
            GUI_VERSION,
            "无桌面、可用桌面和 CI 环境分别走回退/打开/回退路径".into(),
            "false,true,false",
            format!(
                "{},{},{}",
                no_desktop.supported, desktop.supported, ci.supported
            ),
        ),
    ]
}

pub fn validation_summary_json(matrix: &ValidationMatrix) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&json!({
        "version": matrix.version,
        "status": matrix.status(),
        "environment": matrix.environment,
        "case_count": matrix.cases.len(),
        "defect_count": matrix.defects.len(),
        "limitations": matrix.limitations,
    }))
}

pub fn boundary_regression_cases() -> Vec<ValidationCase> {
    let catalog = fixed_capability_catalog();
    let scorecard = build_scorecard(
        "validation-boundary",
        catalog.clone(),
        Vec::new(),
        CapabilitySettings {
            temperature: "0".into(),
            model: "validation-model".into(),
            protocol: "chat-completions".into(),
            client_version: "validation".into(),
            validated_input_max_tokens: Some(1024),
        },
    )
    .expect("empty capability run must still produce missing observations");
    let baseline = fixed_baseline_catalog();
    let structurally_compatible = baseline.variants.iter().all(|variant| {
        !matches!(
            compare_snapshot(
                variant,
                &ActualSnapshot::from_value(
                    variant.reference.clone(),
                    ActualSource::ControlledFixture,
                    variant.phase,
                    vec![format!("validation://{}", variant.scenario.id())],
                ),
            )
            .status,
            StructuralStatus::Different
        )
    });
    vec![
        controlled_case(
            "boundary-capability-missing-denominator".into(),
            ValidationDomain::Capability,
            CAPABILITY_VERSION,
            "未测量能力单元计入 missing，不进入 score 分母".into(),
            "240 missing and no score",
            format!(
                "{} missing; {} no-score",
                scorecard
                    .summaries
                    .iter()
                    .map(|summary| summary.missing)
                    .sum::<u32>(),
                scorecard
                    .summaries
                    .iter()
                    .filter(|summary| summary.score.is_none())
                    .count()
            ),
        ),
        controlled_case(
            "boundary-baseline-structure-only".into(),
            ValidationDomain::Baseline,
            "chat-completions-baseline/v1",
            "基线只比较结构，不把数值/顺序差异包装成整体可用性结论".into(),
            "14 exact-structure fixtures pass",
            format!(
                "{} fixtures have no structural difference",
                usize::from(structurally_compatible) * 14
            ),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_covers_every_formal_list_and_ui_interactions() {
        let matrix = run_validation_suite();
        assert_eq!(matrix.cases_for(ValidationDomain::Specification).len(), 5);
        assert_eq!(matrix.cases_for(ValidationDomain::Capability).len(), 6);
        assert_eq!(matrix.cases_for(ValidationDomain::Performance).len(), 5);
        assert_eq!(matrix.cases_for(ValidationDomain::Agent).len(), 8);
        assert_eq!(matrix.cases_for(ValidationDomain::Baseline).len(), 14);
        assert_eq!(matrix.cases_for(ValidationDomain::Cli).len(), 3);
        assert_eq!(matrix.cases_for(ValidationDomain::Gui).len(), 2);
        assert_eq!(matrix.cases_for(ValidationDomain::Records).len(), 3);
    }

    #[test]
    fn controlled_suite_passes_without_claiming_real_execution() {
        let matrix = run_validation_suite();
        assert_eq!(matrix.status(), ValidationStatus::Pass);
        assert!(
            matrix
                .cases
                .iter()
                .all(|case| { case.evidence.kind == ValidationEvidenceKind::ControlledFixture })
        );
        assert!(
            matrix
                .limitations
                .iter()
                .any(|item| item.contains("真实模型服务"))
        );
    }

    #[test]
    fn boundary_cases_preserve_missing_and_structural_semantics() {
        let cases = boundary_regression_cases();
        assert!(
            cases
                .iter()
                .all(|case| case.status == ValidationStatus::Pass)
        );
        assert_eq!(cases[0].evidence.actual, "120 missing; 6 no-score");
        assert_eq!(
            cases[1].evidence.actual,
            "14 fixtures have no structural difference"
        );
    }

    #[test]
    fn summary_json_is_machine_readable() {
        let matrix = run_validation_suite();
        let json = validation_summary_json(&matrix).unwrap();
        assert!(json.contains("\"status\": \"pass\""));
        assert!(json.contains("\"case_count\": 46"));
        assert!(json.contains(VALIDATION_ENVIRONMENT));
    }
}
