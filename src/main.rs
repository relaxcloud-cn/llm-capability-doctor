use std::path::Path;
use std::process::ExitCode;

use clap::Parser;
use model_capability_doctor::analysis::orchestrator::{
    AnalysisConnectionSettings, AnalysisOutcome,
};
use model_capability_doctor::catalog;
use model_capability_doctor::cli::Cli;
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
    println!("========== 自分析完成 ==========");
    println!(
        "可用：{}（PASS {} / FAIL {}）| 不可用：{}",
        outcome.available_count, outcome.pass_count, outcome.fail_count, outcome.unavailable_count
    );
    let directory = outcome.path.parent().unwrap_or_else(|| Path::new("."));
    println!("输出目录：{}", terminal::display_path(directory));
    println!("JSON：{}", file_name(&outcome.path));
    println!("结果：{}", file_name(&outcome.markdown_path));
    if outcome.cancelled {
        println!("分析状态：已取消；未完成批次已标记为 ANALYSIS_UNAVAILABLE");
    }
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
