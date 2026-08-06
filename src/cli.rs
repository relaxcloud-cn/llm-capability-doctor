use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use thiserror::Error;
use url::Url;

#[derive(Parser)]
#[command(
    name = "model-capability-doctor",
    version,
    about = "Model Capability Doctor 0.10.0 - Run all 46 checks",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Complete model endpoint URL. The collector never rewrites it.
    #[arg(long, value_name = "URL")]
    pub url: Option<Url>,

    /// Model name sent to the endpoint.
    #[arg(long, value_name = "MODEL")]
    pub model: Option<String>,

    /// API key. Prefer MODEL_API_KEY to avoid shell history.
    #[arg(long, value_name = "KEY")]
    pub api_key: Option<String>,

    /// Audit log path. Defaults to a timestamped file in the current directory.
    #[arg(long, value_name = "PATH")]
    pub log_file: Option<PathBuf>,

    /// Per-request timeout in seconds. Defaults to 120.
    #[arg(long, default_value_t = 120, value_parser = parse_positive_integer)]
    pub timeout: u64,

    /// Print the 46-item core catalog and exit.
    #[arg(long)]
    pub list_tests: bool,

    /// Disable TLS certificate validation for controlled environments.
    #[arg(long)]
    pub insecure: bool,
}

pub struct Config {
    pub url: Url,
    pub model: String,
    pub api_key: SecretString,
    pub log_file: Option<PathBuf>,
    pub timeout: Duration,
    pub insecure: bool,
}

pub struct SecretString(String);

impl SecretString {
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn expose_for_test(&self) -> &str {
        self.expose()
    }
}

#[derive(Debug, Error)]
pub enum CliError {
    #[error("Missing required --url")]
    MissingUrl,
    #[error("Missing required --model")]
    MissingModel,
    #[error("Missing API key: set MODEL_API_KEY or pass --api-key")]
    MissingApiKey,
}

impl Cli {
    pub fn into_config(self, environment_api_key: Option<String>) -> Result<Config, CliError> {
        let url = self.url.ok_or(CliError::MissingUrl)?;
        let model = self
            .model
            .filter(|value| !value.is_empty())
            .ok_or(CliError::MissingModel)?;
        let api_key = self
            .api_key
            .or(environment_api_key)
            .filter(|value| !value.is_empty())
            .ok_or(CliError::MissingApiKey)?;
        Ok(Config {
            url,
            model,
            api_key: SecretString(api_key),
            log_file: self.log_file,
            timeout: Duration::from_secs(self.timeout),
            insecure: self.insecure,
        })
    }
}

fn parse_positive_integer(value: &str) -> Result<u64, String> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| "must be a positive integer".to_owned())?;
    if parsed == 0 {
        return Err("must be a positive integer".to_owned());
    }
    Ok(parsed)
}
