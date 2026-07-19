use assert_cmd::Command;
use clap::Parser;
use model_capability_doctor::cli::Cli;

fn command() -> Command {
    let mut command = assert_cmd::cargo::cargo_bin_cmd!("model-capability-doctor");
    command.env_remove("MODEL_API_KEY");
    command
}

#[test]
fn help_exposes_compatible_options_and_version() {
    let output = command().arg("--help").output().unwrap();

    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "Model Capability Doctor 0.8.0",
        "--url",
        "--model",
        "--api-key",
        "--log-file",
        "--timeout",
        "--only",
        "--list-tests",
        "--insecure",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in {text}");
    }
}

#[test]
fn explicit_api_key_overrides_environment() {
    let config = Cli::try_parse_from([
        "doctor",
        "--url",
        "https://example.test/v1/chat/completions",
        "--model",
        "fixture-model",
        "--api-key",
        "argument-key",
    ])
    .unwrap()
    .into_config(Some("environment-key".into()))
    .unwrap();

    assert_eq!(config.api_key.expose_for_test(), "argument-key");
}

#[test]
fn environment_api_key_is_used_as_fallback() {
    let config = Cli::try_parse_from([
        "doctor",
        "--url",
        "https://example.test/v1/chat/completions",
        "--model",
        "fixture-model",
    ])
    .unwrap()
    .into_config(Some("environment-key".into()))
    .unwrap();

    assert_eq!(config.api_key.expose_for_test(), "environment-key");
}

#[test]
fn invalid_or_missing_values_exit_two() {
    let cases = [
        vec!["--url"],
        vec!["--model"],
        vec!["--timeout", "0"],
        vec!["--only", "063"],
        vec!["--wat"],
    ];

    for arguments in cases {
        command().args(arguments).assert().code(2);
    }
}

#[test]
fn insecure_is_disabled_by_default_and_opt_in() {
    let base = [
        "doctor",
        "--url",
        "https://example.test/v1/chat/completions",
        "--model",
        "fixture-model",
    ];
    let secure = Cli::try_parse_from(base)
        .unwrap()
        .into_config(Some("key".into()))
        .unwrap();
    let insecure = Cli::try_parse_from(base.into_iter().chain(["--insecure"]))
        .unwrap()
        .into_config(Some("key".into()))
        .unwrap();

    assert!(!secure.insecure);
    assert!(insecure.insecure);
}
