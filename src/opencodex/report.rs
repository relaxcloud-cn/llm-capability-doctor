use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::json;
use thiserror::Error;

use crate::opencodex::contract::{CONTRACT, contract_digest};
use crate::opencodex::runner::OpenCodexOutcome;
use crate::private_file::create_new_private_file;

pub struct ReportPaths {
    pub json: PathBuf,
    pub markdown: PathBuf,
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
    let markdown_path = write_output(log_path, "md", render_markdown(outcome).as_bytes())?;
    Ok(ReportPaths {
        json: json_path,
        markdown: markdown_path,
    })
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
        let report = render_markdown(&OpenCodexOutcome {
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
        });

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
}
