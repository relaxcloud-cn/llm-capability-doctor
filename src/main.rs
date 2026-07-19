use std::process::ExitCode;

use clap::Parser;
use model_capability_doctor::catalog;
use model_capability_doctor::cli::Cli;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.list_tests {
        print!("{}", catalog::render());
        return ExitCode::SUCCESS;
    }

    let environment_api_key = std::env::var("MODEL_API_KEY").ok();
    match cli.into_config(environment_api_key) {
        Ok(config) => {
            match model_capability_doctor::runner::run(config, CancellationToken::new()).await {
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
