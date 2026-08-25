# OpenCodex v2.7.42 Output Contract Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an offline, deterministic OpenCodex v2.7.42 output-compatibility stage that independently tests the `openai-chat`, `anthropic`, and `google` response contracts.

**Architecture:** A new `opencodex` module owns the pinned contract metadata, probe scenarios, deterministic result evaluator, and report writer. It reuses the existing HTTP transport and native stream parsers for request collection, but maps observations into stable `OCX-*` requirements without calling the model to decide pass/fail. The existing 46-check audit stays unchanged; the new optional stage writes a separate private JSON and Markdown report.

**Tech Stack:** Rust, Tokio, reqwest, serde/serde_json, existing protocol stream parsers, httpmock test server.

---

### Task 1: Define The Pinned Offline Contract

**Files:**
- Create: `src/opencodex/mod.rs`
- Create: `src/opencodex/contract.rs`
- Modify: `src/lib.rs`
- Test: `src/opencodex/contract.rs`

- [ ] **Step 1: Write the failing contract metadata test**

```rust
#[test]
fn contract_is_pinned_to_opencodex_v2742_and_has_three_adapters() {
    assert_eq!(CONTRACT.version, "v2.7.42");
    assert_eq!(CONTRACT.commit, "34493b12666a5fd69d69d730b13eabb5ec9d7235");
    assert_eq!(CONTRACT.adapters, [Adapter::OpenAiChat, Adapter::Anthropic, Adapter::Google]);
    assert_eq!(contract_digest().len(), 64);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test opencodex::contract::tests::contract_is_pinned_to_opencodex_v2742_and_has_three_adapters --lib`

Expected: compile failure because `opencodex` does not exist.

- [ ] **Step 3: Add the minimal contract types and static metadata**

```rust
pub enum Adapter { OpenAiChat, Anthropic, Google }

pub struct Contract {
    pub version: &'static str,
    pub commit: &'static str,
    pub adapters: [Adapter; 3],
}

pub static CONTRACT: Contract = Contract {
    version: "v2.7.42",
    commit: "34493b12666a5fd69d69d730b13eabb5ec9d7235",
    adapters: [Adapter::OpenAiChat, Adapter::Anthropic, Adapter::Google],
};

pub static SOURCE_FILES: [SourceFile; 4] = [
    SourceFile::new("src/adapters/openai-chat.ts", "ea32bc0aab76a954ed37e2431c56e8ec31f356dfbfbc60eedc1b4a0c870c4cac"),
    SourceFile::new("src/adapters/anthropic.ts", "8799310036aeaf5b2208cd658ea9965da4b85d8e4368d6231834b09842b25ab7"),
    SourceFile::new("src/adapters/google.ts", "7a4e07ca5351461316a428c1b12ff9359add957015b4c7d3648cb5a87a29505a"),
    SourceFile::new("src/bridge.ts", "ae6be9e720176eb867efc4b3110a82c30959ab853f887387cfa6777a650222de"),
];

pub fn contract_digest() -> String { sha256_hex(canonical_rule_metadata()) }
```

Export the module from `src/lib.rs`. Define a `Rule` type containing the stable ID, Chinese requirement text, pinned source file path, its matching fixed SHA-256 value from `SOURCE_FILES`, and one exact official test path. The contract digest is calculated from the canonical serialized rule metadata, so it changes whenever a rule changes.

- [ ] **Step 4: Run the focused test to verify it passes**

Run: `cargo test opencodex::contract --lib`

Expected: all contract metadata tests pass.

- [ ] **Step 5: Commit the contract foundation**

```bash
git add src/lib.rs src/opencodex/mod.rs src/opencodex/contract.rs
git commit -m "feat: add pinned OpenCodex contract metadata"
```

### Task 2: Evaluate Native Responses As OpenCodex Rules

**Files:**
- Create: `src/opencodex/evaluator.rs`
- Modify: `src/opencodex/mod.rs`
- Test: `src/opencodex/evaluator.rs`

- [ ] **Step 1: Write failing evaluator tests for each adapter's terminal and tool failures**

```rust
#[test]
fn chat_missing_done_fails_the_native_terminal_rule() {
    let result = evaluate_stream(Adapter::OpenAiChat, &missing_done_result());
    assert_eq!(result.failures[0].rule_id, "OCX-CHAT-STREAM-006");
    assert_eq!(result.failures[0].observed_path, "stream_termination");
}

#[test]
fn anthropic_duplicate_tool_id_fails_the_tool_correlation_rule() {
    let result = evaluate_stream(Adapter::Anthropic, &duplicate_tool_id_result());
    assert!(result.failures.iter().any(|failure| failure.rule_id == "OCX-ANTH-TOOL-005"));
}

#[test]
fn google_invalid_function_arguments_fails_the_native_argument_rule() {
    let result = evaluate_stream(Adapter::Google, &invalid_function_arguments_result());
    assert!(result.failures.iter().any(|failure| failure.rule_id == "OCX-GOOGLE-TOOL-004"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test opencodex::evaluator --lib`

Expected: compile failure because `evaluate_stream` and result types do not exist.

- [ ] **Step 3: Implement deterministic evidence-to-rule mapping**

Create these public-to-the-module types:

```rust
pub struct RuleFailure {
    pub rule_id: &'static str,
    pub requirement: &'static str,
    pub observed_path: String,
    pub actual: String,
    pub effect: &'static str,
}

pub struct AdapterResult {
    pub adapter: Adapter,
    pub passed: bool,
    pub failures: Vec<RuleFailure>,
}
```

Map `StreamParseResult.stream_termination`, `contract_errors`, `assistant_turn`, `tool_calls`, and `model_stop_reason` into the pinned rules. Do not use `analysis::prompt`, `analysis::validator`, or a target-model judgement. Deduplicate equivalent failures but retain every distinct response path.

The initial rule catalog must cover malformed/truncated streams, native terminal conditions, error envelopes, invalid text/candidate containers, invalid usage placement, tool ID/name/index/argument shapes, duplicate/conflicting calls, dangling calls, and invalid final completion. Each mapping must use the adapter-specific rule prefix.

- [ ] **Step 4: Run the focused tests to verify they pass**

Run: `cargo test opencodex::evaluator --lib`

Expected: evaluator tests pass and each failure contains a rule ID, requirement, path, actual value type, and effect.

- [ ] **Step 5: Commit the evaluator**

```bash
git add src/opencodex/mod.rs src/opencodex/evaluator.rs
git commit -m "feat: evaluate native streams against OpenCodex rules"
```

### Task 3: Run Three Automatic Adapter Probe Suites

**Files:**
- Create: `src/opencodex/runner.rs`
- Modify: `src/opencodex/mod.rs`
- Test: `src/opencodex/runner.rs`

- [ ] **Step 1: Write failing HTTP fixture tests for independent adapter results**

```rust
#[tokio::test]
async fn runner_reports_chat_pass_and_anthropic_and_google_failures_independently() {
    let outcome = run(settings_for_fixture_server()).await.unwrap();
    assert!(outcome.result(Adapter::OpenAiChat).passed);
    assert!(!outcome.result(Adapter::Anthropic).passed);
    assert!(!outcome.result(Adapter::Google).passed);
}

#[tokio::test]
async fn runner_records_an_http_rejection_as_the_adapter_shape_failure() {
    let outcome = run(settings_for_rejecting_server()).await.unwrap();
    let failure = &outcome.result(Adapter::Google).failures[0];
    assert_eq!(failure.observed_path, "http_status");
    assert_eq!(failure.actual, "404");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test opencodex::runner --lib`

Expected: compile failure because the OpenCodex runner does not exist.

- [ ] **Step 3: Implement adapter-native request scenarios and follow-up loops**

Create `OpenCodexSettings` from the existing endpoint, model, API key, timeout, TLS setting, and cancellation token. For every adapter, execute these scenarios independently:

```text
ordinary response
streamed marker response
single function call
function result followed by final marker
two sequential function calls followed by final marker
```

Use `basic_request`, `tool_request`, `normalize_request_url`, `HttpExecutor`, and `parse_stream` with the corresponding existing protocol:

```text
openai-chat -> Protocol::OpenAiChat / AuthMode::Bearer
anthropic   -> Protocol::AnthropicMessages / AuthMode::XApiKey
google      -> Protocol::GeminiGenerateContent / AuthMode::XGoogApiKey
```

For each request, retain redacted response evidence and run the deterministic evaluator. Build a native follow-up request only when the preceding tool call is valid; otherwise record the failed rule and continue with the next adapter. Do not stop the suite after a different adapter succeeds.

- [ ] **Step 4: Run focused tests to verify they pass**

Run: `cargo test opencodex::runner --lib`

Expected: fixture server proves all three candidates run independently and failed HTTP/shape/stream/tool cases report deterministic failures.

- [ ] **Step 5: Commit the automatic probe runner**

```bash
git add src/opencodex/mod.rs src/opencodex/runner.rs
git commit -m "feat: run automatic OpenCodex adapter probes"
```

### Task 4: Write Private Offline Reports With Exact Failures

**Files:**
- Create: `src/opencodex/report.rs`
- Modify: `src/opencodex/mod.rs`
- Test: `src/opencodex/report.rs`

- [ ] **Step 1: Write failing report tests**

```rust
#[test]
fn markdown_reports_only_pass_or_fail_and_explains_each_failure() {
    let report = render_markdown(&fixture_outcome());
    assert!(report.contains("OpenCodex v2.7.42 / openai-chat: FAIL"));
    assert!(report.contains("OCX-CHAT-TOOL-004: FAIL"));
    assert!(report.contains("Observed response: choices[0].delta.tool_calls[0].function.name"));
    assert!(!report.contains("unsupported"));
    assert!(!report.contains("analysis unavailable"));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test opencodex::report --lib`

Expected: compile failure because report functions do not exist.

- [ ] **Step 3: Implement JSON and Markdown report writers**

Write a collision-safe `*-opencodex-v2742.json` and `*-opencodex-v2742.md` beside the normal audit log with `create_new_private_file`. Include contract version, commit, contract digest, adapter verdicts, and each `RuleFailure`. Redact all API keys, request text, response values, URLs, and headers before serialization. Preserve only structural paths, technical type names, rule IDs, requirement text, and effects.

- [ ] **Step 4: Run focused tests to verify they pass**

Run: `cargo test opencodex::report --lib`

Expected: report tests prove exact failure formatting, no unavailable category, private collision-safe output, and redaction.

- [ ] **Step 5: Commit report generation**

```bash
git add src/opencodex/mod.rs src/opencodex/report.rs
git commit -m "feat: report OpenCodex compatibility failures"
```

### Task 5: Add The Optional CLI Stage And Contract Artifact Verification

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/main.rs`
- Modify: `src/opencodex/contract.rs`
- Create: `tests/opencodex_cli.rs`
- Test: `src/cli.rs`, `tests/opencodex_cli.rs`

- [ ] **Step 1: Write failing CLI tests**

```rust
#[test]
fn opencodex_compatibility_is_opt_in() {
    let cli = Cli::try_parse_from(["doctor", "--url", "https://example.test", "--model", "m", "--api-key", "k"])
        .unwrap();
    assert!(!cli.opencodex_compatibility);
}

#[test]
fn cli_accepts_the_opencodex_compatibility_flag() {
    let cli = Cli::try_parse_from(["doctor", "--url", "https://example.test", "--model", "m", "--api-key", "k", "--opencodex-compatibility"])
        .unwrap();
    assert!(cli.opencodex_compatibility);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test cli::tests::opencodex_compatibility --lib`

Expected: compile failure because the flag and configuration field do not exist.

- [ ] **Step 3: Wire the stage after normal collection**

Add `--opencodex-compatibility`. Preserve the current default behavior and the exact 46-check evidence contract when the flag is absent. When enabled, invoke `opencodex::runner::run` after `runner::run`, print the three adapter pass/fail results and report locations, and return failure only for transport/output-write failures, not for an adapter compatibility `FAIL` verdict.

Generate the embedded contract digest from canonical serialized rule metadata in a deterministic unit-tested function. The first release stores source-file SHA-256 values in the generated contract module; the release validation test must reject empty, duplicate, or malformed digests.

- [ ] **Step 4: Run focused and full verification**

Run: `cargo test cli::tests::opencodex_compatibility --lib && cargo test --test opencodex_cli && cargo test`

Expected: CLI flag tests pass, integration report is emitted only with the flag, and the full suite passes.

- [ ] **Step 5: Commit CLI integration**

```bash
git add src/cli.rs src/main.rs src/opencodex/contract.rs tests/opencodex_cli.rs
git commit -m "feat: expose OpenCodex compatibility checks"
```

### Task 6: Verify The Release Is Offline And Reproducible

**Files:**
- Modify: `README.md`
- Test: `src/opencodex/contract.rs`, `tests/opencodex_cli.rs`

- [ ] **Step 1: Write the failing offline-contract test**

```rust
#[test]
fn contract_has_no_runtime_source_url_or_download_instruction() {
    let serialized = contract_json();
    assert!(!serialized.contains("https://"));
    assert!(!serialized.contains("npm"));
    assert!(!serialized.contains("bun"));
}
```

- [ ] **Step 2: Run the test to verify it fails or exposes an unwanted value**

Run: `cargo test opencodex::contract::tests::contract_has_no_runtime_source_url_or_download_instruction --lib`

Expected: fail until the serialized artifact is constrained to local metadata.

- [ ] **Step 3: Document and enforce the offline invocation**

Add one README command showing `--opencodex-compatibility`, the three automatic candidates, the fixed OpenCodex version, and the rule-level failure report. Do not document any Node.js, Bun, Docker, runtime source checkout, or adapter-selection flag.

- [ ] **Step 4: Run final verification**

Run: `cargo fmt --check && cargo test && cargo run -- --help`

Expected: formatting and all tests pass; help contains `--opencodex-compatibility` and contains no adapter-selection flag.

- [ ] **Step 5: Commit release documentation and verification**

```bash
git add README.md src/opencodex/contract.rs tests/opencodex_cli.rs
git commit -m "docs: describe offline OpenCodex compatibility checks"
```
