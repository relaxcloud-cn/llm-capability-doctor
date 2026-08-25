use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::json;
use thiserror::Error;

use crate::opencodex::contract::{Adapter, CONTRACT, SOURCE_FILES, contract_digest, rule};
use crate::opencodex::runner::OpenCodexOutcome;
use crate::private_file::create_new_private_file;

pub struct ReportPaths {
    pub json: PathBuf,
}

#[derive(Debug, Error)]
pub enum ReportError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub fn write_reports(
    log_path: &Path,
    outcome: &OpenCodexOutcome,
) -> Result<ReportPaths, ReportError> {
    let json_path = write_output(log_path, "json", &render_json(outcome)?)?;
    Ok(ReportPaths { json: json_path })
}

pub fn merge_markdown_gateway_compatibility(
    path: &Path,
    detail: &str,
) -> Result<(), std::io::Error> {
    let mut markdown = std::fs::read_to_string(path)?;
    let marker = "| 模型最低并发要求";
    let position = markdown.find(marker).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "主 Markdown 缺少总体结论插入位置",
        )
    })?;
    markdown.insert_str(position, &format!("{detail}\n"));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)?;
    file.write_all(markdown.as_bytes())?;
    file.flush()
}

pub fn render_gateway_compatibility_detail(
    outcome: &OpenCodexOutcome,
    detected_protocol: &str,
) -> String {
    let mut output = String::new();
    let protocol_label = format!("协议兼容性（`{detected_protocol}`）");
    let adapter = adapter_for_protocol(detected_protocol);
    let result = adapter.and_then(|adapter| {
        outcome
            .results
            .iter()
            .find(|result| result.adapter == adapter)
    });
    match result {
        Some(result) if result.passed => {
            output.push_str(&format!(
                "| {protocol_label} | 通过 | 对应协议兼容性检测通过。 |\n"
            ));
        }
        Some(result) => {
            let reason = result
                .failures
                .iter()
                .map(|failure| {
                    format!(
                        "{}：实际返回 {} 为 {}；影响：{}",
                        failure.rule_id, failure.observed_path, failure.actual, failure.effect
                    )
                })
                .collect::<Vec<_>>()
                .join("；");
            writeln!(
                output,
                "| {protocol_label} | 不通过 | 不通过原因：{} |",
                markdown_cell(&reason)
            )
            .unwrap();
        }
        None => {
            writeln!(
                output,
                "| {protocol_label} | 不通过 | 不通过原因：未找到该协议对应的兼容性检测结果。 |"
            )
            .unwrap();
        }
    }
    output
}

fn adapter_for_protocol(protocol: &str) -> Option<Adapter> {
    match protocol {
        "openai_chat" => Some(Adapter::OpenAiChat),
        "anthropic_messages" => Some(Adapter::Anthropic),
        "gemini_generate_content" => Some(Adapter::Google),
        _ => None,
    }
}

fn markdown_cell(value: &str) -> String {
    value
        .replace('|', "\\|")
        .replace('\n', " ")
        .replace('\r', " ")
}

pub fn render_markdown(outcome: &OpenCodexOutcome) -> String {
    let mut output = String::new();
    writeln!(output, "# OpenCodex v2.7.42 模型输出兼容性\n").unwrap();
    writeln!(output, "规则包校验值：`{}`\n", contract_digest()).unwrap();
    for result in &outcome.results {
        let status = if result.passed { "通过" } else { "不通过" };
        writeln!(
            output,
            "## OpenCodex v2.7.42 / {}：{status}\n",
            result.adapter.id()
        )
        .unwrap();
        if result.passed {
            output.push_str("本轮所有必需返回格式、流结束和工具调用检查均通过。\n\n");
            continue;
        }
        for failure in &result.failures {
            writeln!(output, "### {}：不通过\n", failure.rule_id).unwrap();
            writeln!(output, "OpenCodex 要求：{}\n", failure.requirement).unwrap();
            writeln!(
                output,
                "实际返回：{} 为 {}。\n",
                failure.observed_path, failure.actual
            )
            .unwrap();
            writeln!(output, "影响：{}\n", failure.effect).unwrap();
        }
    }
    output
}

fn render_json(outcome: &OpenCodexOutcome) -> Result<Vec<u8>, serde_json::Error> {
    let results = outcome
        .results
        .iter()
        .map(|result| {
            json!({
                "adapter": result.adapter.id(),
                "status": if result.passed { "PASS" } else { "FAIL" },
                "failures": result.failures.iter().map(|failure| json!({
                    "ruleId": failure.rule_id,
                    "requirement": failure.requirement,
                    "observedPath": failure.observed_path,
                    "actual": failure.actual,
                    "effect": failure.effect,
                    "source": {
                        "file": rule(failure.rule_id).source_file,
                        "sha256": source_digest(rule(failure.rule_id).source_file),
                        "test": rule(failure.rule_id).source_test,
                    },
                })).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    let mut bytes = serde_json::to_vec_pretty(&json!({
        "profile": {
            "name": "OpenCodex output contract",
            "version": CONTRACT.version,
            "commit": CONTRACT.commit,
            "digest": contract_digest(),
        },
        "results": results,
    }))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn source_digest(path: &str) -> &'static str {
    SOURCE_FILES
        .iter()
        .find(|source| source.path == path)
        .map(|source| source.sha256)
        .expect("every OpenCodex rule references a pinned source file")
}

fn write_output(log_path: &Path, extension: &str, bytes: &[u8]) -> Result<PathBuf, std::io::Error> {
    let directory = log_path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(directory)?;
    let stem = log_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("model-doctor");
    for sequence in 0.. {
        let suffix = if sequence == 0 {
            String::new()
        } else {
            format!("-{sequence}")
        };
        let path = directory.join(format!("{stem}-opencodex-v2742{suffix}.{extension}"));
        match create_new_private_file(&path) {
            Ok(mut file) => {
                file.write_all(bytes)?;
                file.flush()?;
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use crate::opencodex::contract::Adapter;
    use crate::opencodex::evaluator::{AdapterResult, RuleFailure};
    use crate::opencodex::runner::OpenCodexOutcome;

    use super::render_markdown;

    #[test]
    fn markdown_reports_only_pass_or_fail_and_explains_each_failure() {
        let report = render_markdown(&fixture_outcome());

        assert!(report.contains("OpenCodex v2.7.42 / openai-chat：不通过"));
        assert!(report.contains("OCX-CHAT-TOOL-004：不通过"));
        assert!(report.contains("OpenCodex 要求：流式工具调用的 function.name 必须是非空字符串。"));
        assert!(
            report.contains("实际返回：choices[0].delta.tool_calls[0].function.name 为 object。")
        );
        assert!(report.contains("影响：OpenCodex 无法生成 Codex 工具调用事件。"));
        assert!(!report.contains("未支持"));
        assert!(!report.contains("分析不可用"));
    }

    #[test]
    fn markdown_section_summarizes_protocol_results_for_the_main_report() {
        let mut outcome = fixture_outcome();
        outcome.results.push(AdapterResult {
            adapter: Adapter::Anthropic,
            passed: true,
            failures: Vec::new(),
        });
        outcome.results.push(AdapterResult {
            adapter: Adapter::Google,
            passed: false,
            failures: vec![RuleFailure {
                rule_id: "OCX-GOOGLE-SHAPE-001",
                requirement: "Google 响应必须包含可读取的 candidates 内容。",
                observed_path: "http_status".into(),
                actual: "401".into(),
                effect: "OpenCodex 无法读取模型响应。".into(),
            }],
        });
        let section = super::render_gateway_compatibility_detail(&outcome, "openai_chat");

        assert!(section.contains("| 协议兼容性（`openai_chat`） | 不通过 |"));
        assert!(section.contains("不通过原因："));
        assert!(section.contains("function.name 为 object"));
        assert!(!section.contains("anthropic"));
        assert!(!section.contains("google"));
        assert!(!section.contains("检测协议："));
    }

    #[test]
    fn merges_protocol_detail_inside_overall_gateway_conclusion() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("doctor-self-analysis.md");
        std::fs::write(
            &path,
            "## 总体结论\n\n| 检测项 | 检测结果 | 说明 |\n|---|---|---|\n| 模型最低并发要求（4 并发） | 通过 | 4/4 成功。 |\n",
        )
        .unwrap();

        super::merge_markdown_gateway_compatibility(
            &path,
            "| 协议兼容性（`openai_chat`） | 通过 | 对应协议兼容性检测通过。 |\n",
        )
        .unwrap();

        let markdown = std::fs::read_to_string(path).unwrap();
        let detail = markdown.find("| 协议兼容性").unwrap();
        let next_requirement = markdown.find("| 模型最低并发要求").unwrap();
        assert!(detail > markdown.find("## 总体结论").unwrap());
        assert!(detail < next_requirement);
        assert!(!markdown.contains("## 协议兼容性"));
    }

    #[test]
    fn json_records_the_offline_source_file_and_digest_for_each_failure() {
        let directory = tempfile::tempdir().unwrap();
        let paths =
            super::write_reports(&directory.path().join("doctor.log"), &fixture_outcome()).unwrap();
        let json = std::fs::read_to_string(paths.json).unwrap();

        assert!(json.contains("src/adapters/openai-chat.ts"));
        assert!(json.contains("ea32bc0aab76a954ed37e2431c56e8ec31f356dfbfbc60eedc1b4a0c870c4cac"));
        assert!(!json.contains("https://"));
    }

    fn fixture_outcome() -> OpenCodexOutcome {
        OpenCodexOutcome {
            results: vec![AdapterResult {
                adapter: Adapter::OpenAiChat,
                passed: false,
                failures: vec![RuleFailure {
                    rule_id: "OCX-CHAT-TOOL-004",
                    requirement: "流式工具调用的 function.name 必须是非空字符串。",
                    observed_path: "choices[0].delta.tool_calls[0].function.name".into(),
                    actual: "object".into(),
                    effect: "OpenCodex 无法生成 Codex 工具调用事件。",
                }],
            }],
        }
    }
}
