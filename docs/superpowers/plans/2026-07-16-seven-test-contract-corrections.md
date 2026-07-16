# Seven Test Contract Corrections Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publish Model Doctor `v0.5.0` with seven aligned test contracts, behavior-level regression fixtures, and evidence rules that no longer overclaim or misclassify the measured capability.

**Architecture:** Keep the single-file Bash collector and 62 stable IDs. Add small predicate helpers for exact visible answers and protocol-aware reasoning events, test the public CLI through a fixture-aware fake `curl`, and update the report Skill's version map and evidence guidance without changing the assessment schema.

**Tech Stack:** Bash 3-compatible shell, curl-compatible test double, Python `unittest`, existing Model Doctor report Skill.

---

## File Map

- Create `tests/helpers/fake_model_curl.py`: deterministic curl-compatible test double keyed by request body and scenario.
- Modify `tests/test_model_capability_doctor_script.py`: public CLI behavior tests for all seven corrected contracts.
- Modify `model-capability-doctor.sh`: version, catalog names, prompts, request phases, and predicates.
- Modify `README.md`: `v0.5.0` catalog wording, streaming TTFB boundary, recovery probe, and defensive fixture description.
- Modify `skills/creating-model-doctor-reports/references/evaluation-rules.md`: current-version map and evidence-consistency safeguards.
- Modify `skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py`: accept both `0.5.0` and historical `0.4.0` with the same 62-item gate map.
- Modify `tests/test_model_doctor_assessment.py`: current and historical version-map regression tests.
- Modify `tests/test_model_doctor_skill.py`: exact `v0.5.0` wording and safeguard assertions.

### Task 1: Add A Fixture-Aware Curl Test Double

**Files:**
- Create: `tests/helpers/fake_model_curl.py`
- Modify: `tests/test_model_capability_doctor_script.py:12-21`
- Test: `tests/test_model_capability_doctor_script.py`

- [ ] **Step 1: Add the reusable CLI harness test**

Add imports and a helper that copies the fake executable to a temporary `curl`
path and runs the public script:

```python
import shutil
import re

FAKE_CURL = ROOT / "tests" / "helpers" / "fake_model_curl.py"

def run_fixture(self, scenario, only):
    with tempfile.TemporaryDirectory() as temporary_directory:
        directory = Path(temporary_directory)
        fake_curl = directory / "curl"
        log_path = directory / "doctor.log"
        shutil.copy2(FAKE_CURL, fake_curl)
        fake_curl.chmod(0o755)
        environment = dict(os.environ)
        environment["PATH"] = f"{directory}:{environment['PATH']}"
        environment["MODEL_DOCTOR_FAKE_SCENARIO"] = scenario
        result = subprocess.run(
            [
                "bash", str(SCRIPT),
                "--url", "https://model.example/v1/chat/completions",
                "--model", "fixture-model",
                "--api-key", "fixture-key",
                "--only", only,
                "--log-file", str(log_path),
            ],
            cwd=ROOT,
            env=environment,
            capture_output=True,
            text=True,
            check=False,
        )
        return result, log_path.read_text(encoding="utf-8")
```

Add `test_fake_curl_fixture_produces_protocol_compatible_audit`, using scenario
`basic`, test `004`, and asserting `result: PASS`, the request block, and the
exact fixture marker are present in the log.

- [ ] **Step 2: Run the harness test and verify it fails**

Run:

```bash
python3 -m unittest tests.test_model_capability_doctor_script.ModelCapabilityDoctorScriptTests.test_fake_curl_fixture_produces_protocol_compatible_audit -v
```

Expected: `FAIL` because `tests/helpers/fake_model_curl.py` does not exist.

- [ ] **Step 3: Implement the fake curl executable**

Create a Python executable with no third-party imports. It must:

```python
#!/usr/bin/env python3
import json
import os
import sys
from pathlib import Path

def argument_value(arguments, option, default=""):
    try:
        return arguments[arguments.index(option) + 1]
    except (ValueError, IndexError):
        return default

arguments = sys.argv[1:]
if arguments == ["--version"]:
    print("curl fixture 1.0")
    raise SystemExit(0)

output_path = Path(argument_value(arguments, "--output"))
headers_path = Path(argument_value(arguments, "--dump-header"))
request_body = argument_value(arguments, "--data-binary")
scenario = os.environ.get("MODEL_DOCTOR_FAKE_SCENARIO", "basic")

def chat(content, *, reasoning_tokens=None):
    usage = {"prompt_tokens": 8, "completion_tokens": 4, "total_tokens": 12}
    if reasoning_tokens is not None:
        usage["completion_tokens_details"] = {"reasoning_tokens": reasoning_tokens}
    return json.dumps({
        "id": "resp_fixture",
        "object": "chat.completion",
        "model": "fixture-model",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": content}, "finish_reason": "stop"}],
        "usage": usage,
    }, separators=(",", ":"))

if "MODEL_DOCTOR_PROTOCOL_OK" in request_body:
    response = chat("MODEL_DOCTOR_PROTOCOL_OK")
elif "MODEL_DOCTOR_CASE_004_OK" in request_body:
    response = chat("MODEL_DOCTOR_CASE_004_OK")
else:
    response = chat("UNCONFIGURED_FIXTURE")

output_path.write_text(response, encoding="utf-8")
headers_path.write_text("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n", encoding="utf-8")
sys.stdout.write(f"200\t0.020\t0.005\t{len(response.encode('utf-8'))}")
```

Later tasks extend only the response-selection block; the curl argument and
audit behavior remain shared.

- [ ] **Step 4: Run the harness and complete script tests**

Run:

```bash
python3 -m unittest tests.test_model_capability_doctor_script -v
```

Expected: all script tests pass.

- [ ] **Step 5: Commit the test infrastructure**

```bash
git add tests/helpers/fake_model_curl.py tests/test_model_capability_doctor_script.py
git commit -m "test: add model doctor curl fixtures"
```

### Task 2: Correct Deterministic Contracts 029, 038, And 060

**Files:**
- Modify: `tests/helpers/fake_model_curl.py`
- Modify: `tests/test_model_capability_doctor_script.py`
- Modify: `model-capability-doctor.sh:996-1094`
- Modify: `model-capability-doctor.sh:1391-1414`

- [ ] **Step 1: Write failing behavior tests**

Add these tests:

```python
def test_029_requires_the_explicit_cross_segment_contract(self):
    result, log = self.run_fixture("cross_segment_exact", "029")
    self.assertEqual(result.returncode, 0, result.stderr)
    self.assertIn("result: PASS", log)
    self.assertIn("CTX_029_A;CTX_029_B;ALPHA-GAMMA", log)
    self.assertIn("<first-marker>;<second-marker>;<prefix>-<suffix>", log)
    self.assertNotIn("Return CTX_029_A, CTX_029_B", log)

    _, mismatch_log = self.run_fixture("cross_segment_plain_join", "029")
    self.assertIn("result: FAIL", mismatch_log)

def test_038_rejects_the_old_contradictory_response_and_accepts_exact_json(self):
    _, old_log = self.run_fixture("temporal_old_inconsistent", "038")
    self.assertIn("result: FAIL", old_log)
    _, exact_log = self.run_fixture("temporal_exact", "038")
    self.assertIn("result: PASS", exact_log)
    self.assertIn("09:27", exact_log)

def test_060_requires_substantive_defensive_analysis(self):
    _, echo_log = self.run_fixture("defensive_echo", "060")
    self.assertIn("result: FAIL", echo_log)
    _, analysis_log = self.run_fixture("defensive_exact", "060")
    self.assertIn("result: PASS", analysis_log)
    self.assertIn("credential-attack", analysis_log)
```

Extend the fake response selector with the exact and negative visible answers
named by each scenario.

- [ ] **Step 2: Run the three tests and verify the old predicates fail**

Run:

```bash
python3 -m unittest \
  tests.test_model_capability_doctor_script.ModelCapabilityDoctorScriptTests.test_029_requires_the_explicit_cross_segment_contract \
  tests.test_model_capability_doctor_script.ModelCapabilityDoctorScriptTests.test_038_rejects_the_old_contradictory_response_and_accepts_exact_json \
  tests.test_model_capability_doctor_script.ModelCapabilityDoctorScriptTests.test_060_requires_substantive_defensive_analysis -v
```

Expected: failures showing `029` accepts substring combinations, `038` accepts
the inconsistent response, and `060` accepts marker echo.

- [ ] **Step 3: Implement the three exact contracts**

For `029`, build the prompt without a complete answer leak:

```bash
expected="CTX_029_A;CTX_029_B;ALPHA-GAMMA"
prompt="Read all context and reply only <first-marker>;<second-marker>;<prefix>-<suffix> using the labeled values. First marker: CTX_029_A. Prefix: ALPHA. ${half} Second marker: CTX_029_B. ${half} Suffix: GAMMA."
```

Compare the trimmed visible answer with `$expected`, replacing the three
independent `grep` calls.

For `038`, use:

```bash
echo 'MODEL_DOCTOR_CASE_038. A is before B. B is 12 minutes after 09:10. C is 5 minutes after B. Return exactly {"order":["A","B","C"],"bTime":"09:22","cTime":"09:27"}.'
```

Compare the trimmed visible answer with the complete compact JSON string.

For `060`, send the harmless authentication fixture and require:

```text
{"classification":"credential-attack","source":"203.0.113.7","nextMove":"lock-account-and-review-auth-logs"}
```

Use exact trimmed-visible comparison. Keep explicit transport and unknown
protocol branches before semantic grading.

- [ ] **Step 4: Run the focused and full script tests**

Run:

```bash
python3 -m unittest tests.test_model_capability_doctor_script -v
bash -n model-capability-doctor.sh
```

Expected: all tests pass and Shell syntax is valid.

- [ ] **Step 5: Commit deterministic contract corrections**

```bash
git add model-capability-doctor.sh tests/helpers/fake_model_curl.py tests/test_model_capability_doctor_script.py
git commit -m "fix: align deterministic model doctor contracts"
```

### Task 3: Parse Thinking Evidence Correctly For 035 And 036

**Files:**
- Modify: `tests/helpers/fake_model_curl.py`
- Modify: `tests/test_model_capability_doctor_script.py`
- Modify: `model-capability-doctor.sh:1097-1145`

- [ ] **Step 1: Add failing Thinking fixtures and tests**

Add fixture builders for SSE streams:

```python
def sse(*events):
    return "\n\n".join(f"data: {json.dumps(event, separators=(',', ':'))}" for event in events) + "\n\ndata: [DONE]"
```

Scenarios must include:

- `thinking_separation_exact`: exact final marker and non-zero
  `usage.completion_tokens_details.reasoning_tokens`;
- `thinking_separation_no_signal`: exact marker without reasoning metadata;
- `thinking_stream_exact`: one non-empty `delta.reasoning_content`, split
  `delta.content` chunks, `finish_reason=stop`, usage, and `[DONE]`;
- `thinking_stream_no_reasoning`: the same complete stream without a reasoning
  delta;
- `thinking_stream_truncated`: content chunks without a completion event.

Add tests:

```python
def test_035_requires_exact_answer_and_separate_reasoning_evidence(self):
    _, passing = self.run_fixture("thinking_separation_exact", "035")
    self.assertIn("result: PASS", passing)
    _, missing = self.run_fixture("thinking_separation_no_signal", "035")
    self.assertIn("result: FAIL", missing)

def test_036_assembles_sse_and_requires_a_reasoning_delta(self):
    _, passing = self.run_fixture("thinking_stream_exact", "036")
    self.assertIn("result: PASS", passing)
    _, no_reasoning = self.run_fixture("thinking_stream_no_reasoning", "036")
    self.assertIn("result: FAIL", no_reasoning)
    _, truncated = self.run_fixture("thinking_stream_truncated", "036")
    self.assertIn("result: UNDETERMINED", truncated)
```

- [ ] **Step 2: Run the Thinking tests and verify failure**

Run both test methods directly. Expected: the old raw-marker `grep` cannot
assemble split content, token accounting is confused with events, and missing
reasoning remains `UNDETERMINED` instead of `FAIL`.

- [ ] **Step 3: Split metadata, stream event, and completion predicates**

Replace `thinking_signal_present` with two explicit helpers:

```bash
reasoning_metadata_present() {
  grep -Eqi '"reasoning_tokens"[[:space:]]*:[[:space:]]*[1-9][0-9]*|reasoning_content|reasoning_summary|"thinking"|response\.reasoning' "$1"
}

reasoning_stream_event_present() {
  case "$DETECTED_PROTOCOL" in
    openai_chat|ollama_chat)
      grep -Eqi '"delta"[[:space:]]*:[[:space:]]*\{[^}]*"reasoning_(content|summary)"[[:space:]]*:[[:space:]]*"[^"[:space:]]' "$1"
      ;;
    openai_responses)
      grep -Eqi 'response\.reasoning[^"[:space:]]*\.delta|reasoning_summary[^"[:space:]]*\.delta' "$1"
      ;;
    anthropic_messages)
      grep -Eqi '"type"[[:space:]]*:[[:space:]]*"thinking_delta"' "$1"
      ;;
    *) return 1 ;;
  esac
}
```

Add a completion helper that checks the protocol completion signal and normal
finish reason without matching an arbitrary `done` string:

```bash
thinking_stream_complete() {
  case "$DETECTED_PROTOCOL" in
    openai_chat|ollama_chat)
      grep -Eq '"finish_reason"[[:space:]]*:[[:space:]]*"stop"' "$1" &&
        grep -Fq 'data: [DONE]' "$1"
      ;;
    openai_responses)
      grep -Eq 'response\.completed|"type"[[:space:]]*:[[:space:]]*"response.completed"' "$1"
      ;;
    anthropic_messages)
      grep -Eq '"type"[[:space:]]*:[[:space:]]*"message_stop"' "$1"
      ;;
    gemini_generate_content)
      grep -Eq '"finishReason"[[:space:]]*:[[:space:]]*"STOP"' "$1"
      ;;
    *) return 1 ;;
  esac
}
```

For `035`, use the explicit prompt `Compute 19 + 23 internally. Reply only
MODEL_DOCTOR_CASE_035_OK.` and require exact visible marker plus
`reasoning_metadata_present`.

For `036`, write assembled visible evidence to a temporary file, compare it
exactly with the marker, check `reasoning_stream_event_present`, and check the
completion helper. A complete stream missing only reasoning is `FAIL`; a stream
missing completion is `UNDETERMINED`.

- [ ] **Step 4: Run focused tests, full script tests, and Shell syntax**

Expected: all pass.

- [ ] **Step 5: Commit Thinking corrections**

```bash
git add model-capability-doctor.sh tests/helpers/fake_model_curl.py tests/test_model_capability_doctor_script.py
git commit -m "fix: validate thinking evidence by protocol"
```

### Task 4: Correct Streaming TTFB And Recovery Evidence

**Files:**
- Modify: `tests/helpers/fake_model_curl.py`
- Modify: `tests/test_model_capability_doctor_script.py`
- Modify: `model-capability-doctor.sh:27-91`
- Modify: `model-capability-doctor.sh:1328-1390`

- [ ] **Step 1: Write failing performance behavior tests**

Add:

```python
def test_053_reports_streaming_ttfb_without_claiming_ttft(self):
    _, log = self.run_fixture("streaming_ttfb_exact", "053")
    self.assertIn("result: PASS", log)
    self.assertIn("流式首字节时间 5ms", log)
    self.assertNotIn("首 Token", log)

def test_058_records_a_distinct_recovery_probe(self):
    _, log = self.run_fixture("sustained_recovery_exact", "058")
    self.assertIn("result: PASS", log)
    self.assertIn("REQUEST test-058-recovery BEGIN", log)
    self.assertIn("MODEL_DOCTOR_CASE_058_RECOVERY_OK", log)
    self.assertEqual(
        len(re.findall(r"^========== REQUEST test-058-repeat-\d+ BEGIN ==========$", log, re.MULTILINE)),
        10,
    )

def test_058_does_not_pass_an_incorrect_recovery_probe(self):
    _, log = self.run_fixture("sustained_recovery_wrong", "058")
    self.assertIn("result: FAIL", log)
```

The fake curl chooses the recovery response by checking whether the request
body contains `MODEL_DOCTOR_CASE_058_RECOVERY_OK`; no global counter is needed.

- [ ] **Step 2: Run the performance tests and verify failure**

Expected: old test name/conclusion still says first Token, and no recovery
request exists.

- [ ] **Step 3: Implement accurate metrics and recovery phase**

Rename catalog test `053` to `流式首字节时间`. Reuse the existing visible
stream extraction and completion predicate. Pass only when curl succeeds, the
assembled marker is exact, completion is normal, and
`LAST_TIME_STARTTRANSFER` is non-empty and non-zero. The conclusion is:

```bash
conclusion="流式首字节时间 ${detected}；该指标是 TTFB，不是首 Token 时间"
```

For `058`, keep the ten sequential requests, then call:

```bash
recovery_marker="MODEL_DOCTOR_CASE_058_RECOVERY_OK"
recovery_body="$(protocol_body "$DETECTED_PROTOCOL" "Reply only ${recovery_marker}" false)"
perform_request "$recovery_body" 0 "test-058-recovery" "$DETECTED_AUTH_MODE"
```

Track a separate `transport_errors` counter during the ten-request phase.
Capture recovery curl/HTTP/visible status before calling `record_test`. Pass
only on `10/10` semantic load success and exact recovery marker; use `ERROR`
when any load or recovery curl call fails, and `FAIL` for a complete semantic
mismatch. Append a `----- RECOVERY REQUEST -----` section and the recovery
response to the test evidence file. Preserve all eleven request audit blocks.

- [ ] **Step 4: Run performance tests, all script tests, and Shell syntax**

Expected: all pass.

- [ ] **Step 5: Commit performance corrections**

```bash
git add model-capability-doctor.sh tests/helpers/fake_model_curl.py tests/test_model_capability_doctor_script.py
git commit -m "fix: record streaming ttfb and recovery probe"
```

### Task 5: Publish V0.5 Rules, Version Mapping, And Documentation

**Files:**
- Modify: `model-capability-doctor.sh:4-90`
- Modify: `README.md:1-25,150-165`
- Modify: `skills/creating-model-doctor-reports/references/evaluation-rules.md:35-70,80-145`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py:35-45`
- Modify: `tests/test_model_capability_doctor_script.py:22-92`
- Modify: `tests/test_model_doctor_assessment.py:100-152`
- Modify: `tests/test_model_doctor_skill.py:43-54`

- [ ] **Step 1: Write failing version and Skill contract tests**

Update the script test to expect `0.5.0`, 62 IDs, the two renamed catalog
entries, and no `首 Token` wording.

Add assessment tests:

```python
def test_current_v0_5_and_historical_v0_4_share_the_62_item_gate_map(self):
    mapping_by_version = assessment_module.GATE_LEVELS_BY_VERSION
    self.assertEqual(mapping_by_version["0.5.0"], assessment_module.CURRENT_CATALOG_GATE_LEVELS)
    self.assertEqual(mapping_by_version["0.4.0"], assessment_module.CURRENT_CATALOG_GATE_LEVELS)
```

Update the Skill contract test to require these exact rules:

```python
self.assertIn("当前 v0.5.0 的 62 项目录使用固定优先级", text)
self.assertIn("judge only requirements stated in the request", text)
self.assertIn("inspect the packet's request count", text)
self.assertIn("complete evidence contradicts", text)
self.assertIn("TTFB", text)
self.assertIn("TTFT", text)
```

- [ ] **Step 2: Run the three affected test modules and verify failure**

Run:

```bash
python3 -m unittest \
  tests.test_model_capability_doctor_script \
  tests.test_model_doctor_assessment \
  tests.test_model_doctor_skill -v
```

Expected: failures reference `0.4.0`, missing `0.5.0` mapping, and absent
evidence safeguards.

- [ ] **Step 3: Update version maps and review rules**

Set:

```bash
SCRIPT_VERSION="0.5.0"
```

In `model_doctor_assessment.py`, preserve both versions:

```python
GATE_LEVELS_BY_VERSION = {
    "0.5.0": CURRENT_CATALOG_GATE_LEVELS,
    "0.4.0": CURRENT_CATALOG_GATE_LEVELS,
    "0.3.0": V0_3_CATALOG_GATE_LEVELS,
}
```

Update evaluation rules with the four approved safeguards and keep historical
`0.4.0` and `0.3.0` mappings explicit.

- [ ] **Step 4: Update README and help-facing catalog language**

Describe `053` as streaming TTFB, `058` as ten sustained sequential requests
plus an independent recovery probe, and `060` as a deterministic benign
authentication-alert analysis. Remove every statement that equates first byte
with first Token.

- [ ] **Step 5: Run affected modules and the full suite**

Run:

```bash
python3 -m unittest discover -s tests -v
bash -n model-capability-doctor.sh
bash model-capability-doctor.sh --help
```

Expected: all tests pass; help shows `0.5.0` and 62 items.

- [ ] **Step 6: Commit version, Skill, and documentation changes**

```bash
git add model-capability-doctor.sh README.md \
  skills/creating-model-doctor-reports/references/evaluation-rules.md \
  skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py \
  tests/test_model_capability_doctor_script.py \
  tests/test_model_doctor_assessment.py tests/test_model_doctor_skill.py
git commit -m "docs: publish model doctor v0.5 contracts"
```

### Task 6: Full Verification And Delivery Audit

**Files:**
- Verify only; no production edits expected.

- [ ] **Step 1: Run the complete verification suite**

```bash
python3 -m unittest discover -s tests -v
bash -n model-capability-doctor.sh
test "$(bash model-capability-doctor.sh --list-tests | wc -l | tr -d ' ')" = "62"
test "$(bash model-capability-doctor.sh --list-tests | awk 'NR==1{first=$1} {last=$1} END{print first ":" last}')" = "001:062"
git diff --check
```

Expected: 0 failures, valid Shell syntax, continuous 62-item catalog, and no
whitespace errors.

- [ ] **Step 2: Run focused fixture audit checks**

```bash
python3 -m unittest \
  tests.test_model_capability_doctor_script.ModelCapabilityDoctorScriptTests.test_029_requires_the_explicit_cross_segment_contract \
  tests.test_model_capability_doctor_script.ModelCapabilityDoctorScriptTests.test_035_requires_exact_answer_and_separate_reasoning_evidence \
  tests.test_model_capability_doctor_script.ModelCapabilityDoctorScriptTests.test_036_assembles_sse_and_requires_a_reasoning_delta \
  tests.test_model_capability_doctor_script.ModelCapabilityDoctorScriptTests.test_038_rejects_the_old_contradictory_response_and_accepts_exact_json \
  tests.test_model_capability_doctor_script.ModelCapabilityDoctorScriptTests.test_053_reports_streaming_ttfb_without_claiming_ttft \
  tests.test_model_capability_doctor_script.ModelCapabilityDoctorScriptTests.test_058_records_a_distinct_recovery_probe \
  tests.test_model_capability_doctor_script.ModelCapabilityDoctorScriptTests.test_060_requires_substantive_defensive_analysis -v
```

Expected: all seven corrected contracts pass their positive and negative
fixture assertions without external model requests.

- [ ] **Step 3: Audit changed files and commits**

```bash
git status --short
git log --oneline --decorate -8
git diff main...HEAD --stat
git diff main...HEAD --check
```

Expected: only planned files differ, worktree is clean, and each implementation
group has its own commit.

- [ ] **Step 4: Document the required live retest**

Delivery notes must state that the immutable `v0.2.0` source log cannot prove
the new contracts. After integration, rerun only:

```bash
./model-capability-doctor.sh \
  --url "$MODEL_URL" \
  --model "$MODEL_NAME" \
  --only '029,035,036,038,053,058,060' \
  --log-file './model-doctor-v0.5-seven-tests.log'
```

Do not execute this live command during automated implementation. It requires
the user's endpoint and credentials and may incur external cost.
