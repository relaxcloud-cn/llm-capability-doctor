# Evidence-Only Model Doctor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert Model Doctor into a versioned evidence-only curl collector and a binary Skill evaluator with no legacy-log compatibility.

**Architecture:** Bash owns deterministic request construction, protocol orchestration, curl execution, redaction, and explicit request-to-test manifests. Python parses only `llm-capability-doctor.evidence.v1`; the Skill authors the sole `PASS` or `FAIL` verdicts and the renderer derives binary category and readiness output. Tool follow-ups round-trip observed assistant objects for all five supported protocols instead of synthesizing history.

**Tech Stack:** Bash 3.2, curl, POSIX-style awk/sed/grep, Python 3 standard library, `unittest`, self-contained HTML/CSS/JavaScript.

---

## File Map

- `model-capability-doctor.sh`: evidence-only request scheduler and audit-log writer.
- `tests/helpers/fake_model_curl.py`: deterministic five-protocol curl fixture.
- `tests/test_model_capability_doctor_script.py`: collector contract, request-chain, CLI, signal, and redaction tests.
- `skills/creating-model-doctor-reports/scripts/model_doctor_log.py`: strict evidence-v1 parser.
- `tests/test_model_doctor_log.py`: parser schema and request-reference tests.
- `skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py`: binary assessment v3 validator and assembler.
- `skills/creating-model-doctor-reports/references/assessment-schema.json`: assessment v3 JSON Schema.
- `tests/test_model_doctor_assessment.py`: binary validation, category, and overall-gate tests.
- `skills/creating-model-doctor-reports/scripts/model_doctor_html.py`: binary customer report renderer.
- `skills/creating-model-doctor-reports/assets/report.css`: remove conditional/unknown presentation rules.
- `tests/test_model_doctor_html.py`: binary labels, complete evidence, CSP, and escaping tests.
- `skills/creating-model-doctor-reports/SKILL.md`: new-log-only evidence review workflow.
- `skills/creating-model-doctor-reports/references/evaluation-rules.md`: binary semantic rules.
- `tests/test_model_doctor_skill.py`: Skill structure and forbidden-language tests.
- `tests/test_model_doctor_end_to_end.py`: five-protocol collector-to-HTML contract.
- `README.md`: version 0.7.0 breaking workflow.
- `docs/superpowers/specs/2026-07-17-adaptive-concurrency-stress-test-design.md`: mark the old mixed-responsibility stress design as superseded.
- `docs/superpowers/plans/2026-07-17-adaptive-concurrency-stress-test.md`: mark the old implementation plan as non-executable.

## Task 1: Restore Test Infrastructure and Define the New Collector Contract

**Files:**
- Restore: `tests/__init__.py`
- Restore: `tests/helpers/fake_model_curl.py`
- Restore: `tests/test_model_capability_doctor_script.py`
- Modify: `tests/test_model_capability_doctor_script.py`

- [ ] **Step 1: Restore the maintained pre-removal test files without restoring legacy compatibility tests**

Run:

```bash
git checkout 4ee380f^ -- \
  tests/__init__.py \
  tests/helpers/fake_model_curl.py \
  tests/test_model_capability_doctor_script.py \
  tests/test_model_doctor_assessment.py \
  tests/test_model_doctor_html.py \
  tests/test_model_doctor_log.py \
  tests/test_model_doctor_skill.py \
  tests/fixtures/model-doctor/minimal.log \
  tests/fixtures/model-doctor/mixed.log
```

Do not restore `test_model_doctor_legacy.py` or `legacy-v0.1.log`.

- [ ] **Step 2: Add failing tests for the evidence-only log surface**

Add collector tests with these assertions:

```python
FORBIDDEN_JUDGMENT_FIELDS = (
    "result:",
    "expected:",
    "detected:",
    "conclusion:",
    "pass:",
    "fail:",
    "unsupported:",
    "undetermined:",
    "skipped:",
    "error:",
)

def test_collector_writes_evidence_schema_without_judgments(self):
    result, log = self.run_fixture("basic", "004")
    self.assertEqual(result.returncode, 0, result.stderr)
    self.assertIn("script_version: 0.7.0", log)
    self.assertIn("log_schema: llm-capability-doctor.evidence.v1", log)
    self.assertIn("request_refs: test-004", log)
    for forbidden in FORBIDDEN_JUDGMENT_FIELDS:
        self.assertNotIn(forbidden, log)

def test_shell_source_has_no_test_grader(self):
    source = SCRIPT.read_text(encoding="utf-8")
    for forbidden in (
        "record_test()",
        "PASS_COUNT",
        "FAIL_COUNT",
        "UNSUPPORTED_COUNT",
        "UNDETERMINED_COUNT",
        "SKIPPED_COUNT",
        "ERROR_COUNT",
    ):
        self.assertNotIn(forbidden, source)
```

- [ ] **Step 3: Add failing tests for explicit shared evidence references**

Add this manifest-only helper so collector tests do not depend on the parser
upgrade in Task 5:

```python
def manifest_request_refs(log: str, test_id: str) -> list[str]:
    match = re.search(
        rf"^========== TEST-{test_id} BEGIN ==========$"
        rf"(.*?)"
        rf"^========== TEST-{test_id} END ==========$",
        log,
        flags=re.MULTILINE | re.DOTALL,
    )
    if not match:
        raise AssertionError(f"Missing test manifest {test_id}")
    refs = re.search(r"^request_refs: ?(.*)$", match.group(1), flags=re.MULTILINE)
    if not refs or not refs.group(1):
        return []
    return refs.group(1).split(",")
```

Then add:

```python
def test_shared_probe_and_repeat_requests_are_explicitly_referenced(self):
    _, log = self.run_fixture("basic", "002,003,007,055,056")
    protocol_refs = manifest_request_refs(log, "002")
    auth_refs = manifest_request_refs(log, "003")
    usage_refs = manifest_request_refs(log, "007")
    repeat_refs = manifest_request_refs(log, "055")
    percentile_refs = manifest_request_refs(log, "056")
    self.assertTrue(protocol_refs)
    self.assertEqual(
        auth_refs,
        usage_refs,
    )
    self.assertEqual(
        repeat_refs,
        percentile_refs,
    )
    self.assertEqual(len(percentile_refs), 5)
```

- [ ] **Step 4: Run the focused tests and verify RED**

Run:

```bash
python3 -m unittest \
  tests.test_model_capability_doctor_script.ModelCapabilityDoctorScriptTests.test_collector_writes_evidence_schema_without_judgments \
  tests.test_model_capability_doctor_script.ModelCapabilityDoctorScriptTests.test_shell_source_has_no_test_grader \
  tests.test_model_capability_doctor_script.ModelCapabilityDoctorScriptTests.test_shared_probe_and_repeat_requests_are_explicitly_referenced \
  -v
```

Expected: failures showing script version `0.6.1`, emitted `result:` fields, and missing request references for `002`, `003`, `007`, or `056`.

- [ ] **Step 5: Commit the failing contract tests**

```bash
git add tests
git commit -m "test: define evidence-only collector contract"
```

## Task 2: Replace Shell Verdicts with Evidence Manifests

**Files:**
- Modify: `model-capability-doctor.sh`
- Modify: `tests/helpers/fake_model_curl.py`
- Modify: `tests/test_model_capability_doctor_script.py`

- [ ] **Step 1: Add the evidence schema header and manifest writer**

Change constants to:

```bash
SCRIPT_VERSION="0.7.0"
LOG_SCHEMA="llm-capability-doctor.evidence.v1"
```

Add to `write_log_header`:

```bash
echo "log_schema: $LOG_SCHEMA"
echo "selected_test_count: $SELECTED_TEST_COUNT"
```

Replace `record_test` with:

```bash
record_test_manifest() {
  local id="$1" category="$2" name="$3" request_refs="${4:-}"
  TEST_MANIFEST_COUNT=$((TEST_MANIFEST_COUNT + 1))
  {
    echo "========== TEST-${id} BEGIN =========="
    echo "name: $name"
    echo "category: $category"
    echo "completed_at: $(timestamp)"
    echo "request_refs: $request_refs"
    echo "========== TEST-${id} END =========="
    echo
  } >>"$LOG_FILE"
}

append_request_ref() {
  local current="$1" request_id="$2"
  if [[ -n "$current" ]]; then
    printf '%s,%s' "$current" "$request_id"
  else
    printf '%s' "$request_id"
  fi
}
```

Make `perform_request` set `LAST_REQUEST_ID="$request_id"` after writing the request audit.

- [ ] **Step 2: Make protocol probes expose factual request references**

Initialize:

```bash
PROTOCOL_PROBE_REQUEST_REFS=""
SELECTED_PROTOCOL_REQUEST_ID=""
```

In `detect_protocol`, append every probe ID immediately after `perform_request`:

```bash
PROTOCOL_PROBE_REQUEST_REFS="$(append_request_ref "$PROTOCOL_PROBE_REQUEST_REFS" "$LAST_REQUEST_ID")"
```

When an envelope is selected, set:

```bash
SELECTED_PROTOCOL_REQUEST_ID="$LAST_REQUEST_ID"
```

`run_protocol_test` records `PROTOCOL_PROBE_REQUEST_REFS`. Tests `003` and `007` record `SELECTED_PROTOCOL_REQUEST_ID` when present, otherwise all probe references.

- [ ] **Step 3: Remove semantic graders from interface, structured, context, text, and thinking handlers**

Each handler must only construct requests, call `perform_request`, collect IDs, and call `record_test_manifest`.

For one-request tests use:

```bash
perform_request "$body" 0 "test-${id}" "$DETECTED_AUTH_MODE"
record_test_manifest "$id" "$category" "$name" "$LAST_REQUEST_ID"
```

For Thinking test `033`, always issue both low and high requests and record:

```bash
request_refs="$(append_request_ref "$request_refs" "$LAST_REQUEST_ID")"
```

Delete grading-only functions once no request builder uses them:

```text
normalize_json_text
input_token_count
extract_visible_text
trim_text
compact_text
semantic_text
write_visible_evidence
json_envelope_complete
visible_answer_extractable
thinking_metadata_type
thinking_stream_event_present
stream_termination_present
stream_normal_finish_present
stream_abnormal_finish_present
stream_completed_normally
stream_content_event_present
write_stream_visible_evidence
is_explicit_guardrail_response
```

Keep JSON extraction helpers that are used solely to build protocol-correlated follow-up requests.

- [ ] **Step 4: Replace the run summary with collection facts**

Write only:

```bash
echo "completed_at: $completed_at"
echo "duration_seconds: $duration_seconds"
echo "request_count: $REQUEST_COUNT"
echo "test_manifest_count: $TEST_MANIFEST_COUNT"
```

Terminal output reports duration, total curl requests, manifest count, and log path. Remove all status counters and initialize only `TEST_MANIFEST_COUNT=0`.

- [ ] **Step 5: Fix bounded CLI and signal behavior while touching collection control flow**

Before every value-taking option, require a following argument:

```bash
require_option_value() {
  local option="$1" remaining="$2"
  if (( remaining < 2 )); then
    echo "Missing value for ${option}" >&2
    exit 2
  fi
}
```

Use `require_option_value "$1" "$#"` before reading `$2`.

Replace the signal trap with:

```bash
trap cleanup EXIT
trap 'cleanup; trap - EXIT; exit 130' INT
trap 'cleanup; trap - EXIT; exit 143' TERM
```

- [ ] **Step 6: Run the focused tests and verify GREEN**

Run:

```bash
bash -n model-capability-doctor.sh
python3 -m unittest tests.test_model_capability_doctor_script -v
```

Expected: collector tests pass; old assertions about script results are removed or rewritten as evidence assertions.

- [ ] **Step 7: Commit the collector-boundary change**

```bash
git add model-capability-doctor.sh tests/helpers/fake_model_curl.py tests/test_model_capability_doctor_script.py
git commit -m "refactor: make model doctor evidence only"
```

## Task 3: Complete Five-Protocol Tool Evidence Chains

**Files:**
- Modify: `model-capability-doctor.sh`
- Modify: `tests/helpers/fake_model_curl.py`
- Modify: `tests/test_model_capability_doctor_script.py`

- [ ] **Step 1: Add failing protocol-matrix tests for `047-049`**

Parameterize fixtures for `openai_chat`, `openai_responses`, `anthropic_messages`, `gemini_generate_content`, and `ollama_chat`:

```python
def test_tool_followups_round_trip_observed_assistant_turns_for_all_protocols(self):
    for protocol in SUPPORTED_PROTOCOLS:
        with self.subTest(protocol=protocol):
            _, _, parsed = self.run_fixture(
                f"{protocol}_tool_chain", "047,048,049", include_parsed=True
            )
            for test_id in ("047", "048", "049"):
                refs = parsed["tests"][test_id]["requestRefs"]
                self.assertEqual(len(refs), 2)
                first = parsed["requests"][refs[0]]
                follow = parsed["requests"][refs[1]]
                self.assert_followup_round_trips(protocol, first, follow, test_id)
```

`assert_followup_round_trips` must verify the unique fixture sentinel embedded in the actual assistant call is present unchanged in the follow-up request. It must also verify the correlated call ID for every protocol that defines one and Ollama's tool name.

- [ ] **Step 2: Verify RED**

Run:

```bash
python3 -m unittest \
  tests.test_model_capability_doctor_script.ModelCapabilityDoctorScriptTests.test_tool_followups_round_trip_observed_assistant_turns_for_all_protocols \
  -v
```

Expected: OpenAI Chat fails because it synthesizes assistant history; Anthropic, Gemini, and Ollama fail because they have no follow-up request.

- [ ] **Step 3: Add a raw JSON member extractor for orchestration**

Implement `json_raw_member FILE KEY MODE` as a portable awk scanner that:

- tracks quoted strings and escape sequences;
- finds a JSON object key followed by `:`;
- returns the complete raw scalar, object, or array value with balanced nesting;
- supports `first` and `last` selection; and
- exits nonzero on incomplete JSON.

Cover it through actual protocol follow-up tests rather than exposing it as a public API.

- [ ] **Step 4: Expand initial tool catalogs for sequential tests**

Include `get_time` in test `047`'s initial and follow-up tool arrays. Keep the same tool array across Anthropic and Gemini turns as required by their message-history contracts.

- [ ] **Step 5: Build follow-ups from observed protocol objects**

Change `core_tool_followup_body` to accept:

```bash
core_tool_followup_body ID FIRST_RESPONSE_FILE CALL_ID RESPONSE_ID TOOL_NAME
```

Use raw observed members:

```bash
assistant_message="$(json_raw_member "$first_file" message first)"
assistant_content="$(json_raw_member "$first_file" content first)"
candidate_content="$(json_raw_member "$first_file" content first)"
```

Construct protocol bodies exactly as specified in the design:

- OpenAI Chat: original user message, exact assistant message, correlated tool message.
- OpenAI Responses: `previous_response_id` plus `function_call_output`.
- Anthropic: original user message, exact assistant content array, user `tool_result`.
- Gemini: original user content, exact model candidate content, user `functionResponse`.
- Ollama: original user message, exact assistant message, `role=tool` with `tool_name`.

For test `049`, set Anthropic `is_error:true`; other protocols receive an explicit timeout error object or string according to their request contract.

- [ ] **Step 6: Record available evidence without grading missing correlations**

Always record the first request ID. Append the follow-up ID only when the body was safely constructed:

```bash
request_refs="$LAST_REQUEST_ID"
if follow_body="$(core_tool_followup_body ...)"; then
  perform_request "$follow_body" 0 "test-${id}-follow" "$DETECTED_AUTH_MODE"
  request_refs="$(append_request_ref "$request_refs" "$LAST_REQUEST_ID")"
fi
record_test_manifest "$id" "$category" "$name" "$request_refs"
```

Do not emit a status or explanation when the follow-up cannot be built.

- [ ] **Step 7: Verify GREEN and commit**

Run:

```bash
bash -n model-capability-doctor.sh
python3 -m unittest tests.test_model_capability_doctor_script -v
```

Commit:

```bash
git add model-capability-doctor.sh tests/helpers/fake_model_curl.py tests/test_model_capability_doctor_script.py
git commit -m "feat: collect complete tool chains for five protocols"
```

## Task 4: Remove Performance and Guardrail Judgments from Bash

**Files:**
- Modify: `model-capability-doctor.sh`
- Modify: `tests/helpers/fake_model_curl.py`
- Modify: `tests/test_model_capability_doctor_script.py`

- [ ] **Step 1: Write failing tests for raw performance and guardrail collection**

Add:

```python
def test_performance_manifests_reference_raw_requests_without_aggregates(self):
    _, log, parsed = self.run_fixture(
        "wrong_visible_markers", "051,052,053,054,055,056,057,058", include_parsed=True
    )
    self.assertEqual(len(parsed["tests"]["055"]["requestRefs"]), 5)
    self.assertEqual(parsed["tests"]["055"]["requestRefs"], parsed["tests"]["056"]["requestRefs"])
    self.assertEqual(len(parsed["tests"]["057"]["requestRefs"]), 60)
    self.assertEqual(len(parsed["tests"]["058"]["requestRefs"]), 11)
    for forbidden in ("semantic_success=", "p50_ms=", "p95_ms=", "recovery=PASS"):
        self.assertNotIn(forbidden, log)

def test_guardrail_responses_are_recorded_without_shell_interpretation(self):
    _, log, parsed = self.run_fixture(
        "unsafe_and_refused_outputs", "059,060,061,062", include_parsed=True
    )
    self.assertEqual(len(parsed["tests"]["061"]["requestRefs"]), 2)
    self.assertEqual(len(parsed["tests"]["062"]["requestRefs"]), 2)
    self.assertNotIn("越权破坏请求被明确拦截", log)
    self.assertNotIn("疑似过度拒答", log)
```

- [ ] **Step 2: Verify RED**

Run the two new tests and confirm current aggregate and conclusion text causes failure.

- [ ] **Step 3: Reduce repeated collection to request-reference accumulation**

`ensure_core_repeat_samples` performs exactly five requests and sets:

```bash
CORE_REPEAT_REQUEST_REFS=""
```

It does not extract visible text, count successes, sort timings, or write a combined evidence file. Tests `055` and `056` both record `CORE_REPEAT_REQUEST_REFS`.

- [ ] **Step 4: Reduce concurrency collection to curl audit rendering**

`run_parallel_batch` keeps the synchronized gate and curl workers. After each worker completes it only:

- reads curl's factual metrics to render the request audit;
- increments `REQUEST_COUNT`;
- appends the request ID to `BATCH_REQUEST_REFS`.

Delete semantic marker extraction, success counts, rate-limit counts, percentile files, and batch conclusions. Test `057` concatenates the four batch reference lists in execution order and records one manifest.

- [ ] **Step 5: Reduce recovery and guardrail handlers to request scheduling**

Test `058` always performs ten load requests and one recovery request, recording all eleven IDs. Tests `059-060` each perform one request. Tests `061-062` each perform control then experiment and record both IDs. No handler reads response bodies.

- [ ] **Step 6: Verify GREEN and commit**

Run:

```bash
bash -n model-capability-doctor.sh
python3 -m unittest tests.test_model_capability_doctor_script -v
```

Commit:

```bash
git add model-capability-doctor.sh tests/helpers/fake_model_curl.py tests/test_model_capability_doctor_script.py
git commit -m "refactor: keep performance and guardrails as raw evidence"
```

## Task 5: Replace the Parser with the Strict Evidence-v1 Contract

**Files:**
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_log.py`
- Replace: `tests/fixtures/model-doctor/minimal.log`
- Replace: `tests/fixtures/model-doctor/mixed.log`
- Modify: `tests/test_model_doctor_log.py`

- [ ] **Step 1: Write new-format fixtures**

Make `minimal.log` contain one evidence-v1 run, one complete request, one `TEST-001` manifest with `request_refs: test-001`, and a factual run summary.

Make `mixed.log` contain tests `047` and `056`, two tool-chain requests, five shared repeat requests, and explicit manifest references.

Neither fixture may contain a script verdict or status count.

- [ ] **Step 2: Write failing strict-schema tests**

Add:

```python
def test_parser_requires_evidence_v1(self):
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "old.log"
        path.write_text("========== MODEL DOCTOR RUN ==========\nscript_version: 0.6.1\n", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "log_schema"):
            parse_log(path)

def test_parser_uses_only_explicit_request_refs(self):
    parsed = parse_log(FIXTURES / "mixed.log")
    self.assertEqual(parsed["tests"]["047"]["requestRefs"], ["test-047", "test-047-follow"])
    self.assertEqual(len(parsed["tests"]["056"]["requestRefs"]), 5)
    for test in parsed["tests"].values():
        self.assertNotIn("result", test)
        self.assertNotIn("rawResponse", test)
```

Add rejection tests for a manifest reference that names a missing request and for duplicate test/request blocks.

- [ ] **Step 3: Verify RED**

Run:

```bash
python3 -m unittest tests.test_model_doctor_log -v
```

Expected: old parser accepts a missing schema or derives references from request-ID prefixes.

- [ ] **Step 4: Implement strict evidence-v1 parsing**

Set:

```python
PARSED_SCHEMA_VERSION = "llm-capability-doctor.parsed-evidence.v1"
EVIDENCE_LOG_SCHEMA = "llm-capability-doctor.evidence.v1"
```

At parse start require:

```python
if run.get("log_schema") != EVIDENCE_LOG_SCHEMA:
    raise ValueError(f"Unsupported or missing log_schema: {run.get('log_schema')!r}")
```

Parse `request_refs` as an ordered comma-separated list. Reject empty elements, duplicate refs, missing request IDs, duplicate request blocks, duplicate test blocks, and declared count mismatches. Delete `_request_test_id`, `_link_exact_response_requests`, legacy warnings, and version catalog branches.

- [ ] **Step 5: Verify GREEN and commit**

Run:

```bash
python3 -m unittest tests.test_model_doctor_log -v
python3 -m py_compile skills/creating-model-doctor-reports/scripts/*.py
```

Commit:

```bash
git add skills/creating-model-doctor-reports/scripts/model_doctor_log.py tests/fixtures/model-doctor tests/test_model_doctor_log.py
git commit -m "refactor: parse only evidence v1 logs"
```

## Task 6: Make Assessment v3 Strictly Binary

**Files:**
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py`
- Modify: `skills/creating-model-doctor-reports/references/assessment-schema.json`
- Modify: `tests/test_model_doctor_assessment.py`

- [ ] **Step 1: Write failing binary validation tests**

Add:

```python
def test_only_pass_and_fail_are_valid_review_statuses(self):
    for status in ("UNSUPPORTED", "UNDETERMINED", "SKIPPED", "ERROR"):
        with self.subTest(status=status):
            review = valid_review(status=status)
            self.assertTrue(validate_reviews(self.parsed, {"001": review}))

def test_category_and_overall_are_binary(self):
    assessment = assemble_assessment(
        parsed_with_tests("001", "051"),
        {
            "001": valid_review(status="PASS", gate="critical"),
            "051": valid_review(status="FAIL", gate="observation"),
        },
    )
    self.assertEqual({item["status"] for item in assessment["categories"]}, {"PASS", "FAIL"})
    self.assertEqual(assessment["overall"]["verdict"], "READY")
    self.assertNotIn("conditions", assessment["overall"])
```

Add a second overall test proving an important `FAIL` yields `BLOCKED`.

- [ ] **Step 2: Verify RED**

Run assessment tests and confirm non-binary statuses still validate or `CONDITIONAL` is produced.

- [ ] **Step 3: Implement assessment v3**

Set:

```python
ASSESSMENT_SCHEMA_VERSION = "llm-capability-doctor.assessment.v3"
STATUSES = {"PASS", "FAIL"}
```

Use one current 62-item gate map; remove version-specific historical maps.

`_status_counts` returns only `PASS` and `FAIL`. `_category_status` returns `PASS` only when all items pass. `_overall` returns:

```python
blockers = [
    summary(item)
    for item in items
    if item["gateLevel"] in {"critical", "important"}
    and item["reviewedStatus"] == "FAIL"
]
return {
    "verdict": "BLOCKED" if blockers else "READY",
    "counts": _status_counts(items),
    "blockers": blockers,
}
```

Categories contain `failures` with all failed test IDs and no `unknowns`.

- [ ] **Step 4: Replace the JSON schema contract**

Require assessment v3, allow only binary reviewed/category statuses and `READY/BLOCKED`, require `blockers`, and remove `conditions` from required/properties.

- [ ] **Step 5: Verify GREEN and commit**

Run:

```bash
python3 -m unittest tests.test_model_doctor_assessment -v
```

Commit:

```bash
git add skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py skills/creating-model-doctor-reports/references/assessment-schema.json tests/test_model_doctor_assessment.py
git commit -m "feat: make model doctor assessment binary"
```

## Task 7: Render a Binary Customer Report

**Files:**
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_html.py`
- Modify: `skills/creating-model-doctor-reports/assets/report.css`
- Modify: `skills/creating-model-doctor-reports/assets/report.js`
- Modify: `tests/test_model_doctor_html.py`

- [ ] **Step 1: Write failing forbidden-label and evidence tests**

Add:

```python
FORBIDDEN_REPORT_TEXT = (
    "需复测",
    "待补证",
    "无法判定",
    "不支持",
    "跳过",
    "执行错误",
    "CONDITIONAL",
    "UNDETERMINED",
    "UNSUPPORTED",
    "SKIPPED",
    "ERROR",
)

def test_report_is_binary_and_contains_manifest_requests(self):
    html = render_html(binary_assessment_fixture())
    self.assertIn("通过", html)
    self.assertIn("未通过", html)
    for forbidden in FORBIDDEN_REPORT_TEXT:
        self.assertNotIn(forbidden, html)
    self.assertIn("test-047-follow", html)
```

- [ ] **Step 2: Verify RED**

Run HTML tests and confirm conditional labels/status maps cause failure.

- [ ] **Step 3: Simplify status and category rendering**

Use:

```python
STATUS_LABELS = {"PASS": "通过", "FAIL": "未通过"}
CATEGORY_STATUS_LABELS = STATUS_LABELS
```

Category key data is `N/T 通过` plus `未通过：<ids>` when failures exist. Remove unknown handling and conditional defaults. Overall presentation supports only `READY` and `BLOCKED`.

- [ ] **Step 4: Remove dead conditional CSS and JavaScript filters**

Delete selectors and option logic for `UNDETERMINED`, `UNSUPPORTED`, `SKIPPED`, `ERROR`, and `CONDITIONAL`. Preserve responsive layout, CSP, escaping, expand/collapse, and print behavior.

- [ ] **Step 5: Verify GREEN and commit**

Run:

```bash
python3 -m unittest tests.test_model_doctor_html -v
```

Commit:

```bash
git add skills/creating-model-doctor-reports/scripts/model_doctor_html.py skills/creating-model-doctor-reports/assets/report.css skills/creating-model-doctor-reports/assets/report.js tests/test_model_doctor_html.py
git commit -m "feat: render binary model doctor reports"
```

## Task 8: Rewrite the Skill Workflow and Evaluation Rules

**Files:**
- Modify: `skills/creating-model-doctor-reports/SKILL.md`
- Modify: `skills/creating-model-doctor-reports/references/evaluation-rules.md`
- Modify: `tests/test_model_doctor_skill.py`

- [ ] **Step 1: Write failing Skill contract tests**

Require Skill text to contain `llm-capability-doctor.evidence.v1`, `PASS`, and `FAIL`, and reject all old status names, legacy compatibility language, script-verdict comparison language, and retest-status instructions.

- [ ] **Step 2: Verify RED**

Run:

```bash
python3 -m unittest tests.test_model_doctor_skill -v
```

- [ ] **Step 3: Rewrite the Skill workflow**

The Skill must:

1. reject logs without evidence-v1;
2. parse and inventory manifest completeness;
3. inspect only referenced requests;
4. author one binary review per manifest;
5. use `FAIL` for unsupported, transport, missing, malformed, ambiguous, or incomplete evidence;
6. validate assessment v3;
7. render JSON and HTML without overwriting existing outputs; and
8. verify source hash, credential redaction, offline HTML, binary status vocabulary, and evidence completeness.

- [ ] **Step 4: Rewrite evaluation rules**

Keep category-specific semantic criteria, but replace the status section and overall gates with the binary contract. Remove historical catalog mappings and old-log instructions. State that rerun guidance explains a `FAIL`; it does not defer the verdict.

- [ ] **Step 5: Verify GREEN and commit**

Run:

```bash
python3 -m unittest tests.test_model_doctor_skill -v
```

Commit:

```bash
git add skills/creating-model-doctor-reports/SKILL.md skills/creating-model-doctor-reports/references/evaluation-rules.md tests/test_model_doctor_skill.py
git commit -m "docs: define binary evidence review workflow"
```

## Task 9: Update Product Documentation and Supersede the Mixed Stress Plan

**Files:**
- Modify: `README.md`
- Modify: `docs/superpowers/specs/2026-07-17-adaptive-concurrency-stress-test-design.md`
- Modify: `docs/superpowers/plans/2026-07-17-adaptive-concurrency-stress-test.md`
- Modify: `tests/test_model_capability_doctor_script.py`

- [ ] **Step 1: Write failing documentation assertions**

Assert README contains `0.7.0`, `llm-capability-doctor.evidence.v1`, and the statement that Shell records evidence while the Skill is the sole binary evaluator. Assert it contains no instruction to interpret script results or rerun old logs.

- [ ] **Step 2: Verify RED**

Run the documentation-focused collector tests.

- [ ] **Step 3: Rewrite README workflow and breaking-change section**

Document:

- one script execution produces an evidence log, not a diagnostic verdict;
- one Skill execution produces assessment v3 and the HTML report;
- old logs are unsupported and must be recollected with v0.7.0;
- final test results are binary;
- credentials should use `MODEL_API_KEY`;
- `--only` remains available for collecting a subset.

- [ ] **Step 4: Mark the old adaptive stress documents as superseded**

Add a top-level notice to both documents:

```markdown
> Superseded: this document assigns semantic validation, aggregation,
> recommendation, and status to the shell collector. Do not execute this plan.
> A replacement must follow the evidence-only boundary in
> `2026-07-17-evidence-only-collector-design.md`.
```

Do not implement the old stress plan as part of this change.

- [ ] **Step 5: Verify GREEN and commit**

Run the documentation tests, then commit:

```bash
git add README.md docs/superpowers/specs/2026-07-17-adaptive-concurrency-stress-test-design.md docs/superpowers/plans/2026-07-17-adaptive-concurrency-stress-test.md tests/test_model_capability_doctor_script.py
git commit -m "docs: publish evidence-only breaking workflow"
```

## Task 10: Add Five-Protocol End-to-End Verification

**Files:**
- Create: `tests/test_model_doctor_end_to_end.py`
- Modify: `tests/helpers/fake_model_curl.py`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_report.py`

- [ ] **Step 1: Write the failing end-to-end test**

For each supported protocol:

1. run all 62 tests with the fake curl;
2. parse the evidence-v1 log;
3. assert 62 manifests and no unresolved references;
4. build deterministic reviews with only `PASS` and `FAIL`;
5. validate and assemble assessment v3;
6. render HTML;
7. assert every manifest-referenced request appears in the corresponding assessment test; and
8. scan log, JSON, and HTML for forbidden fields appropriate to each layer.

Use this shape:

```python
def test_all_protocols_complete_evidence_to_binary_report(self):
    for protocol in SUPPORTED_PROTOCOLS:
        with self.subTest(protocol=protocol):
            artifacts = self.run_pipeline(protocol)
            self.assertEqual(len(artifacts.parsed["tests"]), 62)
            self.assertEqual(
                set(artifacts.assessment["overall"]["counts"]),
                {"PASS", "FAIL"},
            )
            self.assertNotIn("CONDITIONAL", artifacts.html)
```

- [ ] **Step 2: Verify RED**

Run:

```bash
python3 -m unittest tests.test_model_doctor_end_to_end -v
```

Expected: missing protocol fixtures or incomplete references fail.

- [ ] **Step 3: Complete fixture and CLI integration gaps**

Make `model_doctor_report.py` consistently enforce evidence-v1 and assessment-v3 through `parse`, `summary`, `packet`, `validate`, and `render`. Complete fake responses for all request types and protocols without adding production grading logic.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
python3 -m unittest tests.test_model_doctor_end_to_end -v
```

- [ ] **Step 5: Commit end-to-end coverage**

```bash
git add tests/test_model_doctor_end_to_end.py tests/helpers/fake_model_curl.py skills/creating-model-doctor-reports/scripts/model_doctor_report.py
git commit -m "test: verify evidence-only pipeline end to end"
```

## Task 11: Full Verification and Completion Audit

**Files:**
- Verify all files changed by Tasks 1-10.

- [ ] **Step 1: Run static checks**

```bash
bash -n model-capability-doctor.sh
python3 -m py_compile skills/creating-model-doctor-reports/scripts/*.py tests/*.py tests/helpers/*.py
git diff --check
```

Expected: all commands exit 0.

- [ ] **Step 2: Run the full test suite**

```bash
python3 -m unittest discover -s tests -v
```

Expected: every test passes with zero failures and zero errors.

- [ ] **Step 3: Scan production artifacts for forbidden collector judgments**

```bash
rg -n 'record_test|PASS_COUNT|FAIL_COUNT|UNSUPPORTED_COUNT|UNDETERMINED_COUNT|SKIPPED_COUNT|ERROR_COUNT|echo "result:|echo "expected:|echo "detected:|echo "conclusion:' model-capability-doctor.sh
```

Expected: no matches.

- [ ] **Step 4: Scan Skill production code for non-binary states and legacy support**

```bash
rg -n 'UNSUPPORTED|UNDETERMINED|SKIPPED|ERROR|CONDITIONAL|legacy|compatib' \
  skills/creating-model-doctor-reports
```

Expected: no status or compatibility logic matches. Natural-language evidence excerpts in test fixtures are outside this production scan.

- [ ] **Step 5: Run one full fake collection and validate its manifest closure**

Run the end-to-end helper for one protocol and verify:

```text
script_version: 0.7.0
log_schema: llm-capability-doctor.evidence.v1
selected_test_count: 62
test_manifest_count: 62
```

Every `request_refs` ID must resolve to exactly one request block.

- [ ] **Step 6: Review the actual diff against every design requirement**

Use:

```bash
git diff 27fc706 --stat
git diff 27fc706 -- model-capability-doctor.sh README.md skills tests docs/superpowers
```

Confirm the shell is evidence-only, Skill verdicts are binary, old logs are rejected, all five tool protocols collect complete chains, and existing user logs/reports are untouched.

- [ ] **Step 7: Commit any verification-only corrections**

If verification required corrections, commit only those corrections with:

```bash
git add <corrected-files>
git commit -m "fix: close evidence-only verification gaps"
```
