# Official Streaming Tool Loop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collect and report an official, normally terminated, fully correlated streaming tool loop for checks 046 through 049 across OpenAI Chat Completions, OpenAI Responses, Anthropic Messages, Gemini GenerateContent, and Ollama Chat.

**Architecture:** Keep HTTP transport, runtime protocol parsing, tool-loop orchestration, and report assessment separate. HTTP retains raw and partial bytes plus a transport outcome; lean protocol-specific Rust parsers normalize the fields required to execute one assistant turn while preserving native history; a deterministic four-turn state machine builds official follow-ups. The existing Python `protocolConformance` analyzer remains the exhaustive official-structure authority and gains raw cross-turn request/response correlation; evidence.v3 and the combined assessment.v7 enforce the new contract without reinterpreting evidence.v1 or evidence.v2.

**Tech Stack:** Rust 1.85, Tokio, reqwest, serde/serde_json, eventsource-stream 0.2.3, Python 3 unittest, JSON Schema, raw TCP test fixtures.

---

## Execution Notes

- Execute from the isolated `codex/issue-3-integrated-tool-loop` worktree. Its base is the completed Issue #2 branch `codex/official-protocol-conformance` at `4f24212`, merged with the approved Issue #3 design/plan at `ca37159`. Preserve the untracked reports in the original `main` worktree and do not copy, edit, or delete the ignored legacy root `tests/` directory.
- Before Task 1, require the merged baseline to pass `cargo build --locked`, `cargo test --locked`, and `python3 -m unittest discover -s skills/creating-model-doctor-reports/tests -p 'test_*.py'`. The verified baseline on 2026-08-18 is Rust build/test exit 0 and Python 153/153.
- Preserve the untracked `deepseek-v4-pro-1m-*` reports in the original worktree.
- Read `docs/superpowers/specs/2026-08-18-issue-3-official-streaming-tool-loop-design.md` before Task 1. It is the normative design for classifications, provider contracts, and check semantics.
- Preserve Issue #2's `model_doctor_protocol_conformance.py`, three streaming analyzer modules, `protocolConformance` assessment/schema/HTML/CSS output, and their tests. Rust validates only the official subset required to run a safe live loop; Python owns exhaustive response validation and cross-turn raw request/response validation. Do not invoke Python from Rust.
- Use `superpowers:test-driven-development` for every implementation task. Use `superpowers:writing-skills` for Task 16 and `superpowers:verification-before-completion` for Task 17.
- During implementation, update the checkboxes in this file after each bounded step. Append `git add -f docs/superpowers/plans/2026-08-18-official-streaming-tool-loop.md` to every task's listed staging command so the ignored plan update is included in that task's code commit.

## Task 1: Establish the 0.11.0 Catalog Contract

**Files:**

- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `src/catalog.rs`
- Modify: `src/checks/mod.rs`
- Modify: `src/checks/tools.rs`
- Modify: `src/protocol/tools.rs`
- Modify: `src/cli.rs`

- [x] **Step 1: Add failing catalog and routing tests.**

Add `catalog::tests::catalog_has_47_unique_checks_and_restores_046` in `src/catalog.rs`:

```rust
#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn catalog_has_47_unique_checks_and_restores_046() {
        assert_eq!(CATALOG.len(), 47);
        assert_eq!(
            CATALOG.iter().find(|test| test.id == "046").map(|test| test.name),
            Some("官方工具协议结构合规")
        );
        assert_eq!(
            CATALOG.iter().map(|test| test.id).collect::<HashSet<_>>().len(),
            47
        );
    }
}
```

Add `checks::tools::tests::check_046_is_a_streaming_tool_loop_seed` in `src/checks/tools.rs`. It must assert request ID `test-046`, `stream == true`, prompt marker `MODEL_DOCTOR_CASE_046`, and a tool declaration for `get_weather`.

- [x] **Step 2: Run the focused tests and confirm the expected failures.**

```bash
cargo test --locked --lib catalog::tests::catalog_has_47_unique_checks_and_restores_046 -- --exact
cargo test --locked --lib checks::tools::tests::check_046_is_a_streaming_tool_loop_seed -- --exact
```

Expected: the first test fails at `46 != 47`; the second fails because check 046 is unsupported.

- [x] **Step 3: Restore check 046 and bump the collector contract.**

Make these exact changes:

```text
Cargo package version: 0.11.0
Catalog length: 47
046 category: 工具调用
046 name: 官方工具协议结构合规
046 prompt: MODEL_DOCTOR_CASE_046. Call get_weather for Beijing. After its result, reply only MODEL_DOCTOR_CASE_046_OK.
047 prompt: MODEL_DOCTOR_CASE_047. First call get_weather for Beijing. After its result, call get_time for UTC. After that result, reply only MODEL_DOCTOR_CASE_047_OK.
048 prompt: MODEL_DOCTOR_CASE_048. Call get_weather for Beijing. After the result, reply MODEL_DOCTOR_CASE_048_OK followed by the exact tool result.
049 prompt: MODEL_DOCTOR_CASE_049. Call get_weather for Beijing. If it returns a timeout error, retry get_weather exactly once. After a successful retry, reply only MODEL_DOCTOR_CASE_049_OK.
046 initial request: stream=true
047, 048, 049 initial requests: stream=true
040-045 and 050 behavior: unchanged in this task
CLI summary: Model Capability Doctor 0.11.0 - Run all 47 checks
CLI list help: 47-item catalog
```

Route `046` through `checks::tools::plan`. Update `Cargo.lock` with `cargo check`, then use `--locked` again.

- [x] **Step 4: Run the focused and catalog tests.**

```bash
cargo test --locked --lib catalog::tests -- --nocapture
cargo test --locked --lib checks::tools::tests -- --nocapture
```

Expected: all catalog/routing tests pass and checks 046 through 049 are streaming seeds.

- [x] **Step 5: Commit the catalog baseline.**

```bash
git add Cargo.toml Cargo.lock src/catalog.rs src/checks/mod.rs src/checks/tools.rs src/protocol/tools.rs src/cli.rs
git diff --cached --check
git commit -m "feat: restore official tool protocol check"
```

## Task 2: Add Stable Evidence Types and evidence.v3 Serialization

**Files:**

- Create: `src/evidence.rs`
- Modify: `src/lib.rs`
- Modify: `src/audit.rs`
- Modify: `src/http.rs`

- [x] **Step 1: Add failing wire-name and audit serialization tests.**

Create these tests:

```text
evidence::tests::evidence_values_render_exact_wire_names
audit::tests::run_header_declares_011_and_evidence_v3
audit::tests::request_block_emits_v3_metadata_before_encoded_sections
audit::tests::tool_contract_errors_are_redacted_json_strings
```

The request-block test must assert the exact field order before
`----- CURL COMMAND BEGIN -----`:

```text
transport_outcome
stream_termination
stream_end_signal
model_stop_reason
stream_event_count
tool_contract_status
tool_contract_errors_json
tool_loop_turn
tool_loop_outcome
```

- [x] **Step 2: Run the audit tests and confirm missing symbols/fields.**

```bash
cargo test --locked --lib evidence::tests -- --nocapture
cargo test --locked --lib audit::tests -- --nocapture
```

Expected: compilation fails because `evidence` and the nine request fields do not exist.

- [x] **Step 3: Implement the stable evidence model.**

Define these public types in `src/evidence.rs`:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransportOutcome {
    CompletedEof,
    Timeout,
    UpstreamDisconnect,
    ClientCancelled,
    TransportError,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StreamTermination {
    NotApplicable,
    Completed,
    MissingTerminalEvent,
    Timeout,
    UpstreamDisconnect,
    ClientCancelled,
    TransportError,
    HttpError,
    MalformedStream,
    ProtocolError,
    ModelIncomplete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StreamEndSignal {
    None,
    OpenAiDone,
    OpenAiResponseCompleted,
    AnthropicMessageStop,
    GeminiFinishReason(String),
    OllamaDone,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolContractStatus {
    NotApplicable,
    Conformant,
    NonConformant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolLoopOutcome {
    NotApplicable,
    Continued,
    Completed,
    InvalidTurn,
    TransportFailure,
    MaxTurnsExceeded,
}
```

Implement `Display` with the exact design wire values. `GeminiFinishReason("STOP")` renders `finishReason:STOP`; the other end signals render `none`, `[DONE]`, `response.completed`, `message_stop`, and `done:true`.

Add the nine approved fields to `RequestEvidence`. Render `None` stop reason as `none`, `tool_contract_errors` through `serde_json::to_string`, and redact each error before serialization. Use `env!("CARGO_PKG_VERSION")` for `script_version` and the constant `llm-capability-doctor.evidence.v3` for `log_schema`.

Update both `RequestEvidence` constructors in `src/http.rs` so this task remains
compilable: the success path uses `CompletedEof` plus `NotApplicable`; the
legacy failure path uses `TransportError` for both transport and termination;
all end-signal, contract, loop, count, and error-list fields use their approved
zero/not-applicable values. Task 3 replaces the legacy failure default with
the precise timeout/disconnect/cancellation classification.

- [x] **Step 4: Run audit/evidence tests.**

```bash
cargo test --locked --lib evidence::tests -- --nocapture
cargo test --locked --lib audit::tests -- --nocapture
```

Expected: exact wire-name, field-order, version, and redaction assertions pass.

- [x] **Step 5: Commit the evidence contract.**

```bash
git add src/evidence.rs src/lib.rs src/audit.rs src/http.rs
git diff --cached --check
git commit -m "feat: serialize evidence v3 stream outcomes"
```

## Task 3: Classify HTTP Completion, Timeout, Disconnect, and Cancellation

**Files:**

- Create: `src/test_support.rs`
- Modify: `src/lib.rs`
- Modify: `src/http.rs`
- Modify: `src/audit.rs`

- [ ] **Step 1: Add a raw TCP response fixture.**

Create a test-only helper that binds `127.0.0.1:0`, accepts one or more requests, records request head/body, and executes a script made from these actions:

```rust
pub enum ServerAction {
    Write(Vec<u8>),
    Delay(Duration),
    Checkpoint(Arc<tokio::sync::Notify>),
    Close,
}

pub struct RecordedRequest {
    pub head: String,
    pub body: Vec<u8>,
}
```

The helper must support a declared `Content-Length` larger than emitted bytes, delayed headers, delayed body chunks, and a clean complete response. A checkpoint notifies the test after all preceding actions have finished, so cancellation tests wait for a deterministic request/header/partial-body boundary instead of sleeping.

- [ ] **Step 2: Add failing transport tests.**

Add exactly these tests under `http::tests`:

```text
completed_eof_preserves_body_without_claiming_protocol_completion
timeout_before_headers_is_classified
timeout_during_body_preserves_partial_bytes
disconnect_during_body_preserves_partial_bytes
cancellation_before_headers_wins
cancellation_during_body_preserves_partial_bytes
non_success_status_is_http_error
connect_failure_is_transport_error
cancellation_beats_a_simultaneously_ready_timeout
pre_cancelled_token_does_not_send_request
body_timeout_beats_an_observed_http_500
http_500_beats_a_later_body_disconnect
```

Also add `audit::tests::cancelled_request_flushes_without_run_summary` to prove an appended cancellation block is durable before `finish` is called.

- [ ] **Step 3: Run the focused tests and observe the classification failures.**

```bash
cargo test --locked --lib http::tests -- --nocapture
cargo test --locked --lib audit::tests::cancelled_request_flushes_without_run_summary -- --exact
```

Expected: tests fail because transport errors all share one legacy path and partial timeout/cancellation evidence is not classified.

- [ ] **Step 4: Implement classification with cancellation priority.**

Keep `HttpExecutor::execute` public signature unchanged. Use `biased;` in both cancellation selects. Classify the two fields independently:

```text
transport_outcome precedence:
client cancellation -> timeout -> pre-response transport error
-> body read disconnect -> completed_eof

stream_termination precedence:
client cancellation -> timeout -> pre-response transport error
-> non-2xx http_error -> body read upstream_disconnect
-> temporary not_applicable for a clean 2xx response
```

Always retain accumulated body bytes after headers, including timeout,
disconnect, and non-2xx combinations.

Extract the precedence calculation into a pure private classifier that accepts
observed boolean/status facts. Test `cancelled=true` and `timed_out=true`
directly instead of trying to make two async futures become ready in the same
millisecond. Separately pass an already-cancelled token to `execute` and assert
the raw server accepts no connection; this proves the biased send branch.

For every clean 2xx response, initialize `stream_termination` as `not_applicable`; HTTP does not decide whether protocol terminal evidence exists. Before audit append, Task 12 must replace this temporary value for every stream request with the protocol parser's `completed`, `missing_terminal_event`, `malformed_stream`, `protocol_error`, or `model_incomplete` result. Format the entire reqwest error chain for both send and body errors.

When failures combine, retain transport and protocol precedence separately:

```text
cancellation plus ready timeout  -> transport client_cancelled, termination client_cancelled
HTTP 500 plus body timeout       -> transport timeout, termination timeout
HTTP 500 plus body disconnect    -> transport upstream_disconnect, termination http_error
```

- [ ] **Step 5: Verify and commit transport evidence.**

```bash
cargo test --locked --lib http::tests -- --nocapture
cargo test --locked --lib audit::tests -- --nocapture
git add src/test_support.rs src/lib.rs src/http.rs src/audit.rs
git diff --cached --check
git commit -m "feat: classify stream transport termination"
```

Expected: all transport and combined-precedence cases retain the correct bytes and exact classification.

## Task 4: Decode SSE and NDJSON with Structured Framing

**Files:**

- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `src/protocol/stream/mod.rs`
- Create: `src/protocol/stream/framing.rs`
- Modify: `src/protocol/mod.rs`

- [ ] **Step 1: Add failing framing tests.**

Add these tests under `protocol::stream::framing::tests`:

```text
sse_accepts_lf_and_crlf_boundaries
sse_joins_multiline_data_and_preserves_event_name
sse_ignores_comments_and_decodes_extension_events
sse_is_invariant_across_every_byte_split
sse_reports_trailing_incomplete_frame
ndjson_is_invariant_across_every_byte_split
ndjson_accepts_a_complete_final_object_without_newline
ndjson_rejects_a_partial_final_object
```

`sse_is_invariant_across_every_byte_split` must feed the same fixture at every split offset and compare decoded events with a one-chunk parse. The general fixture naturally contains official optional field-value spacing; do not create a separate spacing test, fixture, or diagnostic.

- [ ] **Step 2: Run the framing module and confirm it is absent.**

```bash
cargo test --locked --lib protocol::stream::framing::tests -- --nocapture
```

Expected: compilation fails because the stream module does not exist.

- [ ] **Step 3: Add the parser dependency and framing API.**

Add `eventsource-stream = "0.2.3"`. Expose this internal API:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SseEvent {
    pub event: String,
    pub data: String,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct DecodedSse {
    pub events: Vec<SseEvent>,
    pub trailing_incomplete_frame: bool,
}

pub(crate) async fn decode_sse_chunks(
    chunks: Vec<Vec<u8>>,
) -> Result<DecodedSse, FramingError>;

pub(crate) fn decode_ndjson_chunks(
    chunks: Vec<Vec<u8>>,
) -> Result<Vec<serde_json::Value>, FramingError>;
```

Use `eventsource_stream::Eventsource` to parse SSE fields and dispatch. The wrapper detects a non-empty undispatched trailing frame so a half packet cannot silently become a missing-terminal case. For NDJSON, concatenate only for line framing, parse each complete line as JSON, and parse a non-newline final tail only when it is a complete JSON object.

- [ ] **Step 4: Run framing tests and the Rust library suite.**

```bash
cargo test --locked --lib protocol::stream::framing::tests -- --nocapture
cargo test --locked --lib
```

Expected: all byte-split variants produce identical events/objects and partial final frames fail.

- [ ] **Step 5: Commit framing.**

```bash
git add Cargo.toml Cargo.lock src/protocol/mod.rs src/protocol/stream/mod.rs src/protocol/stream/framing.rs
git diff --cached --check
git commit -m "feat: add structured stream framing"
```

## Task 5: Normalize Assistant Turns and Parse OpenAI Chat Streams

**Files:**

- Modify: `src/protocol/stream/mod.rs`
- Create: `src/protocol/stream/openai_chat.rs`
- Create: `src/protocol/fixtures/openai_chat_tool.sse`
- Create: `src/protocol/fixtures/openai_chat_final.sse`
- Create: `src/protocol/fixtures/openai_chat_missing_terminal.sse`
- Create: `src/protocol/fixtures/openai_chat_truncated.sse`
- Create: `src/protocol/fixtures/openai_chat_error.sse`
- Create: `src/protocol/fixtures/openai_chat_invalid_arguments.sse`

- [ ] **Step 1: Add the normalized result types and failing Chat tests.**

Use these shared types:

```rust
#[derive(Debug, PartialEq)]
pub struct StreamParseResult {
    pub assistant_turn: Option<AssistantTurn>,
    pub stream_termination: StreamTermination,
    pub stream_end_signal: StreamEndSignal,
    pub model_stop_reason: Option<String>,
    pub event_count: usize,
    pub contract_errors: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssistantTurn {
    pub history: ProtocolHistory,
    pub tool_calls: Vec<ToolCall>,
    pub final_text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub index: usize,
    pub correlation: ToolCorrelation,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolCorrelation {
    Required(String),
    Optional(Option<String>),
    None,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProtocolHistory {
    OpenAiChat(serde_json::Value),
    OpenAiResponses { response_id: String },
    Anthropic(Vec<serde_json::Value>),
    Gemini(serde_json::Value),
    Ollama(serde_json::Value),
}
```

Task 5 exposes only
`openai_chat::parse(body: &[u8]) -> impl Future<Output = StreamParseResult>`;
implement it as `pub(crate) async fn parse`. Tests call that parser
directly. Do not add the central five-protocol dispatcher until Task 9, after
all parser modules exist.

Test argument fragments, assistant history, `finish_reason`, `[DONE]`, final text,
missing `[DONE]`, malformed event JSON, API error objects, malformed accumulated
arguments, and mismatched tool-call indexes/IDs needed by the runtime loop. Do
not duplicate Issue #2's exhaustive response-envelope mutation matrix in Rust.

- [ ] **Step 2: Run Chat parser tests and confirm failure.**

```bash
cargo test --locked --lib protocol::stream::openai_chat::tests -- --nocapture
```

Expected: parser module and normalized types are missing.

- [ ] **Step 3: Implement Chat delta accumulation.**

Require each chunk envelope to contain a stable string `id`, `object == "chat.completion.chunk"`, integer `created`, string `model`, and a `choices` array whose selected choice has integer `index` and object `delta`. Require stable envelope identity across chunks and reject conflicting duplicate choice/tool indexes. Merge `choices[0].delta.tool_calls` by `index`. Carry forward `id`, require `type == "function"`, carry `function.name`, append every string `function.arguments` fragment, then decode one JSON object. Preserve an official assistant message with `role: "assistant"` and complete `tool_calls`.

Classification must be exact:

```text
tool turn: finish_reason=tool_calls plus [DONE] -> completed
final turn: finish_reason=stop plus [DONE]      -> completed
clean EOF without [DONE]                        -> missing_terminal_event
bad SSE/event JSON                              -> malformed_stream
top-level error object                          -> protocol_error
terminal with absent/non-normal finish reason   -> model_incomplete
valid terminal plus malformed tool arguments    -> completed + contract error
```

Use stable contract errors formatted as `chat.<code>:/json/pointer`; do not include model-supplied values.

- [ ] **Step 4: Verify parser fixtures.**

```bash
cargo test --locked --lib protocol::stream::openai_chat::tests -- --nocapture
```

Expected: the tool fixture yields `get_weather`, `{"city":"Beijing"}`, required call ID, `tool_calls`, and `[DONE]`; every failure fixture has the designed classification.

- [ ] **Step 5: Commit Chat parsing.**

```bash
git add src/protocol/stream/mod.rs src/protocol/stream/openai_chat.rs src/protocol/fixtures/openai_chat_*.sse
git diff --cached --check
git commit -m "feat: parse official OpenAI chat tool streams"
```

## Task 6: Parse OpenAI Responses Streams

**Files:**

- Create: `src/protocol/stream/openai_responses.rs`
- Create: `src/protocol/fixtures/openai_responses_tool.sse`
- Create: `src/protocol/fixtures/openai_responses_final.sse`
- Create: `src/protocol/fixtures/openai_responses_missing_terminal.sse`
- Create: `src/protocol/fixtures/openai_responses_truncated.sse`
- Create: `src/protocol/fixtures/openai_responses_error.sse`
- Create: `src/protocol/fixtures/openai_responses_incomplete.sse`
- Create: `src/protocol/fixtures/openai_responses_bad_correlation.sse`
- Modify: `src/protocol/stream/mod.rs`

- [ ] **Step 1: Add failing Responses event-sequence tests.**

Cover `response.created`, `response.output_item.added`,
`response.function_call_arguments.delta`,
`response.function_call_arguments.done`, `response.output_item.done`, and
`response.completed`. Assert the event/data types and IDs needed to reconstruct
the call are correlated, and retain the response ID for
`previous_response_id`. Limit Rust negatives to missing/conflicting runtime
identity, call ID, output index, argument fragments, and terminal fields; the
Python analyzer retains exhaustive envelope coverage.

- [ ] **Step 2: Run the parser tests.**

```bash
cargo test --locked --lib protocol::stream::openai_responses::tests -- --nocapture
```

Expected: module missing.

- [ ] **Step 3: Implement the Responses state machine.**

Require completed function-call items to contain `call_id`, `name`, and JSON object arguments. The `.done` arguments must agree with the assembled deltas. Accumulate final output text from official text events. Preserve `response.id` in `ProtocolHistory::OpenAiResponses`.

Classify `response.completed` only when the embedded response status is `completed`; classify `response.incomplete` as `model_incomplete`; classify `response.failed` and `error` events as `protocol_error`; classify clean EOF without one of those terminal events as `missing_terminal_event`. A terminal stream with correlation/order errors stays wire-completed but is contract non-conformant.

- [ ] **Step 4: Verify all Responses fixtures.**

```bash
cargo test --locked --lib protocol::stream::openai_responses::tests -- --nocapture
```

Expected: normal tool/final streams complete, incomplete/error streams remain distinct, and bad correlation produces a stable contract error.

- [ ] **Step 5: Commit Responses parsing.**

```bash
git add src/protocol/stream/mod.rs src/protocol/stream/openai_responses.rs src/protocol/fixtures/openai_responses_*.sse
git diff --cached --check
git commit -m "feat: parse official OpenAI responses tool streams"
```

## Task 7: Parse Anthropic Messages Streams

**Files:**

- Create: `src/protocol/stream/anthropic.rs`
- Create: `src/protocol/fixtures/anthropic_tool.sse`
- Create: `src/protocol/fixtures/anthropic_final.sse`
- Create: `src/protocol/fixtures/anthropic_missing_terminal.sse`
- Create: `src/protocol/fixtures/anthropic_truncated.sse`
- Create: `src/protocol/fixtures/anthropic_error.sse`
- Create: `src/protocol/fixtures/anthropic_invalid_order.sse`
- Modify: `src/protocol/stream/mod.rs`

- [ ] **Step 1: Add failing Anthropic lifecycle tests.**

Assert `message_start`, content-block start/delta/stop ordering, `message_delta`,
and `message_stop`. Require the event/data types, content indexes, tool-use ID,
name, input fragments, stop reason, and native assistant blocks needed by the
loop. Cover a final text block, ping/extension events, an official error event,
missing terminal, invalid tool-block order, and duplicate runtime indexes. The
Python analyzer retains exhaustive event-envelope mutations.

- [ ] **Step 2: Run the Anthropic tests.**

```bash
cargo test --locked --lib protocol::stream::anthropic::tests -- --nocapture
```

Expected: module missing.

- [ ] **Step 3: Implement Anthropic block reconstruction.**

Require each `tool_use` block to have `id`, `name`, object input, and matching block indices. Preserve the complete ordered assistant content array in `ProtocolHistory::Anthropic`. Accept ping and unknown documented extension event names without changing the lifecycle state.

Require `stop_reason: tool_use` for a tool turn and `stop_reason: end_turn` for a final turn, followed by `message_stop`. An `event: error` frame is `protocol_error`; missing `message_stop` is `missing_terminal_event`; lifecycle/input errors are stable `anthropic.<code>:/path` contract errors.

- [ ] **Step 4: Verify Anthropic fixtures.**

```bash
cargo test --locked --lib protocol::stream::anthropic::tests -- --nocapture
```

Expected: fragmented input becomes `{"city":"Beijing"}`, the assistant blocks are preserved, and all terminal/error cases are distinct.

- [ ] **Step 5: Commit Anthropic parsing.**

```bash
git add src/protocol/stream/mod.rs src/protocol/stream/anthropic.rs src/protocol/fixtures/anthropic_*.sse
git diff --cached --check
git commit -m "feat: parse official Anthropic tool streams"
```

## Task 8: Parse Gemini GenerateContent Streams and Normalize Stream URLs

**Files:**

- Create: `src/protocol/stream/gemini.rs`
- Create: `src/protocol/fixtures/gemini_tool.sse`
- Create: `src/protocol/fixtures/gemini_tool_without_id.sse`
- Create: `src/protocol/fixtures/gemini_final.sse`
- Create: `src/protocol/fixtures/gemini_missing_terminal.sse`
- Create: `src/protocol/fixtures/gemini_truncated.sse`
- Create: `src/protocol/fixtures/gemini_error.sse`
- Create: `src/protocol/fixtures/gemini_incomplete.sse`
- Modify: `src/protocol/stream/mod.rs`
- Modify: `src/protocol/mod.rs`

- [ ] **Step 1: Add failing Gemini parser and URL tests.**

Test both an optional `functionCall.id` and no ID, structured object
`functionCall.args`, complete model content with thought signatures,
`finishReason: STOP`, a non-normal finish reason, missing finish reason, API
error, truncated SSE, malformed tool parts, and ambiguous runtime candidate
selection. The Python analyzer retains exhaustive candidate/part mutations.

Add URL tests for all of these exact transforms:

```text
:generateContent?key=x&trace=1 -> :streamGenerateContent?key=x&trace=1&alt=sse
:streamGenerateContent?alt=json&trace=1 -> :streamGenerateContent?trace=1&alt=sse
non-stream request -> byte-for-byte equivalent URL
```

- [ ] **Step 2: Run Gemini tests and confirm failure.**

```bash
cargo test --locked --lib protocol::stream::gemini::tests -- --nocapture
cargo test --locked --lib protocol::tests::gemini_stream_url_is_normalized -- --exact
```

Expected: parser and URL normalizer are absent.

- [ ] **Step 3: Implement official GenerateContent behavior.**

Require a candidates array and type-correct content/parts. Require model content role `model`; validate candidate index as a unique non-negative integer when present. Accumulate candidate model parts without converting structured `args` into string deltas. Preserve the complete model content and thought signatures in `ProtocolHistory::Gemini`. Map a present ID to `ToolCorrelation::Optional(Some(id))`; map its absence to `ToolCorrelation::Optional(None)` and never invent an ID.

Normal completion requires `finishReason: STOP` plus clean EOF. A different non-empty reason is `model_incomplete` and renders `finishReason:<reason>`; no finish reason is `missing_terminal_event`; an error object is `protocol_error`.

Implement `normalize_request_url(protocol, configured, stream)` for every Gemini stream request, preserving non-`alt` query pairs and setting exactly one `alt=sse` pair.

- [ ] **Step 4: Verify parser and URL behavior.**

```bash
cargo test --locked --lib protocol::stream::gemini::tests -- --nocapture
cargo test --locked --lib protocol::tests -- --nocapture
```

Expected: both ID forms are conformant, URL queries are preserved, and non-STOP endings never report completion.

- [ ] **Step 5: Commit Gemini support.**

```bash
git add src/protocol/mod.rs src/protocol/stream/mod.rs src/protocol/stream/gemini.rs src/protocol/fixtures/gemini_*.sse
git diff --cached --check
git commit -m "feat: parse official Gemini tool streams"
```

## Task 9: Parse Ollama Chat Streams

**Files:**

- Create: `src/protocol/stream/ollama.rs`
- Create: `src/protocol/fixtures/ollama_tool.ndjson`
- Create: `src/protocol/fixtures/ollama_final.ndjson`
- Create: `src/protocol/fixtures/ollama_missing_terminal.ndjson`
- Create: `src/protocol/fixtures/ollama_truncated.ndjson`
- Create: `src/protocol/fixtures/ollama_error.ndjson`
- Create: `src/protocol/fixtures/ollama_incomplete.ndjson`
- Modify: `src/protocol/stream/mod.rs`

- [ ] **Step 1: Add failing Ollama tests.**

Cover accumulation of `message.thinking`, `message.content`, and
`message.tool_calls`; assistant history needed for the follow-up;
object-valued `function.arguments`; optional unique
`message.tool_calls[].function.index`; no required call ID; `done:true`;
optional `done_reason`; `done_reason:length`; error objects; clean EOF without
`done:true`; and a partial final JSON object. The Python analyzer retains
exhaustive object-envelope mutations.

- [ ] **Step 2: Run Ollama tests.**

```bash
cargo test --locked --lib protocol::stream::ollama::tests -- --nocapture
```

Expected: module missing.

- [ ] **Step 3: Implement NDJSON turn accumulation.**

Require every streamed object to have type-correct `model`, `created_at`, `message`, and `done` fields when those fields are part of that official event shape, and require `message.role == "assistant"`. Use `message.tool_calls[].function.index` when present, reject duplicate indexes, and use array order otherwise. Require a function name and object arguments. Store `ToolCorrelation::None`. Preserve the accumulated assistant message in `ProtocolHistory::Ollama` without inserting an OpenAI call ID.

Require `done:true`. Treat absent `done_reason` and `done_reason:stop` as normal; treat `length` as `model_incomplete`; treat an error object as `protocol_error`; treat EOF without `done:true` as `missing_terminal_event`.

Now that all five modules exist, add the public crate dispatcher:

```rust
pub async fn parse_stream(protocol: Protocol, body: &[u8]) -> StreamParseResult;
```

Route each known protocol to its matching parser. `Protocol::Unknown` returns
`protocol_error`, end signal `none`, zero events, no assistant turn, and one
stable `protocol.unknown:/` contract error. Add
`stream::tests::dispatcher_routes_all_protocols_and_rejects_unknown`.

- [ ] **Step 4: Verify Ollama fixtures.**

```bash
cargo test --locked --lib protocol::stream::ollama::tests -- --nocapture
```

Expected: structured arguments remain objects, the final signal is `done:true`, and every abnormal ending is classified.

- [ ] **Step 5: Commit Ollama parsing.**

```bash
git add src/protocol/stream/mod.rs src/protocol/stream/ollama.rs src/protocol/fixtures/ollama_*.ndjson
git diff --cached --check
git commit -m "feat: parse official Ollama tool streams"
```

## Task 10: Build and Validate Official Tool Requests

**Files:**

- Modify: `src/protocol/tools.rs`
- Modify: `src/protocol/mod.rs`
- Modify: `src/checks/tools.rs`

- [ ] **Step 1: Replace legacy follow-up tests with a five-protocol request matrix.**

Add tests that assert exact initial and follow-up shapes for all protocols:

```text
OpenAI Chat: assistant tool_calls then role=tool with matching tool_call_id
OpenAI Responses: previous_response_id plus function_call_output with matching call_id
Anthropic: assistant content then one user content array containing all tool_result blocks first
Gemini: model content then functionResponse with matching name and optional matching ID
Ollama: assistant message then role=tool with tool_name and no OpenAI-only fields
```

Also assert that only Anthropic uses `is_error:true`, Gemini uses `response:{"error":"timeout"}`, and the other three carry `ERROR: timeout` in their documented text/output field.

- [ ] **Step 2: Run the tool protocol tests and confirm legacy-shape failures.**

```bash
cargo test --locked --lib protocol::tools::tests -- --nocapture
```

Expected: Gemini wrongly requires an ID, Ollama contains OpenAI-only fields, and the legacy builder cannot append more than one follow-up.

- [ ] **Step 3: Implement conversation and validation APIs.**

Add these APIs as the replacement path for `build_follow_up` and response
re-parsing:

```rust
pub struct ToolConversation {
    protocol: Protocol,
    current_body: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExecutedToolResult {
    pub call: ToolCall,
    pub output: String,
    pub is_error: bool,
}

pub enum ToolRequestPhase<'a> {
    Initial,
    FollowUp {
        previous: &'a AssistantTurn,
        results: &'a [ExecutedToolResult],
    },
}

impl ToolConversation {
    pub fn from_initial(
        protocol: Protocol,
        body: serde_json::Value,
    ) -> Result<Self, ToolProtocolError>;

    pub fn current_request(&self) -> RequestSpec;

    pub fn append_follow_up(
        &mut self,
        turn: &AssistantTurn,
        results: &[ExecutedToolResult],
    ) -> Result<RequestSpec, ToolProtocolError>;
}

pub fn validate_tool_request(
    protocol: Protocol,
    endpoint: &url::Url,
    stream: bool,
    body: &serde_json::Value,
    phase: ToolRequestPhase<'_>,
) -> Vec<String>;
```

Split Ollama and Unknown from the OpenAI Chat request/tool-definition branches. Ollama must not contain `tool_choice`, `parallel_tool_calls`, or function `strict`. `ToolConversation::from_initial` and `append_follow_up` must return `ToolProtocolError::UnsupportedProtocol` for Unknown rather than emitting a Chat-shaped follow-up. Gemini IDs remain optional. Keep native assistant history intact on every follow-up. Validate `stream == true` independently of provider body fields; for Gemini also validate the normalized stream endpoint action and `alt=sse`. Validation errors use concrete stable forms such as `request.ollama.openai_only_field:/tool_choice` and `request.gemini.invalid_stream_action:/url`.

Keep the existing `build_follow_up` symbol as a temporary compatibility adapter
so the pre-Task-12 runner still compiles. Do not use it in new tests. Task 12
removes the adapter and its runner import after the state-machine path is live.

- [ ] **Step 4: Verify the five-protocol request matrix.**

```bash
cargo test --locked --lib protocol::tools::tests -- --nocapture
```

Expected: every generated request validates with zero errors, while one mutation of every required correlation/path produces the expected stable error.

- [ ] **Step 5: Commit official request generation.**

```bash
git add src/protocol/tools.rs src/protocol/mod.rs src/checks/tools.rs
git diff --cached --check
git commit -m "feat: build official tool follow-up requests"
```

## Task 11: Implement the Deterministic Four-Turn Loop

**Files:**

- Create: `src/protocol/tool_loop.rs`
- Modify: `src/protocol/mod.rs`

- [ ] **Step 1: Add failing pure state-machine tests.**

Add exactly these tests:

```text
weather_call_returns_sunny
time_call_returns_fixed_utc_time
check_049_returns_timeout_then_sunny
unknown_tool_is_an_invalid_turn
invalid_arguments_are_an_invalid_turn
final_answer_completes_the_loop
fourth_assistant_tool_turn_exceeds_the_bound_without_execution
```

Assert no filesystem, network, clock, or environment access occurs.

- [ ] **Step 2: Run the loop tests.**

```bash
cargo test --locked --lib protocol::tool_loop::tests -- --nocapture
```

Expected: module missing.

- [ ] **Step 3: Implement the bounded state machine.**

Use this public contract inside the crate:

```rust
pub(crate) const MAX_ASSISTANT_TURNS: usize = 4;

pub(crate) struct ToolLoopState {
    check_id: &'static str,
    assistant_turns: usize,
    weather_attempts: usize,
}

pub(crate) enum LoopDecision {
    Complete,
    Continue(Vec<ExecutedToolResult>),
    Stop(ToolLoopOutcome),
}

impl ToolLoopState {
    pub(crate) fn new(check_id: &'static str) -> Option<Self>;
    pub(crate) fn advance(&mut self, turn: &AssistantTurn) -> LoopDecision;
}
```

Allow only exact object arguments `{"city":"Beijing"}` for `get_weather` and `{"zone":"UTC"}` for `get_time`. Return `WEATHER_SUNNY` and `TIME_UTC_12:00`. For check 049, the first valid weather call returns `ERROR: timeout` with `is_error=true`; later valid weather calls return `WEATHER_SUNNY`. If assistant turn four contains any tool call, return `MaxTurnsExceeded` before dispatching it.

- [ ] **Step 4: Verify deterministic behavior.**

```bash
cargo test --locked --lib protocol::tool_loop::tests -- --nocapture
```

Expected: exact outputs and the four-turn no-execution boundary pass.

- [ ] **Step 5: Commit the state machine.**

```bash
git add src/protocol/mod.rs src/protocol/tool_loop.rs
git diff --cached --check
git commit -m "feat: add bounded deterministic tool loop"
```

## Task 12: Integrate Stream Parsing and Tool Loops into the Runner

**Files:**

- Modify: `src/runner.rs`
- Create: `src/runner/tests.rs`
- Modify: `src/http.rs`
- Modify: `src/test_support.rs`
- Modify: `src/protocol/tools.rs`

- [ ] **Step 1: Add failing runner tests for generic stream enrichment.**

Add `#[cfg(test)] mod tests;` at the bottom of `src/runner.rs` before creating
`src/runner/tests.rs`, so the tracked test module is compiled.

Add a test-only optional turn hook with separate `turn_committed` and `resume`
`Notify` values. The loop fires `turn_committed` after audit flush and waits on
`resume` before its next cancellation check/send. Production runners have no
hook. This makes the between-turn cancellation test deterministic.

Add these tests:

```text
ordinary_stream_request_records_official_terminal
ordinary_stream_request_records_missing_terminal
concurrent_stream_requests_are_parsed_before_ordered_audit_write
cancelled_http_request_is_flushed_without_run_summary
cancel_between_completed_turns_does_not_send_an_extra_turn
```

The first two cover check 006 behavior without entering a tool loop. Run at
least the first test with `--exact` before the module-wide command so a missing
module declaration cannot produce a false green result.

- [ ] **Step 2: Add failing end-to-end tool-loop matrix tests.**

Use the raw scripted server and the tracked protocol fixtures. For every one of the five protocols, run checks 046, 047, 048, and 049 through `Runner::execute_tool_loop`. Assert:

```text
046 request refs: turn-1, turn-2
047 request refs: turn-1, turn-2, turn-3
048 request refs: turn-1, turn-2
049 request refs: turn-1, turn-2, turn-3
every request stream_termination: completed
every request tool_contract_status: conformant
all non-final outcomes: continued
final outcome: completed
049 contains one error result, one retry result, and no second retry
047 contains get_time({"zone":"UTC"}) after the weather result
final response contains the check-specific 046/047/048/049 marker
048 final response preserves WEATHER_SUNNY exactly
```

Extend `src/test_support.rs` with an independent official stream encoder used
only by runner tests:

```rust
pub enum ScriptedAssistantTurn {
    Tool {
        response_id: String,
        call_id: Option<String>,
        name: String,
        arguments: serde_json::Value,
    },
    Final {
        response_id: String,
        text: String,
    },
}

pub fn encode_official_stream(
    protocol: Protocol,
    turn: &ScriptedAssistantTurn,
) -> Vec<u8>;
```

The encoder emits official provider-native frames and unique Responses IDs for
each turn. It must not call the production parsers or request builders. Parser
unit tests continue loading the tracked raw fixtures; the independent encoder
supplies the per-check time turn and final text needed by the 20 loop cases.

Add separate tests for malformed arguments, missing terminal, upstream disconnect, timeout, client cancellation, and four consecutive tool turns. Each must audit the last attempted turn and stop without an extra request. Every case where no valid response envelope can be parsed must also assert `tool_contract_status == non_conformant` and a non-empty stable contract-error array.

- [ ] **Step 3: Run runner tests and confirm the one-follow-up implementation fails.**

```bash
cargo test --locked --lib runner::tests::ordinary_stream_request_records_official_terminal -- --exact
cargo test --locked --lib runner::tests -- --nocapture
```

Expected: request IDs are not turn-based, streams are not parsed, and checks stop after one follow-up.

- [ ] **Step 4: Refactor collection before audit append.**

Replace immediate append inside the HTTP call path with this sequence:

```text
normalize URL
collect RequestEvidence
if 2xx + completed_eof + stream: parse protocol stream
copy parser termination/end-signal/stop-reason/event-count into evidence
for tool checks: merge preflight and response contract errors
set contract status and loop outcome
append and flush request evidence
only then construct/send another turn
```

Apply parsing to sequential and concurrent ordinary stream requests. Concurrent results remain sorted by original request index before audit append.

For checks 046 through 049, never mark a request conformant when parsing was
skipped. A timeout, disconnect, cancellation, transport error, or HTTP error
adds `response.unavailable:/` and sets `tool_contract_status` to
`non_conformant`. A parser-returned abnormal lifecycle adds its stable
terminal/protocol error and is also non-conformant. Only a parsed, normally
terminated response with no outgoing or incoming violations is conformant.

Delete the temporary `build_follow_up` compatibility adapter and the old
runner import/single-follow-up branch in the same step.

- [ ] **Step 5: Replace the 047-049 special case with the reusable loop.**

Add:

```rust
async fn execute_tool_loop(
    &mut self,
    test: &'static TestCase,
    initial: PlannedRequest,
) -> Result<Vec<RequestEvidence>, RunnerError>;
```

Run it for 046 through 049. Generate `test-<id>-turn-<n>` before each request. Send only when the preceding result was `completed` and `conformant`. Map `timeout`, `upstream_disconnect`, `client_cancelled`, `transport_error`, and `http_error` to `transport_failure`. Map `missing_terminal_event`, `malformed_stream`, `protocol_error`, `model_incomplete`, and outgoing/incoming contract failure to `invalid_turn`. Use `max_turns_exceeded` at the bound and `completed` for a valid final assistant turn. The precise reason remains in `stream_termination`. Append every returned request ID to the manifest in execution order.

Call `ensure_not_cancelled()` immediately before every turn send, including
after the test-only between-turn gate is released.

If protocol detection remains `Unknown`, do not call the tool planner and do
not send a Chat-shaped tool seed. Append the 046-049 manifests with the ordered
protocol-probe request references and stop those checks. Add
`unknown_protocol_reuses_probe_evidence_without_sending_tool_seed`; the v3
report guard must reject PASS because there is no official turn sequence.

When cancellation wins, append and flush the partial request, then return `RunnerError::Interrupted` before manifest and run-summary creation. The cancellation integration test must call the real `run()`, wait on the TCP checkpoint after partial body bytes are sent, cancel, await the runner, then read the log and assert the request block and partial body exist with `client_cancelled`, while no TEST manifest or RUN SUMMARY exists. The between-turn test waits for the test-only `turn_committed` notification, cancels the token, releases `resume`, and asserts no extra turn reaches the server.

- [ ] **Step 6: Run runner and Rust library tests.**

```bash
cargo test --locked --lib runner::tests -- --nocapture
cargo test --locked --lib
```

Expected: all 20 protocol/check combinations complete with official request bodies and ordered evidence; every abnormal fixture stops at exactly the recorded request.

- [ ] **Step 7: Commit runner integration.**

```bash
git add src/runner.rs src/runner/tests.rs src/http.rs src/test_support.rs src/protocol/tools.rs
git diff --cached --check
git commit -m "feat: collect complete streaming tool loops"
```

## Task 13: Parse evidence.v3 Without Reinterpreting Historical Logs

**Files:**

- Create: `skills/creating-model-doctor-reports/scripts/model_doctor_contracts.py`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_log.py`
- Create: `skills/creating-model-doctor-reports/tests/test_model_doctor_evidence_v3.py`
- Preserve: `skills/creating-model-doctor-reports/tests/test_model_doctor_v6.py`
- Preserve: `skills/creating-model-doctor-reports/tests/test_model_doctor_assessment_v7.py`

- [ ] **Step 1: Add a failing evidence.v3 contract suite.**

Create `test_model_doctor_evidence_v3.py` with class
`ModelDoctorEvidenceV3Tests`. Reuse public fixture-building helpers where they
already exist, but do not rename or absorb Issue #2's v6 or assessment-v7
suites. Add the tests below.

Add exactly these tests to the renamed v7 suite:

```text
test_parser_accepts_complete_profile_free_v3_with_47_manifests
test_parser_rejects_v3_missing_046
test_parser_rejects_mixed_v3_contract_variants
test_parser_validates_v3_request_metadata
test_parser_accepts_complete_profile_free_v2
test_v2_contract_remains_46_checks_without_046
test_v1_custom_rejects_046
test_v1_full_still_requires_legacy_46
test_contract_key_rejects_non_mapping_run_without_crashing
```

The metadata test must independently mutate each required field, enum, integer, and JSON error-array shape.

- [ ] **Step 2: Run the focused parser tests.**

```bash
python3 skills/creating-model-doctor-reports/tests/test_model_doctor_evidence_v3.py \
  ModelDoctorEvidenceV3Tests.test_parser_accepts_complete_profile_free_v3_with_47_manifests \
  ModelDoctorEvidenceV3Tests.test_parser_rejects_v3_missing_046 \
  ModelDoctorEvidenceV3Tests.test_parser_rejects_mixed_v3_contract_variants \
  ModelDoctorEvidenceV3Tests.test_parser_validates_v3_request_metadata -v
```

Expected: v3 is unsupported.

- [ ] **Step 3: Centralize exact contract definitions.**

Define these constants and helpers in `model_doctor_contracts.py`:

```python
V1_CONTRACT = ("llm-capability-doctor.evidence.v1", "0.9.0")
V2_CONTRACT = ("llm-capability-doctor.evidence.v2", "0.10.0")
V3_CONTRACT = ("llm-capability-doctor.evidence.v3", "0.11.0")
LEGACY_TEST_IDS = frozenset({
    *(f"{value:03d}" for value in range(1, 21)),
    "022", "024", "031",
    *(f"{value:03d}" for value in range(33, 37)),
    "038",
    *(f"{value:03d}" for value in range(40, 46)),
    *(f"{value:03d}" for value in range(47, 51)),
    *(f"{value:03d}" for value in range(52, 58)),
    "059", "060",
})
V3_TEST_IDS = LEGACY_TEST_IDS | frozenset({"046"})
LEGACY_CORE_TEST_IDS = frozenset({
    "001", "002", "003", "004", "005", "006", "007",
    "009", "010", "011", "012", "013", "014", "019",
    "022", "031", "038", "040", "041", "042", "043",
    "044", "047", "048", "049", "050", "052", "053",
    "054", "055", "057",
})
V3_CORE_TEST_IDS = LEGACY_CORE_TEST_IDS | frozenset({"046"})
ENHANCED_TEST_IDS = frozenset({
    "008", "015", "016", "017", "018", "020", "024",
    "033", "034", "035", "036", "045", "056", "059",
    "060",
})
CONTRACT_TEST_IDS = {
    V1_CONTRACT: LEGACY_TEST_IDS,
    V2_CONTRACT: LEGACY_TEST_IDS,
    V3_CONTRACT: V3_TEST_IDS,
}
CONTRACT_VERDICT_PARTITIONS = {
    V1_CONTRACT: (LEGACY_CORE_TEST_IDS, ENHANCED_TEST_IDS),
    V2_CONTRACT: (LEGACY_CORE_TEST_IDS, ENHANCED_TEST_IDS),
    V3_CONTRACT: (V3_CORE_TEST_IDS, ENHANCED_TEST_IDS),
}
```

`contract_key(run)` first requires a mapping, then returns and validates the
exact `(log_schema, script_version)` pair. Every malformed value raises
`ValueError`, never `AttributeError` or `TypeError`.

- [ ] **Step 4: Validate all v3 request metadata.**

In `_validate_v3_request_metadata`, require the nine evidence fields on every request. Keep all flat metadata as strings in parsed evidence. Validate integer fields with the canonical non-negative decimal pattern `^(0|[1-9][0-9]*)$`, approved enums, `stream_end_signal` exact fixed values or non-empty `finishReason:<value>`, and `tool_contract_errors_json` as a JSON array whose elements are non-empty strings. Rust producer tests own the finite stable error-code forms; the Python parser must not attempt to infer stability from arbitrary text. Enforce `conformant` implies an empty error array and `non_conformant` implies at least one error string.

Before accepting each TEST block, restrict its ID to `CONTRACT_TEST_IDS[contract]`. Keep v1 profile rules unchanged, including the exact legacy full set, and reject 046 even for a v1 custom profile. Require v2 to contain exactly the original 46 manifests and reject 046. Require v3 to contain exactly 47 manifests including 046. Reject `collection_profile` for v2 and v3.

- [ ] **Step 5: Run parser compatibility tests and commit.**

```bash
python3 -m unittest skills/creating-model-doctor-reports/tests/test_model_doctor_evidence_v3.py -v
python3 -m unittest \
  skills/creating-model-doctor-reports/tests/test_model_doctor_assessment_v7.py \
  skills/creating-model-doctor-reports/tests/test_model_doctor_protocol_conformance.py -v
git add skills/creating-model-doctor-reports/scripts/model_doctor_contracts.py \
  skills/creating-model-doctor-reports/scripts/model_doctor_log.py \
  skills/creating-model-doctor-reports/tests/test_model_doctor_evidence_v3.py
git diff --cached --check
git commit -m "feat: parse evidence v3 contract"
```

Expected: v1/v2 compatibility tests and new v3 tests all pass.

## Task 14: Extend the Combined assessment.v7 with Contract-Specific Totals

**Files:**

- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_general_verdict.py`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_html.py`
- Modify: `skills/creating-model-doctor-reports/references/assessment-schema.json`
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_general_verdict.py`
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_evidence_v3.py`
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_assessment_v7.py`
- Test: `skills/creating-model-doctor-reports/tests/test_model_doctor_protocol_conformance.py`

- [ ] **Step 1: Add failing verdict and schema tests.**

Add these tests:

```text
test_v3_partition_has_32_core_and_15_enhanced
test_v3_all_checks_pass_uses_47_totals
test_v3_046_failure_is_core_failure
test_v2_all_checks_pass_keeps_46_totals
test_v1_partial_uses_legacy_46_denominator
test_assessment_v7_schema_allows_only_contract_total_combinations
test_assessment_validator_rejects_contract_total_mismatch
test_html_renders_v3_stream_and_tool_metadata
test_combined_v7_v3_has_protocol_conformance_and_47_32_15_totals
test_combined_v7_v2_preserves_protocol_conformance_and_46_31_15_totals
test_v3_tool_turns_appear_once_in_protocol_conformance_with_check_ids
test_combined_v7_schema_requires_conformance_and_contract_totals
test_combined_v7_validator_rechecks_both_invariant_families
```

- [ ] **Step 2: Run verdict tests and confirm fixed-46 failures.**

```bash
python3 -m unittest skills/creating-model-doctor-reports/tests/test_model_doctor_general_verdict.py -v
python3 -m unittest \
  skills/creating-model-doctor-reports/tests/test_model_doctor_assessment_v7.py \
  skills/creating-model-doctor-reports/tests/test_model_doctor_protocol_conformance.py -v
```

Expected: the existing assessment.v7 and protocol-conformance tests remain
green, while the new evidence.v3 totals/combined-invariant tests fail because
47/32/15 is not yet supported.

- [ ] **Step 3: Make verdict derivation contract-aware.**

Change the API to:

```python
def derive_general_verdict(
    statuses: Mapping[str, str],
    contract: tuple[str, str],
) -> dict:
```

Resolve IDs and partitions only through `CONTRACT_TEST_IDS` and `CONTRACT_VERDICT_PARTITIONS`. Generate statements from the selected totals so v1/v2 remain `46/31/15` and v3 is `47/32/15`. Pass the exact run contract from both `assemble_assessment` and independent assessment validation.

Update every direct `derive_general_verdict` call. Give partial/custom fixtures the
V1 pair, complete 46-check fixtures the V2 pair, and complete 47-check fixtures
the V3 pair; no test fixture may rely on a missing schema/version default.

Keep `protocolConformance = analyze_protocol_conformance(parsed)` in assembly,
keep `validate_protocol_conformance(...)` in independent validation, and keep
`protocolConformance` in `ASSESSMENT_FIELDS`. Change
`_validate_assessment_summary` to receive the exact contract explicitly. An
invalid assessment-owned run contract must produce a validation error instead
of crashing or silently choosing a denominator.

- [ ] **Step 4: Extend the existing v7 schema and HTML fallback.**

Keep `ASSESSMENT_SCHEMA_VERSION`, schema `$id`, and schema `const` at
`llm-capability-doctor.assessment.v7`. Preserve the required top-level
`protocolConformance` field and all of its `$defs`. Extend `generalVerdict`
with `oneOf` branches that allow only these complete combinations:

```text
totalTests=46, totalCoreTests=31, totalEnhancedTests=15
totalTests=47, totalCoreTests=32, totalEnhancedTests=15
```

Each branch sets the corresponding maxima for collected/passed fields. Change HTML's missing `totalTests` fallback from `46` to `0`; valid assessment data always supplies the generated value.

For v3 request rows, render `transport_outcome`, `stream_termination`,
`stream_end_signal`, `tool_contract_status`, `tool_loop_turn`, and
`tool_loop_outcome` in the turn metadata. Render non-empty
`tool_contract_errors_json` beside the request output. Leave historical rows
unchanged when those fields are absent.

- [ ] **Step 5: Verify and commit assessment.v7.**

```bash
python3 -m unittest skills/creating-model-doctor-reports/tests/test_model_doctor_general_verdict.py -v
python3 -m unittest skills/creating-model-doctor-reports/tests/test_model_doctor_evidence_v3.py -v
python3 -m unittest \
  skills/creating-model-doctor-reports/tests/test_model_doctor_assessment_v7.py \
  skills/creating-model-doctor-reports/tests/test_model_doctor_protocol_conformance.py -v
git add skills/creating-model-doctor-reports/scripts/model_doctor_general_verdict.py \
  skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py \
  skills/creating-model-doctor-reports/scripts/model_doctor_html.py \
  skills/creating-model-doctor-reports/references/assessment-schema.json \
  skills/creating-model-doctor-reports/tests/test_model_doctor_general_verdict.py \
  skills/creating-model-doctor-reports/tests/test_model_doctor_evidence_v3.py \
  skills/creating-model-doctor-reports/tests/test_model_doctor_assessment_v7.py
git diff --cached --check
git commit -m "feat: extend assessment v7 for evidence v3"
```

## Task 15: Reject Incomplete or Protocol-Different PASS Reviews for 046-049

**Files:**

- Create: `skills/creating-model-doctor-reports/scripts/model_doctor_tool_loop_conformance.py`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_protocol_conformance.py`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py`
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_evidence_v3.py`
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_protocol_conformance.py`
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_assessment_v7.py`

- [ ] **Step 1: Add failing guard tests.**

Add exactly these tests:

```text
test_v3_tool_pass_guard_accepts_complete_ordered_loop
test_v3_tool_pass_guard_rejects_non_contiguous_turn_ids
test_v3_tool_pass_guard_rejects_incomplete_stream
test_v3_tool_pass_guard_rejects_non_conformant_contract
test_v3_tool_pass_guard_rejects_incomplete_final_loop_outcome
test_v3_tool_pass_guard_rejects_non_continued_intermediate_outcome
test_v3_tool_pass_guard_rejects_non_contiguous_manifest_refs_with_matching_embedded_ids
test_v3_tool_pass_guard_rejects_non_mapping_request
test_v3_tool_pass_guard_rejects_integer_turn_metadata
test_assessment_validator_reports_a_damaged_run_contract_without_crashing
test_v2_tool_reviews_are_not_rescored_by_v3_guard
test_assessment_validator_independently_rejects_tampered_v3_tool_pass
test_v3_tool_pass_guard_rejects_protocol_difference
test_v3_tool_transition_matrix_accepts_all_five_official_follow_ups
test_v3_tool_transition_matrix_rejects_all_five_correlation_mutations
test_protocol_differences_remain_diagnostic_only_for_legacy_and_non_tool_checks
```

- [ ] **Step 2: Run the guard tests and confirm false PASS is currently accepted.**

```bash
python3 skills/creating-model-doctor-reports/tests/test_model_doctor_evidence_v3.py \
  ModelDoctorEvidenceV3Tests.test_v3_tool_pass_guard_rejects_non_contiguous_turn_ids \
  ModelDoctorEvidenceV3Tests.test_v3_tool_pass_guard_rejects_incomplete_stream \
  ModelDoctorEvidenceV3Tests.test_assessment_validator_independently_rejects_tampered_v3_tool_pass \
  ModelDoctorEvidenceV3Tests.test_v3_tool_pass_guard_rejects_protocol_difference -v
```

Expected: all three fail because validation currently checks only document shape/evidence references.

- [ ] **Step 3: Implement the v3-only structural guard twice.**

Define:

```python
V3_TOOL_TEST_IDS = frozenset({"046", "047", "048", "049"})
V3_MIN_TOOL_TURNS = {"046": 2, "047": 3, "048": 2, "049": 3}

def _validate_v3_tool_pass_requests(test_id: str, requests: object) -> list[str]:
    errors: list[str] = []
    if not isinstance(requests, list):
        return [f"Test {test_id} PASS requests must be an array"]

    minimum = V3_MIN_TOOL_TURNS[test_id]
    if len(requests) < minimum:
        errors.append(
            f"Test {test_id} PASS requires at least {minimum} tool-loop turns"
        )

    for turn, request in enumerate(requests, start=1):
        expected_id = f"test-{test_id}-turn-{turn}"
        if not isinstance(request, dict):
            errors.append(
                f"Test {test_id} turn {turn} request must be an object"
            )
            continue
        if request.get("request_id") != expected_id:
            errors.append(
                f"Test {test_id} turn {turn} must reference {expected_id}"
            )
        if request.get("tool_loop_turn") != str(turn):
            errors.append(
                f"Test {test_id} request {expected_id} has mismatched tool_loop_turn"
            )
        if request.get("stream_termination") != "completed":
            errors.append(
                f"Test {test_id} request {expected_id} did not complete its stream"
            )
        if request.get("tool_contract_status") != "conformant":
            errors.append(
                f"Test {test_id} request {expected_id} is not contract-conformant"
            )

        expected_outcome = "completed" if turn == len(requests) else "continued"
        if request.get("tool_loop_outcome") != expected_outcome:
            errors.append(
                f"Test {test_id} request {expected_id} must have "
                f"tool_loop_outcome={expected_outcome}"
            )
    return errors

def _validate_v3_tool_pass_review(
    parsed: dict,
    test_id: str,
    review: object,
) -> list[str]:
    if not isinstance(review, dict):
        return [f"Test {test_id} review must be an object"]
    if review.get("reviewedStatus") != "PASS":
        return []
    try:
        contract = contract_key(parsed.get("run", {}))
    except ValueError as error:
        return [f"Test {test_id} PASS has an invalid run contract: {error}"]
    if contract != V3_CONTRACT:
        return []

    tests = parsed.get("tests", {})
    request_map = parsed.get("requests", {})
    if not isinstance(tests, dict) or not isinstance(request_map, dict):
        return [f"Test {test_id} PASS has invalid parsed evidence maps"]
    test = tests.get(test_id, {})
    if not isinstance(test, dict):
        return [f"Test {test_id} PASS manifest must be an object"]
    request_refs = test.get("requestRefs", [])
    if not isinstance(request_refs, list) or not all(
        isinstance(request_id, str) for request_id in request_refs
    ):
        return [f"Test {test_id} PASS requestRefs must be a string array"]

    expected_refs = [
        f"test-{test_id}-turn-{turn}"
        for turn in range(1, len(request_refs) + 1)
    ]
    errors: list[str] = []
    if request_refs != expected_refs:
        errors.append(
            f"Test {test_id} PASS requestRefs must be contiguous ordered turns"
        )
    missing = [request_id for request_id in request_refs if request_id not in request_map]
    if missing:
        errors.append(
            f"Test {test_id} PASS references missing request {missing[0]}"
        )
        return errors

    mapped_requests = [request_map[request_id] for request_id in request_refs]
    for request_id, request in zip(request_refs, mapped_requests):
        if isinstance(request, dict) and request.get("request_id") != request_id:
            errors.append(
                f"Test {test_id} request map key {request_id} does not match request_id"
            )
    errors.extend(_validate_v3_tool_pass_requests(
        test_id,
        mapped_requests,
    ))
    return errors
```

The request validator must require at least the per-check minimum, exact IDs `test-<id>-turn-1..N`, matching `tool_loop_turn`, `stream_termination == "completed"`, and `tool_contract_status == "conformant"`. Every request except the last must have `tool_loop_outcome == "continued"`; the last must be `completed`.

Apply this only to exact v3/0.11.0 PASS reviews in `validate_reviews`. Repeat the same validation using assessment-owned `run` and `tests[].requests` in `validate_assessment`, so post-assembly tampering cannot bypass it. Do not apply it to v1/v2.

- [ ] **Step 4: Add independent raw cross-turn protocol validation.**

Create this public, side-effect-free API:

```python
def validate_tool_loop_transitions(
    parsed: Mapping[str, object],
) -> dict[str, list[dict[str, str]]]:
    """Return bounded official-shape differences keyed by follow-up request ID."""
```

Only inspect exact evidence.v3 checks 046 through 049. Read each ordered pair
from `tests[test_id].requestRefs`; parse the preceding raw streamed
`responseBody`, then inspect the next raw JSON `requestBody`. Return
`CORRELATION`, `MISSING_FIELD`, `TYPE_MISMATCH`, `UNEXPECTED_FIELD`, or
`VALUE_MISMATCH` entries with RFC 6901 request-body locations and bounded
actual values. Validate these transitions:

Before transition matching, validate every referenced request body for the
provider's tool-loop request subset: JSON object shape, `stream: true`, official
tool declaration nesting and parameter schema, provider-native conversation
roles/items/parts, and absence of fields belonging only to another protocol.
The initial request must pass this validation as well as every follow-up.

```text
openai_chat:
  assistant tool_calls[].id/name/arguments
  -> assistant history plus role=tool, matching tool_call_id, string content

openai_responses:
  response ID plus function_call.call_id/name/arguments
  -> matching previous_response_id and function_call_output.call_id/output

anthropic_messages:
  assistant tool_use id/name/input
  -> immediately following user tool_result with matching tool_use_id;
     all tool_result blocks precede any text; timeout uses is_error=true

gemini_generate_content:
  complete model Content including thoughtSignature and functionCall
  -> preserved model Content plus user functionResponse with matching name,
     matching optional id when present, and object response

ollama_chat:
  accumulated assistant thinking/content/tool_calls
  -> preserved assistant message plus role=tool, matching tool_name, content;
     reject OpenAI-only tool_call_id, tool_choice, parallel_tool_calls, strict
```

Call this function from `analyze_protocol_conformance`. Attach returned
differences to the existing result for the follow-up request, recompute that
result's `status`, and build the summary only after transition differences are
merged. Do not add a second top-level report section.

In `validate_reviews`, compute the deterministic conformance report and reject
a v3 PASS for 046-049 when any associated result is absent or not
`CONSISTENT`. In `validate_assessment`, independently enforce the same rule
from assessment-owned `protocolConformance.results`. Historical inputs and
non-tool checks keep Issue #2's diagnostic-only behavior.

- [ ] **Step 5: Run all guard, transition, and assessment tests.**

```bash
python3 -m unittest skills/creating-model-doctor-reports/tests/test_model_doctor_evidence_v3.py -v
python3 -m unittest \
  skills/creating-model-doctor-reports/tests/test_model_doctor_protocol_conformance.py \
  skills/creating-model-doctor-reports/tests/test_model_doctor_assessment_v7.py -v
```

Expected: complete ordered loops for all five protocols pass; every structural
or cross-turn mutation is rejected; v2 and non-tool behavior are unchanged.

- [ ] **Step 6: Commit the PASS guard and transition audit.**

```bash
git add skills/creating-model-doctor-reports/scripts/model_doctor_tool_loop_conformance.py \
  skills/creating-model-doctor-reports/scripts/model_doctor_protocol_conformance.py \
  skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py \
  skills/creating-model-doctor-reports/tests/test_model_doctor_evidence_v3.py \
  skills/creating-model-doctor-reports/tests/test_model_doctor_protocol_conformance.py \
  skills/creating-model-doctor-reports/tests/test_model_doctor_assessment_v7.py
git diff --cached --check
git commit -m "feat: audit and guard v3 tool loop transitions"
```

## Task 16: Update the Report Skill and User Documentation

**Files:**

- Modify: `skills/creating-model-doctor-reports/SKILL.md`
- Modify: `skills/creating-model-doctor-reports/references/evaluation-rules.md`
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_evidence_v3.py`
- Test: `skills/creating-model-doctor-reports/tests/test_model_doctor_protocol_conformance.py`
- Modify: `README.md`

- [ ] **Step 1: Read and apply `superpowers:writing-skills`.**

Read that skill completely before editing this repository skill. Keep the existing evidence-first, binary PASS/FAIL workflow and only add the v3 contract branch.

- [ ] **Step 2: Add failing documentation contract tests.**

Add:

```text
test_skill_documents_v3_contract_and_assessment_v7
test_rules_define_strict_v3_tool_loop_checks
test_rules_preserve_legacy_contract_interpretation
test_rules_do_not_rescore_legacy_047_049_with_v3_final_answer_rules
test_readme_describes_011_evidence_v3_and_47_checks
```

- [ ] **Step 3: Run the documentation tests.**

```bash
python3 skills/creating-model-doctor-reports/tests/test_model_doctor_evidence_v3.py \
  ModelDoctorEvidenceV3Tests.test_skill_documents_v3_contract_and_assessment_v7 \
  ModelDoctorEvidenceV3Tests.test_rules_define_strict_v3_tool_loop_checks \
  ModelDoctorEvidenceV3Tests.test_rules_preserve_legacy_contract_interpretation \
  ModelDoctorEvidenceV3Tests.test_readme_describes_011_evidence_v3_and_47_checks -v
```

Expected: Issue #2's assessment.v7 and `protocolConformance` documentation is
present, while evidence.v3, collector 0.11.0, 47 checks, and the v3-only
046-049 hard gate are not yet documented.

- [ ] **Step 4: Document exact v3 assessment rules.**

The Skill and evaluation rules must state:

```text
accepted new input: collector 0.11.0 + evidence.v3
output: assessment.v7 for every accepted historical/current input
v3 check 006 PASS: stream_termination is completed
v3 check 046: complete official protocol call/result/final cycle and final marker
v3 check 047: weather, then time, then MODEL_DOCTOR_CASE_047_OK
v3 check 048: final marker plus exact WEATHER_SUNNY preservation
v3 check 049: exactly one timeout retry, successful result, final marker
v3 046-049: every ordered request completed and runtime-conformant, final loop completed
v3 046-049: every associated protocolConformance result is CONSISTENT,
            including raw response -> follow-up request correlation
historical v1/v2: retain their original evidence and tool-check rules
totals: historical 46/31/15; v3 47/32/15
```

Update README release names to `v0.11.0`, evidence to v3, default/list count to 47, tool coverage to include official structure and full loops, and log-file wording to evidence-v3. Replace the blanket URL statement with the precise rule that configured endpoints are preserved except Gemini streaming normalizes `:generateContent` to `:streamGenerateContent` and sets `alt=sse`.

- [ ] **Step 5: Verify and commit documentation.**

```bash
python3 -m unittest skills/creating-model-doctor-reports/tests/test_model_doctor_evidence_v3.py -v
python3 -m unittest skills/creating-model-doctor-reports/tests/test_model_doctor_protocol_conformance.py -v
git add skills/creating-model-doctor-reports/SKILL.md \
  skills/creating-model-doctor-reports/references/evaluation-rules.md \
  skills/creating-model-doctor-reports/tests/test_model_doctor_evidence_v3.py README.md
git diff --cached --check
git commit -m "docs: define evidence v3 tool loop assessment"
```

## Task 17: Run Full Verification and Review the Issue Contract

**Files:**

- Modify only files required by failures attributable to this implementation.

- [ ] **Step 1: Run format, Rust tests, lint, and release build in the isolated worktree.**

```bash
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo build --release --locked
```

Expected: all commands exit 0. If the ignored stale root `tests/` directory appears, stop and correct the worktree setup; do not delete user files to make tests pass.

- [ ] **Step 2: Run the complete report suite and syntax checks.**

```bash
python3 -m unittest discover -s skills/creating-model-doctor-reports/tests -p 'test_*.py'
python3 -m compileall -q skills/creating-model-doctor-reports/scripts
```

Expected: every Python test passes and compileall is silent.

- [ ] **Step 3: Scan for stale active-contract text and placeholders.**

```bash
rg -n "assessment\.v6|Run all 46|46-item|默认执行全部 46|evidence-v2|v0\.10\.0" \
  README.md src skills/creating-model-doctor-reports
PLACEHOLDER_PATTERN='TO''DO|TB''D|implement'' later|similar'' to|handle'' edge cases'
rg -n "$PLACEHOLDER_PATTERN" src skills/creating-model-doctor-reports docs/superpowers
```

Expected: the first search only finds explicit v1/v2 historical compatibility statements and tests for them; the second finds no implementation placeholders added by this work.

- [ ] **Step 4: Review Issue #3 acceptance evidence.**

Confirm from automated tests, not inspection alone:

```text
five official streaming protocol parsers
normal terminal distinct from EOF/timeout/disconnect/cancel
046-049 bounded full loop with ordered manifest refs
official outgoing request/result shape validated
Issue #2 exhaustive protocolConformance retained for every raw response
v3 046-049 raw response -> follow-up request correlation is CONSISTENT
raw/partial response retained
evidence.v3 parser and assessment.v7 compatibility matrix
programmatic false-PASS guard
```

- [ ] **Step 5: Inspect the final diff and commit verification fixes.**

```bash
git diff --check
git status --short
git diff --stat origin/main...HEAD
```

If verification required code changes, return to the task that owns the failed
contract, add only that task's listed files with its exact `git add` command,
rerun its focused test plus the full verification commands, and commit with
`git commit -m "test: verify official streaming tool loops"`.

If no files changed, do not create an empty commit. Then use `superpowers:requesting-code-review`, resolve findings with focused tests, and use `superpowers:finishing-a-development-branch` to present integration options.
