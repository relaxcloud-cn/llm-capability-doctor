# Rust CLI Model Capability Doctor Design

## Summary

Build a native Rust implementation of Model Capability Doctor on the
`codex/rust-cli` branch. The new `model-capability-doctor` binary replaces the
Shell script at runtime while preserving its external command contract, all 62
tests, all five supported model protocols, and the
`llm-capability-doctor.evidence.v1` audit-log contract consumed by the existing
report-generation skill.

The Rust CLI must run without `bash` or `curl`. Release artifacts target Linux
x86_64 and Linux ARM64, while macOS remains a supported development and test
platform. The existing Shell script stays on the branch as a behavior reference
and remains unchanged.

## Goals

- Provide one native `model-capability-doctor` executable.
- Preserve all 62 catalog entries and their request behavior.
- Detect and exercise OpenAI Chat Completions, OpenAI Responses, Anthropic
  Messages, Gemini GenerateContent, and Ollama Chat endpoints.
- Preserve the existing evidence-only boundary: collect auditable input and
  output without grading a model inside the CLI.
- Produce logs that the current Python report parser can consume without
  modification.
- Preserve current options and environment-variable behavior, and add an
  explicit `--insecure` option for controlled self-signed-certificate
  environments.
- Make protocol, request, logging, and test behavior independently testable.

## Non-Goals

- Removing or modifying `model-capability-doctor.sh`.
- Moving report assessment or HTML generation into the Rust binary.
- Adding custom request headers, custom CA files, retries, interactive prompts,
  or configuration files in the first Rust release.
- Changing the evidence schema or adding pass/fail judgments.

## Command Contract

The binary is named `model-capability-doctor` and supports:

```text
MODEL_API_KEY='secret' model-capability-doctor --url URL --model MODEL [options]

Required:
  --url URL
  --model MODEL
  MODEL_API_KEY or --api-key KEY

Options:
  --api-key KEY
  --log-file PATH
  --timeout SECONDS
  --only IDS
  --list-tests
  --insecure
  -h, --help
```

Argument names, required-value behavior, positive-integer timeout validation,
comma-separated three-digit test IDs, and the default timestamped log path
remain compatible with the Shell implementation. An explicit `--api-key`
overrides `MODEL_API_KEY`, matching the current behavior.
Unknown or invalid arguments exit with code 2. Startup failures exit with code
1. A completed evidence collection exits with code 0 even when individual
requests record network or model errors.

`--insecure` is opt-in. Without it, TLS certificates are verified against
system trust roots. With it, certificate validation is disabled, the run header
contains `tls_verification: disabled`, and each rendered audit command contains
`--insecure`. Documentation must warn that this option is intended only for
controlled environments.

## Architecture

The repository root becomes a Cargo package with one binary target. Production
code is divided by responsibility:

- `src/main.rs`: process entry point, signal-aware shutdown, and exit-code
  mapping.
- `src/cli.rs`: command-line parsing, environment fallback, validation, and
  runtime configuration.
- `src/catalog.rs`: the static 62-item catalog, selection, lookup, and `--only`
  validation.
- `src/protocol/`: protocol-neutral types plus adapters for OpenAI Chat,
  OpenAI Responses, Anthropic Messages, Gemini GenerateContent, and Ollama Chat.
- `src/http.rs`: native HTTP execution, streaming reads, timeout handling,
  response metrics, proxy support, and cancellation.
- `src/audit.rs`: evidence-v1 rendering, incremental file writes, request
  references, and run summaries.
- `src/redaction.rs`: API-key masking plus URL, header, response, and error-text
  secret redaction.
- `src/checks/`: focused modules for interface, structured-output, context,
  text, thinking, tool, performance, and guardrail request flows.
- `src/runner.rs`: protocol probing, selected-test dispatch, reusable sample
  coordination, concurrent batches, and run lifecycle.

`clap` defines the command contract. `tokio` supplies async execution,
cancellation, and concurrency. `reqwest` with Rustls performs HTTP without a
native OpenSSL dependency. `serde` and `serde_json` build protocol payloads and
extract tool-call fields without ad hoc JSON parsing.

The HTTP client honors `HTTP_PROXY`, `HTTPS_PROXY`, and `NO_PROXY`. Rustls reads
system trust roots so properly installed private certificate authorities work
without disabling verification.

## Execution Flow

1. Parse and validate options without exposing the API key.
2. Resolve and create the log file, set mode `0600`, and write the run header.
3. Build the selected catalog from `--only`, or select all 62 entries.
4. Run test 001 directly when selected.
5. Before the first later test, probe the current seven protocol/auth
   combinations in the same order as the Shell implementation:
   OpenAI Chat bearer, OpenAI Responses bearer, Anthropic x-api-key, Gemini
   x-goog-api-key, Ollama bearer, OpenAI Chat api-key, and OpenAI Responses
   api-key.
6. Use the selected adapter to create every later request and any protocol-
   specific tool follow-up.
7. Append a complete request audit immediately after each request finishes.
8. Append each test manifest with its exact request references.
9. Append the run summary and print the final log path.

Tests 002, 003, and 007 reuse protocol-probe evidence as the Shell version does.
Tests 055 and 056 share five repeat samples. Test 057 sends gated concurrent
batches of 4, 8, 16, and 32 requests. Test 058 sends ten sustained requests and
a recovery probe. Tool tests 047, 048, and 049 construct a second request from
the actual first response.

## Audit Log Compatibility

The log continues to use `llm-capability-doctor.evidence.v1` and preserves the
current block delimiters, field names, and ordering needed by the parser:

- run header;
- `REQUEST <id>` blocks;
- rendered request command;
- request body;
- response metrics;
- response headers;
- request error output in the existing curl-stderr section name for schema
  compatibility;
- response body;
- `TEST-<id>` manifests;
- run summary and final delimiter.

The legacy field `script_version` remains in the run header because it is part
of the existing contract, even though the producer is a Rust binary. The run
header also includes `collector_runtime: rust`; the parser tolerates additional
header fields. The initial Rust version is `0.8.0`.

The rendered request command remains a reproducible, redacted curl command for
human auditing. It describes the equivalent HTTP request but is never executed
by the Rust program.

Time-to-first-byte is measured when the first response-body bytes arrive. Total
time ends after the body is fully read. Response size is the actual byte count.
HTTP status, headers, network errors, partial bodies, and timeout evidence are
retained even when a request does not succeed.

## Security and Failure Handling

The full API key is never written to normal terminal output. The run header uses
the existing first-four/last-four mask for keys longer than eight characters
and `[MASKED]` for shorter keys. The redactor replaces:

- the API key wherever it occurs;
- credential values from `api_key`, `key`, `token`, and `access_token` URL query
  parameters;
- Authorization, Proxy-Authorization, api-key, x-api-key, x-goog-api-key, and
  Set-Cookie header values;
- known secrets echoed in response bodies or error messages.

The log file is opened with owner-only permissions before requests begin.
Temporary artifacts, if any, use an owner-only temporary directory and are
removed at shutdown.

DNS, connection, TLS, timeout, HTTP, response-decoding, and model-protocol
problems are request evidence, not run-fatal errors. Invalid arguments, an
unusable output path, failure to initialize TLS or the HTTP client, and failure
to secure the log file are run-fatal.

SIGINT and SIGTERM cancel outstanding async tasks. Completed evidence is
flushed before exiting with 130 and 143 respectively. Log writes are serialized
through the audit writer so concurrent requests cannot interleave blocks.

## Testing Strategy

Development follows red-green-refactor. Production behavior is introduced only
after a test demonstrates the missing behavior.

Unit tests cover:

- help and argument validation;
- all 62 catalog rows and selection order;
- request JSON for every protocol and check family;
- protocol-response recognition;
- API-key, URL, header, body, and error redaction;
- audit block formatting and request-reference aggregation;
- tool-call extraction and protocol-specific follow-up bodies.

Integration tests start a local fixture model server and execute the real
binary without external network access. Fixtures cover all five protocols,
streaming events, tool calls and tool results, malformed responses, HTTP
errors, delayed responses, TLS behavior, shared samples, signal cancellation,
and the 4/8/16/32 concurrency schedule.

Compatibility tests feed Rust-produced logs to the existing Python
`model_doctor_log.py` parser and assert that all 62 manifests and their evidence
are readable. Golden assertions cover schema markers and stable field ordering;
volatile timestamps, ports, and timings are normalized rather than frozen.

The full verification gate is:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release
```

CI runs these checks on macOS and produces Linux x86_64 and Linux ARM64 release
artifacts. Release builds use locked dependencies and include `Cargo.lock`.

## Acceptance Criteria

- `model-capability-doctor --list-tests` prints the same 62 catalog rows.
- All existing options behave compatibly, and `--insecure` has the documented
  opt-in behavior.
- A full run and any valid `--only` subset execute through native Rust HTTP.
- All five protocols can be detected and exercised.
- Streaming, multi-turn, tool follow-up, repeat, stress, and recovery requests
  match the Shell request contracts.
- Logs contain complete evidence, no CLI judgments, and no unredacted known
  secrets.
- Existing report tooling reads Rust-generated evidence-v1 logs without code
  changes.
- The release binary does not invoke or require `bash` or `curl`.
- Tests, formatting, linting, and release builds pass on the development
  platform, and CI defines both required Linux release targets.
