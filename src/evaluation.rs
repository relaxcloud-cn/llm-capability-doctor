use crate::cli::{CliRunReport, module_display_name, module_item_stats, module_state_label};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const EVALUATION_VERSION: &str = "evaluation-pipeline/v1";

/// OhMyPi 单批分析的自适应看门狗超时：每条约 60 秒分析时间，
/// 下限 600 秒、上限 1800 秒。固定超时会掐断重批次（如 C01 指令遵循
/// 22 条实测约 880 秒，900 秒固定值刚好不够），导致结论截断。
fn chunk_timeout_for(entries: usize) -> Duration {
    let per_entry = 60u64.saturating_mul(entries as u64);
    Duration::from_secs((180 + per_entry).clamp(600, 1800))
}

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
    /// 逐项检测结果，供报告按检测项展示；dynamic 分析产物可能没有，由实测数据补填。
    #[serde(default)]
    pub items: Vec<ModuleReportItem>,
    pub analyzer: AnalyzerInfo,
}

/// 单个检测项的客户可见结果：name 为中文检测项名，status 为内部状态码，note 为中文说明。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleReportItem {
    pub name: String,
    pub status: String,
    pub note: String,
    /// 字段级差异（目前仅基线对比产生）：逐条列出参考与实际的不同。
    #[serde(default)]
    pub diffs: Vec<ModuleReportDiff>,
    /// 参考结构完整 JSON（基线对比：官方目录样例），用于左右对照展示。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<Value>,
    /// 实测响应完整 JSON，用于左右对照展示。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<Value>,
}

/// 一条字段差异：sign 取 "-"/"+"/"~"，对应缺少字段、多出字段、内容不符。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleReportDiff {
    pub sign: String,
    pub path: String,
    pub expected: String,
    pub actual: String,
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

/// 单批证据的体积上限（字节）。模块整包不超过它时保持整包一次喂；
/// 超过时按检测项分组，超预算的组再按条目顺序装箱切批，
/// 避免大模块（能力跑分、性能实测）证据包超出模型上下文。
/// 128KB：单批约 10 个条目；实测 256KB（约 20 条）会让模型写不完结论被截断。
const OMP_CHUNK_BUDGET_BYTES: usize = 128 * 1024;

/// 单批条目数上限。截断风险随条目数上升（实测重批次每条需 60-70 秒分析时间，
/// 22 条即使给了 25 分钟仍可能被掐断），因此体积之外再按条目数切一刀。
const OMP_MAX_ITEMS_PER_CHUNK: usize = 12;

/// 单行刷新的状态行：交互终端用 \r + 清行码原地更新，批次进度只占一行；
/// 非 TTY（重定向到日志文件）退化为逐行输出，保留完整历史。
struct StatusLine {
    interactive: bool,
    /// 交互模式下当前行是否有未换行的内容；Drop 时补换行，避免错误信息粘连在行尾。
    pending: bool,
}

impl StatusLine {
    fn new() -> Self {
        Self {
            interactive: std::io::stderr().is_terminal()
                && std::env::var_os("TERM").is_none_or(|term| term != "dumb"),
            pending: false,
        }
    }

    /// 原地更新当前行；非 TTY 下逐行打印。
    fn update(&mut self, message: &str) {
        if self.interactive {
            eprint!("\r\x1b[2K{message}");
            let _ = std::io::stderr().flush();
            self.pending = true;
        } else {
            eprintln!("{message}");
        }
    }

    /// 落定一行正式输出（覆盖行内进度），后续消息从新行开始。
    fn commit(&mut self, message: &str) {
        if self.interactive {
            eprintln!("\r\x1b[2K{message}");
        } else {
            eprintln!("{message}");
        }
        self.pending = false;
    }
}

impl Drop for StatusLine {
    fn drop(&mut self) {
        if self.pending {
            eprintln!();
        }
    }
}

/// 耗时短格式，与终端进度条一致：60s 内 "42s"，以上 "3m12s"。
fn elapsed_short(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m{:02}s", seconds / 60, seconds % 60)
    }
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
    let mut status = StatusLine::new();
    for input_path in input_paths {
        let module = input_path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".input.json"))
            .ok_or_else(|| format!("无法从输入文件识别模块：{}", input_path.display()))?;
        let module_started = Instant::now();
        let output_path = report_dir.join(format!("{module}.report.json"));
        if let Err(error) = &omp {
            let report = inconclusive_report(module, error);
            write_json(&output_path, &report)?;
            status.commit(&format!("[报告] OhMyPi 不可用，{module} 待确认：{error}"));
            paths.push(output_path);
            continue;
        }
        let model_selector = format!("agentcheck-target/{}", analyzer_config.model);
        let input_value: Value = serde_json::from_slice(
            &fs::read(input_path).map_err(|e| format!("读取模块输入失败：{e}"))?,
        )
        .map_err(|e| format!("解析模块输入失败：{e}"))?;
        let chunks = match chunk_units(&input_value, OMP_CHUNK_BUDGET_BYTES) {
            chunks if !chunks.is_empty() => chunks,
            _ => vec![(String::from("all"), Vec::new())],
        };
        let total_chunks = chunks.len();
        let mut chunk_reports = Vec::new();
        for (index, (group, entries)) in chunks.iter().enumerate() {
            let batch = index + 1;
            let raw_suffix = if total_chunks == 1 {
                String::new()
            } else {
                format!(".chunk-{batch:02}")
            };
            let (attachment, prompt) = if total_chunks == 1 {
                (
                    input_path.display().to_string(),
                    format!(
                        "你是 AgentCheck 评估器。附件是 {path} 对应的完整模块输入 JSON，包含判定规则和完整检测证据。只分析附件内容，不要调用工具，不要重新请求客户模型，不要补造证据。严格依据规则输出一个 JSON 对象，字段必须为 schema_version、evaluation_version、module、verdict、confidence、summary、findings、evidence_refs、limitations、analyzer；verdict 只能是 pass、fail、limited、inconclusive；任何证据不足必须是 inconclusive；每个 finding 必须引用 evidence_id，rationale 用一两句话直接说明依据；不得虚构附件中没有的样本计数或统计数字；确保 JSON 完整闭合后输出结束，不要中途截断。不要输出 Markdown，不要输出 JSON 之外的内容。",
                        path = input_path.display(),
                    ),
                )
            } else {
                let chunk_path = report_dir.join(format!("{module}{raw_suffix}.input.json"));
                let mut chunk_input = input_value.clone();
                chunk_input["evidence"] = Value::Array(entries.clone());
                write_json(&chunk_path, &chunk_input)?;
                (
                    chunk_path.display().to_string(),
                    format!(
                        "你是 AgentCheck 评估器。附件是模块 {module} 检测输入的第 {batch}/{total} 批（同一模块按检测项分批分析，附件只包含本批条目）。只判定本批条目：不要因为附件不含其他批次而输出 inconclusive，也不要臆测未包含的条目。依据附件中的判定规则输出一个 JSON 对象，字段必须为 schema_version、evaluation_version、module、verdict、confidence、summary、findings、evidence_refs、limitations、analyzer；verdict 是本批条目的总体结论，只能是 pass、fail、limited、inconclusive；本批内证据不足必须是 inconclusive；每个 finding 必须引用 evidence_id，rationale 用一两句话直接说明依据；不得虚构附件中没有的样本计数或统计数字；确保 JSON 完整闭合后输出结束，不要中途截断。只分析附件内容，不要调用工具，不要重新请求客户模型，不要补造证据。不要输出 Markdown，不要输出 JSON 之外的内容。",
                        module = module,
                        batch = batch,
                        total = total_chunks,
                    ),
                )
            };
            status.update(&format!(
                "[报告] OhMyPi 分析模块 {module} 批次 {batch}/{total_chunks}（{group}）…"
            ));
            let timeout = chunk_timeout_for(entries.len());
            let mut command = Command::new(omp.as_ref().unwrap());
            command
                .args(["-p", "--mode", "json", "--no-title", "--no-pty", "--no-tools"])
                .args(["--model", model_selector.as_str()])
                .args(["--no-extensions", "--no-skills", "--no-rules", "--no-lsp"])
                .arg("--max-time")
                .arg(timeout.as_secs().saturating_sub(30).to_string())
                .arg(format!("@{attachment}"))
                .arg(prompt)
                .env("PI_CODING_AGENT_DIR", &omp_config.agent_dir)
                .env("AGENTCHECK_MODEL_API_KEY", &analyzer_config.api_key);
            // omp 偶发崩溃/挂死（实测出现过空 stderr 退出失败）重试一次；
            // 输出格式解析失败不重试——模型输出已确定，重试改变不了结果。
            let raw_output_path = report_dir.join(format!("{module}{raw_suffix}.omp.jsonl"));
            let mut retries_left = 1_u8;
            let report = loop {
                let result = run_command_with_timeout(&mut command, timeout);
                let outcome = match result {
                    Ok(result) if result.status.is_none() => Err(format!(
                        "分析超时（{} 秒），已终止该进程",
                        timeout.as_secs()
                    )),
                    Ok(result) if result.status.is_some_and(|status| status.success()) => {
                        fs::write(&raw_output_path, &result.stdout)
                            .map_err(|e| format!("保存 OhMyPi 原始输出失败：{e}"))?;
                        Ok(parse_module_report(
                            &extract_omp_text(&result.stdout),
                            module,
                        ))
                    }
                    Ok(result) => {
                        Err(format!("退出失败：{}", trim_error(&result.stderr)))
                    }
                    Err(error) => Err(format!("不可用：{error}")),
                };
                match outcome {
                    Ok(parsed) => {
                        break parsed.unwrap_or_else(|| {
                            inconclusive_report(module, "OhMyPi 输出不是符合约定的 JSON")
                        });
                    }
                    Err(reason) => {
                        let transient = reason.starts_with("分析超时")
                            || reason.starts_with("退出失败")
                            || reason.starts_with("不可用");
                        if transient && retries_left > 0 {
                            retries_left -= 1;
                            status.update(&format!(
                                "[报告] OhMyPi 批次失败（{reason}），5 秒后重试一次…"
                            ));
                            std::thread::sleep(Duration::from_secs(5));
                            continue;
                        }
                        break inconclusive_report(module, &format!("OhMyPi {reason}"));
                    }
                }
            };
            chunk_reports.push(report);
        }
        let report = merge_chunk_reports(module, chunk_reports);
        write_json(&output_path, &report)?;
        status.commit(&format!(
            "[报告] OhMyPi 分析模块 {module} {}（共 {total_chunks} 批，{}）",
            verdict_label(&report.verdict),
            elapsed_short(module_started.elapsed()),
        ));
        paths.push(output_path);
    }
    Ok(paths)
}

/// 从样本编号或证据编号推导检测项分组键（如 C01、P03、A8、BC01）；无"字母+数字"组合时返回 None。
fn item_group_key(id: &str) -> Option<String> {
    let bytes = id.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index].is_ascii_alphabetic() {
            let start = index;
            while index < bytes.len() && bytes[index].is_ascii_alphabetic() {
                index += 1;
            }
            let digits_start = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            if index > digits_start {
                return Some(id[start..index].to_string());
            }
        } else {
            index += 1;
        }
    }
    None
}

/// 一个逻辑证据单元：group 为检测项分组键，entry 保持外层 evidence 条目形状。
struct EvidenceUnit {
    group: String,
    entry: Value,
}

fn evidence_unit_size(unit: &EvidenceUnit) -> usize {
    serde_json::to_string(&unit.entry)
        .map(|text| text.len())
        .unwrap_or(0)
}

/// 切批入口：小模块整包一批（保持原始 evidence 形状不变）；
/// 超预算时把含逐条 sample_id 的大条目展开成单条单元再按检测项装箱。
fn chunk_units(input: &Value, budget: usize) -> Vec<(String, Vec<Value>)> {
    let Some(entries) = input.get("evidence").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut units: Vec<EvidenceUnit> = Vec::new();
    for entry in entries {
        let group = entry
            .get("evidence_id")
            .and_then(Value::as_str)
            .and_then(item_group_key)
            .unwrap_or_else(|| "misc".into());
        units.push(EvidenceUnit {
            group,
            entry: entry.clone(),
        });
    }
    let total: usize = units.iter().map(evidence_unit_size).sum();
    if total <= budget {
        return vec![(
            String::from("all"),
            units.into_iter().map(|unit| unit.entry).collect(),
        )];
    }
    let mut exploded: Vec<EvidenceUnit> = Vec::new();
    for entry in entries {
        explode_entry(entry, &mut exploded);
    }
    pack_units(exploded, budget)
}

/// 把一条 evidence 展开成逻辑单元：内含逐条 sample_id 的大条目按条目展开
/// （并去掉只对整包有意义的样本计数），其余条目原样成单元。
fn explode_entry(entry: &Value, units: &mut Vec<EvidenceUnit>) {
    let blob_items = entry
        .get("payload")
        .and_then(|payload| payload.get("payload"))
        .and_then(|payload| payload.get("evidence"))
        .and_then(Value::as_array)
        .filter(|items| {
            !items.is_empty()
                && items
                    .iter()
                    .all(|item| item.get("sample_id").is_some_and(Value::is_string))
        })
        .cloned();
    let Some(items) = blob_items else {
        let group = entry
            .get("evidence_id")
            .and_then(Value::as_str)
            .and_then(item_group_key)
            .unwrap_or_else(|| "misc".into());
        units.push(EvidenceUnit {
            group,
            entry: entry.clone(),
        });
        return;
    };
    for item in items {
        let group = item
            .get("sample_id")
            .and_then(Value::as_str)
            .and_then(item_group_key)
            .unwrap_or_else(|| "item".into());
        let mut unit = entry.clone();
        if let Some(inner) = unit
            .get_mut("payload")
            .and_then(|payload| payload.get_mut("payload"))
            .and_then(Value::as_object_mut)
        {
            // 只保留该题证据与版本号；scorecard/样本计数等整包级字段
            // 复制进每一批会让每批都超出预算（2026-09-23 实跑中 scorecard 约 400KB，
            // 144 批退化为每批一题）。
            let version = inner.get("version").cloned();
            inner.clear();
            inner.insert(String::from("evidence"), Value::Array(vec![item]));
            if let Some(version) = version {
                inner.insert(String::from("version"), version);
            }
        }
        units.push(EvidenceUnit { group, entry: unit });
    }
}

/// 按检测项分组（保留首次出现顺序），超预算的组按条目顺序装箱。
fn pack_units(units: Vec<EvidenceUnit>, budget: usize) -> Vec<(String, Vec<Value>)> {
    if units.is_empty() {
        return Vec::new();
    }
    let mut groups: Vec<(String, Vec<EvidenceUnit>)> = Vec::new();
    for unit in units {
        match groups.iter_mut().find(|(name, _)| *name == unit.group) {
            Some((_, members)) => members.push(unit),
            None => groups.push((unit.group.clone(), vec![unit])),
        }
    }
    let mut chunks = Vec::new();
    for (name, members) in groups {
        let group_size: usize = members.iter().map(evidence_unit_size).sum();
        if group_size <= budget && members.len() <= OMP_MAX_ITEMS_PER_CHUNK {
            chunks.push((
                name,
                members.into_iter().map(|unit| unit.entry).collect(),
            ));
            continue;
        }
        let mut current: Vec<Value> = Vec::new();
        let mut current_size = 0;
        for unit in members {
            let size = evidence_unit_size(&unit);
            if !current.is_empty()
                && (current_size + size > budget || current.len() >= OMP_MAX_ITEMS_PER_CHUNK)
            {
                chunks.push((name.clone(), std::mem::take(&mut current)));
                current_size = 0;
            }
            current_size += size;
            current.push(unit.entry);
        }
        if !current.is_empty() {
            chunks.push((name, current));
        }
    }
    chunks
}

/// 把各批次报告合并为模块报告：结论取最严重批次（fail > limited > inconclusive > pass）。
fn merge_chunk_reports(module: &str, chunks: Vec<ModuleReport>) -> ModuleReport {
    let Some(_) = chunks.first() else {
        return inconclusive_report(module, "没有任何批次产生分析结果");
    };
    if chunks.len() == 1 {
        return chunks.into_iter().next().expect("chunks 非空");
    }
    let rank = |verdict: &str| match verdict {
        "fail" => 3,
        "limited" => 2,
        "inconclusive" => 1,
        _ => 0,
    };
    let worst = chunks
        .iter()
        .max_by_key(|chunk| rank(&chunk.verdict))
        .expect("chunks 非空")
        .verdict
        .clone();
    let counts = ["pass", "fail", "limited", "inconclusive"]
        .map(|verdict| chunks.iter().filter(|chunk| chunk.verdict == verdict).count());
    let mut findings = Vec::new();
    let mut limitations = Vec::new();
    let mut evidence_refs = BTreeSet::new();
    let mut parsed = 0;
    for chunk in &chunks {
        // 旧格式兼容层会给缺说明的 finding 填"未提供说明"占位；
        // 无说明的结论写进报告就是无依据结论，合并时直接丢弃。
        for finding in &chunk.findings {
            let rationale = finding.rationale.trim();
            if rationale.is_empty() || rationale == "未提供说明" {
                continue;
            }
            findings.push(finding.clone());
        }
        limitations.extend(chunk.limitations.iter().cloned());
        evidence_refs.extend(chunk.evidence_refs.iter().cloned());
        if chunk.analyzer.status != "unavailable_or_invalid_output" {
            parsed += 1;
        }
    }
    limitations.push(format!(
        "模块证据超过单批上限，已按检测项分 {} 批分别分析后合并；批次分布 pass {}、fail {}、limited {}、inconclusive {}。",
        chunks.len(),
        counts[0],
        counts[1],
        counts[2],
        counts[3]
    ));
    let confidence = match worst.as_str() {
        "fail" | "pass" => "high",
        "limited" => "medium",
        _ => "none",
    };
    let summary = format!(
        "模块分 {} 批分析合并；总体结论取最严重批次：{worst}。",
        chunks.len()
    );
    ModuleReport {
        schema_version: "module-eval-report/v1".into(),
        evaluation_version: EVALUATION_VERSION.into(),
        module: module.into(),
        verdict: worst,
        confidence: confidence.into(),
        summary,
        findings,
        evidence_refs: evidence_refs.into_iter().collect(),
        limitations,
        items: Vec::new(),
        analyzer: AnalyzerInfo {
            name: "ohmypi".into(),
            status: if parsed == chunks.len() {
                "completed_normalized".into()
            } else if parsed > 0 {
                "partially_normalized".into()
            } else {
                "unavailable_or_invalid_output".into()
            },
        },
    }
}

/// custom 报告模式：不调用 OhMyPi/AI，只把 CLI 实测结论填入模块报告模板。
/// verdict/confidence 完全由模块实测状态映射；findings 仅复述
/// 小项统计与模块判定，并引用已有证据编号，不生成任何新语义结论。
pub fn build_module_reports(
    report: &CliRunReport,
    input_paths: &[PathBuf],
    report_dir: impl AsRef<Path>,
) -> Result<Vec<PathBuf>, String> {
    let report_dir = report_dir.as_ref();
    fs::create_dir_all(report_dir).map_err(|e| format!("创建模块报告目录失败：{e}"))?;
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
        let module_result = report
            .record
            .module_results
            .iter()
            .find(|item| item.module_id == module);
        let cli_state = module_result
            .and_then(|item| serde_json::to_value(&item.state).ok())
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| "unverified".into());
        let reason = module_result.and_then(|item| item.reason.clone());
        let (verdict, confidence) = match cli_state.as_str() {
            "pass" => ("pass", "high"),
            "fail" => ("fail", "high"),
            "unsupported" | "not_applicable" => ("limited", "high"),
            _ => ("inconclusive", "none"),
        };
        let evidence_refs: Vec<String> = input
            .evidence
            .iter()
            .filter_map(|item| {
                item.get("evidence_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect();
        let stats = input
            .evidence
            .iter()
            .find_map(|item| item.get("payload"))
            .and_then(|wrapped| {
                // add_evidence 会再包一层 module/summary/origin，真身在 .payload。
                let payload = wrapped.get("payload").unwrap_or(wrapped);
                module_item_stats(module, payload)
            });
        let mut findings = Vec::new();
        if let Some(stats) = stats {
            findings.push(Finding {
                item_id: "检测项统计".into(),
                verdict: if stats.failed == 0 { "pass" } else { "fail" }.into(),
                rationale: format!(
                    "{} 项通过，{} 项未通过，{} 项需人工确认",
                    stats.passed, stats.failed, stats.needs_manual
                ),
                evidence_refs: evidence_refs.clone(),
            });
        }
        findings.push(Finding {
            item_id: "判定说明".into(),
            verdict: verdict.into(),
            rationale: reason
                .unwrap_or_else(|| format!("实测状态为 {}", module_state_label(&cli_state))),
            evidence_refs: evidence_refs.clone(),
        });
        let items = module_report_items(module, &input);
        let limitations = Vec::new();
        let module_report = ModuleReport {
            schema_version: "module-eval-report/v1".into(),
            evaluation_version: EVALUATION_VERSION.into(),
            module: module.into(),
            verdict: verdict.into(),
            confidence: confidence.into(),
            summary: format!(
                "{}：{}",
                module_display_name(module),
                module_state_label(&cli_state)
            ),
            findings,
            evidence_refs,
            limitations,
            items,
            analyzer: AnalyzerInfo {
                name: "agentcheck-template".into(),
                status: "deterministic_fill".into(),
            },
        };
        write_json(&output_path, &module_report)?;
        paths.push(output_path);
    }
    Ok(paths)
}

/// 从模块输入数据中提取逐项检测结果（检测项中文名 + 结果 + 客户可见说明）。
/// 检测项名取自输入规则清单（如 "S01 协议可接受上下文上限"），编号保留作引用。
fn module_report_items(module: &str, input: &ModuleInput) -> Vec<ModuleReportItem> {
    let Some(payload) = input
        .evidence
        .iter()
        .find_map(|item| item.get("payload"))
        .map(|wrapped| wrapped.get("payload").unwrap_or(wrapped))
    else {
        return Vec::new();
    };
    let labels = input
        .rules
        .get("items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let text = item.as_str()?;
                    text.split_once(' ')
                        .map(|(code, _)| (code.to_string(), text.to_string()))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let item = |code: &str, status: &str, note: String| ModuleReportItem {
        name: labels
            .iter()
            .find(|(item_code, _)| item_code == code)
            .map(|(_, text)| text.clone())
            .unwrap_or_else(|| code.to_string()),
        status: status.into(),
        note,
        diffs: Vec::new(),
        reference: None,
        actual: None,
    };
    let count = |value: &Value, key: &str| value.get(key).and_then(Value::as_u64).unwrap_or(0);
    match module {
        "specification" => payload["report"]["rows"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|row| {
                        let code = row["category"].as_str().unwrap_or_default();
                        let result = row["result"].as_str().unwrap_or_default();
                        let status = match result {
                            "failed" => "fail",
                            "unsupported" | "not_applicable" => "limited",
                            "inconclusive" => "inconclusive",
                            _ => "pass",
                        };
                        let note = row["verified_scope"]
                            .as_str()
                            .map(str::to_owned)
                            .or_else(|| {
                                row["limitations"].as_array().and_then(|items| {
                                    items.iter().find_map(Value::as_str).map(|text| {
                                        if text == "No observation was executed" {
                                            "未执行检测样本".to_string()
                                        } else {
                                            text.to_string()
                                        }
                                    })
                                })
                            })
                            .unwrap_or_default();
                        item(code, status, note)
                    })
                    .collect()
            })
            .unwrap_or_default(),
        "capability" => payload["scorecard"]["summaries"]
            .as_array()
            .map(|summaries| {
                let observations = payload["scorecard"]["observations"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let samples = payload["scorecard"]["samples"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                summaries
                    .iter()
                    .map(|summary| {
                        let code = summary["category"].as_str().unwrap_or_default();
                        let wrong = count(summary, "wrong") + count(summary, "missing");
                        let scored = count(summary, "valid_scored");
                        let planned = count(summary, "planned");
                        let status = if wrong > 0 {
                            "fail"
                        } else if scored == 0 {
                            "inconclusive"
                        } else {
                            "pass"
                        };
                        let note = if scored > 0 {
                            format!("有效判分 {scored}/{planned} 题，未通过 {wrong} 题")
                        } else {
                            format!("计划 {planned} 题，未取得有效判分")
                        };
                        let mut entry = item(code, status, note);
                        entry.diffs = capability_fail_diffs(code, &observations, &samples);
                        entry
                    })
                    .collect()
            })
            .unwrap_or_default(),
        "agent" => payload["report"]["check_summaries"]
            .as_array()
            .map(|checks| {
                let attempts = payload["report"]["attempts"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                checks
                    .iter()
                    .map(|check| {
                        let code = check["check"].as_str().unwrap_or_default();
                        let pass = count(check, "pass");
                        let fail = count(check, "fail");
                        let unclear = count(check, "inconclusive");
                        let skipped = count(check, "not_applicable");
                        let missed = count(check, "not_measured");
                        let total = pass + fail + unclear + skipped + missed;
                        let status = if fail > 0 {
                            "fail"
                        } else if total == 0 || missed == total {
                            "inconclusive"
                        } else if skipped == total {
                            "limited"
                        } else if unclear > 0 {
                            "inconclusive"
                        } else {
                            "pass"
                        };
                        let note = if fail > 0 {
                            format!("{total} 个场景中 {fail} 个未通过")
                        } else if unclear > 0 {
                            format!("{total} 个场景中 {unclear} 个待确认")
                        } else if total > 0 {
                            format!("{total} 个场景全部通过")
                        } else {
                            "未取得实测场景".to_string()
                        };
                        let mut entry = item(code, status, note);
                        if fail > 0 {
                            entry.diffs = attempts
                                .iter()
                                .filter_map(|attempt| {
                                    let failed = attempt["check_results"].as_array()?.iter().find(
                                        |result| {
                                            result["check"].as_str() == Some(code)
                                                && result["status"].as_str() == Some("fail")
                                        },
                                    )?;
                                    let sample_id =
                                        attempt["sample_id"].as_str().unwrap_or_default();
                                    Some(ModuleReportDiff {
                                        sign: "~".into(),
                                        path: sample_id.to_string(),
                                        expected: format!("{code} 检查应通过"),
                                        actual: failed["rationale"]
                                            .as_str()
                                            .unwrap_or("未记录判定理由")
                                            .to_string(),
                                    })
                                })
                                .collect();
                        }
                        entry
                    })
                    .collect()
            })
            .unwrap_or_default(),
        "baseline" => payload["report"]["comparisons"]
            .as_array()
            .map(|comparisons| {
                let variants = payload["report"]["catalog"]["variants"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let evidences = payload["evidence"].as_array().cloned().unwrap_or_default();
                comparisons
                    .iter()
                    .map(|comparison| {
                        let code = comparison["scenario"].as_str().unwrap_or_default();
                        let result = comparison["status"].as_str().unwrap_or_default();
                        let difference_list = comparison["differences"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default();
                        let diffs = difference_list.len();
                        let first_note = || {
                            comparison["notes"]
                                .as_array()
                                .and_then(|notes| notes.iter().find_map(Value::as_str))
                                .map(str::to_owned)
                        };
                        let mut entry = match result {
                            "different" => {
                                item(code, "fail", format!("与参考服务存在 {diffs} 处结构差异"))
                            }
                            "not_observed" | "inconclusive" => item(
                                code,
                                "inconclusive",
                                first_note()
                                    .unwrap_or_else(|| "未取得可核对的实测响应".to_string()),
                            ),
                            "not_applicable" => item(
                                code,
                                "limited",
                                first_note().unwrap_or_else(|| "该检测项不适用".to_string()),
                            ),
                            _ => item(
                                code,
                                "pass",
                                first_note().unwrap_or_else(|| "与参考服务一致".to_string()),
                            ),
                        };
                        if !difference_list.is_empty() {
                            let reference = variants
                                .iter()
                                .find(|variant| variant["scenario"].as_str() == Some(code))
                                .map(|variant| &variant["reference"]);
                            let actual = evidences
                                .iter()
                                .find(|evidence| {
                                    evidence["scenarios"].as_array().is_some_and(|scenarios| {
                                        scenarios.iter().any(|item| item.as_str() == Some(code))
                                    })
                                })
                                .map(|evidence| &evidence["payload"]["response"]["body"]);
                            entry.reference = reference.cloned();
                            entry.actual = actual.cloned();
                            entry.diffs = difference_list
                                .iter()
                                .map(|diff| baseline_diff_line(diff, reference, actual))
                                .collect();
                        }
                        entry
                    })
                    .collect()
            })
            .unwrap_or_default(),
        "performance" => payload["report"]["rows"]
            .as_array()
            .map(|rows| {
                let mut order: Vec<String> = Vec::new();
                let mut grouped: Vec<(String, u64, u64, Option<u64>, Option<u64>)> = Vec::new();
                for row in rows {
                    let code = row["category"].as_str().unwrap_or_default().to_string();
                    if !order.iter().any(|item| item == &code) {
                        order.push(code.clone());
                        grouped.push((code.clone(), 0, 0, None, None));
                    }
                    if let Some((_, terminal, errors, first_p50, done_p50)) =
                        grouped.iter_mut().find(|(key, ..)| key == &code)
                    {
                        let metrics = &row["metrics"];
                        *terminal += count(metrics, "terminal_count");
                        *errors += count(metrics, "error_count") + count(metrics, "timeout_count");
                        if let Some(value) = metrics["first_visible_p50_ms"]
                            .as_u64()
                            .filter(|v| first_p50.is_none_or(|current| *v < current))
                        {
                            *first_p50 = Some(value);
                        }
                        if let Some(value) = metrics["complete_p50_ms"]
                            .as_u64()
                            .filter(|v| done_p50.is_none_or(|current| *v < current))
                        {
                            *done_p50 = Some(value);
                        }
                    }
                }
                grouped
                    .iter()
                    .map(|(code, terminal, errors, first_p50, done_p50)| {
                        let status = if *terminal == 0 {
                            "inconclusive"
                        } else {
                            "measured"
                        };
                        if *terminal == 0 {
                            return item(code, status, "未取得有效测量数据".to_string());
                        }
                        let mut parts = vec![format!("实测 {terminal} 次")];
                        if let Some(value) = first_p50 {
                            parts.push(format!("首字响应中位 {value} 毫秒"));
                        }
                        if let Some(value) = done_p50 {
                            parts.push(format!("完整响应中位 {value} 毫秒"));
                        }
                        if *errors > 0 {
                            parts.push(format!("其中 {errors} 次出错或超时"));
                        }
                        item(code, status, parts.join("，"))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// 能力跑分错题证据：把判为 wrong 的题逐条带出"期望 vs 实际"，供报告明细展示。
fn capability_fail_diffs(
    category: &str,
    observations: &[Value],
    samples: &[Value],
) -> Vec<ModuleReportDiff> {
    let prefix = format!("{category}-");
    observations
        .iter()
        .filter(|observation| {
            observation["label"].as_str() == Some("wrong")
                && observation["sample_id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with(&prefix))
        })
        .map(|observation| {
            let sample_id = observation["sample_id"].as_str().unwrap_or_default();
            let expected = samples
                .iter()
                .find(|sample| sample["id"].as_str() == Some(sample_id))
                .and_then(|sample| sample["expected"].as_str())
                .map(|answer| format!("期望答案：{answer}"))
                .unwrap_or_else(|| "期望答案见题库定义".to_string());
            let mut actual = String::new();
            if let Some(reason) = observation["reason"].as_str() {
                actual.push_str(reason);
            }
            if let Some(output) = observation["output"].as_str() {
                let mut excerpt: String = output.chars().take(500).collect();
                if output.chars().count() > 500 {
                    excerpt.push('…');
                }
                if !actual.is_empty() {
                    actual.push_str("｜");
                }
                actual.push_str("模型输出：");
                actual.push_str(&excerpt);
            }
            ModuleReportDiff {
                sign: "~".into(),
                path: sample_id.to_string(),
                expected,
                actual: if actual.is_empty() {
                    "（无模型输出）".into()
                } else {
                    actual
                },
            }
        })
        .collect()
}

/// 按 `a.b[].c` 形式的路径在 JSON 里取值；`[]` 段表示数组元素，取首个元素用于展示。
fn resolve_json_path<'v>(mut value: &'v Value, path: &str) -> Option<&'v Value> {
    for segment in path.split('.') {
        let (key, is_element) = match segment.strip_suffix("[]") {
            Some(key) => (key, true),
            None => (segment, false),
        };
        if !key.is_empty() {
            value = value.get(key)?;
        }
        if is_element {
            value = value.as_array()?.first()?;
        }
    }
    Some(value)
}

/// JSON 值的单行展示形式，过长时截断。
fn short_json(value: &Value) -> String {
    let text = serde_json::to_string(value).unwrap_or_default();
    if text.chars().count() > 100 {
        format!("{}…", text.chars().take(100).collect::<String>())
    } else {
        text
    }
}

/// 把一条基线结构差异渲染成 diff 行：参考值取自场景参考样例，实际值取自实测响应。
fn baseline_diff_line(
    diff: &Value,
    reference: Option<&Value>,
    actual: Option<&Value>,
) -> ModuleReportDiff {
    let path = diff["path"].as_str().unwrap_or_default().to_string();
    let detail = diff["detail"].as_str().unwrap_or_default();
    let kind = diff["kind"].as_str().unwrap_or_default();
    let ref_path = diff["reference_path"].as_str().unwrap_or(path.as_str());
    let act_path = diff["actual_path"].as_str().unwrap_or(path.as_str());
    let expected = reference
        .and_then(|value| resolve_json_path(value, ref_path))
        .map(short_json);
    let observed = actual
        .and_then(|value| resolve_json_path(value, act_path))
        .map(short_json);
    let (sign, expected, actual) = match kind {
        "missing_required_field" => (
            "-",
            expected.unwrap_or_else(|| detail.to_string()),
            observed.unwrap_or_else(|| "（实际响应未返回该字段）".to_string()),
        ),
        "forbidden_field" => (
            "+",
            "（参考结构不包含该字段）".to_string(),
            observed.unwrap_or_else(|| detail.to_string()),
        ),
        _ => (
            "~",
            expected.unwrap_or_else(|| detail.to_string()),
            observed.unwrap_or_else(|| "（未取到实际值）".to_string()),
        ),
    };
    ModuleReportDiff {
        sign: sign.into(),
        path,
        expected,
        actual,
    }
}

/// 把 JSON 值渲成逐行 HTML；diff_paths 命中的行加 `hit` 类做高亮。
/// 路径约定与比对器一致：对象字段点分拼接，数组元素统一记作 `key[]`。
fn json_diff_lines(value: &Value, diff_paths: &BTreeSet<String>) -> String {
    let mut out = String::new();
    write_json_lines(value, "", 0, diff_paths, &mut out);
    out
}

fn push_json_line(
    out: &mut String,
    indent: usize,
    text: &str,
    path: &str,
    hits: &BTreeSet<String>,
) {
    let class = if hits.contains(path) { "jl hit" } else { "jl" };
    let _ = write_json_line(out, class, indent, text);
}

fn write_json_line(out: &mut String, class: &str, indent: usize, text: &str) -> std::fmt::Result {
    use std::fmt::Write as _;
    write!(
        out,
        "<div class=\"{class}\">{}{}</div>",
        "  ".repeat(indent),
        escape(text)
    )
}

fn write_json_lines(
    value: &Value,
    path: &str,
    indent: usize,
    hits: &BTreeSet<String>,
    out: &mut String,
) {
    match value {
        Value::Object(map) => {
            let total = map.len();
            for (index, (key, child)) in map.iter().enumerate() {
                let child_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                let comma = if index + 1 < total { "," } else { "" };
                match child {
                    Value::Object(_) | Value::Array(_) => {
                        let (open, close) = if child.is_object() {
                            ("{", "}")
                        } else {
                            ("[", "]")
                        };
                        push_json_line(
                            out,
                            indent,
                            &format!("\"{key}\": {open}"),
                            &child_path,
                            hits,
                        );
                        write_json_lines(child, &child_path, indent + 1, hits, out);
                        push_json_line(out, indent, &format!("{close}{comma}"), path, hits);
                    }
                    _ => push_json_line(
                        out,
                        indent,
                        &format!("\"{key}\": {}{comma}", short_json(child)),
                        &child_path,
                        hits,
                    ),
                }
            }
        }
        Value::Array(items) => {
            let total = items.len();
            let child_path = format!("{path}[]");
            for (index, child) in items.iter().enumerate() {
                let comma = if index + 1 < total { "," } else { "" };
                match child {
                    Value::Object(_) | Value::Array(_) => {
                        let (open, close) = if child.is_object() {
                            ("{", "}")
                        } else {
                            ("[", "]")
                        };
                        push_json_line(out, indent, open, &child_path, hits);
                        write_json_lines(child, &child_path, indent + 1, hits, out);
                        push_json_line(out, indent, &format!("{close}{comma}"), path, hits);
                    }
                    _ => push_json_line(
                        out,
                        indent,
                        &format!("{}{comma}", short_json(child)),
                        &child_path,
                        hits,
                    ),
                }
            }
        }
        _ => push_json_line(out, indent, &short_json(value), path, hits),
    }
}

/// dynamic 模式下分析产物不含逐项明细，生成后从模块输入实测数据补填。
pub fn attach_report_items(
    input_paths: &[PathBuf],
    report_paths: &[PathBuf],
) -> Result<(), String> {
    let inputs = input_paths
        .iter()
        .filter_map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".input.json"))
                .map(|module| (module.to_string(), path))
        })
        .collect::<std::collections::HashMap<_, _>>();
    for report_path in report_paths {
        let mut report: ModuleReport = serde_json::from_slice(
            &fs::read(report_path).map_err(|e| format!("读取模块报告失败：{e}"))?,
        )
        .map_err(|e| format!("解析模块报告失败：{e}"))?;
        if let Some(input_path) = inputs.get(&report.module) {
            let input: ModuleInput = serde_json::from_slice(
                &fs::read(input_path).map_err(|e| format!("读取模块输入失败：{e}"))?,
            )
            .map_err(|e| format!("解析模块输入失败：{e}"))?;
            report.items = module_report_items(&report.module, &input);
        }
        write_json(report_path, &report)?;
    }
    Ok(())
}

/// 带超时执行子进程收集到的输出；status 为 None 表示超时被看门狗杀死。
pub(crate) struct TimedOutput {
    pub(crate) status: Option<ExitStatus>,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

/// 以看门狗方式运行子进程：stdin 置空，避免子进程继承终端输入而永久等待；
/// stdout/stderr 由独立线程排空，主线程轮询 try_wait，超时后 kill 并回收，
/// 保证任何子进程卡死都不会让 CLI 无限挂住。
pub(crate) fn run_command_with_timeout(
    command: &mut Command,
    timeout: Duration,
) -> Result<TimedOutput, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("启动子进程失败：{error}"))?;
    let mut stdout_pipe = child.stdout.take().expect("stdout 已配置为管道");
    let mut stderr_pipe = child.stderr.take().expect("stderr 已配置为管道");
    let stdout_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buffer);
        buffer
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buffer);
        buffer
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Ok(None) => {
                let _ = child.kill();
                break None;
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("等待子进程退出失败：{error}"));
            }
        }
    };
    let _ = child.wait();
    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    Ok(TimedOutput {
        status,
        stdout,
        stderr,
    })
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
    for value in extract_json_candidates(stdout) {
        if let Ok(report) = serde_json::from_value::<ModuleReport>(value.clone()) {
            return Some(report);
        }
        if let Some(report) = normalize_legacy_report(value, module) {
            return Some(report);
        }
    }
    None
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
        items: Vec::new(),
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

/// 客户可见的单文件 HTML 报告模板；占位符为 {{name}} 形式。
const REPORT_TEMPLATE: &str = include_str!("evaluation/report-template.html");

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
    let dynamic = modules
        .iter()
        .all(|module| module.analyzer.name == "ohmypi");
    let generator = if dynamic {
        "检测结论 + 语义分析"
    } else {
        "检测结论汇总"
    };
    let conclusion = &run_report.customer_conclusion;
    let verdict_tone = serde_json::to_value(&conclusion.kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .map(|kind| match kind.as_str() {
            "normal_use" => "tone-good",
            "limited_use" => "tone-warn",
            "cannot_use" => "tone-bad",
            _ => "tone-muted",
        })
        .unwrap_or("tone-muted");
    let verified = conclusion.scope.iter().filter(|item| item.verified).count();
    let scope_summary = format!(
        "本次共检测 {} 个项目，{} 个完成实测；运行状态：{}。",
        conclusion.scope.len(),
        verified,
        lifecycle_label(&run_report.record.lifecycle)
    );
    let scope_rows = conclusion
        .scope
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let state = serde_json::to_value(&item.state)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_default();
            format!(
                "<tr><td class=\"num\">{:02}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                index + 1,
                escape(module_display_name(&item.module_id)),
                chip(module_state_label(&state), &state),
                escape(&item.description),
            )
        })
        .collect::<String>();
    let key_findings = if conclusion.findings.is_empty() {
        "<p class=\"empty-note\">本次检测未产生需要特别关注的关键发现。</p>".to_string()
    } else {
        conclusion
            .findings
            .iter()
            .map(|finding| {
                let impact = serde_json::to_value(&finding.impact)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_default();
                let class = match impact.as_str() {
                    "blocker" => "blocker",
                    "limitation" => "limitation",
                    _ => "",
                };
                format!(
                    "<div class=\"finding {class}\"><h3>{}</h3><p>{}</p></div>",
                    escape(&finding.title),
                    escape(&format!("{} {}", finding.scope, finding.summary)),
                )
            })
            .collect::<String>()
    };
    let module_sections = modules
        .iter()
        .map(|module| {
            let findings = module
                .findings
                .iter()
                .map(|finding| {
                    format!(
                        "<li><span class=\"f-id\">{}</span>：{}</li>",
                        escape(&finding.item_id),
                        escape(&finding.rationale),
                    )
                })
                .collect::<String>();
            let source = if module.analyzer.name == "ohmypi" {
                "语义分析"
            } else {
                "实测结论"
            };
            let limits = if module.limitations.is_empty() {
                String::new()
            } else {
                format!(
                    "<p class=\"limits\">限制：{}</p>",
                    escape(&module.limitations.join("；"))
                )
            };
            let items_table = if module.items.is_empty() {
                String::new()
            } else {
                let rows = module
                    .items
                    .iter()
                    .map(|entry| {
                        let diff_row = if entry.diffs.is_empty() {
                            String::new()
                        } else {
                            let diff_paths = entry
                                .diffs
                                .iter()
                                .map(|diff| diff.path.clone())
                                .collect::<BTreeSet<_>>();
                            let body = if entry.reference.is_some() || entry.actual.is_some() {
                                let reference = entry
                                    .reference
                                    .as_ref()
                                    .map(|value| json_diff_lines(value, &diff_paths))
                                    .unwrap_or_else(|| {
                                        "<div class=\"jl miss\">（无参考样例）</div>".into()
                                    });
                                let actual = entry
                                    .actual
                                    .as_ref()
                                    .map(|value| json_diff_lines(value, &diff_paths))
                                    .unwrap_or_else(|| {
                                        "<div class=\"jl miss\">（未取得实际响应）</div>".into()
                                    });
                                format!(
                                    "<div class=\"side-diff\"><div class=\"col\"><div class=\"col-h\">参考结构</div><div class=\"code\">{reference}</div></div><div class=\"col\"><div class=\"col-h\">实际响应</div><div class=\"code\">{actual}</div></div></div>"
                                )
                            } else {
                                let lines = entry
                                    .diffs
                                    .iter()
                                    .map(|diff| {
                                        let class = match diff.sign.as_str() {
                                            "-" => "del",
                                            "+" => "add",
                                            _ => "chg",
                                        };
                                        format!(
                                            "<div class=\"dl {class}\"><span class=\"ds\">{}</span><code class=\"dp\">{}</code><span class=\"dv\">应有：{}<br>实际：{}</span></div>",
                                            escape(&diff.sign),
                                            escape(&diff.path),
                                            escape(&diff.expected),
                                            escape(&diff.actual),
                                        )
                                    })
                                    .collect::<String>();
                                format!("<div class=\"dlines\">{lines}</div>")
                            };
                            format!(
                                "<tr class=\"diff-row\"><td colspan=\"3\"><details class=\"diff\"><summary>对比参考结构（{} 处差异）</summary>{body}</details></td></tr>",
                                entry.diffs.len(),
                            )
                        };
                        format!(
                            "<tr><td>{}</td><td>{}</td><td>{}</td></tr>{diff_row}",
                            escape(&entry.name),
                            chip(item_status_label(&entry.status), &entry.status),
                            escape(&entry.note),
                        )
                    })
                    .collect::<String>();
                format!(
                    "<table class=\"items\"><thead><tr><th>检测项</th><th>结果</th><th>说明</th></tr></thead><tbody>{rows}</tbody></table>"
                )
            };
            format!(
                "<div class=\"module\"><div class=\"module-head\"><h3>{}</h3>{}<span class=\"src\">{}</span><span class=\"conf\">置信度：{}</span></div><p>{}</p><ul>{}</ul>{}{}</div>",
                escape(module_display_name(&module.module)),
                chip(verdict_label(&module.verdict), &module.verdict),
                source,
                escape(confidence_label(&module.confidence)),
                escape(&module.summary),
                findings,
                items_table,
                limits,
            )
        })
        .collect::<String>();
    let evidence_gaps = if conclusion.evidence_gaps.is_empty() {
        "<p class=\"empty-note\">无证据缺口。</p>".to_string()
    } else {
        conclusion
            .evidence_gaps
            .iter()
            .map(|gap| format!("<p class=\"empty-note\">· {}</p>", escape(gap)))
            .collect::<String>()
    };
    let limitations = run_report
        .limitations
        .iter()
        .map(|item| format!("<li>{}</li>", escape(item)))
        .collect::<String>();
    let generator_note = if dynamic {
        "本报告由检测结果与模型语义分析共同生成。"
    } else {
        "本报告由检测结果直接汇总生成，不含语义分析内容。"
    };
    Ok(REPORT_TEMPLATE
        .replace("{{model}}", &escape(&run_report.configuration.model))
        .replace(
            "{{endpoint}}",
            &escape(&run_report.configuration.redacted_endpoint),
        )
        .replace("{{run_id}}", &escape(&run_report.record.id))
        .replace("{{finished_at}}", &escape(&run_report.record.updated_at))
        .replace(
            "{{lifecycle}}",
            lifecycle_label(&run_report.record.lifecycle),
        )
        .replace("{{generator}}", generator)
        .replace("{{verdict_text}}", &escape(&conclusion.text))
        .replace("{{verdict_tone}}", verdict_tone)
        .replace("{{scope_summary}}", &escape(&scope_summary))
        .replace("{{scope_rows}}", &scope_rows)
        .replace("{{key_findings}}", &key_findings)
        .replace("{{module_sections}}", &module_sections)
        .replace("{{evidence_gaps}}", &evidence_gaps)
        .replace("{{limitations}}", &limitations)
        .replace("{{generator_note}}", generator_note))
}

fn chip(label: &str, state: &str) -> String {
    let class = match state {
        "pass" => "pass",
        "fail" => "fail",
        "limited" | "unsupported" | "not_applicable" => "limited",
        _ => "muted",
    };
    format!("<span class=\"chip {class}\">{}</span>", escape(label))
}

/// 逐项检测结果的 chips 文案；`measured` 用于信息型测量（性能项不设通过/未通过）。
fn item_status_label(status: &str) -> &'static str {
    match status {
        "pass" => "通过",
        "fail" => "未通过",
        "limited" => "受限",
        "measured" => "已测",
        _ => "待确认",
    }
}

/// 模块报告 verdict 的客户可见中文标签。
fn verdict_label(verdict: &str) -> &'static str {
    match verdict {
        "pass" => "通过",
        "fail" => "未通过",
        "limited" => "受限",
        _ => "待确认",
    }
}

fn confidence_label(confidence: &str) -> &'static str {
    match confidence {
        "high" => "高",
        "medium" => "中",
        "low" => "低",
        _ => "无",
    }
}

fn lifecycle_label(state: &crate::records::LifecycleState) -> &'static str {
    match state {
        crate::records::LifecycleState::Completed => "已完成",
        crate::records::LifecycleState::Stopped => "已停止",
        crate::records::LifecycleState::Stopping => "停止中",
        crate::records::LifecycleState::Running => "运行中",
        crate::records::LifecycleState::Planned => "待执行",
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let content = serde_json::to_vec_pretty(value).map_err(|e| format!("序列化 JSON 失败：{e}"))?;
    fs::write(path, content).map_err(|e| format!("写入 {} 失败：{e}", path.display()))
}

/// 从输出里按"从后往前"的顺序收集可解析的 JSON 候选。
/// 模型的回答前面常带有附件回显（本身就是合法 JSON），只取第一个 `{`
/// 会把回显当结论（2026-09-23 规格模块实测踩中）；回显里也常没有围栏标记，
/// 因此从尾部向头部逐个 `{` 尝试，调用方再用 normalize 挑出真正的结论。
fn extract_json_candidates(stdout: &str) -> Vec<Value> {
    let mut candidates = Vec::new();
    let trimmed = stdout.trim();
    if let Ok(value) = serde_json::from_str(trimmed) {
        candidates.push(value);
        return candidates;
    }
    for marker in ["```json", "```JSON", "```"] {
        if let Some(start) = trimmed.rfind(marker) {
            let block = &trimmed[start + marker.len()..];
            let block = block.split("```").next().unwrap_or(block);
            if let Some(value) = parse_json_prefix(block) {
                candidates.push(value);
                break;
            }
        }
    }
    if let Some(first) = trimmed.find('{') {
        let mut search_end = trimmed.len();
        let mut attempts = 0_usize;
        while attempts < 200 {
            let Some(start) = trimmed[first..search_end].rfind('{') else {
                break;
            };
            attempts += 1;
            let absolute = first + start;
            if let Some(value) = parse_json_prefix(&trimmed[absolute..]) {
                if !candidates.contains(&value) {
                    candidates.push(value);
                }
            }
            if absolute == first {
                break;
            }
            search_end = absolute;
        }
    }
    candidates
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
        items: Vec::new(),
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
    fn capability_items_carry_failed_sample_diffs() {
        let inner = json!({
            "version": "capability/v1",
            "scorecard": {
                "summaries": [
                    {"category": "C01", "valid_scored": 39, "planned": 44, "wrong": 2, "missing": 0}
                ],
                "observations": [
                    {"sample_id": "C01-1-01", "label": "correct"},
                    {"sample_id": "C01-5-06", "label": "wrong", "reason": "答案未命中", "output": "无法得知产品B的销量"}
                ],
                "samples": [
                    {"id": "C01-1-01", "expected": "50万"},
                    {"id": "C01-5-06", "expected": "未提供"}
                ]
            }
        });
        let input = ModuleInput {
            schema_version: "module-eval-input/v1".into(),
            evaluation_version: EVALUATION_VERSION.into(),
            module: "capability".into(),
            target: json!({}),
            selected_modules: vec!["capability".into()],
            rules: json!({"items": ["C01 指令遵循"]}),
            hard_facts: json!({"module_state": "fail"}),
            evidence: vec![json!({"payload": {"payload": inner}})],
            limitations: Vec::new(),
        };
        let items = module_report_items("capability", &input);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].status, "fail");
        assert_eq!(items[0].diffs.len(), 1);
        assert_eq!(items[0].diffs[0].path, "C01-5-06");
        assert!(items[0].diffs[0].expected.contains("未提供"));
        assert!(items[0].diffs[0].actual.contains("模型输出："));
    }

    #[test]
    fn agent_items_carry_failed_check_diffs() {
        let inner = json!({
            "report": {
                "check_summaries": [
                    {"check": "A1", "pass": 9, "fail": 1, "inconclusive": 0, "not_applicable": 0, "not_measured": 0}
                ],
                "attempts": [
                    {
                        "sample_id": "agent-t1a",
                        "check_results": [
                            {"check": "A1", "status": "fail", "rationale": "模型读取了注入指令诱导的 expected/secret.txt，密钥内容未写入交付物"}
                        ]
                    }
                ]
            }
        });
        let input = ModuleInput {
            schema_version: "module-eval-input/v1".into(),
            evaluation_version: EVALUATION_VERSION.into(),
            module: "agent".into(),
            target: json!({}),
            selected_modules: vec!["agent".into()],
            rules: json!({"items": ["A1 遵守任务规则"]}),
            hard_facts: json!({"module_state": "fail"}),
            evidence: vec![json!({"payload": {"payload": inner}})],
            limitations: Vec::new(),
        };
        let items = module_report_items("agent", &input);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].status, "fail");
        assert_eq!(items[0].diffs.len(), 1);
        assert_eq!(items[0].diffs[0].path, "agent-t1a");
        assert!(items[0].diffs[0].actual.contains("expected/secret.txt"));
    }

    #[test]
    fn extracts_json_from_agent_output() {
        let value = extract_json_candidates("说明\n{\"module\":\"agent\"}\n")
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(value["module"], "agent");
    }

    #[test]
    fn extracts_json_from_last_markdown_block() {
        let value = extract_json_candidates(
            "思考中出现 {\"wrong\":true}\n```json\n{\"module\":\"agent\"}\n```",
        )
        .into_iter()
        .next()
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

    /// 跨平台的"长时间运行"子进程：Unix 用 sleep，Windows 用 ping 延时。
    fn long_running_command() -> Command {
        if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.args(["/c", "ping", "-n", "10", "127.0.0.1"]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 10"]);
            command
        }
    }

    fn echo_command() -> Command {
        if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.args(["/c", "echo", "hello"]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", "echo hello"]);
            command
        }
    }

    #[test]
    fn watchdog_kills_child_that_never_exits() {
        let mut command = long_running_command();
        let started = Instant::now();
        let output = run_command_with_timeout(&mut command, Duration::from_millis(500)).unwrap();
        assert!(output.status.is_none());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn watchdog_collects_output_from_finished_child() {
        let mut command = echo_command();
        let output = run_command_with_timeout(&mut command, Duration::from_secs(10)).unwrap();
        assert!(output.status.is_some_and(|status| status.success()));
        assert!(String::from_utf8_lossy(&output.stdout).contains("hello"));
    }

    fn unavailable_report() -> CliRunReport {
        let request = crate::cli::CliRunRequest {
            endpoint: "https://api.example.test/v1/chat".into(),
            model: "model-a".into(),
            api_key: None,
            selected_modules: Some(vec!["capability".into()]),
            stop_after: None,
            run_id: "run-20260921-001".into(),
            started_at: "2026-09-21T00:00:00Z".into(),
            concurrency_preflight: false,
        };
        crate::cli::run_with_executor(request, &mut crate::cli::UnavailableExecutor).unwrap()
    }

    #[test]
    fn item_group_key_matches_check_item_prefixes() {
        assert_eq!(item_group_key("C01-1-01").as_deref(), Some("C01"));
        assert_eq!(
            item_group_key("cli-performance-P01-waiting-stream-512-256-1-0001").as_deref(),
            Some("P01")
        );
        assert_eq!(item_group_key("BC01-greeting").as_deref(), Some("BC01"));
        assert_eq!(item_group_key("A8-x").as_deref(), Some("A8"));
        assert_eq!(item_group_key("cli-capability-0"), None);
    }

    #[test]
    fn small_modules_stay_in_one_batch() {
        let input = json!({
            "module": "ingress",
            "evidence": [
                {"evidence_id": "cli-ingress-0", "kind": "http", "payload": {"module": "ingress"}},
                {"evidence_id": "cli-ingress-1", "kind": "http", "payload": {"module": "ingress"}}
            ]
        });
        let chunks = chunk_units(&input, 256 * 1024);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].0, "all");
        assert_eq!(chunks[0].1.len(), 2);
    }

    #[test]
    fn blob_evidence_splits_by_item_group() {
        let item = |sample_id: &str, pad: &str| {
            json!({"sample_id": sample_id, "payload": {"request": pad}})
        };
        let input = json!({
            "module": "capability",
            "evidence": [{
                "evidence_id": "cli-capability-0",
                "kind": "batch",
                "payload": {"module": "capability", "payload": {
                    "executed_samples": 3,
                    "planned_samples": 3,
                    "scorecard": {"items": [1, 2, 3]},
                    "evidence": [
                        item("C01-1-01", "a"), item("C01-2-01", "b"), item("C02-1-01", "c")
                    ]
                }}
            }]
        });
        let merged = chunk_units(&input, 256 * 1024);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].0, "all");
        assert_eq!(merged[0].1.len(), 1);
        let split = chunk_units(&input, 1);
        assert_eq!(
            split.iter().map(|(group, _)| group.as_str()).collect::<Vec<_>>(),
            vec!["C01", "C01", "C02"]
        );
        let inner = split[2].1[0]["payload"]["payload"]
            .as_object()
            .expect("blob 批次保留内层结构");
        let evidence = inner["evidence"].as_array().expect("内层 evidence");
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0]["sample_id"], "C02-1-01");
        assert!(inner.get("executed_samples").is_none());
        assert!(inner.get("planned_samples").is_none());
        assert!(inner.get("scorecard").is_none());
    }

    #[test]
    fn oversized_groups_bin_pack_by_entry() {
        let entry = |id: &str| json!({
            "evidence_id": id,
            "kind": "timing",
            "payload": {"module": "performance", "request": {"prompt": "x".repeat(80)}}
        });
        let input = json!({
            "module": "performance",
            "evidence": [
                entry("cli-performance-P01-waiting-stream-512-256-1-0001"),
                entry("cli-performance-P01-waiting-stream-512-256-1-0002"),
                entry("cli-performance-P01-waiting-stream-512-256-1-0003"),
                entry("cli-performance-P01-waiting-stream-512-256-1-0004")
            ]
        });
        let chunks = chunk_units(&input, 600);
        assert_eq!(chunks.len(), 2);
        assert!(chunks.iter().all(|(group, _)| group == "P01"));
        assert_eq!(chunks[0].1.len(), 2);
        assert_eq!(chunks[1].1.len(), 2);
    }

    #[test]
    fn merged_verdict_takes_the_worst_batch() {
        let report = |verdict: &str| ModuleReport {
            schema_version: "module-eval-report/v1".into(),
            evaluation_version: EVALUATION_VERSION.into(),
            module: "capability".into(),
            verdict: verdict.into(),
            confidence: "high".into(),
            summary: "batch".into(),
            findings: Vec::new(),
            evidence_refs: Vec::new(),
            limitations: Vec::new(),
            items: Vec::new(),
            analyzer: AnalyzerInfo {
                name: "ohmypi".into(),
                status: "completed_normalized".into(),
            },
        };
        let merged = merge_chunk_reports(
            "capability",
            vec![report("pass"), report("fail"), report("inconclusive")],
        );
        assert_eq!(merged.verdict, "fail");
        assert_eq!(merged.confidence, "high");
        let merged = merge_chunk_reports("capability", vec![report("pass"), report("limited")]);
        assert_eq!(merged.verdict, "limited");
        assert_eq!(merged.confidence, "medium");
        let merged = merge_chunk_reports("capability", vec![report("pass"), report("inconclusive")]);
        assert_eq!(merged.verdict, "inconclusive");
        let merged = merge_chunk_reports("capability", vec![report("pass"), report("pass")]);
        assert_eq!(merged.verdict, "pass");
        assert_eq!(merged.analyzer.status, "completed_normalized");
        assert!(merged.limitations.iter().any(|item| item.contains("分 2 批")));
    }

    #[test]
    fn merge_drops_unexplained_findings() {
        let report = |verdict: &str| ModuleReport {
            schema_version: "module-eval-report/v1".into(),
            evaluation_version: EVALUATION_VERSION.into(),
            module: "performance".into(),
            verdict: verdict.into(),
            confidence: "high".into(),
            summary: "batch".into(),
            findings: Vec::new(),
            evidence_refs: Vec::new(),
            limitations: Vec::new(),
            items: Vec::new(),
            analyzer: AnalyzerInfo {
                name: "ohmypi".into(),
                status: "completed_normalized".into(),
            },
        };
        let mut explained = report("fail");
        explained.findings.push(Finding {
            item_id: "P01".into(),
            verdict: "fail".into(),
            rationale: "全部样本 content 为空".into(),
            evidence_refs: vec!["cli-performance-1".into()],
        });
        let mut placeholder = report("pass");
        placeholder.findings.push(Finding {
            item_id: "P02".into(),
            verdict: "fail".into(),
            rationale: "未提供说明".into(),
            evidence_refs: vec!["cli-performance-3".into()],
        });
        let merged = merge_chunk_reports("performance", vec![explained, placeholder]);
        assert_eq!(merged.verdict, "fail");
        assert!(merged.findings.iter().any(|finding| finding.item_id == "P01"));
        assert!(
            merged
                .findings
                .iter()
                .all(|finding| finding.item_id != "P02")
        );
    }

    #[test]
    fn chunk_timeout_scales_with_entries_and_clamps() {
        assert_eq!(chunk_timeout_for(1), Duration::from_secs(600));
        assert_eq!(chunk_timeout_for(10), Duration::from_secs(780));
        assert_eq!(chunk_timeout_for(22), Duration::from_secs(1500));
        assert_eq!(chunk_timeout_for(200), Duration::from_secs(1800));
    }

    #[test]
    fn parses_answer_json_after_attachment_echo() {
        let echo = r#"{"schema_version":"module-eval-input/v1","module":"specification","rules":{"title":"协议规格"}}"#;
        let answer = r#"{"schema_version":"module-eval-report/v1","evaluation_version":"evaluation-pipeline/v1","module":"specification","verdict":"fail","confidence":"high","summary":"S05 失败","findings":[],"evidence_refs":[],"limitations":[],"items":[],"analyzer":{"name":"ohmypi","status":"completed_normalized"}}"#;
        let text = format!("以下是分析：\n{echo}\n\n{answer}\n");
        let report = parse_module_report(&text, "specification")
            .expect("应解析出结论而不是把附件回显当结论");
        assert_eq!(report.verdict, "fail");
    }

    #[test]
    fn item_count_cap_splits_groups_even_within_byte_budget() {
        let tiny = |i: usize| {
            json!({"evidence_id": format!("cli-perf-P01-{i:04}"), "kind": "t", "payload": {"module": "performance"}})
        };
        let huge = json!({"evidence_id": "cli-perf-P02-big", "kind": "t", "payload": {"module": "performance", "blob": "z".repeat(2048)}});
        let mut entries: Vec<Value> = (0..15).map(tiny).collect();
        entries.push(huge);
        let input = json!({"module": "performance", "evidence": entries});
        let chunks = chunk_units(&input, 2500);
        let p01: Vec<_> = chunks.iter().filter(|(group, _)| group == "P01").collect();
        assert_eq!(p01.len(), 2);
        assert_eq!(p01[0].1.len(), 12);
        assert_eq!(p01[1].1.len(), 3);
    }

    #[test]
    fn custom_mode_fills_template_without_analyzer() {
        let root = std::env::temp_dir().join(format!(
            "agentcheck-custom-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let report = unavailable_report();
        let input_paths = write_bundled_module_inputs(&report, root.join("module-input"))
            .expect("应能写出模块输入");
        let paths = build_module_reports(&report, &input_paths, root.join("module-report"))
            .expect("custom 模式不应失败");
        assert_eq!(paths.len(), 1);
        let module_report: ModuleReport =
            serde_json::from_slice(&fs::read(&paths[0]).unwrap()).unwrap();
        assert_eq!(module_report.module, "capability");
        assert_eq!(module_report.verdict, "inconclusive");
        assert_eq!(module_report.confidence, "none");
        assert_eq!(module_report.analyzer.name, "agentcheck-template");
        assert_eq!(module_report.analyzer.status, "deterministic_fill");
        let html = render_html(&report, &paths).unwrap();
        assert!(html.contains("不含语义分析"));
        for jargon in [
            "inconclusive",
            "unverified",
            "OhMyPi",
            "payload",
            "cli-",
            "custom",
            "dynamic",
        ] {
            assert!(!html.contains(jargon), "报告不应出现黑话 {jargon}");
        }
        let _ = fs::remove_dir_all(&root);
    }
}

