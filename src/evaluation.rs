use crate::cli::CliRunReport;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

pub const EVALUATION_VERSION: &str = "evaluation-pipeline/v1";

#[derive(Debug, Clone)]
pub struct AnalyzerConfig {
    pub endpoint: String,
    pub model: String,
    pub api_key: String,
}

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
    let rules: Value = serde_json::from_slice(
        &fs::read(rules_path.as_ref()).map_err(|e| format!("读取评估规则失败：{e}"))?,
    )
    .map_err(|e| format!("解析评估规则失败：{e}"))?;
    write_module_inputs_with_rules(report, output_dir.as_ref(), &rules)
}

pub fn write_bundled_module_inputs(
    report: &CliRunReport,
    output_dir: impl AsRef<Path>,
) -> Result<Vec<PathBuf>, String> {
    let rules: Value = serde_json::from_str(include_str!("../rules/agentcheck-evaluation.json"))
        .map_err(|e| format!("解析内置评估规则失败：{e}"))?;
    write_module_inputs_with_rules(report, output_dir.as_ref(), &rules)
}

fn write_module_inputs_with_rules(
    report: &CliRunReport,
    output_dir: &Path,
    rules: &Value,
) -> Result<Vec<PathBuf>, String> {
    fs::create_dir_all(output_dir).map_err(|e| format!("创建模块输入目录失败：{e}"))?;
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
    analyzer_config: &AnalyzerConfig,
) -> Result<Vec<PathBuf>, String> {
    let report_dir = report_dir.as_ref();
    fs::create_dir_all(report_dir).map_err(|e| format!("创建模块报告目录失败：{e}"))?;
    let omp = resolve_omp_path();
    let omp_config = OmpConfig::new(analyzer_config)?;
    let mut paths = Vec::new();
    for input_path in input_paths {
        let module = input_path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".input.json"))
            .ok_or_else(|| format!("无法从输入文件识别模块：{}", input_path.display()))?;
        let output_path = report_dir.join(format!("{module}.report.json"));
        let input: ModuleInput = serde_json::from_slice(
            &fs::read(input_path).map_err(|e| format!("读取模块输入失败：{e}"))?,
        )
        .map_err(|e| format!("解析模块输入失败：{e}"))?;
        let cli_state = input
            .hard_facts
            .get("module_state")
            .and_then(Value::as_str)
            .unwrap_or("inconclusive");
        if let Err(error) = &omp {
            let mut report = inconclusive_report(module, error);
            report.limitations.push(format!(
                "CLI 原始状态为 {cli_state}；OhMyPi 未完成分析，不能形成最终模块结论"
            ));
            write_json(&output_path, &report)?;
            paths.push(output_path);
            continue;
        }
        let model_selector = format!("agentcheck-target/{}", analyzer_config.model);
        let prompt = format!(
            "你是 AgentCheck 评估器。附件是 {path} 对应的完整模块输入 JSON，包含判定规则和完整检测证据。只分析附件内容，不要调用工具，不要重新请求客户模型，不要补造证据。严格依据规则输出一个 JSON 对象，字段必须为 schema_version、evaluation_version、module、verdict、confidence、summary、findings、evidence_refs、limitations、analyzer；verdict 只能是 pass、fail、limited、inconclusive；任何证据不足必须是 inconclusive；每个 finding 必须引用 evidence_id。不要输出 Markdown，不要输出 JSON 之外的内容。",
            path = input_path.display(),
        );
        let result = Command::new(omp.as_ref().unwrap())
            .args([
                "-p",
                "--mode",
                "json",
                "--no-title",
                "--no-pty",
                "--no-tools",
            ])
            .args(["--model", model_selector.as_str()])
            .args(["--no-extensions", "--no-skills", "--no-rules", "--no-lsp"])
            .arg(format!("@{}", input_path.display()))
            .arg(prompt)
            .env("PI_CODING_AGENT_DIR", &omp_config.agent_dir)
            .env("AGENTCHECK_MODEL_API_KEY", &analyzer_config.api_key)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();
        let report = match result {
            Ok(result) if result.status.success() => {
                let raw_output_path = report_dir.join(format!("{module}.omp.jsonl"));
                fs::write(&raw_output_path, &result.stdout)
                    .map_err(|e| format!("保存 OhMyPi 原始输出失败：{e}"))?;
                parse_module_report(&extract_omp_text(&result.stdout), module).unwrap_or_else(
                    || inconclusive_report(module, "OhMyPi 输出不是符合约定的 JSON"),
                )
            }
            Ok(result) => inconclusive_report(
                module,
                &format!("OhMyPi 退出失败：{}", trim_error(&result.stderr)),
            ),
            Err(error) => inconclusive_report(module, &format!("OhMyPi 不可用：{error}")),
        };
        let report = merge_cli_and_analyzer_result(report, cli_state);
        write_json(&output_path, &report)?;
        paths.push(output_path);
    }
    Ok(paths)
}

fn merge_cli_and_analyzer_result(mut report: ModuleReport, cli_state: &str) -> ModuleReport {
    if report.verdict == "inconclusive" {
        report.limitations.push(format!(
            "CLI 原始状态为 {cli_state}；OhMyPi 结论为 inconclusive，最终不归因于模型能力"
        ));
        return report;
    }
    if matches!(
        cli_state,
        "inconclusive" | "invalid_execution" | "unverified"
    ) {
        report.verdict = "inconclusive".into();
        report.confidence = "none".into();
        report.summary =
            format!("CLI 原始状态为 {cli_state}，证据不足；OhMyPi 的结果不能扩大检测范围");
        report.findings.clear();
        report
            .limitations
            .push("CLI 执行证据不足，语义分析结果不作为模型能力结论".into());
    }
    report
}

pub(crate) struct OmpConfig {
    root: PathBuf,
    pub(crate) agent_dir: PathBuf,
}

impl OmpConfig {
    pub(crate) fn new(config: &AnalyzerConfig) -> Result<Self, String> {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| format!("生成 OMP 临时目录失败：{e}"))?
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("agentcheck-omp-{}-{suffix}", std::process::id()));
        let agent_dir = root.join("agent");
        fs::create_dir_all(&agent_dir).map_err(|e| format!("创建 OMP 配置目录失败：{e}"))?;
        let models = format!(
            "providers:\n  agentcheck-target:\n    baseUrl: {}\n    api: openai-completions\n    apiKey: AGENTCHECK_MODEL_API_KEY\n    models:\n      - id: {}\n        name: {}\n        api: openai-completions\n        reasoning: false\n        input: [text]\n        contextWindow: 128000\n        maxTokens: 8192\n",
            yaml_quote(&omp_base_url(&config.endpoint)),
            yaml_quote(&config.model),
            yaml_quote(&config.model),
        );
        let models_path = agent_dir.join("models.yml");
        let mut file =
            fs::File::create(&models_path).map_err(|e| format!("写入 OMP 模型配置失败：{e}"))?;
        file.write_all(models.as_bytes())
            .map_err(|e| format!("写入 OMP 模型配置失败：{e}"))?;
        Ok(Self { root, agent_dir })
    }
}

impl Drop for OmpConfig {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(crate) fn resolve_omp_path() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("OMP_BIN") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!("OMP_BIN 指向的文件不存在：{}", path.display()));
    }
    if let Some(path) = crate::runtime::omp_path()? {
        return Ok(path);
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            let bundled = parent.join(if cfg!(windows) { "omp.exe" } else { "omp" });
            if bundled.is_file() {
                return Ok(bundled);
            }
        }
    }
    Ok(PathBuf::from(if cfg!(windows) { "omp.exe" } else { "omp" }))
}

fn extract_omp_text(stdout: &[u8]) -> String {
    let mut text = String::new();
    for line in String::from_utf8_lossy(stdout).lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) == Some("message_end") {
            append_message_text(&mut text, value.get("message"));
        } else if value.get("type").and_then(Value::as_str) == Some("agent_end") {
            let mut final_text = String::new();
            if let Some(messages) = value.get("messages").and_then(Value::as_array) {
                for message in messages {
                    if message.get("role").and_then(Value::as_str) == Some("assistant") {
                        append_message_text(&mut final_text, Some(message));
                    }
                }
            }
            if !final_text.is_empty() {
                text = final_text;
            }
        }
    }
    if text.is_empty() {
        String::from_utf8_lossy(stdout).into_owned()
    } else {
        text
    }
}

fn parse_module_report(stdout: &str, module: &str) -> Option<ModuleReport> {
    let value = extract_json(stdout)?;
    if let Ok(report) = serde_json::from_value::<ModuleReport>(value.clone()) {
        return Some(report);
    }
    normalize_legacy_report(value, module)
}

fn normalize_legacy_report(value: Value, module: &str) -> Option<ModuleReport> {
    let evaluation = value.get("evaluation_result");
    let verdict = normalize_verdict(
        evaluation
            .and_then(|item| {
                item.get("overall_verdict")
                    .or_else(|| item.get("module_state"))
            })
            .or_else(|| value.get("overall_verdict"))
            .or_else(|| value.get("module_state"))
            .or_else(|| value.get("verdict"))
            .and_then(Value::as_str)?,
    )?;
    let mut evidence_refs: Vec<String> = value
        .get("evidence_refs")
        .and_then(Value::as_array)
        .map(|refs| {
            refs.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let mut findings = Vec::new();
    if let Some(rules) = value.get("rule_results").and_then(Value::as_array) {
        for rule in rules {
            let Some(item_id) = rule.get("rule").and_then(Value::as_str) else {
                continue;
            };
            let Some(verdict) = rule
                .get("verdict")
                .and_then(Value::as_str)
                .and_then(normalize_verdict)
            else {
                continue;
            };
            findings.push(Finding {
                item_id: item_id.into(),
                verdict,
                rationale: rule
                    .get("evidence")
                    .map(format_evidence)
                    .unwrap_or_else(|| "未提供说明".into()),
                evidence_refs: evidence_refs.clone(),
            });
        }
    }
    if let Some(rules) = value.get("rules_verdict").and_then(Value::as_object) {
        for (item_id, rule) in rules {
            let Some(verdict) = rule
                .get("verdict")
                .or_else(|| rule.get("result"))
                .and_then(Value::as_str)
                .and_then(normalize_verdict)
            else {
                continue;
            };
            findings.push(Finding {
                item_id: item_id.clone(),
                verdict,
                rationale: rule
                    .get("evidence")
                    .map(format_evidence)
                    .unwrap_or_else(|| "未提供说明".into()),
                evidence_refs: evidence_refs.clone(),
            });
        }
    }
    if let Some(rules) = value.get("verdicts").and_then(Value::as_object) {
        for (item_id, rule) in rules {
            let Some(verdict) = rule
                .get("verdict")
                .or_else(|| rule.get("result"))
                .and_then(Value::as_str)
                .and_then(normalize_verdict)
            else {
                continue;
            };
            if let Some(evidence_id) = rule
                .get("evidence")
                .and_then(|item| item.get("evidence_id"))
                .and_then(Value::as_str)
                && !evidence_refs.iter().any(|item| item == evidence_id)
            {
                evidence_refs.push(evidence_id.to_owned());
            }
            findings.push(Finding {
                item_id: item_id.clone(),
                verdict,
                rationale: rule
                    .get("evidence")
                    .map(format_evidence)
                    .unwrap_or_else(|| "未提供说明".into()),
                evidence_refs: evidence_refs.clone(),
            });
        }
    }
    if let Some(items) = value.get("findings").and_then(Value::as_array) {
        for item in items {
            let Some(item_id) = item
                .get("rule")
                .or_else(|| item.get("item_id"))
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            let Some(verdict) = item
                .get("verdict")
                .or_else(|| item.get("kind"))
                .or_else(|| item.get("result"))
                .and_then(Value::as_str)
                .and_then(normalize_verdict)
            else {
                continue;
            };
            let item_evidence_refs = item
                .get("evidence_refs")
                .and_then(Value::as_array)
                .map(|refs| {
                    refs.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .filter(|refs| !refs.is_empty())
                .unwrap_or_else(|| evidence_refs.clone());
            findings.push(Finding {
                item_id: item_id.into(),
                verdict,
                rationale: item
                    .get("detail")
                    .or_else(|| item.get("rationale"))
                    .or_else(|| item.get("evidence"))
                    .map(format_evidence)
                    .unwrap_or_else(|| "未提供说明".into()),
                evidence_refs: item_evidence_refs,
            });
        }
    }
    Some(ModuleReport {
        schema_version: "module-eval-report/v1".into(),
        evaluation_version: value
            .get("evaluation_version")
            .and_then(Value::as_str)
            .unwrap_or(EVALUATION_VERSION)
            .into(),
        module: value
            .get("module")
            .and_then(Value::as_str)
            .unwrap_or(module)
            .into(),
        verdict,
        confidence: value
            .get("confidence")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .into(),
        summary: evaluation
            .and_then(|item| item.get("summary"))
            .or_else(|| value.get("summary"))
            .map(format_evidence)
            .as_deref()
            .unwrap_or("OhMyPi 已完成分析")
            .into(),
        findings,
        evidence_refs,
        limitations: value
            .get("limitations")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        analyzer: AnalyzerInfo {
            name: "ohmypi".into(),
            status: "completed_normalized".into(),
        },
    })
}

fn format_evidence(value: &Value) -> String {
    match value {
        Value::Array(items) => items
            .iter()
            .map(format_evidence)
            .collect::<Vec<_>>()
            .join("；"),
        Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}

fn normalize_verdict(value: &str) -> Option<String> {
    matches!(value, "pass" | "fail" | "limited" | "inconclusive").then(|| value.to_owned())
}

fn append_message_text(output: &mut String, message: Option<&Value>) {
    let Some(content) = message
        .and_then(|value| value.get("content"))
        .and_then(Value::as_array)
    else {
        return;
    };
    for item in content {
        if item.get("type").and_then(Value::as_str) == Some("text") {
            if let Some(value) = item.get("text").and_then(Value::as_str) {
                output.push_str(value);
            }
        }
    }
}

fn yaml_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn omp_base_url(endpoint: &str) -> String {
    endpoint
        .trim_end_matches('/')
        .strip_suffix("/chat/completions")
        .unwrap_or(endpoint.trim_end_matches('/'))
        .to_owned()
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
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Some(value);
    }
    for marker in ["```json", "```JSON", "```"] {
        if let Some(start) = trimmed.rfind(marker) {
            let block = &trimmed[start + marker.len()..];
            let block = block.split("```").next().unwrap_or(block);
            if let Some(value) = parse_json_prefix(block) {
                return Some(value);
            }
        }
    }
    parse_json_prefix(trimmed)
}

fn parse_json_prefix(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let mut deserializer = serde_json::Deserializer::from_str(&text[start..]);
    Value::deserialize(&mut deserializer).ok()
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

    #[test]
    fn extracts_json_from_last_markdown_block() {
        let value =
            extract_json("思考中出现 {\"wrong\":true}\n```json\n{\"module\":\"agent\"}\n```")
                .unwrap();
        assert_eq!(value["module"], "agent");
    }

    #[test]
    fn extracts_final_text_from_omp_json_events() {
        let output = br#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"\u65e7\u7ed3\u679c"}]}}
{"type":"agent_end","messages":[{"role":"assistant","content":[{"type":"text","text":"\u6700\u7ec8\u7ed3\u679c"}]}]}"#;
        assert_eq!(extract_omp_text(output), "最终结果");
    }

    #[test]
    fn extracts_only_assistant_text_from_omp_agent_end() {
        let output = br#"{"type":"agent_end","messages":[{"role":"user","content":[{"type":"text","text":"{\"module\":\"ingress\"}"}]},{"role":"assistant","content":[{"type":"text","text":"{\"verdict\":\"pass\"}"}]}]}"#;
        assert_eq!(extract_omp_text(output), "{\"verdict\":\"pass\"}");
    }

    #[test]
    fn quotes_omp_yaml_values() {
        assert_eq!(
            yaml_quote("https://example.test/v1"),
            "'https://example.test/v1'"
        );
        assert_eq!(yaml_quote("model's-id"), "'model''s-id'");
    }

    #[test]
    fn strips_chat_completion_suffix_for_omp_base_url() {
        assert_eq!(
            omp_base_url("https://example.test/v1"),
            "https://example.test/v1"
        );
        assert_eq!(
            omp_base_url("https://example.test/v1/chat/completions"),
            "https://example.test/v1"
        );
    }

    #[test]
    fn normalizes_legacy_omp_report_shape() {
        let value = json!({
            "evaluation_version": EVALUATION_VERSION,
            "module": "ingress",
            "evaluation_result": {
                "module_state": "fail",
                "overall_verdict": "fail",
                "summary": "响应内容不符合要求"
            },
            "rule_results": [{"rule": "响应内容提取", "verdict": "fail", "evidence": "证据说明"}],
            "evidence_refs": ["cli-ingress-0"]
        });
        let report = normalize_legacy_report(value, "ingress").unwrap();
        assert_eq!(report.verdict, "fail");
        assert_eq!(report.analyzer.status, "completed_normalized");
        assert_eq!(report.findings[0].evidence_refs, vec!["cli-ingress-0"]);
    }

    #[test]
    fn normalizes_rules_verdict_omp_report_shape() {
        let value = json!({
            "evaluation_version": EVALUATION_VERSION,
            "module": "ingress",
            "overall_verdict": "pass",
            "module_state": "pass",
            "rules_verdict": {"JSON 响应解析": {"verdict": "pass", "evidence": ["HTTP 200"]}},
            "evidence_refs": ["cli-ingress-0"],
            "summary": "解析通过"
        });
        let report = normalize_legacy_report(value, "ingress").unwrap();
        assert_eq!(report.verdict, "pass");
        assert_eq!(report.findings[0].rationale, "HTTP 200");
    }

    #[test]
    fn normalizes_current_omp_findings_report_shape() {
        let value = json!({
            "module": "ingress",
            "verdict": "pass",
            "confidence": "high",
            "summary": "服务可连接",
            "findings": [{
                "rule": "HTTP 请求与鉴权",
                "verdict": "pass",
                "detail": "HTTP 200",
                "evidence_refs": ["cli-ingress-0"]
            }],
            "evidence_refs": ["cli-ingress-0"],
            "limitations": []
        });
        let report = normalize_legacy_report(value, "ingress").unwrap();
        assert_eq!(report.verdict, "pass");
        assert_eq!(report.confidence, "high");
        assert_eq!(report.findings[0].item_id, "HTTP 请求与鉴权");
        assert_eq!(report.findings[0].evidence_refs, vec!["cli-ingress-0"]);
    }

    #[test]
    fn analyzer_inconclusive_cannot_become_model_failure() {
        let report = inconclusive_report("capability", "OhMyPi 退出失败");
        let merged = merge_cli_and_analyzer_result(report, "fail");
        assert_eq!(merged.verdict, "inconclusive");
        assert!(
            merged
                .limitations
                .iter()
                .any(|item| item.contains("不归因于模型能力"))
        );
    }

    #[test]
    fn insufficient_cli_evidence_overrides_analyzer_success() {
        let report = ModuleReport {
            schema_version: "module-eval-report/v1".into(),
            evaluation_version: EVALUATION_VERSION.into(),
            module: "capability".into(),
            verdict: "pass".into(),
            confidence: "high".into(),
            summary: "分析通过".into(),
            findings: Vec::new(),
            evidence_refs: Vec::new(),
            limitations: Vec::new(),
            analyzer: AnalyzerInfo {
                name: "ohmypi".into(),
                status: "completed".into(),
            },
        };
        let merged = merge_cli_and_analyzer_result(report, "inconclusive");
        assert_eq!(merged.verdict, "inconclusive");
        assert_eq!(merged.confidence, "none");
        assert!(merged.findings.is_empty());
    }
}
