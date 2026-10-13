//! Frozen report layout + deterministic data binding. No model calls.
use crate::{engine::atomic_json, model::RunResult, transport::HttpEvidence};
use anyhow::{Context, Result};
use serde_json::json;
use std::{
    fs,
    io::{Cursor, Write},
    path::Path,
};
use zip::{ZipWriter, write::SimpleFileOptions};

mod view;

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
    for name in ["http", "agent", "observations"] {
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
    if run.schema_version >= 2 {
        zip.start_file("evidence-index.json", options)?;
        zip.write_all(&serde_json::to_vec_pretty(&run.evidence)?)?;
    }
    zip.start_file("README.txt", options)?;
    zip.write_all("本包来自对应检测编号的实际记录。请求、原始响应、工具过程、来源快照和最终文件均按目录保留。密钥已脱敏；重放时从 MODEL_API_KEY 环境变量提供密钥。地址中脱敏的查询参数需要由技术人员按原配置补足。报告生成阶段没有调用任何模型。\n".as_bytes())?;
    Ok(zip.finish()?.into_inner())
}
pub fn render(run: &RunResult, zip: &[u8]) -> Result<String> {
    view::render(run, zip)
}
fn atomic_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().context("产物路径没有目录")?)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
pub fn write(run: &mut RunResult, directory: &Path) -> Result<()> {
    if run.schema_version >= 2 {
        for reference in run.evidence.values() {
            crate::judgment::verify_reference(directory, reference)?;
        }
        atomic_json(&directory.join("evidence-index.json"), &run.evidence)?;
    }
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
    use crate::{
        config::{Protocol, Target},
        model::Status,
    };
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
        assert!(html.contains("尚无参考实测"));
        assert!(!html.contains("示例报告"));
        assert!(!html.contains("页面演示设置"));
        let mut archive = zip::ZipArchive::new(Cursor::new(zip)).unwrap();
        assert!(archive.by_name("result.json").is_ok());
    }

    #[test]
    fn report_preserves_compatibility_notes_and_concurrency_probe_boundaries() {
        let mut run = RunResult::new(Target {
            model: "test".into(),
            endpoint: "https://example.org/v1/chat/completions".into(),
            protocol: Protocol::ChatCompletions,
        });
        let expected = crate::dataset::SourceSnapshot::generate(crate::model::CHECK_VERSION)
            .expected(None)
            .unwrap();
        run.update(crate::checks::evaluate_schema(
            &format!(
                "```json\n{}\n```",
                serde_json::to_string(&expected).unwrap()
            ),
            &expected,
            std::time::Instant::now(),
            vec![],
        ));
        let check = &mut run.checks[4];
        check.status = Status::Passed;
        check.title = "已实测至少支持 32 并发（已达本次探测上限，最大值未确认）".into();
        check
            .facts
            .insert("probe_mode".into(), json!("concurrent_requests"));
        check.facts.insert("rounds".into(),json!([{"concurrency":32,"successful":32,"failed":0,"rate_limited":0,"longest_elapsed_ms":100,"mean_elapsed_ms":90}]));
        let html = render(&run, &[]).unwrap();
        assert!(html.contains("本项兼容通过不代表服务支持原生严格 JSON Schema"));
        assert!(html.contains("并发档位与耗时明细"));
        assert!(html.contains("已达本次探测上限，最大值未确认"));
        assert!(html.contains("历史记录未设置响应时间合格阈值"));
        assert!(!html.contains("平均 ≤ 10 秒"));
        let check = &mut run.checks[4];
        check
            .facts
            .insert("max_mean_elapsed_ms".into(), json!(10_000));
        check
            .facts
            .insert("max_tail_elapsed_ms".into(), json!(20_000));
        check.facts.insert("rounds".into(), json!([
            {"concurrency":16,"successful":16,"failed":0,"mean_elapsed_ms":2000,"longest_elapsed_ms":4000,"tail_metric":"maximum","failure_reason":"passed"},
            {"concurrency":32,"successful":32,"failed":0,"mean_elapsed_ms":3000,"p95_elapsed_ms":21000,"longest_elapsed_ms":22000,"tail_metric":"p95","failure_reason":"p95_latency"}
        ]));
        let html = render(&run, &[]).unwrap();
        assert!(html.contains("平均 ≤ 10 秒、P95 ≤ 20 秒"));
        assert!(html.contains("最慢 4.00 秒"));
        assert!(html.contains("P95 21.00 秒"));
        assert!(html.contains("响应过慢（P95 耗时超标）"));
        assert!(!html.contains("历史记录未设置响应时间合格阈值"));
        assert!(html.contains("短 JSON 回显负载"));
        run.checks[4]
            .facts
            .insert("workload_type".into(), json!("short_text"));
        let html = render(&run, &[]).unwrap();
        assert!(html.contains("普通短文本负载"));
        assert!(html.contains("不要求 JSON、固定措辞或原样回显"));
        assert!(!html.contains("短 JSON 回显负载"));
    }
}
