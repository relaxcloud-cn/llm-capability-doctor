//! Frozen report layout + deterministic data binding. No model calls.
use crate::{
    engine::atomic_json,
    model::{CheckId, RunResult, Status, Verdict},
    transport::HttpEvidence,
};
use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Cursor, Write},
    path::Path,
};
use zip::{ZipWriter, write::SimpleFileOptions};
fn html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn script_json(value: &Value) -> String {
    value
        .to_string()
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
fn files(root: &Path, base: &Path, output: &mut Vec<std::path::PathBuf>) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let meta = fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            if entry.file_name() != "config" {
                files(&path, base, output)?;
            }
        } else if meta.is_file()
            && entry.file_name() != "isolation.ts"
            && path.strip_prefix(base)?.components().next().is_some()
        {
            output.push(path);
        }
    }
    Ok(())
}
pub fn archive(directory: &Path, run: &RunResult) -> Result<Vec<u8>> {
    build_archive(directory, &mut run.clone(), false)
}
fn build_archive(directory: &Path, run: &mut RunResult, finalize: bool) -> Result<Vec<u8>> {
    let started = std::time::Instant::now();
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o600);
    let mut paths = vec![];
    for name in ["http", "agent"] {
        let path = directory.join(name);
        if path.is_dir() {
            files(&path, directory, &mut paths)?;
        }
    }
    paths.sort();
    for path in paths {
        let relative = path
            .strip_prefix(directory)?
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = fs::read(&path)?;
        zip.start_file(&relative, options)?;
        zip.write_all(&bytes)?;
        if let Ok(evidence) = serde_json::from_slice::<HttpEvidence>(&bytes) {
            let parent = format!("requests/{}", evidence.id);
            let curl = format!(
                "curl --request POST {} \\\n  --header 'Content-Type: application/json' \\\n  --header \"Authorization: Bearer ${{MODEL_API_KEY}}\" \\\n  --data-raw {}\n",
                shell_quote(&evidence.endpoint),
                shell_quote(&evidence.request.to_string())
            );
            zip.start_file(format!("{parent}/request.sh"), options)?;
            zip.write_all(curl.as_bytes())?;
            for attempt in evidence.attempts {
                zip.start_file(
                    format!("{parent}/attempt-{:02}-response.txt", attempt.number),
                    options,
                )?;
                zip.write_all(attempt.response_body.as_bytes())?;
                zip.start_file(
                    format!("{parent}/attempt-{:02}-metadata.json", attempt.number),
                    options,
                )?;
                zip.write_all(&serde_json::to_vec_pretty(&json!({"http_status":attempt.http_status,"headers":attempt.headers,"elapsed_ms":attempt.elapsed_ms,"error":attempt.error}))?)?;
            }
        }
    }
    if finalize {
        run.elapsed_ms = run
            .elapsed_ms
            .saturating_add(started.elapsed().as_millis() as u64);
        run.finished_at = Some(chrono::Utc::now());
        run.report_path = Some(directory.join("report.html").to_string_lossy().into_owned());
    }
    zip.start_file("result.json", options)?;
    zip.write_all(&serde_json::to_vec_pretty(run)?)?;
    zip.start_file("README.txt", options)?;
    zip.write_all("本包来自对应检测编号的实际记录。请求、原始响应、工具过程、来源快照和最终文件均按目录保留。密钥已脱敏；重放时从 MODEL_API_KEY 环境变量提供密钥。地址中脱敏的查询参数需要由技术人员按原配置补足。报告生成阶段没有调用任何模型。\n".as_bytes())?;
    Ok(zip.finish()?.into_inner())
}
fn name(id: CheckId) -> &'static str {
    match id {
        CheckId::Response => "response",
        CheckId::Tools => "tools",
        CheckId::Schema => "schema",
        CheckId::Context => "context",
        CheckId::Performance => "performance",
        CheckId::Continuous => "continuous",
        CheckId::Recovery => "recovery",
        CheckId::Multiturn => "multiturn",
    }
}
fn note(id: CheckId) -> &'static str {
    match id {
        CheckId::Response => "检查接口兼容性、普通响应和流式响应",
        CheckId::Tools => "检查工具选择、参数和结果使用",
        CheckId::Schema => "检查输出是否符合指定格式和字段要求",
        CheckId::Context => "探测模型支持的最大上下文长度",
        CheckId::Performance => "检查并发请求能否完成，以及最长和平均用时",
        CheckId::Continuous => "检查多个工具步骤能否衔接并完成任务",
        CheckId::Recovery => "检查工具临时出错后能否恢复并完成任务",
        CheckId::Multiturn => "检查新增要求后的任务继续与结果更新",
    }
}
fn observation(check: &crate::model::CheckResult) -> Value {
    let mut value = json!({"title":check.title,"status":match check.status{Status::Passed=>"passed",Status::Failed=>"failed",_=>"incomplete"}});
    if check.id == CheckId::Context && check.status == Status::Passed {
        value["maxContextTokens"] = check
            .facts
            .get("maximum_context_tokens")
            .filter(|v| !v.is_null())
            .or_else(|| check.facts.get("verified_lower_bound_tokens"))
            .cloned()
            .unwrap_or(Value::Null);
        value["maximumConfirmed"] = check
            .facts
            .get("maximum_confirmed")
            .cloned()
            .unwrap_or(json!(false));
    }
    if check.id == CheckId::Performance && check.status == Status::Passed {
        value["concurrency"] = check
            .facts
            .get("concurrency")
            .cloned()
            .unwrap_or(Value::Null);
        value["maxElapsedSeconds"] = check
            .facts
            .get("longest_elapsed_ms")
            .and_then(Value::as_f64)
            .map(|n| json!(n / 1000.0))
            .unwrap_or(Value::Null);
        value["meanElapsedSeconds"] = check
            .facts
            .get("mean_elapsed_ms")
            .and_then(Value::as_f64)
            .map(|n| json!(n / 1000.0))
            .unwrap_or(Value::Null);
    }
    value
}
fn human_fact(fact: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(fact) {
        if let Some(diff) = value["differences"].as_array().and_then(|v| v.first()) {
            let id = diff["id"].as_str().unwrap_or("未知记录");
            let field = diff["field"].as_str().unwrap_or("字段");
            if field == "record" {
                return if diff["actual"].is_null() {
                    format!("结果遗漏记录 {id}。")
                } else {
                    format!("结果包含未要求的记录 {id}。")
                };
            }
            let label = match field {
                "owner" => "负责人",
                "version" => "版本号",
                _ => field,
            };
            let display = |v: &Value| {
                v.as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| v.to_string())
            };
            return format!(
                "记录 {id} 的{label}应为 {}，实际为 {}。",
                display(&diff["expected"]),
                display(&diff["actual"])
            );
        }
        if let Some(errors) = value["process_errors"].as_array() {
            return errors
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("；");
        }
    }
    fact.into()
}
fn issues(run: &RunResult) -> String {
    let mut cards = String::new();
    for check in run.checks.iter().filter(|c| c.status != Status::Passed) {
        let findings = if check.findings.is_empty() {
            vec![(
                check.title.clone(),
                if run.cancelled && !check.finished {
                    "本次提前停止，此项没有完成。".into()
                } else {
                    "本项尚未取得可确认的完整检测记录。".into()
                },
                "完整接入要求尚未得到验证。".into(),
                "请完成或重新运行本项检测。".into(),
            )]
        } else {
            check
                .findings
                .iter()
                .map(|f| {
                    (
                        f.title.clone(),
                        human_fact(&f.fact),
                        f.impact.clone(),
                        f.suggestion.clone(),
                    )
                })
                .collect()
        };
        for (index, (title, fact, impact, suggestion)) in findings.into_iter().enumerate() {
            let tone = if check.status == Status::Failed {
                "!"
            } else {
                "—"
            };
            let state_class = if check.status == Status::Failed {
                ""
            } else {
                " incomplete"
            };
            cards.push_str(&format!("<article class=\"error-block{state_class}\" id=\"issue-{}-{index}\"><div class=\"error-head\"><span class=\"error-icon\" aria-hidden=\"true\">{tone}</span><h3>{}</h3></div><p class=\"error-lead\">{}：{}</p><div class=\"impact-grid\"><div><h4>会有什么影响</h4><p>{}</p></div><div><h4>建议怎么处理</h4><p>{}</p></div></div></article>",name(check.id),html(&title),html(check.id.name()),html(&fact),html(&impact),html(&suggestion)));
        }
    }
    if cards.is_empty() {
        cards="<article class=\"error-block\"><div class=\"error-head\"><h3>本次检测未发现阻止接入的问题</h3></div><p>建议按本次验证的协议、版本和负载接入，并保留本次检测记录。</p></article>".into();
    }
    cards
}
fn replace_between(source: &mut String, start: &str, end: &str, replacement: &str) -> Result<()> {
    let a = source.find(start).context("报告模板起点缺失")?;
    let b = source[a + start.len()..]
        .find(end)
        .context("报告模板终点缺失")?
        + a
        + start.len();
    source.replace_range(a..b, replacement);
    Ok(())
}
pub fn render(run: &RunResult, zip: &[u8]) -> Result<String> {
    let mut document =
        include_str!("../docs/report-reference/agentcheck-baseline-report.html").to_owned();
    document = document.replace(
        "<title>AgentCheck · 模型使用情况对照 · 示例</title>",
        "<title>AgentCheck · 模型使用检测报告</title>",
    );
    replace_between(
        &mut document,
        " <div class=\"page-title\">",
        " <nav class=\"report-nav\"",
        &format!(
            " <div class=\"page-title\"><h1>模型使用检测报告</h1><p class=\"page-subtitle\">查看能否接入平台，以及与 OpenAI 模型的逐项对照。</p><div class=\"page-meta\"><span>检测模型：{}</span><span>检测日期：{}</span></div></div>\n",
            html(&run.target.model),
            run.started_at
                .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
                .format("%Y-%m-%d"),
        ),
    )?;
    let major = run
        .checks
        .iter()
        .find(|c| c.status == Status::Failed)
        .or_else(|| run.checks.iter().find(|c| c.status != Status::Passed));
    let reason = major
        .map(|c| format!("{}：{}", c.id.name(), c.title))
        .unwrap_or_else(|| "本次全部必测项目均达到接入要求。".into());
    let suggestion = if run.verdict == Verdict::Passed {
        "建议按本次验证的协议、版本和负载接入。"
    } else if run.verdict == Verdict::Failed {
        "先处理对应问题，再重新检测。"
    } else {
        "补齐未完成或缺少证据的项目，再确认接入结论。"
    };
    let tone = match run.verdict {
        Verdict::Passed => "passed",
        Verdict::Failed => "failed",
        Verdict::Incomplete => "incomplete",
    };
    replace_between(
        &mut document,
        "  <div class=\"hero\">",
        " </section>",
        &format!(
            "  <div class=\"hero {tone}\"><div class=\"verdict-row\"><span class=\"verdict-symbol\" aria-hidden=\"true\">{}</span><div><h3 id=\"verdict\">{}</h3><p>主要原因：{}</p></div></div><div class=\"hero-next\"><p><strong>建议：</strong>{}</p><div class=\"hero-actions\"><a class=\"button\" href=\"#comparison\">查看逐项对比 ↓</a><a class=\"plain-btn\" href=\"#issue\">直接看问题</a></div></div></div>\n",
            if run.verdict == Verdict::Passed {
                "✓"
            } else if run.verdict == Verdict::Failed {
                "!"
            } else {
                "—"
            },
            run.verdict.title(),
            html(&reason),
            suggestion
        ),
    )?;
    document = document.replace(
        "示例模型 A · 本次测试结果",
        &format!("{} · 本次测试结果", html(&run.target.model)),
    );
    let attention = run
        .checks
        .iter()
        .filter(|c| c.status != Status::Passed)
        .count();
    replace_between(
        &mut document,
        "   <article class=\"error-block\"",
        " </section>",
        &format!("{}\n", issues(run)),
    )
    .or_else(|_| {
        replace_between(
            &mut document,
            "  <article class=\"error-block\"",
            " </section>",
            &format!("{}\n", issues(run)),
        )
    })?;
    document = document.replace("1 项需要处理", &format!("{attention} 项需关注"));
    document = document
        .replace("ZIP · 示例记录", "ZIP · 本次完整记录")
        .replace("已下载示例记录", "已发起检测记录下载")
        .replace(
            "AgentCheck · 示例报告 · 不是真实检测结果",
            "AgentCheck · 本次实际检测报告",
        );
    let a = document
        .find("<aside class=\"prototype\"")
        .context("原型设置区缺失")?;
    let b = document[a..].find("</aside>").unwrap() + a + "</aside>".len();
    document.replace_range(a..b, "");
    document = document.replace(
        "const modeControl=document.getElementById('reference-mode');",
        "const modeControl={value:'missing',addEventListener(){}};",
    );
    let data:Vec<_>=run.checks.iter().map(|check|json!({"id":name(check.id),"group":if check.id.is_agent(){"智能体任务检测"}else{"模型接口检测"},"name":check.id.name(),"note":note(check.id),"customer":observation(check),"official":{"title":"暂无实测数据","status":"incomplete"},"agent":check.id.is_agent()})).collect();
    let start = document
        .find("const checks=")
        .context("报告检测数据入口缺失")?;
    let end = document[start..]
        .find(";\n")
        .context("报告检测数据终点缺失")?
        + start
        + 2;
    document.replace_range(
        start..end,
        &format!("const checks={};", script_json(&json!(data))),
    );
    document = document.replace("status:'incomplete'};", "status:observation.status};");
    let start = document
        .find("const recordArchive = ")
        .or_else(|| document.find("const recordArchive="))
        .context("报告下载包入口缺失")?;
    let end = document[start..]
        .find(";\n")
        .context("报告下载包终点缺失")?
        + start
        + 2;
    document.replace_range(
        start..end,
        &format!(
            "const recordArchive={};",
            serde_json::to_string(&STANDARD.encode(zip))?
        ),
    );
    document = document.replace(
        "AgentCheck-检测记录-示例.zip",
        &format!("AgentCheck-{}-检测记录.zip", run.id),
    );
    let binding = r#"
byId('reference-column-note').textContent='gpt-6-astra · 暂无匹配的官方实测记录';
byId('reference-description').textContent='未取得匹配的官方实测记录';
byId('reference-badge').textContent='暂无实测记录';
byId('source-copy').innerHTML='<dl class="record-kv"><div><dt>对照模型</dt><dd>gpt-6-astra</dd></div><div><dt>资料状态</dt><dd>尚未提供匹配的官方实测结果</dd></div></dl>';
byId('source-rule').textContent='OpenAI 对照资料缺失不改变客户模型本次自身的验收结论。';
byId('comparison-note').textContent='本页客户模型结论来自本次实际记录；官方对照列等待匹配的实测资料。';
"#.to_owned();
    document = document.replace("</script>", &format!("{binding}</script>"));
    document=document.replace("</style>",".hero.passed{border-color:#cae4d8;border-left-color:var(--green)}.hero.passed h3{color:var(--green)}.hero.passed .verdict-symbol{background:#e5f4eb;color:var(--green)}.hero.incomplete{border-color:#dbe2ec;border-left-color:#8b99ae}.hero.incomplete h3{color:#61718a}.hero.incomplete .verdict-symbol{background:#edf1f6;color:#7b8798}.error-block+.error-block{margin-top:14px}.error-block.incomplete{border-color:#dbe2ec}.error-block.incomplete .error-icon{background:#edf1f6;color:#7b8798}</style>");
    document = document.replace(
        "href=\"#issue\">查看问题与建议",
        "href=\"#issue-'+item.id+'-0\">查看问题与建议",
    );
    Ok(document)
}
fn atomic_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().context("产物路径没有目录")?)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
pub fn write(run: &mut RunResult, directory: &Path) -> Result<()> {
    let zip = build_archive(directory, run, true)?;
    let document = render(run, &zip)?;
    atomic_file(&directory.join("records.zip"), &zip)?;
    atomic_file(&directory.join("report.html"), document.as_bytes())?;
    run.report_path = Some(directory.join("report.html").to_string_lossy().into_owned());
    atomic_json(&directory.join("result.json"), run)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Protocol, Target};
    #[test]
    fn production_report_uses_real_data_escapes_script_and_contains_real_zip() {
        let mut run = RunResult::new(Target {
            model: "<script>alert(1)</script>".into(),
            endpoint: "https://example.org/v1/chat/completions".into(),
            protocol: Protocol::ChatCompletions,
        });
        run.checks[0].status = Status::Passed;
        run.checks[0].title = "普通响应、流式响应均正常".into();
        let dir = tempfile::tempdir().unwrap();
        let zip = archive(dir.path(), &run).unwrap();
        let html = render(&run, &zip).unwrap();
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("暂无匹配的官方实测记录"));
        assert!(!html.contains("示例报告"));
        assert!(!html.contains("页面演示设置"));
        let mut archive = zip::ZipArchive::new(Cursor::new(zip)).unwrap();
        assert!(archive.by_name("result.json").is_ok());
    }
}
