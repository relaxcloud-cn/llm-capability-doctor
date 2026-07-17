# Evidence-Only Model Doctor Design

## Summary

Replace the current mixed collector-and-grader architecture with a strict
two-stage contract:

1. `model-capability-doctor.sh` executes curl request sequences and records
   complete, redacted evidence.
2. `creating-model-doctor-reports` is the only component that evaluates that
   evidence and produces binary capability verdicts.

This is an intentional breaking change. The new Skill accepts only the new log
format. Logs, assessment JSON, and reports from earlier script versions are not
supported or upgraded.

## Goals

- Remove every capability verdict, semantic comparison, status counter, and
  recommendation from the shell collector.
- Preserve complete request inputs, response outputs, response headers, curl
  stderr, transport metrics, timestamps, and request relationships.
- Give every selected test an explicit manifest that identifies all requests
  needed to evaluate that test, including shared and multi-turn evidence.
- Complete tool-result request chains for OpenAI Chat Completions, OpenAI
  Responses, Anthropic Messages, Gemini GenerateContent, and Ollama Chat.
- Make the Skill produce only `PASS` or `FAIL` for tests and categories, and
  only `READY` or `BLOCKED` overall.
- Ensure missing, malformed, unsupported, timed-out, or incomplete evidence is
  a `FAIL` with a precise reason, never a third status.
- Keep the customer-site workflow as one portable Bash script using curl and
  common system text tools.

## Non-Goals

- Preserving compatibility with old log formats or assessment schemas.
- Teaching the shell script to make better semantic judgments.
- Hiding collection failures or capability limitations.
- Calling the tested endpoint again during Skill evaluation.
- Replacing curl with a Python, Node.js, Docker, or SDK-based collector.
- Proving an upstream model identity from its requested or returned name.

## Responsibility Boundary

### Shell Collector

The shell collector may:

- choose a request envelope and authentication header after protocol probes;
- build deterministic prompts and tool schemas;
- extract protocol correlation data required to construct the next curl, such
  as a response ID, tool-call ID, tool name, or raw assistant message;
- schedule sequential, repeated, streaming, and concurrent curl requests;
- redact credentials and write complete audit blocks;
- record factual collection metadata and explicit request references.

The shell collector must not:

- emit `PASS`, `FAIL`, `UNSUPPORTED`, `UNDETERMINED`, `SKIPPED`, or `ERROR` as
  test results;
- compare a visible answer with an expected answer;
- infer whether a model supports a capability;
- parse a response for the purpose of grading it;
- calculate semantic success rates, capability percentiles, category results,
  readiness, or recommended concurrency;
- write `result`, `expected`, `detected`, or `conclusion` fields;
- fabricate or normalize a model's assistant turn before a tool-result
  follow-up.

Transport values such as curl exit code, HTTP status, byte count, and timing are
evidence fields, not judgments.

### Skill Evaluator

The Skill must:

- validate the new evidence-log version before evaluation;
- parse every test manifest and all referenced requests;
- judge protocol structure, visible answers, tool calls, multi-turn
  correlation, safety behavior, and performance samples;
- assign exactly one binary status to every discovered test;
- derive category and overall results only from Skill-authored verdicts;
- retain observable reasons, limitations, and rerun instructions without
  creating a third status.

## New Log Contract

The collector version becomes `0.7.0`, with log schema
`llm-capability-doctor.evidence.v1` in the run header.

### Run Header

The run header contains only collection facts:

```text
========== MODEL DOCTOR RUN ==========
run_id: MD-...
script_version: 0.7.0
log_schema: llm-capability-doctor.evidence.v1
started_at: ...
url: ...
model: ...
api_key: ...
curl_version: ...
selected_test_count: 62
```

### Request Blocks

The existing request audit remains the evidence of record. Each request block
contains:

- request ID, start and completion timestamp;
- protocol, authentication mode, and stream flag;
- redacted curl reproduction command;
- complete request body;
- curl exit code, HTTP status, total time, TTFB, and downloaded bytes;
- redacted response headers and curl stderr; and
- complete redacted response body.

### Test Manifests

Each selected test receives one manifest after its request sequence:

```text
========== TEST-047 BEGIN ==========
name: Serial tool calling
category: Tool calling
completed_at: ...
request_refs: test-047,test-047-follow
========== TEST-047 END ==========
```

The manifest has no raw-response copy and no judgment fields. An empty
`request_refs` value is allowed only when request construction was impossible;
the protocol probes and absence of a test request are then the complete
observable evidence and the Skill returns `FAIL`.

Shared evidence is explicit:

- `002` references every protocol probe.
- `003` and `007` reference the selected successful protocol probe, or all
  probes when none was selected.
- `055` and `056` reference the same five repeat requests.
- All multi-turn, repeated, recovery, and concurrent tests list every request
  in chronological order.

### Run Summary

The summary contains collection facts only:

```text
========== RUN SUMMARY ==========
completed_at: ...
duration_seconds: ...
request_count: ...
test_manifest_count: ...
========== END ==========
```

It contains no test-status counts.

## Request Orchestration

### Protocol Discovery

Protocol discovery remains an orchestration decision. Every attempted probe is
logged. Selecting a request builder does not create a capability verdict. Tests
`002`, `003`, and `007` reference the relevant probe requests explicitly.

### Tool Follow-Ups

Tests `047-049` must use the actual first model response when constructing the
follow-up request:

- OpenAI Responses uses the observed `previous_response_id` and `call_id`.
- OpenAI Chat reuses the exact observed assistant `message` object and appends
  the correlated `tool` message.
- Anthropic Messages reuses the exact observed assistant `content` array and
  appends a user `tool_result` block with the observed `tool_use_id`.
- Gemini GenerateContent reuses the exact observed candidate `content` and
  appends a user `functionResponse` with the observed call ID and function
  name.
- Ollama Chat reuses the exact observed assistant `message` and appends a
  `tool` message with the observed `tool_name`.

When a required raw object or correlation field is absent, the collector does
not invent one. It records the first request, writes the manifest with the
available request reference, and continues to the next test. The Skill judges
the incomplete tool contract as `FAIL`.

### Performance Collection

The shell schedules the defined requests and records raw per-request metrics.
It does not decide semantic success or calculate capability conclusions.

- `051-054` each retain their direct request evidence.
- `055-056` share five explicitly referenced requests.
- `057` records every configured concurrent request. P50, P95, rate-limit
  counts, semantic success, and readiness impact are computed by the Skill.
- `058` records ten load requests and one distinct recovery request.

Any future adaptive stress scheduler may stop or bound traffic using factual
operational safety limits, but throughput scoring, eligibility, recommended
concurrency, and capability status belong to the Skill. The existing adaptive
concurrency design and plan must be revised to preserve this boundary before
implementation.

## Binary Assessment Contract

The assessment schema becomes `llm-capability-doctor.assessment.v3`.

Per-test `reviewedStatus` allows only:

- `PASS`: complete observable evidence satisfies the test contract.
- `FAIL`: every other outcome, including wrong output, unsupported capability,
  transport failure, timeout, malformed response, missing request, incomplete
  follow-up, or ambiguous evidence.

`confidence` remains explanatory metadata and never creates another status.
`limitations` and `retestInstructions` remain available on failed tests.

Category status is `PASS` only when every test in that category passes;
otherwise it is `FAIL`.

The overall verdict is:

- `READY` when every critical and important test passes.
- `BLOCKED` when any critical or important test fails.

Observation failures remain explicit `FAIL` results and failed category rows,
but do not independently change the overall agent-platform gate. There is no
`CONDITIONAL` verdict.

## Parser and Renderer

The parser accepts only `llm-capability-doctor.evidence.v1`. A missing or
different `log_schema` is a hard validation error. Legacy parsing helpers,
legacy fixtures, compatibility links, old catalog mappings, and v2 assessment
acceptance are deleted.

The assessment JSON and HTML must not contain:

- script-authored result fields;
- unknown or conditional status counts;
- `unknowns`, `conditions`, `criticalFailures` based on non-binary states;
- labels such as "需复测", "待补证", "无法判定", "不支持", "跳过", or
  "执行错误".

The HTML continues to show complete request/response evidence, factual metrics,
test logic, failure reasons, limitations, and rerun instructions.

## Error Handling

- Invalid CLI syntax exits before creating a log and prints a bounded error.
- Once logging begins, every curl outcome is recorded and collection continues
  when the next request can be constructed safely.
- A signal handler cleans up and exits immediately with a nonzero code.
- Missing protocol correlation data never triggers fabricated follow-up input.
- Parser or Skill validation failures stop report generation; they are not
  converted into a capability status because no trustworthy assessment exists.

## Testing

Restore a maintained local test suite and use test-driven development.

Collector tests must prove:

- the shell source and generated logs contain no judgment fields or status
  counters;
- all 62 selected tests produce one manifest;
- successful fixture runs give every manifest the complete expected request
  references, including `002`, `003`, `007`, `056`, and `047-049` for every
  supported protocol;
- tool follow-up bodies round-trip actual assistant content and correlation
  fields rather than synthetic replacements;
- transport errors remain raw request evidence;
- malformed CLI values terminate instead of looping;
- `INT` and `TERM` clean up and exit;
- logs remain redacted and auditable.

Skill tests must prove:

- only the new log schema is accepted;
- only `PASS` and `FAIL` reviews validate;
- missing and ambiguous evidence validates only as `FAIL`;
- categories and overall output are binary;
- JSON and HTML contain no legacy or conditional status text;
- every rendered evidence panel contains all manifest-referenced requests;
- credentials remain redacted and the HTML remains offline and escaped.

End-to-end verification must run the collector against deterministic fake
implementations of all five protocols, parse each log, create binary reviews,
validate assessment v3, render HTML, and scan both artifacts for forbidden
legacy fields and labels.

## Delivery

The implementation changes:

- `model-capability-doctor.sh`;
- the report Skill instructions, evaluation rules, schema, parser, assessment
  assembler, and HTML renderer;
- the adaptive concurrency design and plan so future work preserves the
  evidence-only boundary;
- a restored automated test suite; and
- `README.md` to describe the breaking `0.7.0` workflow.

Existing user log and report files in the worktree are never modified.
