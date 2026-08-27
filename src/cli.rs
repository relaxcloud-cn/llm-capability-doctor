use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use thiserror::Error;
use url::Url;

#[derive(Parser)]
#[command(
    name = "model-capability-doctor",
    version,
    about = concat!("Model Capability Doctor ", env!("CARGO_PKG_VERSION"), " - Run all 42 checks"),
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Complete model endpoint URL. Fragments are rejected; Google streaming methods may be derived.
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

    /// Per-request timeout in seconds. Defaults to 300.
    #[arg(long, default_value_t = 300, value_parser = parse_positive_integer)]
    pub timeout: u64,

    /// Print the 42-item catalog and exit.
    #[arg(long)]
    pub list_tests: bool,

    /// Disable TLS certificate validation for controlled environments.
    #[arg(long)]
    pub insecure: bool,

    /// Analyze completed evidence by calling the tested model through the same endpoint. Enabled by default.
    #[arg(long, default_value_t = true)]
    pub self_analyze: bool,

    /// Test whether model responses satisfy LLM gateway output contracts. Enabled by default.
    #[arg(long, default_value_t = true)]
    pub llm_gateway_compatibility: bool,
}

pub struct Config {
    pub url: Url,
    pub model: String,
    pub api_key: SecretString,
    pub log_file: Option<PathBuf>,
    pub timeout: Duration,
    pub insecure: bool,
    pub self_analyze: bool,
    pub llm_gateway_compatibility: bool,
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
    #[error("Endpoint URL fragments are not allowed")]
    UrlFragmentNotAllowed,
    #[error("Missing required --model")]
    MissingModel,
    #[error("Missing API key: set MODEL_API_KEY or pass --api-key")]
    MissingApiKey,
}

impl Cli {
    pub fn into_config(self, environment_api_key: Option<String>) -> Result<Config, CliError> {
        let url = self.url.ok_or(CliError::MissingUrl)?;
        if url.fragment().is_some() {
            return Err(CliError::UrlFragmentNotAllowed);
        }
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
            self_analyze: self.self_analyze,
            llm_gateway_compatibility: self.llm_gateway_compatibility,
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

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};
    use url::Url;

    use super::{Cli, CliError};

    fn cli_with_url(value: &str) -> Cli {
        Cli {
            url: Some(Url::parse(value).unwrap()),
            model: Some("test-model".to_owned()),
            api_key: Some("secret".to_owned()),
            log_file: None,
            timeout: 300,
            list_tests: false,
            insecure: false,
            self_analyze: false,
            llm_gateway_compatibility: false,
        }
    }

    #[test]
    fn self_analysis_is_enabled_by_default() {
        let default = Cli::try_parse_from([
            "doctor",
            "--url",
            "https://example.test/v1/chat/completions",
            "--model",
            "m",
            "--api-key",
            "k",
        ])
        .unwrap();
        assert!(default.self_analyze);
        assert_eq!(default.timeout, 300);

        let enabled = Cli::try_parse_from([
            "doctor",
            "--url",
            "https://example.test/v1/chat/completions",
            "--model",
            "m",
            "--api-key",
            "k",
            "--self-analyze",
        ])
        .unwrap();
        assert!(enabled.self_analyze);
    }

    #[test]
    fn llm_gateway_compatibility_is_enabled_by_default() {
        let default = Cli::try_parse_from([
            "doctor",
            "--url",
            "https://example.test/v1/chat/completions",
            "--model",
            "m",
            "--api-key",
            "k",
        ])
        .unwrap();
        assert!(default.llm_gateway_compatibility);

        let enabled = Cli::try_parse_from([
            "doctor",
            "--url",
            "https://example.test/v1/chat/completions",
            "--model",
            "m",
            "--api-key",
            "k",
            "--llm-gateway-compatibility",
        ])
        .unwrap();
        assert!(enabled.llm_gateway_compatibility);
    }

    #[test]
    fn help_exposes_no_analysis_destination() {
        let help = Cli::command().render_long_help().to_string();

        assert!(help.contains("--self-analyze"));
        assert!(!help.contains("--analysis-url"));
        assert!(!help.contains("--upload"));
    }

    fn config_error(value: &str) -> CliError {
        match cli_with_url(value).into_config(None) {
            Ok(_) => panic!("URL fragment was accepted"),
            Err(error) => error,
        }
    }

    #[test]
    fn rejects_non_empty_endpoint_fragment() {
        let error = config_error("https://example.test/v1/chat/completions#section");

        assert!(matches!(&error, CliError::UrlFragmentNotAllowed));
        assert_eq!(error.to_string(), "Endpoint URL fragments are not allowed");
    }

    #[test]
    fn rejects_empty_endpoint_fragment() {
        let error = config_error("https://example.test/v1/chat/completions#");

        assert!(matches!(&error, CliError::UrlFragmentNotAllowed));
        assert_eq!(error.to_string(), "Endpoint URL fragments are not allowed");
    }

    #[test]
    fn accepts_fragment_free_endpoint() {
        let config = cli_with_url("https://example.test/v1/chat/completions")
            .into_config(None)
            .unwrap();

        assert_eq!(
            config.url.as_str(),
            "https://example.test/v1/chat/completions"
        );
    }

    #[test]
    fn help_describes_the_42_item_catalog() {
        let help = Cli::command().render_long_help().to_string();

        assert!(help.contains("Run all 42 checks"));
        assert!(help.contains("Print the 42-item catalog and exit"));
        assert!(!help.contains("43 checks"));
        assert!(!help.contains("43-item"));
    }
}
