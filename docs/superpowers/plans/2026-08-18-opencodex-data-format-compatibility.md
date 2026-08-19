# OpenCodex Data-Format Compatibility Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Make Model Doctor v0.11 collect and report evidence that directly tests the model-side data formats consumed by OpenCodex 2.7.42, without running OpenCodex or ClawOps.

**Architecture:** Keep the Rust collector evidence-only. Tighten its protocol requests, response detection, SSE reading, tool loops, and Google URL derivation; then let the Python report pipeline deterministically derive the OpenCodex compatibility result from eight reviewed checks. Version the evidence and assessment contracts so old logs remain readable without inheriting the new conclusion.

**Tech Stack:** Rust 2024, reqwest/tokio/serde_json, Python 3 unittest, JSON Schema, static HTML/CSS.

---

## Task 1: Enforce protocol-native synchronous envelopes

**Files:**
- Modify: `src/protocol/mod.rs`
- Create: `src/protocol/tests.rs`

1. Add failing unit tests for valid OpenAI Chat, OpenAI Responses, Anthropic Messages, Google GenerateContent, and Ollama envelopes. Assert nested lookalike keys, empty required arrays or IDs, wrong field types, and text outside model-visible paths are rejected.
2. Run `cargo test protocol::tests::matches_response -- --nocapture` and confirm failures come from the current recursive key search.
3. Replace recursive `has_key`, `has_string`, and `has_string_prefix` matching with exact root-path and field-type checks. Keep Ollama detectable but do not classify it as OpenCodex-compatible here.
4. Re-run the focused test, then `cargo test --lib`.
5. Commit as `feat: enforce protocol response envelopes`.

## Task 2: Inspect SSE incrementally and honor OpenCodex termination rules

**Files:**
- Create: `src/protocol/stream.rs`
- Modify: `src/protocol/mod.rs`
- Modify: `src/http.rs`

1. Add failing tests for LF/CRLF records, multiple `data` lines, records split across chunks, a final record without newline, and protocol-specific malformed JSON behavior.
2. Add failing tests for immediate stop on Chat `[DONE]`, explicit error events, and Responses failed/incomplete; assert Chat finish reason/usage, Responses completed, Anthropic message stop/stop reason, and Google finish reason/usage only become successful after normal EOF.
3. Run `cargo test protocol::stream -- --nocapture` and confirm the tests fail because no stream inspector exists.
4. Implement `StreamInspector` with `push`, `finish`, terminal state, and `Continue|Stop`. Preserve the first error, accept only `data: ` for Chat and `data:` for Google, and drop malformed Anthropic JSON while failing malformed Chat/Responses/Google frames.
5. Add local TCP fixture tests in `src/http.rs`: append the complete received chunk before inspection; immediate terminals may stop; fallback terminals must wait for EOF and still timeout if the server hangs; no terminal at EOF is recorded as an incomplete stream.
6. Run `cargo test protocol::stream http -- --nocapture`, then `cargo test --lib`.
7. Commit as `feat: validate protocol stream termination`.

## Task 3: Match OpenCodex tool naming, streaming, and call correlation

**Files:**
- Modify: `src/protocol/tools.rs`
- Create: `src/protocol/tools_tests.rs`
- Modify: `src/checks/tools.rs` only if the request builder API requires it

1. Add failing request-contract tests: checks 040 and 045 are streaming for all four supported families; non-Google bodies carry `stream:true`; 041 and 047 use native Responses namespace `doctor/get_weather`, while Chat/Anthropic/Google expose `doctor__get_weather` plus bare `get_time`.
2. Add failing follow-up tests: Chat uses `tool_call_id`, Responses uses `call_id`, Anthropic uses `tool_use_id`; Google always replaces any upstream ID with a deterministic local ID and replays the same ID in both `functionCall` and `functionResponse`.
3. Run `cargo test protocol::tools_tests -- --nocapture` and confirm the current non-stream/bare-name/upstream-Google-ID behavior fails.
4. Implement the minimal request and observation changes. Keep 047 non-streaming because its second turn consumes the first JSON response. Require non-empty protocol-native IDs and names where OpenCodex requires them.
5. Add representative 040/045 stream fixtures proving tool delta frames do not terminate the stream before the native terminal signal.
6. Run `cargo test protocol::tools_tests protocol::stream -- --nocapture`, then `cargo test --lib`.
7. Commit as `feat: align tool contracts with OpenCodex`.

## Task 4: Derive Google streaming URLs and emit evidence v3

**Files:**
- Modify: `src/runner.rs`
- Modify: `src/audit.rs`
- Modify: `src/cli.rs`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `README.md`

1. Add failing `runner` tests for Google stream URLs: convert a terminal `:generateContent` path, retain an existing `:streamGenerateContent`, replace or add only `alt=sse`, preserve other query pairs, and leave non-stream, non-Google, and custom paths byte-for-byte unchanged.
2. Run `cargo test runner::tests -- --nocapture` and confirm the configured URL is currently returned unchanged.
3. Implement a pure URL resolver and call it from `request_input`, so evidence records the actual URL used.
4. Add failing audit tests for `script_version: 0.11.0`, `log_schema: llm-capability-doctor.evidence.v3`, and `compatibility_profile: opencodex-2.7.42-data-format`; then centralize those constants and update CLI/package/readme release names.
5. Run focused tests, `cargo fmt --check`, and `cargo test --lib`.
6. Commit as `feat: emit OpenCodex evidence v3`.

## Task 5: Derive assessment v7 OpenCodex compatibility

**Files:**
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_log.py`
- Create: `skills/creating-model-doctor-reports/scripts/model_doctor_opencodex_compatibility.py`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py`
- Modify: `skills/creating-model-doctor-reports/references/assessment-schema.json`
- Create: `skills/creating-model-doctor-reports/tests/test_model_doctor_opencodex_compatibility.py`
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_v6.py`
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_general_verdict.py`

1. Add failing parser tests for the exact v0.11/evidence.v3/profile tuple, all 46 manifests, missing/unknown profile, mixed versions, forbidden `collection_profile`, and continued v1/v2 support.
2. Add failing derivation tests for the eight required IDs `002,004,005,006,040,041,043,047`: four supported families plus all PASS gives `PASS`; unsupported/unknown protocol or any required failure gives `FAIL`; test 045 has no effect; v1/v2 gives `NOT_ASSESSED`.
3. Run the new unittest module and confirm failures are due to missing v3 and derivation support.
4. Implement the v3 parser contract and pure derivation module with fixed profile, labels, statements, ordering, and scope boundary.
5. Upgrade assessment output to v7. Keep reviews at v2 and forbid reviewers from supplying `openCodexCompatibility`; assemble it from parsed run metadata, final test statuses, and verified protocol family; independently recompute it in validation to reject tampering.
6. Extend the closed JSON schema with all eight required compatibility fields and constants. Update existing version assertions without weakening legacy behavior.
7. Run `python3 -m unittest discover -s skills/creating-model-doctor-reports/tests -p 'test_*.py' -v`.
8. Commit as `feat: derive OpenCodex compatibility verdict`.

## Task 6: Render, document, and verify the complete contract

**Files:**
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_html.py`
- Modify: `skills/creating-model-doctor-reports/assets/report.css`
- Modify: `skills/creating-model-doctor-reports/references/evaluation-rules.md`
- Modify: `skills/creating-model-doctor-reports/SKILL.md`
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_v6.py`
- Modify: `README.md`

1. Add failing HTML tests for PASS/FAIL/NOT_ASSESSED, escaped dynamic fields, a single compatibility section, and exact section order: detection information, OpenCodex compatibility, general verdict, domain table, evidence.
2. Implement an unframed compatibility band showing label, statement, profile, protocol family, required/failed IDs, and the fixed scope boundary. Add responsive wrapping and print protection without introducing deployment or READY/BLOCKED wording.
3. Update evaluation rules for exact envelopes, SSE records and terminals, streamed 040/045 reconstruction, namespace 041, schema 043, and correlated 047. State explicitly that only the eight IDs gate compatibility and 045 remains enhanced/general capability.
4. Update the skill workflow and README for v0.11/evidence.v3/assessment.v7 and the sole Google streaming URL exception.
5. Run fresh verification:
   - `cargo fmt --check`
   - `cargo clippy --all-targets --all-features -- -D warnings`
   - `cargo test --all-targets --all-features --locked`
   - `cargo build --release --locked`
   - `python3 -m unittest discover -s skills/creating-model-doctor-reports/tests -p 'test_*.py' -v`
6. Review the full diff against the approved design, run a final independent code review, and fix every Critical or Important finding.
7. Commit as `docs: document OpenCodex compatibility contract` only if this task has remaining changes; otherwise amend nothing.

## Completion Criteria

- Collector v0.11 emits evidence.v3 with the exact OpenCodex 2.7.42 data-format profile.
- The eight hard checks distinguish envelope, stream, namespace, schema, tool reconstruction, and call-correlation failures.
- Google streaming URL and internal tool-call ID behavior match the inspected adapter source.
- Assessment v7 deterministically reports PASS, FAIL, or NOT_ASSESSED; v1/v2 remain readable and never gain a retroactive compatibility decision.
- All verification commands pass using only local fixtures; OpenCodex, ClawOps, customer endpoints, and network access are not required.
