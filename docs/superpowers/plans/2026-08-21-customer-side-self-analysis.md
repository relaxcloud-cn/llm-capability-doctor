# Customer-Side Target-Model Self-Analysis Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an opt-in Rust CLI phase that sends bounded evidence packets to the tested model through the same configured endpoint and writes validated self-analysis JSON without moving the audit log outside the customer environment.

**Architecture:** The collector remains unchanged and finishes evidence.v4 before analysis begins. A native Rust reader validates the local log, a packet/rule layer compacts each manifest's owned evidence, a protocol-aware client calls the already detected target, and a validator constrains model-authored candidates before a local orchestrator writes `self-analysis.v1`.

**Tech Stack:** Rust 2024, clap, reqwest, serde/serde_json, base64, sha2, tokio, existing protocol and redaction modules, httpmock/tempfile tests.

---

## File Map

- Modify `Cargo.toml`: add the direct SHA-256 dependency.
- Modify `src/cli.rs`: add the opt-in flag and configuration field.
- Modify `src/runner.rs`: expose the detected protocol and authentication mode after collection.
- Modify `src/protocol/mod.rs`: build high-output-budget analysis requests.
- Modify `src/lib.rs`: export the analysis module.
- Modify `src/main.rs`: run optional self-analysis after the collection summary and preserve collection success on analysis failure.
- Create `src/analysis/mod.rs`: public analyzer API and shared error/outcome types.
- Create `src/analysis/evidence_reader.rs`: strict evidence.v4 parsing, hashing, and typed records.
- Create `src/analysis/rules.rs`: compact per-check rubrics and deterministic fail gates.
- Create `src/analysis/packet.rs`: bounded evidence excerpts and deterministic batches.
- Create `src/analysis/prompt.rs`: versioned untrusted-evidence prompt and JSON response schema.
- Create `src/analysis/validator.rs`: candidate schema/reference checks and hard-gate enforcement.
- Create `src/analysis/client.rs`: same-origin protocol-native model requests and assistant-text extraction.
- Create `src/analysis/orchestrator.rs`: batch/repair loop, partial failure handling, and private output writing.
- Modify `README.md`: document the optional customer-side command and local artifact.

### Task 1: CLI And Collection Handoff

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/runner.rs`

- [ ] **Step 1: Write failing CLI and outcome tests**

Add to `src/cli.rs` tests:

```rust
#[test]
fn self_analysis_is_opt_in() {
    let disabled = Cli::try_parse_from([
        "doctor", "--url", "https://example.test/v1/chat/completions",
        "--model", "m", "--api-key", "k",
    ]).unwrap();
    assert!(!disabled.self_analyze);

    let enabled = Cli::try_parse_from([
        "doctor", "--url", "https://example.test/v1/chat/completions",
        "--model", "m", "--api-key", "k", "--self-analyze",
    ]).unwrap();
    assert!(enabled.self_analyze);
}

#[test]
fn help_exposes_no_analysis_destination() {
    let help = Cli::command().render_long_help().to_string();
    assert!(help.contains("--self-analyze"));
    assert!(!help.contains("--analysis-url"));
    assert!(!help.contains("--upload"));
}
```

Extend the existing runner protocol-detection test in `src/runner/tests.rs` with:

```rust
assert_eq!(outcome.detected_protocol, Protocol::OpenAiChat);
assert_eq!(outcome.detected_auth_mode, AuthMode::Bearer);
```

- [ ] **Step 2: Run the focused tests and verify failure**

Run:

```bash
cargo test cli::tests --lib
```

Expected: compilation fails because `Cli::self_analyze` does not exist.

- [ ] **Step 3: Add the flag, config value, and runner outcome fields**

Add to `Cli` and `Config` in `src/cli.rs`:

```rust
/// Analyze the completed evidence locally by calling the tested model through the same endpoint.
#[arg(long)]
pub self_analyze: bool,

// Config
pub self_analyze: bool,
```

Set `self_analyze: self.self_analyze` in `Cli::into_config` and `self_analyze: false` in direct test constructors.

Add to `RunOutcome` in `src/runner.rs`:

```rust
pub detected_protocol: Protocol,
pub detected_auth_mode: AuthMode,
```

Capture the two fields after `runner.execute().await?` and place them in `RunOutcome`:

```rust
let detected_protocol = runner.detected_protocol;
let detected_auth_mode = runner.detected_auth_mode;
runner.audit.finish(duration, Local::now())?;
Ok(RunOutcome {
    duration,
    request_count: runner.audit.request_count(),
    manifest_count: runner.audit.manifest_count(),
    log_path,
    detected_protocol,
    detected_auth_mode,
})
```

- [ ] **Step 4: Run focused and full tests**

Run `cargo test cli::tests --lib` and `cargo test runner::tests --lib`.

Expected: all CLI and runner tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/cli.rs src/runner.rs src/runner/tests.rs
git commit -m "feat: expose customer-side self-analysis option"
```

### Task 2: Native Evidence V4 Reader

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/lib.rs`
- Create: `src/analysis/mod.rs`
- Create: `src/analysis/evidence_reader.rs`

- [ ] **Step 1: Write parser contract tests**

Create tests inside `src/analysis/evidence_reader.rs` using a `complete_log()` helper that emits the exact header, one base64 request block, all `catalog::CATALOG` manifests, and a matching run summary. Cover:

```rust
#[test]
fn reads_complete_v4_and_preserves_ordered_refs() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("doctor.log");
    std::fs::write(&path, complete_log()).unwrap();
    let redactor = Redactor::new("secret-token", &Url::parse("https://example.test/v1/chat/completions").unwrap());

    let parsed = read(&path, &redactor).unwrap();

    assert_eq!(parsed.tests.len(), 46);
    assert_eq!(parsed.requests.len(), 1);
    assert_eq!(parsed.tests["001"].request_refs, vec!["test-shared"]);
    assert_eq!(parsed.source.sha256.len(), 64);
}

#[test]
fn rejects_incomplete_run_before_analysis() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("doctor.log");
    std::fs::write(&path, complete_log().replace("========== END ==========", "")).unwrap();
    let redactor = Redactor::new("secret-token", &Url::parse("https://example.test").unwrap());

    assert!(matches!(read(&path, &redactor), Err(EvidenceError::MissingRunSummary)));
}

#[test]
fn redacts_known_credentials_from_decoded_sections() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("doctor.log");
    std::fs::write(&path, complete_log().replace("cmVzcG9uc2U=", "c2VjcmV0LXRva2Vu" )).unwrap();
    let redactor = Redactor::new("secret-token", &Url::parse("https://example.test").unwrap());

    let parsed = read(&path, &redactor).unwrap();

    assert_eq!(parsed.requests["test-shared"].response_body, "[REDACTED]");
}
```

- [ ] **Step 2: Run the parser test and verify failure**

Run `cargo test analysis::evidence_reader::tests --lib`.

Expected: compilation fails because the analysis module and `read` do not exist.

- [ ] **Step 3: Implement typed parsing and hashing**

Add `sha2 = "0.10.9"` to dependencies and export `pub mod analysis;` from `src/lib.rs`.

Define the core records in `src/analysis/evidence_reader.rs`:

```rust
#[derive(Clone, Debug)]
pub struct ParsedEvidence {
    pub source: EvidenceSource,
    pub run: BTreeMap<String, String>,
    pub requests: BTreeMap<String, ParsedRequest>,
    pub tests: BTreeMap<String, ParsedTest>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedRequest {
    pub request_id: String,
    pub metadata: BTreeMap<String, String>,
    pub metrics: BTreeMap<String, String>,
    pub request_body: String,
    pub response_headers: String,
    pub stderr: String,
    pub response_body: String,
}

#[derive(Clone, Debug)]
pub struct ParsedTest {
    pub id: String,
    pub name: String,
    pub category: String,
    pub request_refs: Vec<String>,
}
```

Implement `read(path, redactor)` with these exact validation gates before returning:

```rust
require(&run, "script_version", env!("CARGO_PKG_VERSION"))?;
require(&run, "log_schema", "llm-capability-doctor.evidence.v4")?;
require(&run, "section_encoding", "base64")?;
require(&run, "compatibility_profile", "opencodex-2.7.42-data-format")?;
require(&run, "collector_runtime", "rust")?;
if run.contains_key("collection_profile") {
    return Err(EvidenceError::ForbiddenRunField("collection_profile"));
}
if !text.contains("========== RUN SUMMARY ==========")
    || !text.ends_with("========== END ==========\n")
{
    return Err(EvidenceError::MissingRunSummary);
}
```

Decode `REQUEST BODY`, `CURL STDERR`, and `RESPONSE BODY` with `BASE64.decode`, apply `Redactor::redact_text` immediately, reject duplicate blocks and references, require the exact 46 catalog IDs, and verify declared request/manifest counts. Compute SHA-256 with `Sha256::digest(raw)` and lowercase hex formatting.

- [ ] **Step 4: Run parser tests and regression tests**

Run `cargo test analysis::evidence_reader::tests --lib` then `cargo test --all-targets`.

Expected: parser tests and all existing tests pass.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/lib.rs src/analysis/mod.rs src/analysis/evidence_reader.rs
git commit -m "feat: parse completed evidence for local analysis"
```

### Task 3: Rules, Compaction, And Batching

**Files:**
- Create: `src/analysis/rules.rs`
- Create: `src/analysis/packet.rs`
- Modify: `src/analysis/mod.rs`

- [ ] **Step 1: Write compaction and isolation tests**

Add these tests in `src/analysis/packet.rs`:

```rust
#[test]
fn packet_contains_only_manifest_owned_requests() {
    let parsed = parsed_fixture_with_two_requests();
    let packet = build_packet(&parsed, "019").unwrap();
    assert_eq!(packet.requests.len(), 1);
    assert_eq!(packet.requests[0].request_id, "test-019");
    assert!(!serde_json::to_string(&packet).unwrap().contains("unrelated-secret-marker"));
}

#[test]
fn compaction_keeps_head_and_tail_inside_limit() {
    let input = format!("HEAD{}TAIL", "x".repeat(20_000));
    let compact = bounded_excerpt(&input, 4_096);
    assert!(compact.starts_with("HEAD"));
    assert!(compact.ends_with("TAIL"));
    assert!(compact.len() <= 4_096);
    assert!(compact.contains("bytes omitted"));
}

#[test]
fn batches_respect_count_and_serialized_byte_limits() {
    let packets = packet_fixtures(9, 20_000);
    let batches = batch_packets(packets, 4, 65_536).unwrap();
    assert!(batches.iter().all(|batch| batch.len() <= 4));
    assert!(batches.iter().all(|batch| serde_json::to_vec(batch).unwrap().len() <= 65_536));
}
```

Add a rules completeness test in `src/analysis/rules.rs`:

```rust
#[test]
fn every_catalog_check_has_one_rule() {
    let rule_ids: HashSet<_> = RULES.iter().map(|rule| rule.test_id).collect();
    let catalog_ids: HashSet<_> = CATALOG.iter().map(|test| test.id).collect();
    assert_eq!(rule_ids, catalog_ids);
}
```

- [ ] **Step 2: Run tests and verify failure**

Run `cargo test analysis:: --lib`.

Expected: compilation fails because packet and rule APIs do not exist.

- [ ] **Step 3: Implement compact rules and packets**

Define:

```rust
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisRule {
    pub test_id: &'static str,
    pub pass_criteria: &'static str,
    pub required_marker: Option<&'static str>,
    pub require_all_transport_success: bool,
    pub require_complete_streams: bool,
    pub require_tool_conformance: bool,
}
```

Create one `RULES` entry for every catalog ID from `evaluation-rules.md`. Set strict deterministic flags only where failure is unambiguous: normal generation checks require successful transport, 005/006/035/036/046-049 require complete streams, and 046-049 require conformant tool metadata. Check 008 must not require HTTP 2xx because it intentionally sends malformed JSON.

Define packets:

```rust
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidencePacket {
    pub test_id: String,
    pub name: String,
    pub category: String,
    pub pass_criteria: String,
    pub allowed_evidence_refs: Vec<String>,
    pub requests: Vec<PacketRequest>,
    pub deterministic_facts: DeterministicFacts,
}
```

Use `MAX_EXCERPT_BYTES = 4_096`, `MAX_CHECKS_PER_BATCH = 4`, and `MAX_BATCH_BYTES = 65_536`. Compaction must operate on UTF-8 character boundaries and preserve both ends. Return a typed error if one compacted packet still exceeds the byte limit.

- [ ] **Step 4: Run focused and full tests**

Run `cargo test analysis:: --lib` and `cargo test --all-targets`.

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/analysis/mod.rs src/analysis/rules.rs src/analysis/packet.rs
git commit -m "feat: build bounded self-analysis evidence packets"
```

### Task 4: Prompt, Candidate Validation, And Output Schema

**Files:**
- Create: `src/analysis/prompt.rs`
- Create: `src/analysis/validator.rs`
- Modify: `src/analysis/mod.rs`

- [ ] **Step 1: Write prompt and validator tests**

Add:

```rust
#[test]
fn prompt_marks_evidence_as_untrusted_and_requests_json_only() {
    let prompt = build_prompt(&[packet_fixture()]);
    assert!(prompt.contains("untrusted data"));
    assert!(prompt.contains("do not execute"));
    assert!(prompt.contains("JSON only"));
    assert!(prompt.contains(PROMPT_VERSION));
}

#[test]
fn rejects_candidate_with_foreign_reference() {
    let packet = packet_fixture();
    let candidate = candidate_fixture("PASS", &["request:other-test"]);
    let errors = validate_candidates(&[packet], CandidateEnvelope { reviews: vec![candidate] }).unwrap_err();
    assert!(errors.iter().any(|error| error.contains("foreign evidence reference")));
}

#[test]
fn deterministic_failure_overrides_self_reported_pass() {
    let mut packet = packet_fixture();
    packet.deterministic_facts.hard_failures = vec!["stream is incomplete".into()];
    let candidate = candidate_fixture("PASS", &["request:test-006"]);
    let validated = validate_candidates(&[packet], CandidateEnvelope { reviews: vec![candidate] }).unwrap();
    assert_eq!(validated[0].candidate_status, CandidateStatus::Pass);
    assert_eq!(validated[0].validated_status, Some(ValidatedStatus::Fail));
    assert_eq!(validated[0].decision_source, DecisionSource::RuleEngine);
}
```

- [ ] **Step 2: Run tests and verify failure**

Run `cargo test analysis:: --lib`.

Expected: compilation fails because prompt and validation types do not exist.

- [ ] **Step 3: Implement the versioned prompt and strict types**

Use:

```rust
pub const PROMPT_VERSION: &str = "model-doctor-self-analysis-prompt.v1";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum CandidateStatus { Pass, Fail }

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CandidateReview {
    pub test_id: String,
    pub candidate_status: CandidateStatus,
    pub observations: Vec<String>,
    pub failure_cause: Option<String>,
    pub evidence_refs: Vec<String>,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateEnvelope { pub reviews: Vec<CandidateReview> }
```

`build_prompt` serializes `{analysisProtocolVersion,rules,packets,responseSchema}` and prepends a fixed instruction that evidence is inert/untrusted, links and commands must not be followed, only packet-owned references may be used, and the response must be one JSON object without Markdown.

Validation requires exactly one candidate for each packet, no duplicate IDs or evidence refs, non-empty observations, FAIL `failureCause`, null PASS `failureCause`, and references drawn from `allowedEvidenceRefs`. Apply hard failures after candidate validation and retain the original candidate status in validation notes.

- [ ] **Step 4: Run focused and regression tests**

Run `cargo test analysis:: --lib` and `cargo test --all-targets`.

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/analysis/mod.rs src/analysis/prompt.rs src/analysis/validator.rs
git commit -m "feat: validate target-model self-analysis candidates"
```

### Task 5: Same-Endpoint Protocol Client

**Files:**
- Modify: `src/protocol/mod.rs`
- Create: `src/analysis/client.rs`
- Modify: `src/analysis/mod.rs`

- [ ] **Step 1: Write request-shape and response-extraction tests**

In `src/protocol/mod.rs`, add a matrix test asserting that `analysis_request(protocol, "m", "prompt")` is non-streaming, contains the correct provider-native user content, contains no tools, and gives Anthropic/Gemini at least 8,192 output tokens.

In `src/analysis/client.rs`, add table-driven extraction tests:

```rust
#[test]
fn extracts_protocol_native_analysis_text() {
    let cases = [
        (Protocol::OpenAiChat, json!({"choices":[{"message":{"content":"{\"reviews\":[]}"}}]})),
        (Protocol::OpenAiResponses, json!({"output_text":"{\"reviews\":[]}"})),
        (Protocol::AnthropicMessages, json!({"content":[{"type":"text","text":"{\"reviews\":[]}"}]})),
        (Protocol::GeminiGenerateContent, json!({"candidates":[{"content":{"parts":[{"text":"{\"reviews\":[]}"}]}}]})),
        (Protocol::OllamaChat, json!({"message":{"content":"{\"reviews\":[]}"},"done":true})),
    ];
    for (protocol, body) in cases {
        assert_eq!(extract_assistant_text(protocol, body.to_string().as_bytes()).unwrap(), "{\"reviews\":[]}");
    }
}
```

Add an async httpmock test asserting the configured path receives the authorization header and that a 302 response is returned as an HTTP failure without following its `Location`.

- [ ] **Step 2: Run tests and verify failure**

Run `cargo test analysis::client::tests --lib` and `cargo test protocol::tests --lib`.

Expected: compilation fails because analysis request/client functions do not exist.

- [ ] **Step 3: Implement request builder and client**

Add:

```rust
pub fn analysis_request(protocol: Protocol, model: &str, prompt: &str) -> RequestSpec {
    let mut request = basic_request(protocol, model, prompt, false);
    let object = request.body.as_object_mut().expect("analysis request is an object");
    match protocol {
        Protocol::OpenAiChat | Protocol::OpenAiResponses => {
            object.insert("temperature".into(), 0.into());
        }
        Protocol::AnthropicMessages => {
            object.insert("max_tokens".into(), 8192.into());
            object.insert("temperature".into(), 0.into());
        }
        Protocol::GeminiGenerateContent => {
            let generation = object.get_mut("generationConfig").and_then(Value::as_object_mut)
                .expect("Gemini generationConfig");
            generation.insert("maxOutputTokens".into(), 8192.into());
            generation.insert("temperature".into(), 0.into());
        }
        Protocol::OllamaChat => {
            object.insert("options".into(), serde_json::json!({"temperature": 0}));
        }
        Protocol::Unknown => {}
    }
    request
}
```

Define `AnalysisTarget` with URL, model, API key, timeout, TLS policy, detected protocol, and auth mode. Define an `AnalysisClient` trait whose async `analyze(&self, prompt, cancellation)` method returns `CandidateEnvelope`; this keeps orchestration testable without network I/O. `ModelAnalysisClient::analyze` must build `RequestInput`, call the existing `HttpExecutor`, require a successful transport plus HTTP 2xx, extract only the protocol-native assistant content path, and deserialize `CandidateEnvelope`. Do not pass tools or write the returned `RequestEvidence` to `AuditWriter`.

For OpenAI Responses, accept root `output_text` when present and otherwise concatenate `output[type=message].content[type=output_text].text`. Anthropic concatenates `content[type=text].text`, Gemini concatenates candidate-zero text parts, and the other protocols use their single native assistant-content field. Empty or non-string native content is `ClientError::MissingAssistantContent`.

Reject `Protocol::Unknown`. Before every request, compare the normalized URL's `(scheme, host, port_or_known_default)` with the configured origin; return `OriginChanged` before network I/O if they differ.

- [ ] **Step 4: Run client and full tests**

Run `cargo test analysis::client::tests --lib`, `cargo test protocol::tests --lib`, and `cargo test --all-targets`.

Expected: all tests pass and redirect tests observe exactly one request.

- [ ] **Step 5: Commit**

```bash
git add src/protocol/mod.rs src/analysis/mod.rs src/analysis/client.rs
git commit -m "feat: call tested model for local evidence analysis"
```

### Task 6: Orchestration, Private Output, And CLI Integration

**Files:**
- Create: `src/analysis/orchestrator.rs`
- Modify: `src/analysis/mod.rs`
- Modify: `src/main.rs`
- Modify: `README.md`

- [ ] **Step 1: Write orchestrator and main-path tests**

Use a fake client that records prompts and returns queued envelopes:

```rust
#[tokio::test]
async fn repairs_one_invalid_batch_then_succeeds() {
    let client = FakeClient::new([
        Err(ClientError::InvalidJson("missing reviews".into())),
        Ok(valid_envelope_for(&["001", "002", "003", "004"])),
    ]);
    let outcome = analyze_with_client(settings_fixture(), &client, CancellationToken::new()).await.unwrap();
    assert_eq!(client.call_count(), 2);
    assert!(client.prompts()[1].contains("missing reviews"));
    assert_eq!(outcome.unavailable_count, 0);
}

#[tokio::test]
async fn failed_batch_is_explicit_and_later_batches_continue() {
    let client = FakeClient::new([
        Err(ClientError::Transport("offline".into())),
        Ok(valid_envelope_for(&["005", "006", "007", "008"])),
    ]);
    let outcome = analyze_with_client(settings_fixture(), &client, CancellationToken::new()).await.unwrap();
    assert_eq!(outcome.unavailable_count, 4);
    assert_eq!(outcome.available_count, 4);
    assert_eq!(client.call_count(), 2);
}

#[test]
fn output_collision_never_overwrites_existing_analysis() {
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("doctor.log");
    let first = reserve_output_path(&log).unwrap();
    std::fs::write(&first, "existing").unwrap();
    let second = reserve_output_path(&log).unwrap();
    assert_ne!(first, second);
    assert_eq!(std::fs::read_to_string(first).unwrap(), "existing");
}
```

Add a CLI integration assertion that running without `--self-analyze` retains the current completion output and creates no `*-self-analysis.json` file.

- [ ] **Step 2: Run tests and verify failure**

Run `cargo test analysis::orchestrator::tests --lib`.

Expected: compilation fails because orchestration APIs do not exist.

- [ ] **Step 3: Implement orchestration and schema**

Define the top-level artifact:

```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SelfAnalysisArtifact {
    schema_version: &'static str,
    generated_at: String,
    source: AnalysisSource,
    target: AnalysisTargetMetadata,
    prompt_version: &'static str,
    provenance: &'static str,
    tests: Vec<AnalysisTestResult>,
    batches: Vec<BatchResult>,
    counts: AnalysisCounts,
}
```

Use exact constants `llm-capability-doctor.self-analysis.v1` and `TARGET_MODEL_SELF_ANALYSIS`. `AnalysisSource` contains the local source-log path, byte size, and SHA-256. `AnalysisTargetMetadata` contains only the redacted endpoint, requested model, detected protocol, and authentication mode; it never contains the API key. `AnalysisTestResult` has `analysisState`, nullable candidate/validated status, nullable decision source, observations, failure cause, evidence refs, limitations, and validation notes. Counts separate PASS, FAIL, and unavailable. `BatchResult` records test IDs, attempt count, and a redacted terminal error when the batch is unavailable.

For each batch, call once. Only invalid JSON, a missing or incomplete candidate envelope, or local validation errors receive one repair call containing the original prompt plus the concrete error. Transport, HTTP, origin-policy, and cancellation errors are not repairable and mark the batch unavailable immediately. A failed repair marks only that batch unavailable. Cancellation stops new calls, marks all remaining checks unavailable, serializes completed work, and returns an outcome with `cancelled: true`.

Create output with private permissions using a new `pub(crate)` private-file helper factored from `audit.rs`. Serialize to a temporary sibling, flush, and atomically rename it to the reserved collision-safe path so interrupted writes do not look complete.

- [ ] **Step 4: Wire `main.rs` after collection**

Before moving `Config` into `runner::run`, create optional credentials only when enabled:

```rust
let analysis_settings = config.self_analyze.then(|| AnalysisConnectionSettings {
    url: config.url.clone(),
    model: config.model.clone(),
    api_key: config.api_key.expose().to_owned(),
    timeout: config.timeout,
    insecure: config.insecure,
});
```

After printing the normal log path, combine settings with `outcome.detected_protocol` and `outcome.detected_auth_mode`, then run the analyzer under the existing cancellation token. Print `分析文件：<path>` on success. On analyzer error print `客户侧分析失败：<cause>` and still return `ExitCode::SUCCESS`; the evidence run has already completed.

- [ ] **Step 5: Document and verify customer usage**

Add to `README.md`:

```markdown
## 客户侧自分析（可选）

在原命令末尾增加 `--self-analyze`。CLI 会在 46 项检测完成后，使用同一个接口和模型分析本地证据，并在日志旁生成 `*-self-analysis.json`。CLI 不包含日志上传地址；自分析请求不会写入检测日志，也不会影响检测指标。
```

Run:

```bash
cargo fmt --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo build --release
```

Expected: formatting is clean, all tests pass, clippy emits no warnings, and the release binary builds.

- [ ] **Step 6: Commit**

```bash
git add src/analysis/mod.rs src/analysis/orchestrator.rs src/audit.rs src/main.rs README.md
git commit -m "feat: run target-model analysis inside customer environment"
```
