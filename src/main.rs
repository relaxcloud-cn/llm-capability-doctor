use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use clap::Parser;
use model_capability_doctor::analysis::orchestrator::{
    AnalysisConnectionSettings, AnalysisOutcome,
};
use model_capability_doctor::catalog;
use model_capability_doctor::cli::Cli;
use model_capability_doctor::opencodex::report::ReportPaths;
use model_capability_doctor::opencodex::runner::{OpenCodexOutcome, OpenCodexSettings};
use model_capability_doctor::terminal;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> ExitCode {
    let arguments: Vec<_> = std::env::args_os().collect();
    if let Some(option) = missing_value_option(&arguments) {
        eprintln!("Missing value for {option}");
        return ExitCode::from(2);
    }
    let cli = match Cli::try_parse_from(arguments) {
        Ok(cli) => cli,
        Err(error) => {
            let exit_code = u8::try_from(error.exit_code()).unwrap_or(1);
            let _ = error.print();
            return ExitCode::from(exit_code);
        }
    };
    if cli.list_tests {
        print!("{}", catalog::render());
        return ExitCode::SUCCESS;
    }

    let environment_api_key = std::env::var("MODEL_API_KEY").ok();
    match cli.into_config(environment_api_key) {
        Ok(config) => {
            let analysis_connection = config.self_analyze.then(|| AnalysisConnectionSettings {
                url: config.url.clone(),
                model: config.model.clone(),
                api_key: config.api_key.expose().to_owned(),
                timeout: config.timeout,
                insecure: config.insecure,
            });
            let opencodex_connection = config.opencodex_compatibility.then(|| {
                (
                    config.url.clone(),
                    config.model.clone(),
                    config.api_key.expose().to_owned(),
                    config.timeout,
                    config.insecure,
                )
            });
            let cancellation = CancellationToken::new();
            let shutdown = shutdown_signal();
            tokio::pin!(shutdown);
            let run = model_capability_doctor::runner::run(config, cancellation.clone());
            tokio::pin!(run);
            let result = tokio::select! {
                result = &mut run => Some(result),
                exit_code = &mut shutdown => {
                    cancellation.cancel();
                    let _ = (&mut run).await;
                    return ExitCode::from(exit_code);
                }
            };
            match result.expect("runner branch always returns a result") {
                Ok(outcome) => {
                    print_collection_outcome(&outcome);
                    if let Some((url, model, api_key, timeout, insecure)) = opencodex_connection {
                        println!();
                        println!("正在检测 OpenCodex v2.7.42 模型输出兼容性……");
                        let compatibility =
                            model_capability_doctor::opencodex::runner::run(OpenCodexSettings {
                                url,
                                model,
                                api_key,
                                timeout,
                                insecure,
                                cancellation: cancellation.clone(),
                            });
                        tokio::pin!(compatibility);
                        let compatibility_outcome = tokio::select! {
                            result = &mut compatibility => result,
                            exit_code = &mut shutdown => {
                                cancellation.cancel();
                                let _ = (&mut compatibility).await;
                                return ExitCode::from(exit_code);
                            }
                        };
                        let compatibility_outcome = match compatibility_outcome {
                            Ok(value) => value,
                            Err(error) => {
                                eprintln!("OpenCodex 兼容性检测失败：{error}");
                                return ExitCode::FAILURE;
                            }
                        };
                        let report_paths =
                            match model_capability_doctor::opencodex::report::write_reports(
                                &outcome.log_path,
                                &compatibility_outcome,
                            ) {
                                Ok(paths) => paths,
                                Err(error) => {
                                    eprintln!("OpenCodex 兼容性报告写入失败：{error}");
                                    return ExitCode::FAILURE;
                                }
                            };
                        print_opencodex_outcome(&compatibility_outcome, &report_paths);
                    }
                    if let Some(connection) = analysis_connection {
                        println!();
                        println!("正在准备本地证据分析……");
                        let analysis = model_capability_doctor::analysis::orchestrator::analyze(
                            connection.for_run(
                                outcome.log_path.clone(),
                                outcome.detected_protocol,
                                outcome.detected_auth_mode,
                            ),
                            cancellation.clone(),
                        );
                        tokio::pin!(analysis);
                        let analysis_result = tokio::select! {
                            result = &mut analysis => result,
                            exit_code = &mut shutdown => {
                                cancellation.cancel();
                                match (&mut analysis).await {
                                    Ok(analysis_outcome) => print_analysis_outcome(&analysis_outcome),
                                    Err(error) => eprintln!("客户侧分析未完成：{error}"),
                                }
                                return ExitCode::from(exit_code);
                            }
                        };
                        match analysis_result {
                            Ok(analysis_outcome) => print_analysis_outcome(&analysis_outcome),
                            Err(error) => {
                                eprintln!("客户侧分析失败（检测日志已正常生成）：{error}")
                            }
                        }
                    }
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::FAILURE
                }
            }
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}

fn print_opencodex_outcome(outcome: &OpenCodexOutcome, paths: &ReportPaths) {
    println!("========== OpenCodex v2.7.42 模型输出兼容性 ==========");
    for result in &outcome.results {
        let status = if result.passed { "通过" } else { "不通过" };
        println!("{}：{status}", result.adapter.id());
    }
    println!("JSON：{}", file_name(&paths.json));
    println!("结果：{}", file_name(&paths.markdown));
}

fn print_collection_outcome(outcome: &model_capability_doctor::runner::RunOutcome) {
    println!("========== 检测完成 / 采集阶段 ==========");
    println!(
        "耗时：{} 秒 | 请求：{} | 检测项：{}",
        outcome.duration.as_secs(),
        outcome.request_count,
        outcome.manifest_count
    );
    print_output_location("日志", &outcome.log_path);
}

fn print_analysis_outcome(outcome: &AnalysisOutcome) {
    print!("{}", format_analysis_outcome(outcome));
}

fn format_analysis_outcome(outcome: &AnalysisOutcome) -> String {
    let mut output = String::new();
    writeln!(output, "========== 自分析完成 ==========").unwrap();
    writeln!(
        output,
        "可用：{}（PASS {} / FAIL {}）| 不可用：{}",
        outcome.available_count, outcome.pass_count, outcome.fail_count, outcome.unavailable_count
    )
    .unwrap();
    let directory = outcome.path.parent().unwrap_or_else(|| Path::new("."));
    writeln!(output, "输出目录：{}", terminal::display_path(directory)).unwrap();
    writeln!(output, "JSON：{}", file_name(&outcome.path)).unwrap();
    writeln!(output, "结果：{}", file_name(&outcome.markdown_path)).unwrap();
    writeln!(
        output,
        "失败 cURL 日志：{}",
        file_name(&outcome.failed_curl_log_path)
    )
    .unwrap();
    if outcome.cancelled {
        writeln!(
            output,
            "分析状态：已取消；未完成批次已标记为 ANALYSIS_UNAVAILABLE"
        )
        .unwrap();
    }
    output
}

fn print_output_location(label: &str, path: &Path) {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    println!("输出目录：{}", terminal::display_path(directory));
    println!("{label}：{}", file_name(path));
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || terminal::display_path(path),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn missing_value_option(arguments: &[std::ffi::OsString]) -> Option<&str> {
    const OPTIONS_WITH_VALUES: [&str; 5] =
        ["--url", "--model", "--api-key", "--log-file", "--timeout"];
    if arguments.iter().any(|argument| {
        matches!(
            argument.to_str(),
            Some("-h" | "--help" | "-V" | "--version")
        )
    }) {
        return None;
    }
    let last = arguments.last()?.to_str()?;
    OPTIONS_WITH_VALUES.contains(&last).then_some(last)
}

#[cfg(unix)]
fn shutdown_signal() -> impl std::future::Future<Output = u8> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut interrupt = signal(SignalKind::interrupt()).expect("unable to install SIGINT handler");
    let mut terminate = signal(SignalKind::terminate()).expect("unable to install SIGTERM handler");
    async move {
        tokio::select! {
            _ = interrupt.recv() => 130,
            _ = terminate.recv() => 143,
        }
    }
}

#[cfg(not(unix))]
fn shutdown_signal() -> impl std::future::Future<Output = u8> {
    async {
        let _ = tokio::signal::ctrl_c().await;
        130
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{AnalysisOutcome, format_analysis_outcome};

    #[test]
    fn analysis_summary_lists_the_failed_curl_log() {
        let outcome = AnalysisOutcome {
            path: PathBuf::from("result.json"),
            markdown_path: PathBuf::from("result.md"),
            failed_curl_log_path: PathBuf::from("doctor-failed-curls.log"),
            available_count: 45,
            pass_count: 44,
            fail_count: 1,
            unavailable_count: 1,
            cancelled: false,
        };

        assert!(
            format_analysis_outcome(&outcome).contains("失败 cURL 日志：doctor-failed-curls.log")
        );
    }
}
