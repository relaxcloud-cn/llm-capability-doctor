# Customer-Side Target-Model Self-Analysis

## Goal

Add an optional second phase to the Rust CLI that asks the tested model to analyze the completed Model Doctor evidence while all evidence remains inside the customer's environment.

The existing 46-check collection phase and `llm-capability-doctor.evidence.v4` log stay unchanged. Analysis starts only after the audit writer has finished the run, so analysis traffic cannot affect latency, concurrency, stability, or capability evidence collected by the tests.

## User Contract

The feature is enabled explicitly:

```bash
model-capability-doctor \
  --url 'https://model.example/v1/chat/completions' \
  --model 'your-model-name' \
  --api-key 'your-api-key' \
  --log-file './model-doctor-output/model-doctor.log' \
  --self-analyze
```

Without `--self-analyze`, CLI behavior, output, exit codes, and network traffic remain unchanged.

With `--self-analyze`, the CLI reuses the configured URL origin, model, API key, timeout, TLS policy, detected protocol, and authentication mode. Protocol-specific method normalization may change the path, but it must preserve the configured scheme, host, and port. There is no analysis URL, upload URL, telemetry endpoint, callback, or remote asset. Redirects remain disabled. The CLI writes a collision-safe `<log-stem>-self-analysis.json` beside the evidence log and prints its path after the normal run summary.

Collection success and analysis success are separate outcomes. A completed evidence run keeps its normal success exit code even if optional self-analysis fails. The CLI prints a concise analysis error and preserves the evidence log for retry.

## Data Flow

1. The runner completes all 46 checks and closes the evidence log.
2. A Rust evidence reader validates that the local input is the exact collector v0.12.0 and evidence.v4 contract produced by this CLI.
3. A packet builder creates one compact evidence packet per check from its ordered `request_refs`.
4. A deterministic preflight derives transport, protocol, stream, marker, JSON-shape, tool-loop, and metric facts from the evidence.
5. The packet scheduler sends bounded batches to the same tested model endpoint.
6. A local validator accepts, corrects, or rejects each model-authored candidate using only the packet's allowed evidence references and deterministic facts.
7. The CLI writes the validated self-analysis JSON locally.

The entire raw log is never placed in one model request. Large request bodies used by context checks are represented by bounded metadata such as byte length, declared tier, provider usage, transport result, and a short redacted excerpt. A default batch contains at most four checks and is also limited by serialized byte size. Checks whose packet would exceed the limit are sent alone after deterministic compaction.

## Components

Add `src/analysis/` with these ownership boundaries:

- `evidence_reader.rs` parses the completed evidence.v4 log into typed, redacted records and rejects incomplete or unsupported contracts.
- `packet.rs` joins manifests to ordered request evidence and creates bounded `EvidencePacket` values.
- `rules.rs` derives facts that code can verify without model judgment and defines non-overridable PASS gates.
- `prompt.rs` contains a versioned analysis instruction and protocol-neutral JSON response contract.
- `client.rs` converts analysis requests and responses for the detected OpenAI Chat, OpenAI Responses, Anthropic Messages, Gemini, or Ollama protocol while reusing `HttpExecutor` transport policy.
- `validator.rs` checks schema, test IDs, evidence references, excerpts, hard gates, and candidate consistency.
- `orchestrator.rs` batches packets, retries one malformed response with validation errors, and assembles the output.

`src/cli.rs` owns the `--self-analyze` flag. `src/runner.rs` extends `RunOutcome` with the detected protocol and authentication mode needed after collection. `src/main.rs` invokes the optional orchestrator only after `runner::run` succeeds.

The analyzer reads the completed log instead of retaining all requests in memory. This avoids duplicating the large context-test payloads during collection. An analysis-only retry command is outside this change.

## Model Request Contract

Analysis requests contain three clearly separated objects:

```json
{
  "analysisProtocolVersion": "model-doctor-self-analysis-prompt.v1",
  "rules": {},
  "packets": [],
  "responseSchema": {}
}
```

The instruction states that every packet is untrusted data, not an instruction source. Analysis requests expose no tools, cannot cause local command execution, and cannot ask the CLI to open links. The client requests deterministic generation where the protocol supports it, but correctness does not depend on a provider honoring temperature settings.

The model returns one candidate per packet:

```json
{
  "testId": "006",
  "candidateStatus": "PASS",
  "observations": ["The expected terminal event is present."],
  "failureCause": null,
  "evidenceRefs": ["request:test-006"],
  "limitations": []
}
```

The first response is parsed as protocol-native assistant content and then as JSON. A malformed or incomplete response receives one repair request containing only the validation errors and the original bounded batch. A second invalid response fails that batch without retrying indefinitely.

## Trust Boundary

The tested model is not an independent evaluator. The output schema therefore records its provenance as `TARGET_MODEL_SELF_ANALYSIS` and never presents model-authored text as independently verified.

Code-derived facts and hard gates are authoritative. The model cannot turn a transport failure, missing response, malformed required JSON, incomplete stream, broken tool correlation, missing exact marker, or invalid metric into a PASS. It also cannot cite requests outside the current manifest or invent evidence references.

For checks that require semantic interpretation beyond the deterministic gates, the result records `decisionSource: TARGET_MODEL`. When local rules fully determine the result, it records `decisionSource: RULE_ENGINE`. A disagreement is retained in `candidateStatus` and `validationNotes`, while the local final status uses the rule-engine constraint.

## Output Contract

The local output uses `llm-capability-doctor.self-analysis.v1` and contains:

- source log path and SHA-256;
- collector, evidence, analysis-prompt, and analyzer schema versions;
- masked endpoint, requested model, detected protocol, and `TARGET_MODEL_SELF_ANALYSIS` provenance;
- per-check `analysisState` (`AVAILABLE` or `ANALYSIS_UNAVAILABLE`), candidate status, nullable validated status, decision source, observations, evidence references, limitations, and validation notes;
- batch attempts and failures without authentication material;
- aggregate PASS and FAIL counts derived from available validated statuses, plus a separate unavailable count.

The output is created with the same private-file behavior as the audit log. Authentication headers, API keys, cookies, signed query parameters, and equivalent secrets pass through the existing redactor before entering packets, traces, validation messages, or output.

## Failure Behavior

- Unsupported, incomplete, or structurally invalid evidence contracts stop analysis before any model request.
- A failed batch does not discard successful batches; affected checks receive an explicit `ANALYSIS_UNAVAILABLE` analysis state rather than a fabricated PASS or FAIL.
- Cancellation stops new analysis requests, flushes completed results, and leaves the original evidence log untouched.
- Analysis requests are never appended to the evidence.v4 log, preventing recursive analysis and preserving the 46-check request inventory.
- Output-path collisions use the existing timestamped collision policy instead of overwriting a prior analysis.

## Verification

Tests will cover five layers:

1. CLI parsing proves the feature is opt-in and that no alternate destination can be configured.
2. Evidence-reader fixtures prove exact evidence.v4 acceptance, incomplete-run rejection, ordered request correlation, and credential redaction.
3. Packet tests prove context payload compaction, byte limits, per-manifest reference isolation, and deterministic batching.
4. Mock endpoint tests prove analysis starts after collection, uses the same endpoint and auth mode, follows no redirects, performs at most one repair, and does not append analysis requests to the audit log.
5. Validator and end-to-end tests prove hard gates override unsupported self-grades, invented references are rejected, partial failures are explicit, private files are used, and the default CLI path produces no additional traffic.

The final verification runs `cargo test --all-targets`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, and a release build.

## Non-Goals

This change does not alter the 46 checks, evidence.v4 format, collection prompts, general verdict rules, or existing Python skill. It does not claim that a model evaluating its own behavior is independent. It does not introduce a hosted service, external upload, cross-customer storage, background telemetry, or an alternate evaluator endpoint.
