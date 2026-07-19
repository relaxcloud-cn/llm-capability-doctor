use std::process::ExitCode;

use clap::Parser;
use model_capability_doctor::cli::Cli;

fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.list_tests {
        eprintln!("test catalog is not wired yet");
        return ExitCode::FAILURE;
    }

    let environment_api_key = std::env::var("MODEL_API_KEY").ok();
    match cli.into_config(environment_api_key) {
        Ok(_) => {
            eprintln!("collector is not wired yet");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}
