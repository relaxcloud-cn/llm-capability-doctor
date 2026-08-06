use std::process::ExitCode;

use clap::Parser;
use model_capability_doctor::catalog;
use model_capability_doctor::cli::Cli;
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
                    println!("================ 检测完成 ================");
                    println!("总耗时：{}秒", outcome.duration.as_secs());
                    println!("总请求数：{}", outcome.request_count);
                    println!("测试清单数：{}", outcome.manifest_count);
                    println!();
                    println!("日志文件：{}", outcome.log_path.display());
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
