use crate::cli::CliRunReport;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const EVALUATION_VERSION: &str = "evaluation-pipeline/v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInput {
    pub schema_version: String,
    pub evaluation_version: String,
    pub module: String,
    pub target: Value,
    pub selected_modules: Vec<String>,
    pub rules: Value,
    pub hard_facts: Value,
    pub evidence: Vec<Value>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleReport {
    pub schema_version: String,
    pub evaluation_version: String,
    pub module: String,
    pub verdict: String,
    pub confidence: String,
    pub summary: String,
    pub findings: Vec<Finding>,
    pub evidence_refs: Vec<String>,
    pub limitations: Vec<String>,
    pub analyzer: AnalyzerInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub item_id: String,
    pub verdict: String,
    pub rationale: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzerInfo {
    pub name: String,
    pub status: String,
}

pub fn write_module_inputs(
    report: &CliRunReport,
    output_dir: impl AsRef<Path>,
    rules_path: impl AsRef<Path>,
) -> Result<Vec<PathBuf>, String> {
    let output_dir = output_dir.as_ref();
    fs::create_dir_all(output_dir).map_err(|e| format!("创建模块输入目录失败：{e}"))?;
    let rules: Value = serde_json::from_slice(
        &fs::read(rules_path.as_ref()).map_err(|e| format!("读取评估规则失败：{e}"))?,
    )
    .map_err(|e| format!("解析评估规则失败：{e}"))?;
    let mut paths = Vec::new();
    for module in &report.selected_modules {
        let evidence = report
            .record
            .evidence
            .iter()
            .filter_map(|item| {
                let module_name = item.payload.get("module").and_then(Value::as_str)?;
                (module_name == module).then(|| {
                    json!({
                        "evidence_id": item.id,
                        "kind": item.kind,
                        "captured_at": item.captured_at,
                        "digest": item.digest,
                        "payload": item.payload,
                    })
                })
            })
            .collect::<Vec<_>>();
        let module_rules = rules
            .get("modules")
            .and_then(|value| value.get(module))
            .cloned()
            .unwrap_or(Value::Null);
        let input = ModuleInput {
            schema_version: "module-eval-input/v1".into(),
            evaluation_version: EVALUATION_VERSION.into(),
            module: module.clone(),
            target: serde_json::to_value(&report.record.target)
                .map_err(|e| format!("序列化目标失败：{e}"))?,
            selected_modules: report.selected_modules.clone(),
            rules: module_rules,
            hard_facts: json!({
                "module_state": report.record.module_results.iter().find(|item| &item.module_id == module).map(|item| &item.state),
                "execution_origin": report.execution_origin,
            }),
            evidence,
            limitations: report.limitations.clone(),
        };
        let path = output_dir.join(format!("{module}.input.json"));
        write_json(&path, &input)?;
        paths.push(path);
    }
    Ok(paths)
}

pub fn analyze_modules(
    input_paths: &[PathBuf],
    report_dir: impl AsRef<Path>,
) -> Result<Vec<PathBuf>, String> {
    let report_dir = report_dir.as_ref();
    fs::create_dir_all(report_dir).map_err(|e| format!("创建模块报告目录失败：{e}"))?;
    let omp = std::env::var("OMP_BIN").unwrap_or_else(|_| "omp".into());
    let mut paths = Vec::new();
    for input_path in input_paths {
        let module = input_path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".input.json"))
            .ok_or_else(|| format!("无法从输入文件识别模块：{}", input_path.display()))?;
        let output_path = report_dir.join(format!("{module}.report.json"));
        let prompt = format!(
            "你是 AgentCheck 评估器。只读取文件 {path}。文件中包含判定规则和该模块的完整检测证据。严格依据规则分析，不要重新请求客户模型，不要补造证据。输出一个 JSON 对象，字段必须为 schema_version、evaluation_version、module、verdict、confidence、summary、findings、evidence_refs、limitations、analyzer；verdict 只能是 pass、fail、limited、inconclusive；任何证据不足必须是 inconclusive；每个 finding 必须引用 evidence_id。不要输出 Markdown，不要输出 JSON 之外的内容。",
            path = input_path.display()
        );
        let result = Command::new(&omp).arg("-p").arg(prompt).output();
        let report = match result {
            Ok(result) if result.status.success() => {
                extract_json(&String::from_utf8_lossy(&result.stdout))
                    .and_then(|value| serde_json::from_value::<ModuleReport>(value).ok())
                    .unwrap_or_else(|| {
                        inconclusive_report(module, "OhMyPi 输出不是符合约定的 JSON")
                    })
            }
            Ok(result) => inconclusive_report(
                module,
                &format!("OhMyPi 退出失败：{}", trim_error(&result.stderr)),
            ),
            Err(error) => inconclusive_report(module, &format!("OhMyPi 不可用：{error}")),
        };
        write_json(&output_path, &report)?;
        paths.push(output_path);
    }
    Ok(paths)
}

pub fn render_html(
    run_report: &CliRunReport,
    module_report_paths: &[PathBuf],
) -> Result<String, String> {
    let mut modules = Vec::new();
    for path in module_report_paths {
        let value: ModuleReport =
            serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("解析模块报告失败：{e}"))?;
        modules.push(value);
    }
    let overall = if modules.iter().any(|module| module.verdict == "fail") {
        "limited"
    } else if modules.is_empty()
        || modules
            .iter()
            .any(|module| module.verdict == "inconclusive")
    {
        "inconclusive"
    } else {
        "pass"
    };
    let module_rows = modules.iter().map(|module| {
        let findings = module.findings.iter().map(|finding| format!(
            "<li><strong>{}</strong>：{}；证据：{}</li>",
            escape(&finding.item_id), escape(&finding.rationale), escape(&finding.evidence_refs.join(", "))
        )).collect::<String>();
        format!(
            "<section><h2>{}</h2><p class=\"verdict\">结论：{} · 置信度：{}</p><p>{}</p><ul>{}</ul><p class=\"muted\">限制：{}</p></section>",
            escape(&module.module), escape(&module.verdict), escape(&module.confidence), escape(&module.summary), findings, escape(&module.limitations.join("；"))
        )
    }).collect::<String>();
    Ok(format!(
        "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>AgentCheck 检测报告</title><style>body{{margin:0;background:#f4f6f8;color:#17212b;font:15px/1.65 -apple-system,BlinkMacSystemFont,\"Segoe UI\",\"PingFang SC\",sans-serif}}main{{max-width:1000px;margin:0 auto;padding:36px 20px}}section,header{{background:#fff;border:1px solid #d9e0e7;border-radius:8px;padding:20px;margin:16px 0}}h1{{margin:0 0 6px}}h2{{margin:0 0 8px}}.muted{{color:#5b6875}}.verdict{{font-weight:700;color:#1769aa}}</style></head><body><main><header><h1>AgentCheck 模型兼容性检测报告</h1><p>模型：{}</p><p>整体结论：{}</p><p class=\"muted\">本报告由 CLI 检测证据和 OhMyPi 模块分析生成。</p></header>{}</main></body></html>",
        escape(&run_report.configuration.model),
        overall,
        module_rows
    ))
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let content = serde_json::to_vec_pretty(value).map_err(|e| format!("序列化 JSON 失败：{e}"))?;
    fs::write(path, content).map_err(|e| format!("写入 {} 失败：{e}", path.display()))
}

fn extract_json(stdout: &str) -> Option<Value> {
    let trimmed = stdout.trim();
    serde_json::from_str(trimmed).ok().or_else(|| {
        let start = trimmed.find('{')?;
        let end = trimmed.rfind('}')?;
        serde_json::from_str(&trimmed[start..=end]).ok()
    })
}

fn inconclusive_report(module: &str, reason: &str) -> ModuleReport {
    ModuleReport {
        schema_version: "module-eval-report/v1".into(),
        evaluation_version: EVALUATION_VERSION.into(),
        module: module.into(),
        verdict: "inconclusive".into(),
        confidence: "none".into(),
        summary: reason.into(),
        findings: Vec::new(),
        evidence_refs: Vec::new(),
        limitations: vec![reason.into()],
        analyzer: AnalyzerInfo {
            name: "ohmypi".into(),
            status: "unavailable_or_invalid_output".into(),
        },
    }
}

fn trim_error(bytes: &[u8]) -> String {
    let value = String::from_utf8_lossy(bytes).trim().to_string();
    if value.is_empty() {
        "无错误输出".into()
    } else {
        value.chars().take(500).collect()
    }
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_json_from_agent_output() {
        let value = extract_json("说明\n{\"module\":\"agent\"}\n").unwrap();
        assert_eq!(value["module"], "agent");
    }
}
