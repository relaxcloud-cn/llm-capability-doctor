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
fn options_without_values_keep_the_shell_error_contract() {
    for option in [
        "--url",
        "--model",
        "--api-key",
        "--log-file",
        "--timeout",
        "--only",
    ] {
        let output = command().arg(option).output().unwrap();

        assert_eq!(output.status.code(), Some(2), "option {option}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            stderr.contains(&format!("Missing value for {option}")),
            "option {option}: {stderr}"
        );
    }
}

#[test]
fn help_and_version_take_priority_over_trailing_incomplete_options() {
    let help = command().args(["--help", "--url"]).output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8(help.stdout).unwrap().contains("Usage:"));

    let version = command().args(["--version", "--url"]).output().unwrap();
    assert!(version.status.success());
    assert!(
        String::from_utf8(version.stdout)
            .unwrap()
            .contains("model-capability-doctor 0.8.0")
    );
}

#[test]
fn duplicate_only_ids_are_rejected_before_creating_a_log() {
    let directory = tempfile::tempdir().unwrap();
    let log_path = directory.path().join("doctor.log");
    let output = command()
        .args([
            "--url",
            "https://example.test/v1/chat/completions",
            "--model",
            "fixture-model",
            "--api-key",
            "fixture-key",
            "--only",
            "001,001",
            "--log-file",
            log_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("Duplicate --only test ID: 001")
    );
    assert!(!log_path.exists());
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

#[test]
fn list_tests_matches_the_shell_catalog_byte_for_byte() {
    let rust = command().arg("--list-tests").output().unwrap().stdout;
    let shell = std::process::Command::new("bash")
        .args(["model-capability-doctor.sh", "--list-tests"])
        .output()
        .unwrap()
        .stdout;

    assert_eq!(rust, shell);
}

#[test]
fn documentation_publishes_the_rust_cli_workflow_and_security_warning() {
    let readme = std::fs::read_to_string("README.md").unwrap();
    for expected in [
        "cargo build --release",
        "MODEL_API_KEY",
        "model-capability-doctor",
        "--only",
        "--list-tests",
        "--insecure",
        "危险",
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "Shell 参考实现",
    ] {
        assert!(readme.contains(expected), "README is missing {expected:?}");
    }
}

#[test]
fn workflow_verifies_macos_and_builds_both_linux_targets() {
    let workflow = std::fs::read_to_string(".github/workflows/rust-cli.yml").unwrap();
    for expected in [
        "macos-latest",
        "cargo fmt --check",
        "cargo clippy --all-targets --all-features -- -D warnings",
        "cargo test --all-targets --all-features --locked",
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "de0fac2e4500dabe0009e67214ff5f5447ce83dd",
        "21b0f18dc621b25bfae556ff2791fca4173121e8",
        "043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
    ] {
        assert!(
            workflow.contains(expected),
            "workflow is missing {expected:?}"
        );
    }
}
