use crate::cli::{CliRunReport, module_display_name, module_item_stats, module_state_label};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const EVALUATION_VERSION: &str = "evaluation-pipeline/v1";

/// OhMyPi 单次模块分析的看门狗超时；子进程超过该时长直接杀死并按 inconclusive 处理。
const OMP_ANALYSIS_TIMEOUT: Duration = Duration::from_secs(600);

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
        eprintln!("[报告] OhMyPi 分析模块 {module}…");
        let mut command = Command::new(omp.as_ref().unwrap());
        command
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
            .arg("--max-time")
            .arg(
                OMP_ANALYSIS_TIMEOUT
                    .as_secs()
                    .saturating_sub(30)
                    .to_string(),
            )
            .arg(format!("@{}", input_path.display()))
            .arg(prompt)
            .env("PI_CODING_AGENT_DIR", &omp_config.agent_dir)
            .env("AGENTCHECK_MODEL_API_KEY", &analyzer_config.api_key);
        let result = run_command_with_timeout(&mut command, OMP_ANALYSIS_TIMEOUT);
        let report = match result {
            Ok(result) if result.status.is_none() => inconclusive_report(
                module,
                &format!(
                    "OhMyPi 分析超时（{} 秒），已终止该进程",
                    OMP_ANALYSIS_TIMEOUT.as_secs()
                ),
            ),
            Ok(result) if result.status.is_some_and(|status| status.success()) => {
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
                rationale: if stats.failed == 0 {
                    format!("{} 个检测项全部通过", stats.total)
                } else {
                    format!("{}/{} 个检测项未通过", stats.failed, stats.total)
                },
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
                        item(code, status, note)
                    })
                    .collect()
            })
            .unwrap_or_default(),
        "agent" => payload["report"]["check_summaries"]
            .as_array()
            .map(|checks| {
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
                        item(code, status, note)
                    })
                    .collect()
            })
            .unwrap_or_default(),
        "baseline" => payload["report"]["comparisons"]
            .as_array()
            .map(|comparisons| {
                comparisons
                    .iter()
                    .map(|comparison| {
                        let code = comparison["scenario"].as_str().unwrap_or_default();
                        let result = comparison["status"].as_str().unwrap_or_default();
                        let diffs = comparison["differences"]
                            .as_array()
                            .map(|items| items.len())
                            .unwrap_or(0);
                        let (status, note) = match result {
                            "different" => ("fail", format!("与参考服务存在 {diffs} 处结构差异")),
                            "not_observed" | "inconclusive" => {
                                ("inconclusive", "未取得可核对的实测响应".to_string())
                            }
                            "not_applicable" => ("limited", "该检测项不适用".to_string()),
                            _ => ("pass", "与参考服务一致".to_string()),
                        };
                        item(code, status, note)
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
                        format!(
                            "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
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
            items: Vec::new(),
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
        };
        crate::cli::run_with_executor(request, &mut crate::cli::UnavailableExecutor).unwrap()
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
