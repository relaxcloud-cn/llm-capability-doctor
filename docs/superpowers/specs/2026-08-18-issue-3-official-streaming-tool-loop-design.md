# Issue 3 Official Streaming Tool Loop Design

## Status

Approved interactively on 2026-08-18. This design implements GitHub Issue #3
for all five protocols supported by the collector and restores check 046 as a
dedicated official tool-protocol conformance check.

Issue: <https://github.com/relaxcloud-cn/llm-capability-doctor/issues/3>

## Problem

The current collector does not prove a complete tool loop. Checks 047, 048,
and 049 send a non-streaming initial request, optionally send one follow-up,
and then stop even if the model asks for another tool. The evidence therefore
cannot prove this complete chain:

```text
assistant tool call -> correlated tool result -> next assistant turn(s) -> final answer
```

The HTTP collector also treats a clean response-body EOF as success. It does
not distinguish an official protocol terminal event from a missing terminal
event, timeout, interrupted connection, protocol error, or client
cancellation.

Finally, the tool checks validate tool names and arguments semantically but do
not independently prove that the streamed response and the follow-up tool
result use the provider's official structure.

## Goals

1. Run checks 046 through 049 as complete, bounded, streaming tool loops.
2. Use the official request, stream-event, correlation, tool-result, and
   terminal contracts for OpenAI Chat Completions, OpenAI Responses,
   Anthropic Messages, Gemini GenerateContent, and Ollama Chat.
3. Record transport termination, protocol termination, tool-contract
   conformance, and every ordered request in auditable evidence.
4. Make reporting pass checks 046 through 049 only when the complete loop and
   its official structure are observable.
5. Preserve support for historical collector contracts without reinterpreting
   their evidence under the new rules.

## Non-Goals

- Supporting protocols outside the five already detected by the collector.
- Executing arbitrary model-selected code or external tools.
- Migrating Gemini GenerateContent to the newer Gemini Interactions API.
- Adding a separate diagnostic or fixture for whitespace after the SSE
  `data:` field. The stream decoder follows the SSE field grammar, but the
  previously discussed whitespace incident is not a separate acceptance case.
- Changing the semantic purpose of tool checks 040 through 045 or 050.

## Contract Versions and Catalog

Check 046 previously existed as `Call ID integrity`. Restore the ID and expand
its purpose to `official tool protocol conformance`. It runs one complete tool
call and final-answer cycle. Checks 047 through 049 remain separate because
they test serial orchestration, tool-result fidelity, and failure recovery.

The fixed catalog becomes 47 checks:

- 32 core checks, including restored check 046.
- 15 enhanced checks, unchanged.

The compatibility matrix is:

| Collector | Evidence contract | Required manifests | Report interpretation |
|---|---|---:|---|
| 0.9.0 | evidence.v1 | historical profile-dependent set | historical rules |
| 0.10.0 | evidence.v2 | original 46 checks | original 46-check rules |
| 0.11.0 | evidence.v3 | new 47 checks | complete streaming-loop rules |

The new contract uses `llm-capability-doctor.evidence.v3` because the required
manifest set and the meaning of checks 047 through 049 change, not merely
because additive request fields are present. The parsed-evidence and authored
review shapes remain compatible. The assembled report moves from
`assessment.v6` to `assessment.v7` because the deterministic verdict changes
from 46 checks and 31 core checks to 47 checks and 32 core checks.

The current report tooling continues accepting evidence.v1 and evidence.v2.
Assessment v7 represents historical inputs with their contract-specific
totals; it does not invent check 046 for an old log or re-score an old 047-049
chain using the 0.11.0 criteria.

## Architecture

Separate transport collection, protocol parsing, and tool-loop orchestration:

```text
HTTP byte stream
    -> transport outcome and raw bytes
    -> protocol-specific stream parser
    -> normalized assistant turn and contract validation
    -> deterministic tool dispatcher
    -> protocol-specific official follow-up request
    -> next streamed assistant turn
    -> final assistant answer or explicit bounded failure
```

### Transport Layer

`src/http.rs` remains responsible for HTTP behavior only. It records status,
headers, partial raw body, timings, and one transport outcome:

- `completed_eof`: the body stream ended without a transport error;
- `timeout`: request or body read reached the configured timeout;
- `upstream_disconnect`: headers were received and body reading failed;
- `client_cancelled`: the cancellation token won the request/body race;
- `transport_error`: DNS, connect, TLS, or another pre-response failure.

The transport layer does not call a clean EOF a valid model stream. The
protocol parser makes that decision using the raw body and detected protocol.
Partial bytes remain in `response_body` for disconnect, timeout, and
cancellation evidence.

### Normalized Stream Result

Add a protocol stream module that returns a normalized result without losing
the protocol-native history needed for the next request:

```text
StreamParseResult
  assistant_turn
    protocol_history
    tool_calls[]
      correlation
      name
      arguments
    final_text
  terminal_signal
  event_count
  contract_violations[]
```

One central SSE decoder implements field names, optional field-value spacing,
multi-line `data` values, comments, event names, and blank-line dispatch. The
protocol parsers consume decoded events rather than splitting raw text on
provider-specific string prefixes. Ollama uses a separate incremental NDJSON
decoder. `event_count` counts every successfully decoded SSE event or NDJSON
object, including ignorable ping/extension events.

`protocol_history` preserves the official assistant payload, including fields
such as Gemini thought signatures, instead of reconstructing a lossy generic
message. Tool arguments are executed only after every fragment has been
assembled into valid JSON and the stream has an official normal terminal.

The parser accepts documented optional fields and forward-compatible extension
events. "Officially conformant" means required field names, types, event
relationships, event ordering, correlation values, argument JSON, and terminal
semantics are valid. It does not mean byte-for-byte equality, fixed JSON field
order, or rejection of documented extensions.

## Official Protocol Contracts

### OpenAI Chat Completions

- Keep the configured Chat Completions endpoint and set `stream: true`.
- Decode SSE JSON chunks and accumulate `choices[0].delta.tool_calls` by
  `index`.
- Require the completed call to contain `id`, `type: "function"`, function
  `name`, and JSON `arguments`. Fields that are officially present only on the
  first delta are carried forward while later argument fragments are appended.
- Require a tool-call turn to finish with `finish_reason: "tool_calls"` and the
  stream to finish with `data: [DONE]`.
- Send the accumulated assistant message followed by
  `role: "tool"`, the matching `tool_call_id`, and `content`.
- Record the model stop reason separately: tool turns require `tool_calls` and
  final-answer turns require `stop` before `[DONE]`.

### OpenAI Responses

- Keep the configured Responses endpoint and set `stream: true`.
- Correlate `response.output_item.added`,
  `response.function_call_arguments.delta`,
  `response.function_call_arguments.done`, and output-item completion by item
  ID and output index.
- Require the completed `function_call` to contain `call_id`, `name`, and valid
  JSON `arguments`.
- Send `type: "function_call_output"`, the matching `call_id`, and `output`,
  using `previous_response_id` to continue the response.
- Accept `response.completed` with a completed response status as normal
  completion. `response.incomplete` is a model-incomplete termination;
  `response.failed` and stream `error` events are protocol errors.

### Anthropic Messages

- Keep the configured Messages endpoint and set `stream: true`.
- Validate the official message event flow: `message_start`, content-block
  start/delta/stop events, `message_delta`, and `message_stop`. Ping and
  documented future event types do not invalidate the stream.
- Accumulate `input_json_delta.partial_json` for a `tool_use` block and require
  its `id`, `name`, object input, and `stop_reason: "tool_use"`.
- Preserve the assistant content blocks exactly. Send an immediately following
  user content block with `type: "tool_result"`, matching `tool_use_id`,
  content, and `is_error: true` for the simulated timeout in check 049.
- If one assistant turn contains multiple client `tool_use` blocks, return one
  result for every block in the same immediately following user message, with
  all `tool_result` blocks before any text.
- An `event: error` frame is a protocol error; `message_stop` is the normal
  terminal signal. Tool turns require `stop_reason: "tool_use"`; final-answer
  turns require `stop_reason: "end_turn"`.

### Gemini GenerateContent

- Stay on the official GenerateContent REST contract.
- For streamed turns, normalize a configured `:generateContent` action to
  `:streamGenerateContent`, preserve unrelated query parameters, and set
  `alt=sse`. An already-streaming action is left streaming and normalized to
  one `alt=sse` parameter.
- Decode SSE `GenerateContentResponse` objects. Accumulate candidate content
  and require an official `functionCall` with `name` and object `args`.
- Treat `functionCall.id` as optional because the GenerateContent schema does
  not require it for every model generation. When present, preserve it and
  echo it exactly in `functionResponse`; when absent, do not invent one and
  correlate these single-call test turns by function name and turn order.
- Preserve the complete model content, including thought signatures. Send a
  user `functionResponse` with the same `name`, optional matching `id`, and the
  structured `response` object.
- Gemini has no `[DONE]` sentinel for this endpoint. Normal completion requires
  `finishReason: STOP` followed by clean transport EOF. Another non-empty
  finish reason is an observed model-incomplete termination, not normal
  completion.

### Ollama Chat

- Keep `/api/chat`, set `stream: true`, and decode newline-delimited JSON.
- Use Ollama's documented request body and tool definition. Do not send
  OpenAI-only `tool_choice`, `parallel_tool_calls`, or function `strict`
  fields to Ollama.
- Accumulate assistant `thinking`, `content`, and
  `message.tool_calls[].function` values as documented.
- Require each function call's name and object arguments. Validate `index` when
  it is present; otherwise use array order. Ollama does not require an
  OpenAI-style call ID, so conformance must not invent one.
- Send the accumulated assistant message followed by `role: "tool"`, matching
  `tool_name`, and `content`.
- Require the final NDJSON object to contain `done: true`.
- Record `done_reason` when present. `stop` is normal; a truncating reason such
  as `length` is model-incomplete. Absence remains valid for Ollama versions
  whose documented response omits this optional field.

## Protocol and Termination Evidence

Every request records the following flat request metadata before its encoded
sections:

```text
transport_outcome: completed_eof | timeout | upstream_disconnect |
                   client_cancelled | transport_error
stream_termination: not_applicable | completed | missing_terminal_event |
                    timeout | upstream_disconnect | client_cancelled |
                    transport_error | http_error | malformed_stream |
                    protocol_error | model_incomplete
stream_end_signal: none | [DONE] | response.completed | message_stop |
                   finishReason:<value> | done:true
model_stop_reason: none | <protocol-native value>
stream_event_count: <non-negative integer>
tool_contract_status: not_applicable | conformant | non_conformant
tool_contract_errors_json: <single-line JSON string array>
tool_loop_turn: <zero or positive integer>
tool_loop_outcome: not_applicable | continued | completed |
                   invalid_turn | transport_failure |
                   max_turns_exceeded
```

The full redacted raw response remains the primary event record. The new fields
make classification explicit and machine-readable without replacing raw
evidence. `tool_loop_turn` is the assistant request sequence number inside one
tool loop; zero means the request is not part of such a loop. Contract errors
contain stable codes and JSON paths, not provider content or credentials.

Classification uses this precedence:

1. Cancellation token observed: `client_cancelled`.
2. Reqwest timeout: `timeout`.
3. No HTTP response because of another transport failure: `transport_error`.
4. Non-success HTTP response: `http_error`.
5. Body read failure after headers: `upstream_disconnect`.
6. Invalid SSE/NDJSON framing or invalid event JSON: `malformed_stream`.
7. Official stream error/failure event: `protocol_error`.
8. Clean EOF without the protocol terminal: `missing_terminal_event`.
9. Official terminal with a truncated or non-normal model stop reason:
   `model_incomplete`.
10. Official terminal with the expected model stop reason: `completed`.

An officially terminated stream with malformed accumulated tool arguments is
still `stream_termination: completed`; it is
`tool_contract_status: non_conformant` and is never executed. Stream
completion describes the wire lifecycle, not semantic success.

For checks 046 through 049, `tool_contract_status` covers both the outgoing
request and the incoming response envelope. A final assistant turn with no
tool call is `conformant` when its official response/event structure is valid;
the field is not limited to turns that contain a tool call.

No tool is executed from a turn unless `stream_termination` is `completed` and
`tool_contract_status` is `conformant`.

## Bounded Tool Loop

Checks 046 through 049 use one reusable state machine with
`MAX_ASSISTANT_TURNS = 4`. Every streamed assistant response counts, whether it
contains tool calls or the final answer. The state machine performs these
steps:

1. Send the initial official streaming request.
2. Collect raw evidence and parse the complete assistant turn.
3. Stop explicitly if transport, stream termination, or contract validation
   fails.
4. If the turn contains tool calls, execute only deterministic allowlisted
   fixtures, append official tool results, and stream the next turn.
5. If the turn contains no tool calls, record the final assistant turn and end
   the collection loop. Semantic PASS/FAIL remains the reporting Skill's job.
6. If turn four still requests a tool, record
   `tool_loop_outcome: max_turns_exceeded` and do not execute it or send a
   fifth assistant request.

Every request is audited before the next turn. Request IDs are stable and
ordered, for example `test-047-turn-1`, `test-047-turn-2`, and
`test-047-turn-3`. The manifest is appended after collection and contains every
request ID in execution order, including the last failed or bounded turn.

Before each send, a protocol request validator checks the endpoint action,
tool declarations, stream flag, conversation roles/content, correlation
fields, and result shape. Runtime response validation and the reporting Skill
then inspect the actual wire response. This prevents a conformant provider
response from hiding a collector-generated non-conformant follow-up request.

The deterministic dispatcher has no external effects:

- Normal `get_weather(city="Beijing")` returns `WEATHER_SUNNY`.
- `get_time(zone="UTC")` returns `TIME_UTC_12:00`.
- Check 049 carries the application error `ERROR: timeout` inside the
  protocol's official tool-result envelope for the first correct weather call,
  then
  `WEATHER_SUNNY` for the single correct retry.
- Unknown tools and invalid arguments are not run. The loop records an invalid
  turn rather than invoking arbitrary code.

The check-049 result envelope is protocol-specific. Only Anthropic defines a
dedicated `is_error` flag; the other protocols carry the application error in
their documented result content/output field:

| Protocol | Official follow-up representation |
|---|---|
| OpenAI Chat | tool message `content: "ERROR: timeout"` |
| OpenAI Responses | `function_call_output.output: "ERROR: timeout"` |
| Anthropic Messages | `tool_result` with `is_error: true` and matching `tool_use_id` |
| Gemini GenerateContent | matching name/order and optional ID in `functionResponse`, with `response: {"error":"timeout"}` |
| Ollama Chat | tool message with matching `tool_name` and `content: "ERROR: timeout"` |

## Check Semantics

### 046 Official Tool Protocol Conformance

Expected chain:

```text
streamed get_weather call
-> officially correlated WEATHER_SUNNY result
-> streamed final MODEL_DOCTOR_CASE_046_OK answer
```

PASS requires every request and response structure to be conformant for the
detected protocol, valid reconstructed argument JSON, correct correlation, a
normal terminal on every streamed turn, and the final marker. Extra documented
optional fields are allowed.

### 047 Serial Tool Calls

Expected chain:

```text
streamed get_weather call
-> WEATHER_SUNNY
-> streamed get_time call
-> TIME_UTC_12:00
-> streamed final MODEL_DOCTOR_CASE_047_OK answer
```

PASS requires exactly this serial order, official correlations, conformant
structures, normal terminals, and the final marker.

### 048 Tool Result Fidelity

Expected chain:

```text
streamed get_weather call
-> WEATHER_SUNNY
-> streamed final answer beginning with MODEL_DOCTOR_CASE_048_OK
```

PASS additionally requires the final answer to preserve `WEATHER_SUNNY`
exactly without replacing, omitting, or inventing the tool result.

### 049 Tool Failure Recovery

Expected chain:

```text
streamed get_weather call
-> official error result containing ERROR: timeout
-> exactly one streamed retry of get_weather(city="Beijing")
-> WEATHER_SUNNY
-> streamed final MODEL_DOCTOR_CASE_049_OK answer
```

PASS requires exactly one retry, no fabricated success before the successful
result, official correlation and error representation, conformant structures,
normal terminals, and the final marker.

## Cancellation Behavior

The existing top-level cancellation token remains the source of client
cancellation. If cancellation occurs during an HTTP request, the collector
writes the partial request evidence with
`stream_termination: client_cancelled`, flushes it, and exits with the existing
signal-specific code. A cancelled run has no complete run summary and is not a
valid report input. The evidence still distinguishes client cancellation from
timeout or upstream disconnect for diagnosis.

## Reporting Changes

The reporting Skill and scripts apply rules by the exact evidence
schema/collector version pair.

- The parser accepts the new evidence.v3/0.11.0 pair and requires exactly 47
  manifests, including 046. Existing supported pairs retain their old expected
  manifest sets.
- Check 006 uses `stream_termination: completed` for new evidence instead of
  merely finding an end-marker substring in a raw body.
- Check 046 is a core check. A non-conformant request/response pair is a direct
  protocol-contract failure.
- Checks 047 through 049 require every ordered request, every normal terminal,
  conformant tool structures, and their complete final-answer chains.
- A missing follow-up, max-turn stop, malformed stream, transport failure, or
  ambiguous correlation is FAIL, never partial PASS.
- Assessment assembly and validation programmatically reject a PASS review for
  evidence.v3 checks 046 through 049 when request references are not contiguous
  ordered turns, any referenced stream is not `completed`, any referenced tool
  contract is not `conformant`, or the final request does not record
  `tool_loop_outcome: completed`. Human semantic review still evaluates the
  exact expected calls, results, retry count, and final markers.
- Reviews remain `reviews.v2`. The assembled output becomes assessment.v7 and
  derives contract-specific totals. New complete runs use 47 total, 32 core,
  and 15 enhanced checks. Historical evidence.v2 runs remain 46/31/15.

## Test Strategy

### Protocol Fixture Tests

Store small, tracked official-shape fixtures under `src/protocol/fixtures/` and
load them from Rust unit tests. For each protocol, cover:

- arbitrary HTTP byte boundaries without changing the decoded events/objects;
- official assistant history and tool-result follow-up shape;
- official normal terminal;
- missing terminal after clean EOF;
- malformed or truncated final event;
- protocol-native error event.

OpenAI Chat, OpenAI Responses, and Anthropic additionally cover their official
string argument-delta reconstruction. Gemini and Ollama fixtures keep
`functionCall.args` and `function.arguments` as structured objects; they must
not invent string argument deltas that those official protocols do not define.

The fixtures use deterministic Model Doctor markers and no vendor credentials.

### Loop Tests

Use scripted assistant turns to test 046, 047, 048, and 049 through their final
answers for all five protocols. Verify ordered request IDs, matching
correlations, exact deterministic tool results, one retry in 049, and the
four-assistant-turn limit.

### Transport Tests

Use a local raw TCP fixture server to exercise conditions that high-level HTTP
mocks cannot represent reliably:

- complete body and clean EOF;
- clean EOF without a terminal event;
- partial frame followed by connection abort;
- body timeout after headers;
- cancellation while awaiting headers and while reading the body;
- non-2xx HTTP response.

Assert that partial response bytes and the correct classification are retained.

### Evidence and Report Tests

Rust tests verify every new evidence field, ordered manifest references, 47
selected manifests, redaction, and incomplete cancellation output. Python Skill
tests verify all three supported evidence contracts, v3 required-manifest
validation, assessment.v7 totals, strict 046-049 evaluation guidance, and
historical v2 compatibility.

Because the repository ignores the root `tests/` directory, new Rust fixtures
and test modules live under tracked `src/` paths. Existing tracked Skill tests
remain under `skills/creating-model-doctor-reports/tests/`.

## Verification

Before completion, run:

```bash
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release --locked
python3 -m unittest discover -s skills/creating-model-doctor-reports/tests -p 'test_*.py'
```

No live provider credentials are required for the contract or loop tests.

## Normative References

The implementation snapshot uses the official documentation available on
2026-08-18:

- OpenAI function calling and streaming:
  <https://developers.openai.com/api/docs/guides/function-calling#streaming>
- OpenAI Responses streaming events:
  <https://platform.openai.com/docs/api-reference/responses-streaming>
- Anthropic streaming Messages:
  <https://platform.claude.com/docs/en/build-with-claude/streaming>
- Anthropic tool-call handling:
  <https://platform.claude.com/docs/en/agents-and-tools/tool-use/handle-tool-calls>
- Gemini GenerateContent function calling:
  <https://ai.google.dev/gemini-api/docs/generate-content/function-calling>
- Gemini GenerateContent REST reference:
  <https://ai.google.dev/api/generate-content>
- Ollama tool calling and streaming:
  <https://docs.ollama.com/capabilities/tool-calling> and
  <https://docs.ollama.com/api/streaming>

## Security and Evidence Boundaries

- Tool execution is a closed deterministic map; model-selected names never
  dispatch shell commands, network calls, or filesystem operations.
- Raw request and response evidence continues through the existing credential
  redactor before it is written.
- Tool-contract validation reports observable wire-format violations. It does
  not attribute the violation to the model, gateway, proxy, or provider without
  separate evidence.
- A structurally conformant tool call proves API compatibility for the observed
  request only. It does not prove reliability across untested models, gateways,
  or traffic conditions.
