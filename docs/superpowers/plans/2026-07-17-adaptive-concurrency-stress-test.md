# Adaptive Concurrency Stress Test Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace test `057`'s one-wave concurrency snapshot with an opt-in sustained stress test that recommends the lowest concurrency on the highest eligible output-token-throughput plateau.

**Architecture:** Keep the deployable collector as one Bash script: script-local functions build the standardized workload, run closed-loop workers, validate samples, reduce level metrics, enforce safety stops, and choose refinement levels. Extend the existing Python report pipeline to parse a versioned stress-summary block, carry it unchanged into the assessment, validate cross-field consistency, and render a compact customer-facing performance table.

**Tech Stack:** Bash 3-compatible shell, curl timing metrics, awk/sed, Python 3 standard library, unittest, self-contained HTML/CSS

---

## File Map

- Modify `model-capability-doctor.sh`: publish contract `0.7.0`, add stress CLI options, standardized workload, sustained scheduler, sample validation, reducers, safety stops, refinement, and summary output.
- Create `tests/__init__.py`, `tests/helpers/fake_model_curl.py`, and `tests/test_model_capability_doctor_script.py`: restore focused collector coverage without restoring unrelated removed tests.
- Modify `skills/creating-model-doctor-reports/scripts/model_doctor_log.py` and create `tests/test_model_doctor_log.py`: parse and validate stress evidence.
- Modify `skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py`, `references/assessment-schema.json`, and create `tests/test_model_doctor_assessment.py`: carry stress data into the formal artifact.
- Modify `skills/creating-model-doctor-reports/scripts/model_doctor_html.py`, `assets/report.css`, and create `tests/test_model_doctor_html.py`: render the recommendation and level table.
- Modify `skills/creating-model-doctor-reports/references/evaluation-rules.md`, `SKILL.md`, and `README.md`: document collection and review semantics.

## Task 1: Lock the v0.7.0 CLI and Opt-In Contract

**Files:**
- Create: `tests/__init__.py`
- Create: `tests/helpers/fake_model_curl.py`
- Create: `tests/test_model_capability_doctor_script.py`
- Modify: `model-capability-doctor.sh:4-24,80-87,1790-1867`

- [ ] **Step 1: Create the focused fake curl**

Create an empty `tests/__init__.py`. Create `tests/helpers/fake_model_curl.py` with the complete initial fixture:

```python
#!/usr/bin/env python3
import json
import os
import re
import sys
from pathlib import Path


def argument_value(arguments, option, default=""):
    try:
        return arguments[arguments.index(option) + 1]
    except (ValueError, IndexError):
        return default


def chat(content, input_tokens=1024, output_tokens=256):
    return json.dumps(
        {
            "id": "stress-fixture",
            "object": "chat.completion",
            "model": "fixture-model",
            "choices": [{"message": {"role": "assistant", "content": content}, "finish_reason": "stop"}],
            "usage": {
                "prompt_tokens": input_tokens,
                "completion_tokens": output_tokens,
                "total_tokens": input_tokens + output_tokens,
            },
        },
        separators=(",", ":"),
    )


arguments = sys.argv[1:]
if arguments == ["--version"]:
    print("curl fixture 1.0")
    raise SystemExit(0)

output_path = Path(argument_value(arguments, "--output"))
headers_path = Path(argument_value(arguments, "--dump-header"))
request_body = argument_value(arguments, "--data-binary")
response = chat("MODEL_DOCTOR_PROTOCOL_OK")
http_status = "200"
time_total = "0.020"
time_starttransfer = "0.005"
marker = re.search(r"(MODEL_DOCTOR_STRESS_[A-Za-z0-9_]+_OK)", request_body)
if marker:
    words = " ".join(f"word{index:03d}" for index in range(1, 257))
    response = chat(f"{words} {marker.group(1)}")

output_path.write_text(response, encoding="utf-8")
headers_path.write_text(f"HTTP/1.1 {http_status} Fixture\r\ncontent-type: application/json\r\n\r\n", encoding="utf-8")
sys.stdout.write(f"{http_status}\t{time_total}\t{time_starttransfer}\t{len(response.encode('utf-8'))}")
```

- [ ] **Step 2: Write failing CLI tests**

Create `tests/test_model_capability_doctor_script.py`:

```python
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "model-capability-doctor.sh"
FAKE_CURL = ROOT / "tests" / "helpers" / "fake_model_curl.py"


class AdaptiveStressScriptTests(unittest.TestCase):
    def run_script(self, *arguments, scenario="healthy"):
        with tempfile.TemporaryDirectory() as temporary_directory:
            directory = Path(temporary_directory)
            fake_curl = directory / "curl"
            log_path = directory / "doctor.log"
            shutil.copy2(FAKE_CURL, fake_curl)
            fake_curl.chmod(0o755)
            environment = dict(os.environ)
            environment.update({
                "PATH": f"{directory}:{environment['PATH']}",
                "MODEL_DOCTOR_FAKE_SCENARIO": scenario,
                "MODEL_DOCTOR_STRESS_TEST_MODE": "1",
                "MODEL_DOCTOR_STRESS_TEST_SAMPLES_PER_WORKER": "2",
            })
            result = subprocess.run(
                ["bash", str(SCRIPT), *arguments, "--log-file", str(log_path)],
                cwd=ROOT, env=environment, capture_output=True, text=True, check=False,
            )
            log = log_path.read_text(encoding="utf-8") if log_path.exists() else ""
            return result, log

    def base_arguments(self):
        return [
            "--url", "https://model.example/v1/chat/completions",
            "--model", "fixture-model", "--api-key", "fixture-key", "--only", "057",
        ]

    def test_help_and_catalog_publish_v070_stress_contract(self):
        help_result = subprocess.run(["bash", str(SCRIPT), "--help"], capture_output=True, text=True)
        list_result = subprocess.run(["bash", str(SCRIPT), "--list-tests"], capture_output=True, text=True)
        self.assertIn("Model Capability Doctor 0.7.0", help_result.stdout)
        self.assertIn("--stress-mode", help_result.stdout)
        self.assertIn("--stress-max-concurrency", help_result.stdout)
        self.assertIn("057\t性能与稳定性\t自适应并发压测", list_result.stdout)

    def test_057_is_skipped_without_explicit_standard_mode(self):
        result, log = self.run_script(*self.base_arguments())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("result: SKIPPED", log)
        self.assertIn("--stress-mode standard", log)
        self.assertNotIn("MODEL DOCTOR STRESS 057 BEGIN", log)

    def test_stress_max_concurrency_rejects_invalid_values(self):
        for value in ("0", "65", "not-a-number"):
            result, _ = self.run_script(
                *self.base_arguments(), "--stress-mode", "standard",
                "--stress-max-concurrency", value,
            )
            self.assertEqual(result.returncode, 2)
            self.assertIn("--stress-max-concurrency must be an integer from 1 to 64", result.stderr)
```

- [ ] **Step 3: Run tests and verify RED**

Run `python3 -m unittest tests.test_model_capability_doctor_script -v`.

Expected: failures identify version `0.6.1`, missing options, old test name, and the old automatic concurrency run.

- [ ] **Step 4: Implement the CLI contract**

In `model-capability-doctor.sh`, set:

```bash
SCRIPT_VERSION="0.7.0"
STRESS_MODE="off"
STRESS_MAX_CONCURRENCY=64
```

Rename catalog row `057` to `自适应并发压测`. Add help entries for `--stress-mode MODE` (`off|standard`) and `--stress-max-concurrency COUNT` (`1-64`). Add parser branches:

```bash
--stress-mode) STRESS_MODE="${2:-}"; shift 2 ;;
--stress-max-concurrency) STRESS_MAX_CONCURRENCY="${2:-}"; shift 2 ;;
```

Validate:

```bash
if [[ "$STRESS_MODE" != "off" && "$STRESS_MODE" != "standard" ]]; then
  echo "--stress-mode must be off or standard" >&2
  exit 2
fi
if [[ ! "$STRESS_MAX_CONCURRENCY" =~ ^[0-9]+$ ]] \
  || (( STRESS_MAX_CONCURRENCY < 1 || STRESS_MAX_CONCURRENCY > 64 )); then
  echo "--stress-max-concurrency must be an integer from 1 to 64" >&2
  exit 2
fi
```

At the start of test `057`, preserve protocol error handling, then record `SKIPPED` with a concrete rerun command when mode is `off`.

- [ ] **Step 5: Verify GREEN and commit**

Run:

```bash
python3 -m unittest tests.test_model_capability_doctor_script -v
bash -n model-capability-doctor.sh
```

Expected: three tests pass and Bash syntax exits `0`.

Commit:

```bash
git add model-capability-doctor.sh tests
git commit -m "test: define adaptive stress CLI contract"
```

## Task 2: Build the Workload and Sustained Scheduler

**Files:**
- Modify: `tests/helpers/fake_model_curl.py`
- Modify: `tests/test_model_capability_doctor_script.py`
- Modify: `model-capability-doctor.sh:607-760,1560-1740`

- [ ] **Step 1: Write failing scheduler tests**

Add `import re` and:

```python
    def run_standard(self, maximum="8", scenario="healthy"):
        return self.run_script(
            *self.base_arguments(), "--stress-mode", "standard",
            "--stress-max-concurrency", maximum, scenario=scenario,
        )

    def test_standard_run_uses_coarse_levels_and_multiple_samples(self):
        result, log = self.run_standard("8")
        self.assertEqual(result.returncode, 0, result.stderr)
        for concurrency in (1, 2, 4, 8):
            self.assertIn(f"level_kind: coarse\nconcurrency: {concurrency}", log)
            self.assertIn(f"test-057-c{concurrency}-measure-w1-s2", log)

    def test_workload_is_unique_large_and_nonce_bound(self):
        _, log = self.run_standard("2")
        bodies = [
            item.split("----- REQUEST BODY END -----", 1)[0]
            for item in log.split("----- REQUEST BODY BEGIN -----")[1:]
            if "MODEL_DOCTOR_STRESS_" in item
        ]
        self.assertTrue(all(len(body) >= 4096 for body in bodies))
        markers = [re.search(r"MODEL_DOCTOR_STRESS_[A-Za-z0-9_]+_OK", body).group(0) for body in bodies]
        self.assertEqual(len(markers), len(set(markers)))

    def test_non_power_of_two_maximum_is_measured(self):
        _, log = self.run_standard("6")
        for concurrency in (1, 2, 4, 6):
            self.assertIn(f"concurrency: {concurrency}", log)
        self.assertNotIn("concurrency: 8", log)
```

- [ ] **Step 2: Run scheduler tests and verify RED**

Run `python3 -m unittest tests.test_model_capability_doctor_script -v`.

Expected: failures because sustained workers and stress request IDs do not exist.

- [ ] **Step 3: Add workload and level helpers**

Add after `generate_filler`:

```bash
output_token_count() {
  grep -Eo '"(completion_tokens|output_tokens|candidatesTokenCount|eval_count)"[[:space:]]*:[[:space:]]*[0-9]+' "$1" 2>/dev/null \
    | tail -n 1 | grep -Eo '[0-9]+$' || true
}

stress_levels() {
  local maximum="$1" level=1
  while (( level <= maximum )); do printf '%s\n' "$level"; level=$((level * 2)); done
  (( maximum > 1 && maximum != level / 2 )) && printf '%s\n' "$maximum"
}

stress_prompt() {
  local nonce="$1" marker="MODEL_DOCTOR_STRESS_${1}_OK"
  printf 'Nonce %s. Write 256 English words and finish with %s. Payload: %s' \
    "$nonce" "$marker" "$(generate_filler 4096)"
}
```

- [ ] **Step 4: Implement closed-loop workers**

Add production timings and test-only controls:

```bash
STRESS_WARMUP_SECONDS=5
STRESS_MEASURE_SECONDS=30
STRESS_BASELINE_MIN_SECONDS=45
STRESS_BASELINE_MAX_SECONDS=90
STRESS_BASELINE_TARGET_SAMPLES=15
STRESS_TEST_MODE="${MODEL_DOCTOR_STRESS_TEST_MODE:-0}"
STRESS_TEST_SAMPLES_PER_WORKER="${MODEL_DOCTOR_STRESS_TEST_SAMPLES_PER_WORKER:-0}"
```

Implement `stress_curl_sample` by reusing `parallel_curl_worker`'s auth/curl fields and writing `.body`, `.headers`, `.stderr`, `.metrics`, `.exit`, `.started`, `.completed`, and `.request-body`.

Implement the worker loop exactly around a shared gate and stop file:

```bash
stress_worker() {
  local concurrency="$1" phase="$2" worker="$3" gate_file="$4" stop_file="$5"
  local sequence=0 now=0 cutoff=0 nonce="" marker="" body="" prefix=""
  while [[ ! -f "$gate_file" ]]; do sleep 0.01; done
  cutoff="$(cat "$gate_file")"
  while [[ ! -f "$stop_file" ]]; do
    now="$(date +%s)"
    [[ "$STRESS_TEST_MODE" != "1" && "$now" -ge "$cutoff" ]] && break
    [[ "$STRESS_TEST_MODE" == "1" && "$sequence" -ge "$STRESS_TEST_SAMPLES_PER_WORKER" ]] && break
    sequence=$((sequence + 1))
    nonce="C${concurrency}_${phase}_W${worker}_S${sequence}"
    marker="MODEL_DOCTOR_STRESS_${nonce}_OK"
    body="$(protocol_body "$DETECTED_PROTOCOL" "$(stress_prompt "$nonce")" false)"
    prefix="$RUN_TMP_DIR/test-057-c${concurrency}-${phase}-w${worker}-s${sequence}"
    stress_curl_sample "$body" "$prefix" "$DETECTED_AUTH_MODE"
    printf '%s\t%s\t%s\t%s\t%s\n' "$concurrency" "$phase" "$worker" "$sequence" "$marker" >"${prefix}.sample"
  done
  : >"$RUN_TMP_DIR/test-057-c${concurrency}-${phase}-w${worker}.done"
}
```

`run_stress_phase` creates a gate containing the cutoff epoch, launches exactly `concurrency` workers, waits for every PID, then renders and redacts audits in lexical request-ID order. Warm-up samples include `sample_phase: warmup` metadata but are excluded later.

- [ ] **Step 5: Implement the ascending coarse scheduler**

Add `run_stress_level` and:

```bash
run_adaptive_stress_test() {
  local id="$1" concurrency=""
  STRESS_LEVEL_RECORDS="$RUN_TMP_DIR/test-${id}.stress-levels"
  STRESS_SUMMARY_FILE="$RUN_TMP_DIR/test-${id}.stress-summary"
  : >"$STRESS_LEVEL_RECORDS"
  for concurrency in $(stress_levels "$STRESS_MAX_CONCURRENCY"); do
    run_stress_level "$concurrency" "coarse"
  done
  STRESS_STATUS="UNDETERMINED"
  STRESS_CONCLUSION="持续压测样本已采集，等待指标归并"
  STRESS_DETECTED="levels=$(stress_levels "$STRESS_MAX_CONCURRENCY" | paste -sd, -)"
}
```

Concurrency `1` uses two fully drained warm-up completions and measures until 45 seconds plus 15 completions or the 90-second hard limit. Other levels use 5 seconds warm-up and a 30-second measured phase. Test mode uses the fixed sample cap and no clock wait.

In this task, case `057` records the explicit `UNDETERMINED` scheduler result above after all request audits. Task 3 replaces that result with the complete reducer and recommendation; no undefined reducer is called in this intermediate commit.

- [ ] **Step 6: Verify GREEN and commit**

Run:

```bash
python3 -m unittest tests.test_model_capability_doctor_script -v
bash -n model-capability-doctor.sh
```

Expected: all scheduler tests pass in seconds.

Commit:

```bash
git add model-capability-doctor.sh tests/helpers/fake_model_curl.py tests/test_model_capability_doctor_script.py
git commit -m "feat: run sustained adaptive stress levels"
```

## Task 3: Reduce Samples, Stop Safely, Refine, and Recommend

**Files:**
- Modify: `tests/helpers/fake_model_curl.py`
- Modify: `tests/test_model_capability_doctor_script.py`
- Modify: `model-capability-doctor.sh:99-114,607-760,1608-1725`

- [ ] **Step 1: Add deterministic fake profiles**

Extract concurrency from the nonce and use:

```python
scenario = os.environ.get("MODEL_DOCTOR_FAKE_SCENARIO", "healthy")
concurrency_match = re.search(r"_C([0-9]+)_", marker.group(1)) if marker else None
concurrency = int(concurrency_match.group(1)) if concurrency_match else 1
profiles = {
    "healthy": {1: ("1.000", 100), 2: ("1.050", 190), 3: ("1.100", 285), 4: ("1.100", 300), 6: ("1.250", 303), 8: ("1.600", 250)},
    "plateau": {1: ("1.000", 100), 2: ("1.000", 195), 3: ("1.000", 292), 4: ("1.000", 300), 6: ("1.000", 303), 8: ("1.000", 250)},
}
time_total, output_tokens = profiles.get(scenario, profiles["healthy"]).get(concurrency, ("5.000", 128))
```

Add `overload_at_8`, `missing_usage_all`, and `missing_usage_mixed` branches. Overload samples return HTTP 429 or transport timeout; missing-usage responses omit `completion_tokens`.

- [ ] **Step 2: Write failing reducer tests**

Add:

```python
    def test_level_summary_reports_rates_tokens_and_percentiles(self):
        _, log = self.run_standard("4", "healthy")
        self.assertRegex(log, r"level: concurrency=4,kind=coarse,attempted=8,successful=8,success_rate=1\.0000,.*p95_ms=1100.*eligible=1")

    def test_plateau_chooses_lower_concurrency(self):
        _, log = self.run_standard("8", "plateau")
        self.assertIn("recommended_concurrency: 4", log)
        self.assertIn("score_type: output_tps", log)

    def test_refinement_measures_midpoints(self):
        _, log = self.run_standard("8", "healthy")
        self.assertIn("level_kind: refinement\nconcurrency: 3", log)
        self.assertIn("level_kind: refinement\nconcurrency: 6", log)

    def test_safety_stop_prevents_higher_levels(self):
        _, log = self.run_standard("64", "overload_at_8")
        self.assertIn("severe_stop_reason: error_rate", log)
        self.assertNotIn("level_kind: coarse\nconcurrency: 16", log)

    def test_usage_fallback_requires_consistent_availability(self):
        _, fallback = self.run_standard("4", "missing_usage_all")
        self.assertIn("score_type: successful_rps", fallback)
        _, mixed = self.run_standard("4", "missing_usage_mixed")
        self.assertIn("recommendation_status: UNDETERMINED", mixed)
        self.assertIn("recommendation_reason: mixed_score_availability", mixed)
```

- [ ] **Step 3: Run tests and verify RED**

Run `python3 -m unittest tests.test_model_capability_doctor_script -v`.

Expected: failures because level reduction, refinement, and recommendation fields are absent.

- [ ] **Step 4: Implement sample validation and reducers**

`validate_stress_sample` must require curl success, HTTP 2xx, normal protocol completion, the nonce marker as final visible token, 200-320 preceding whitespace words, numeric positive `time_total`, and extractable visible text. It writes fixed TSV columns:

```text
request_id phase curl_exit http_status semantic_success failure_reason latency_ms input_tokens output_tokens rate_limited http_5xx timed_out transport_error
```

`reduce_stress_level` includes every measured attempt, sorts only successful latencies, calculates nearest-rank P50/P95/P99/max, uses elapsed synchronized-start-to-final-completion time for RPS/TPS, and writes four-decimal rates. Output TPS is `not_available` unless every semantic success has output usage. In test mode only, derive elapsed time as the maximum per-worker sum of reported `time_total`; production always uses the actual synchronized wall-clock interval.

Eligibility is:

```bash
eligible=1
awk -v success="$success_rate" 'BEGIN { exit !(success >= 0.99) }' || eligible=0
(( attempted < 100 && successful != attempted )) && eligible=0
(( p95_ms > STRESS_LATENCY_LIMIT_MS )) && eligible=0
(( missing_success_latency > 0 )) && eligible=0
```

- [ ] **Step 5: Implement severe-stop monitoring**

Workers append one completion class to an append-only level file. A monitor creates the shared stop file after three consecutive `timeout` lines. After reduction, set a severe stop when success is below 90%, combined error rate is at least 10%, or P95 exceeds four times baseline. Break before launching the next higher coarse level.

- [ ] **Step 6: Implement refinement and recommendation**

Add:

```bash
stress_midpoints() {
  local lower="$1" best="$2" upper="$3"
  (( lower > 0 && best - lower > 1 )) && echo $(((lower + best) / 2))
  (( upper > best && upper - best > 1 )) && echo $(((best + upper) / 2))
}

within_three_percent() {
  awk -v candidate="$1" -v maximum="$2" 'BEGIN { exit !(candidate >= maximum * 0.97) }'
}
```

Run distinct midpoints around the best coarse level. Choose the lowest eligible concurrency within 3% of maximum output TPS. If all eligible levels lack usage, use successful RPS. Mixed availability is `UNDETERMINED`. Build the stable interval as the maximal adjacent measured sequence containing the recommendation whose scores remain at least 97% of maximum.

- [ ] **Step 7: Emit the versioned summary and test result**

Write:

```text
========== MODEL DOCTOR STRESS 057 BEGIN ==========
schema_version: model-doctor.stress.v1
stress_mode: standard
workload_input_characters: 4096
workload_visible_word_min: 200
workload_visible_word_max: 320
max_concurrency: 64
baseline_p95_ms: 1000
latency_limit_ms: 2000
score_type: output_tps
level: concurrency=1,kind=coarse,attempted=2,successful=2,success_rate=1.0000,rps=1.8182,output_tps=465.4545,p50_ms=1000,p95_ms=1000,p99_ms=1000,max_ms=1000,rate_limited=0,http_5xx=0,timeouts=0,transport_errors=0,eligible=1
recommended_concurrency: 4
stable_interval: 4-6
recommendation_status: PASS
recommendation_reason: measured_plateau
severe_stop_reason: none
========== MODEL DOCTOR STRESS 057 END ==========
```

Test `057` is `PASS` with an eligible recommendation above `1`, `FAIL` when confirmed overload leaves only `1`, `UNDETERMINED` for insufficient/mixed evidence, and `ERROR` for runner failure.

- [ ] **Step 8: Verify GREEN and commit**

Run:

```bash
python3 -m unittest tests.test_model_capability_doctor_script -v
bash -n model-capability-doctor.sh
```

Expected: all collector tests pass.

Commit:

```bash
git add model-capability-doctor.sh tests/helpers/fake_model_curl.py tests/test_model_capability_doctor_script.py
git commit -m "feat: recommend best measured concurrency"
```

## Task 4: Parse and Validate Structured Stress Evidence

**Files:**
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_log.py:92-160,216-307`
- Create: `tests/test_model_doctor_log.py`

- [ ] **Step 1: Write failing parser tests**

Create a minimal v0.7.0 log builder and these tests in `tests/test_model_doctor_log.py`:

```python
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT_DIR = ROOT / "skills" / "creating-model-doctor-reports" / "scripts"
sys.path.insert(0, str(SCRIPT_DIR))

from model_doctor_log import parse_log, test_packet  # noqa: E402


def valid_stress_log():
    levels = "\n".join(
        [
            "level: concurrency=1,kind=coarse,attempted=2,successful=2,success_rate=1.0000,rps=1.8000,output_tps=460.8000,p50_ms=1000,p95_ms=1000,p99_ms=1000,max_ms=1000,rate_limited=0,http_5xx=0,timeouts=0,transport_errors=0,eligible=1",
            "level: concurrency=2,kind=coarse,attempted=4,successful=4,success_rate=1.0000,rps=3.6000,output_tps=921.6000,p50_ms=1050,p95_ms=1050,p99_ms=1050,max_ms=1050,rate_limited=0,http_5xx=0,timeouts=0,transport_errors=0,eligible=1",
            "level: concurrency=4,kind=coarse,attempted=8,successful=8,success_rate=1.0000,rps=7.0000,output_tps=1792.0000,p50_ms=1100,p95_ms=1100,p99_ms=1100,max_ms=1100,rate_limited=0,http_5xx=0,timeouts=0,transport_errors=0,eligible=1",
            "level: concurrency=6,kind=refinement,attempted=12,successful=12,success_rate=1.0000,rps=7.1000,output_tps=1817.6000,p50_ms=1250,p95_ms=1250,p99_ms=1250,max_ms=1250,rate_limited=0,http_5xx=0,timeouts=0,transport_errors=0,eligible=1",
            "level: concurrency=8,kind=coarse,attempted=16,successful=16,success_rate=1.0000,rps=6.2000,output_tps=1587.2000,p50_ms=1600,p95_ms=1600,p99_ms=1600,max_ms=1600,rate_limited=0,http_5xx=0,timeouts=0,transport_errors=0,eligible=1",
        ]
    )
    return f"""========== MODEL DOCTOR RUN ==========
script_version: 0.7.0
test_count: 1
url: https://model.example/v1/chat/completions
model: fixture-model
api_key: [REDACTED]
========== REQUEST test-057-c4-measure-w1-s1 BEGIN ==========
request_id: test-057-c4-measure-w1-s1
protocol: openai_chat
stream: 0
----- REQUEST BODY BEGIN -----
{{"model":"fixture-model","messages":[{{"role":"user","content":"stress"}}]}}
----- REQUEST BODY END -----
----- RESPONSE METRICS BEGIN -----
curl_exit_code: 0
http_status: 200
time_total: 1.100
time_starttransfer: 0.100
size_download: 128
----- RESPONSE METRICS END -----
----- RESPONSE BODY BEGIN -----
{{"choices":[{{"message":{{"content":"ok"}},"finish_reason":"stop"}}],"usage":{{"completion_tokens":256}}}}
----- RESPONSE BODY END -----
========== REQUEST test-057-c4-measure-w1-s1 END ==========
========== MODEL DOCTOR STRESS 057 BEGIN ==========
schema_version: model-doctor.stress.v1
stress_mode: standard
workload_input_characters: 4096
workload_visible_word_min: 200
workload_visible_word_max: 320
max_concurrency: 8
baseline_p95_ms: 1000
latency_limit_ms: 2000
score_type: output_tps
{levels}
recommended_concurrency: 4
stable_interval: 4-6
recommendation_status: PASS
recommendation_reason: measured_plateau
severe_stop_reason: none
========== MODEL DOCTOR STRESS 057 END ==========
========== TEST-057 BEGIN ==========
category: 性能与稳定性
name: 自适应并发压测
result: PASS
conclusion: 推荐并发 4
========== TEST-057 END ==========
========== RUN SUMMARY ==========
request_count: 1
========== END ==========
"""


class StressLogParserTests(unittest.TestCase):
    def write_log(self, value):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        path = Path(directory.name) / "stress.log"
        path.write_text(value, encoding="utf-8")
        return path

    def test_parser_attaches_typed_stress_summary_to_057(self):
        parsed = parse_log(self.write_log(valid_stress_log()))
        stress = parsed["tests"]["057"]["stress"]
        self.assertEqual(stress["schemaVersion"], "model-doctor.stress.v1")
        self.assertEqual(stress["recommendedConcurrency"], 4)
        self.assertEqual(stress["stableInterval"], [4, 6])
        self.assertEqual([level["concurrency"] for level in stress["levels"]], [1, 2, 4, 6, 8])
        self.assertIsInstance(stress["levels"][0]["p95Ms"], int)
        self.assertIsInstance(stress["levels"][0]["outputTokensPerSecond"], float)

    def test_parser_clears_unmeasured_recommendation_and_warns(self):
        log = valid_stress_log().replace("recommended_concurrency: 4", "recommended_concurrency: 5")
        parsed = parse_log(self.write_log(log))
        self.assertIsNone(parsed["tests"]["057"]["stress"]["recommendedConcurrency"])
        self.assertTrue(any("unmeasured concurrency" in item for item in parsed["warnings"]))

    def test_packet_includes_stress_and_only_linked_requests(self):
        parsed = parse_log(self.write_log(valid_stress_log()))
        packet = test_packet(parsed, "057")
        self.assertIn("stress", packet["test"])
        self.assertTrue(all(request["request_id"].startswith("test-057-") for request in packet["requests"]))
```

- [ ] **Step 2: Run parser tests and verify RED**

Run `python3 -m unittest tests.test_model_doctor_log -v`.

Expected: errors because no stress parser or `stress` field exists.

- [ ] **Step 3: Implement bounded parsing**

Add `_stress_block`, `_typed_number`, `_parse_level`, and `_parse_stress_summary`. Only inspect exact begin/end markers. Split `level:` records on commas and then first `=`. Map `not_available` to `None` and produce:

```python
{
    "schemaVersion": "model-doctor.stress.v1",
    "mode": "standard",
    "workload": {"inputCharacters": 4096, "visibleWordMin": 200, "visibleWordMax": 320},
    "maxConcurrency": 64,
    "baselineP95Ms": 1000,
    "latencyLimitMs": 2000,
    "scoreType": "output_tps",
    "levels": [{
        "concurrency": 1, "kind": "coarse", "attempted": 2, "successful": 2,
        "successRate": 1.0, "successfulRequestsPerSecond": 1.8182,
        "outputTokensPerSecond": 465.4545, "p50Ms": 1000, "p95Ms": 1000,
        "p99Ms": 1000, "maxMs": 1000, "rateLimited": 0, "http5xx": 0,
        "timeouts": 0, "transportErrors": 0, "eligible": True,
    }],
    "recommendedConcurrency": 4,
    "stableInterval": [4, 6],
    "recommendationStatus": "PASS",
    "recommendationReason": "measured_plateau",
    "severeStopReason": None,
}
```

Validate nonnegative metrics, unique positive concurrency, `successful <= attempted`, success-rate consistency within `0.0001`, `latencyLimitMs == 2 * baselineP95Ms`, recommendation measured and eligible, and stable endpoints measured and eligible. Invalid cross-fields add warnings and clear the affected recommendation; they never invent values.

Attach the result only to `tests["057"]["stress"]`. `test_packet` already returns the test dictionary and linked requests.

- [ ] **Step 4: Verify GREEN and commit**

Run `python3 -m unittest tests.test_model_doctor_log -v`.

Expected: all parser tests pass.

Commit:

```bash
git add skills/creating-model-doctor-reports/scripts/model_doctor_log.py tests/test_model_doctor_log.py
git commit -m "feat: parse adaptive stress evidence"
```

## Task 5: Extend the Assessment Contract

**Files:**
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py:18-47,150-164,206-295`
- Modify: `skills/creating-model-doctor-reports/references/assessment-schema.json`
- Create: `tests/test_model_doctor_assessment.py`

- [ ] **Step 1: Write failing assessment tests**

Create a minimal parsed object containing v0.7.0 test `057`, requests, a valid stress object, and this review helper:

```python
def stress_evidence():
    return {
        "schemaVersion": "model-doctor.stress.v1", "mode": "standard",
        "workload": {"inputCharacters": 4096, "visibleWordMin": 200, "visibleWordMax": 320},
        "maxConcurrency": 8, "baselineP95Ms": 1000, "latencyLimitMs": 2000,
        "scoreType": "output_tps",
        "levels": [
            {"concurrency": 1, "kind": "coarse", "attempted": 2, "successful": 2, "successRate": 1.0, "successfulRequestsPerSecond": 1.8, "outputTokensPerSecond": 460.8, "p50Ms": 1000, "p95Ms": 1000, "p99Ms": 1000, "maxMs": 1000, "rateLimited": 0, "http5xx": 0, "timeouts": 0, "transportErrors": 0, "eligible": True},
            {"concurrency": 4, "kind": "coarse", "attempted": 8, "successful": 8, "successRate": 1.0, "successfulRequestsPerSecond": 7.0, "outputTokensPerSecond": 1792.0, "p50Ms": 1100, "p95Ms": 1100, "p99Ms": 1100, "maxMs": 1100, "rateLimited": 0, "http5xx": 0, "timeouts": 0, "transportErrors": 0, "eligible": True},
            {"concurrency": 6, "kind": "refinement", "attempted": 12, "successful": 12, "successRate": 1.0, "successfulRequestsPerSecond": 7.1, "outputTokensPerSecond": 1817.6, "p50Ms": 1250, "p95Ms": 1250, "p99Ms": 1250, "maxMs": 1250, "rateLimited": 0, "http5xx": 0, "timeouts": 0, "transportErrors": 0, "eligible": True},
        ],
        "recommendedConcurrency": 4, "stableInterval": [4, 6],
        "recommendationStatus": "PASS", "recommendationReason": "measured_plateau",
        "severeStopReason": None,
    }


def valid_review():
    return {
        "testId": "057", "reviewedStatus": "PASS", "confidence": "high",
        "gateLevel": "important", "conclusion": "标准负载下推荐并发为 4。",
        "logic": {
            "purpose": "寻找最佳并发。", "method": "执行持续并发阶梯与细化。",
            "passCriteria": ["存在满足成功率与 P95 门槛的吞吐平台。"],
            "failCriteria": ["并发大于 1 时均确认过载。"],
            "capabilityBoundary": "不构成 SLA。",
        },
        "evidenceRefs": ["request:test-057-c4-measure-w1-s1"],
        "evidenceExcerpts": ["推荐并发 4。"], "limitations": [], "retestInstructions": [],
    }
```

Tests:

```python
class StressAssessmentTests(unittest.TestCase):
    def setUp(self):
        request_id = "test-057-c4-measure-w1-s1"
        self.parsed = {
            "run": {"script_version": "0.7.0", "model": "fixture-model"},
            "source": {"fileName": "stress.log", "size": 1, "sha256": "0" * 64},
            "tokenTotals": {}, "warnings": [],
            "requests": {request_id: {"request_id": request_id, "metrics": {"http_status": "200", "time_total": "1.100"}, "requestBody": "{}", "responseBody": "{}"}},
            "tests": {"057": {"id": "057", "category": "性能与稳定性", "name": "自适应并发压测", "requestRefs": [request_id], "stress": stress_evidence()}},
        }

    def test_v070_keeps_057_important_and_carries_stress_data(self):
        assessment = assemble_assessment(self.parsed, {"057": valid_review()})
        item = assessment["tests"][0]
        self.assertEqual(item["gateLevel"], "important")
        self.assertEqual(item["stress"]["recommendedConcurrency"], 4)
        self.assertEqual(item["rawObservation"], "推荐并发 4 · 稳定区间 4-6 · output_tps")
        self.assertEqual(assessment["categories"][0]["performance"]["recommendedConcurrency"], 4)

    def test_validation_rejects_category_test_stress_mismatch(self):
        assessment = assemble_assessment(self.parsed, {"057": valid_review()})
        assessment["categories"][0]["performance"]["recommendedConcurrency"] = 8
        self.assertTrue(any("performance summary" in item for item in validate_assessment(assessment)))
```

- [ ] **Step 2: Run tests and verify RED**

Run `python3 -m unittest tests.test_model_doctor_assessment -v`.

Expected: v0.7.0 is unknown and stress/performance fields are absent.

- [ ] **Step 3: Implement mapping and propagation**

Add `"0.7.0": CURRENT_CATALOG_GATE_LEVELS` to `CATALOG_GATE_LEVELS`. Import `deepcopy`.

Replace the v0.6.0-only raw-observation branch with:

```python
if str(test.get("id")) == "057" and isinstance(test.get("stress"), dict):
    stress = test["stress"]
    recommended = stress.get("recommendedConcurrency")
    interval = stress.get("stableInterval") or []
    if recommended is not None and interval:
        return (
            f"推荐并发 {recommended} · 稳定区间 {interval[0]}-{interval[-1]} · "
            f"{stress.get('scoreType', 'unknown score')}"
        )
```

For test `057`, set `"stress": deepcopy(test["stress"])`. Add the same deep-copied object as `category["performance"]` only for `性能与稳定性`. `validate_assessment` must compare those objects and ensure the recommended level exists and is eligible. Reviewed status remains the only formal verdict.

- [ ] **Step 4: Extend the schema**

Add optional `stress` to test item properties and optional `performance` to category item properties. The stress definition requires schema version, mode, workload, max concurrency, baseline, latency limit, score type, levels, recommendation status, reason, and safety stop. Level definitions require every metric from Task 4; nullable metrics use `{"type": ["number", "null"]}`. Set `additionalProperties: false` on new stress structures. Historical and skipped tests do not require stress.

- [ ] **Step 5: Verify GREEN and commit**

Run:

```bash
python3 -m unittest tests.test_model_doctor_assessment -v
python3 -m json.tool skills/creating-model-doctor-reports/references/assessment-schema.json >/dev/null
```

Expected: all tests pass and schema JSON parses.

Commit:

```bash
git add skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py skills/creating-model-doctor-reports/references/assessment-schema.json tests/test_model_doctor_assessment.py
git commit -m "feat: carry stress recommendation into assessment"
```

## Task 6: Render the Customer Performance Summary

**Files:**
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_html.py:45-78,131-177,196-239`
- Modify: `skills/creating-model-doctor-reports/assets/report.css`
- Create: `tests/test_model_doctor_html.py`

- [ ] **Step 1: Write failing HTML tests**

Create `tests/test_model_doctor_html.py` with the normal report-script `sys.path` setup, `ASSET_DIR`, and this complete fixture:

```python
from copy import deepcopy


STRESS = {
    "schemaVersion": "model-doctor.stress.v1",
    "mode": "standard",
    "workload": {"inputCharacters": 4096, "visibleWordMin": 200, "visibleWordMax": 320},
    "maxConcurrency": 8,
    "baselineP95Ms": 1000,
    "latencyLimitMs": 2000,
    "scoreType": "output_tps",
    "levels": [
        {"concurrency": 1, "kind": "coarse", "attempted": 2, "successful": 2, "successRate": 1.0, "successfulRequestsPerSecond": 1.8, "outputTokensPerSecond": 460.8, "p50Ms": 1000, "p95Ms": 1000, "p99Ms": 1000, "maxMs": 1000, "rateLimited": 0, "http5xx": 0, "timeouts": 0, "transportErrors": 0, "eligible": True},
        {"concurrency": 4, "kind": "coarse", "attempted": 8, "successful": 8, "successRate": 1.0, "successfulRequestsPerSecond": 7.0, "outputTokensPerSecond": 1792.0, "p50Ms": 1100, "p95Ms": 1100, "p99Ms": 1100, "maxMs": 1100, "rateLimited": 0, "http5xx": 0, "timeouts": 0, "transportErrors": 0, "eligible": True},
        {"concurrency": 6, "kind": "refinement", "attempted": 12, "successful": 12, "successRate": 1.0, "successfulRequestsPerSecond": 7.1, "outputTokensPerSecond": 1817.6, "p50Ms": 1250, "p95Ms": 1250, "p99Ms": 1250, "maxMs": 1250, "rateLimited": 0, "http5xx": 0, "timeouts": 0, "transportErrors": 0, "eligible": True},
    ],
    "recommendedConcurrency": 4,
    "stableInterval": [4, 6],
    "recommendationStatus": "PASS",
    "recommendationReason": "measured_plateau",
    "severeStopReason": None,
}


def stress_assessment():
    counts = {"ERROR": 0, "FAIL": 0, "PASS": 1, "SKIPPED": 0, "UNDETERMINED": 0, "UNSUPPORTED": 0}
    return {
        "schemaVersion": "llm-capability-doctor.assessment.v2",
        "generatedAt": "2026-07-17T00:00:00+00:00",
        "source": {"fileName": "stress.log", "size": 1, "sha256": "0" * 64},
        "run": {"model": "fixture-model", "url": "https://model.example", "api_key": "[REDACTED]"},
        "overall": {"verdict": "READY", "counts": counts, "blockers": [], "conditions": []},
        "categories": [{"name": "性能与稳定性", "status": "PASS", "counts": counts, "criticalFailures": [], "unknowns": [], "performance": deepcopy(STRESS)}],
        "tests": [{
            "testId": "057", "category": "性能与稳定性", "name": "自适应并发压测",
            "gateLevel": "important", "reviewedStatus": "PASS", "confidence": "high",
            "conclusion": "标准负载下推荐并发为 4。",
            "logic": {"purpose": "寻找最佳并发。", "method": "持续压测。", "passCriteria": ["存在合格并发。"], "failCriteria": ["全部过载。"], "capabilityBoundary": "不构成 SLA。"},
            "rawObservation": "推荐并发 4 · 稳定区间 4-6 · output_tps",
            "metrics": [], "evidenceRefs": ["test:057:raw"], "evidenceExcerpts": ["推荐并发 4。"],
            "limitations": [], "retestInstructions": [], "requests": [], "stress": deepcopy(STRESS),
        }],
        "tokenTotals": {}, "warnings": [],
    }


class StressHtmlTests(unittest.TestCase):
    def test_summary_surfaces_recommendation_and_boundary(self):
        html = render_report(stress_assessment(), ASSET_DIR)
        self.assertIn("推荐并发 4", html)
        self.assertIn("稳定区间 4-6", html)
        self.assertIn("P95 上限 2000ms", html)
        self.assertIn("约 4096 字符输入、200-320 个可见英文词输出", html)

    def test_detail_renders_levels_and_marks_recommendation(self):
        html = render_report(stress_assessment(), ASSET_DIR)
        self.assertIn('<table class="stress-level-table">', html)
        for label in ("并发", "成功率", "output tokens/s", "P50", "P95", "P99"):
            self.assertIn(label, html)
        self.assertIn('class="stress-level-recommended"', html)
        self.assertEqual(html.count('data-concurrency="4"'), 1)

    def test_values_are_escaped_and_report_stays_offline(self):
        value = stress_assessment()
        value["tests"][0]["stress"]["recommendationReason"] = '<img src=x onerror=alert(1)>'
        html = render_report(value, ASSET_DIR)
        self.assertIn("&lt;img", html)
        self.assertNotIn("<img src=x", html)
        self.assertNotRegex(html, r'(?:src|href)=["\']https?://')
```

- [ ] **Step 2: Run tests and verify RED**

Run `python3 -m unittest tests.test_model_doctor_html -v`.

Expected: failures because no stress summary or table renderer exists.

- [ ] **Step 3: Render category wording and table**

In `_category_key_data`:

```python
performance = category.get("performance")
if isinstance(performance, dict):
    recommended = performance.get("recommendedConcurrency")
    interval = performance.get("stableInterval") or []
    if recommended is not None and interval:
        return f"推荐并发 {recommended}；稳定区间 {interval[0]}-{interval[-1]}；P95 上限 {performance.get('latencyLimitMs')}ms"
```

Update `_category_conclusion` to say the result is client-observed for this endpoint, region, network, standardized workload, and time window, not an SLA.

Add `_stress_level_table(stress)`. Escape every value with `_e`. Render columns for concurrency, kind, sample count, success rate, successful RPS, provider output tokens/s, P50/P95/P99, 429, other errors, and eligibility. Add `stress-level-recommended` only to the recommended row. Insert the summary/table before `_request_evidence` in test `057` details.

- [ ] **Step 4: Add responsive CSS**

Add `.stress-summary`, `.stress-level-scroll`, `.stress-level-table`, and `.stress-level-recommended`. Use existing colors plus one restrained green indicator, `overflow-x: auto`, no nested cards, radius at most 8px, dark-mode contrast, and print rules that do not clip the table.

- [ ] **Step 5: Verify GREEN and commit**

Run `python3 -m unittest tests.test_model_doctor_html -v`.

Expected: all HTML tests pass and no external resources appear.

Commit:

```bash
git add skills/creating-model-doctor-reports/scripts/model_doctor_html.py skills/creating-model-doctor-reports/assets/report.css tests/test_model_doctor_html.py
git commit -m "feat: render best concurrency recommendation"
```

## Task 7: Update Review Rules and Operator Documentation

**Files:**
- Modify: `skills/creating-model-doctor-reports/references/evaluation-rules.md:34-57,160-177`
- Modify: `skills/creating-model-doctor-reports/SKILL.md:24-55`
- Modify: `README.md:19-50`
- Modify: `tests/test_model_doctor_log.py`

- [ ] **Step 1: Add a failing documentation contract test**

Add `ROOT = Path(__file__).resolve().parents[1]` and:

```python
    def test_v070_rules_define_adaptive_stress_boundaries(self):
        rules = (
            ROOT / "skills" / "creating-model-doctor-reports" /
            "references" / "evaluation-rules.md"
        ).read_text(encoding="utf-8")
        for expected in (
            "v0.7.0", "provider-reported output tokens/s", "successful requests/s",
            "P95 <= 2 × concurrency-1 P95", "99%", "3%", "不构成 SLA",
        ):
            self.assertIn(expected, rules)
```

- [ ] **Step 2: Run the contract and verify RED**

Run:

```bash
python3 -m unittest tests.test_model_doctor_log.StressLogParserTests.test_v070_rules_define_adaptive_stress_boundaries -v
```

Expected: failure because v0.7.0 rules are absent.

- [ ] **Step 3: Update evaluation and Skill instructions**

Make v0.7.0 the current 62-test catalog with unchanged gate levels and `057` important. In Performance and Stability, require reviewers to reconcile every level's attempts, semantic successes, error counts, measured duration, token availability, percentile sample count, baseline, adaptive limit, refinement levels, safety stop, and recommendation. Explicitly prohibit calling provider output tokens/s visible-token speed, treating TTFB as TTFT, or claiming SLA/fleet capacity.

In `SKILL.md`, add v0.7.0 stress evidence to workflow step 5. Require `UNDETERMINED` when the structured summary contradicts request evidence or recommends an unmeasured/ineligible level.

- [ ] **Step 4: Document exact operator usage**

Add to README:

```bash
./model-capability-doctor.sh \
  --url 'https://model.example/v1/messages' \
  --model 'customer-model' \
  --api-key '...' \
  --only '057' \
  --stress-mode standard \
  --stress-max-concurrency 64 \
  --log-file './stress-test.log'
```

Document the opt-in billable load, 8-12 minute estimate, 4,096-character/200-320-word standard workload, max 64, 99% success gate, 2x baseline P95 gate, 3% throughput plateau, safety stops, and example:

```text
推荐并发：20；稳定区间：16-24；20 并发下吞吐 1,240 provider output tokens/s，成功率 100%，P95 3.2s。
```

State that omitting standard stress mode skips important test `057`, producing a conditional readiness gate.

- [ ] **Step 5: Verify GREEN and commit**

Run:

```bash
python3 -m unittest tests.test_model_doctor_log -v
python3 -m unittest tests.test_model_capability_doctor_script tests.test_model_doctor_assessment tests.test_model_doctor_html -v
```

Expected: all focused tests pass.

Commit:

```bash
git add README.md skills/creating-model-doctor-reports/SKILL.md skills/creating-model-doctor-reports/references/evaluation-rules.md tests/test_model_doctor_log.py
git commit -m "docs: explain adaptive stress interpretation"
```

## Task 8: End-to-End Fixture and Final Verification

**Files:**
- Modify: `tests/test_model_capability_doctor_script.py`
- Modify: `tests/test_model_doctor_html.py`
- Verify: all files changed by Tasks 1-7

- [ ] **Step 1: Add a failing round-trip test**

Insert the report script directory in `sys.path` and import `parse_log`, `assemble_assessment`, and `render_report`. Add this helper to `AdaptiveStressScriptTests` so its temporary log survives until test cleanup:

```python
    def run_standard_to_path(self, maximum="8", scenario="healthy"):
        temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(temporary_directory.cleanup)
        directory = Path(temporary_directory.name)
        fake_curl = directory / "curl"
        log_path = directory / "doctor.log"
        shutil.copy2(FAKE_CURL, fake_curl)
        fake_curl.chmod(0o755)
        environment = dict(os.environ)
        environment.update({
            "PATH": f"{directory}:{environment['PATH']}",
            "MODEL_DOCTOR_FAKE_SCENARIO": scenario,
            "MODEL_DOCTOR_STRESS_TEST_MODE": "1",
            "MODEL_DOCTOR_STRESS_TEST_SAMPLES_PER_WORKER": "2",
        })
        result = subprocess.run(
            ["bash", str(SCRIPT), *self.base_arguments(), "--stress-mode", "standard",
             "--stress-max-concurrency", maximum, "--log-file", str(log_path)],
            cwd=ROOT, env=environment, capture_output=True, text=True, check=False,
        )
        return result, log_path
```

Add the review helper and round-trip test:

```python
    def valid_stress_review(self, status):
        return {
            "testId": "057", "reviewedStatus": status, "confidence": "high",
            "gateLevel": "important", "conclusion": "压测推荐值已完成复核。",
            "logic": {
                "purpose": "寻找最佳并发。", "method": "持续压测。",
                "passCriteria": ["存在合格推荐。"], "failCriteria": ["全部过载。"],
                "capabilityBoundary": "不构成 SLA。",
            },
            "evidenceRefs": ["request:test-057-c4-measure-w1-s1"],
            "evidenceExcerpts": ["结构化压测摘要与请求证据一致。"],
            "limitations": [], "retestInstructions": [],
        }

    def test_stress_log_round_trips_to_customer_report(self):
        result, log_path = self.run_standard_to_path(maximum="8", scenario="healthy")
        self.assertEqual(result.returncode, 0, result.stderr)
        parsed = parse_log(log_path)
        stress = parsed["tests"]["057"]["stress"]
        review = self.valid_stress_review(status=stress["recommendationStatus"])
        assessment = assemble_assessment(parsed, {"057": review})
        html = render_report(assessment, ASSET_DIR)
        recommended = stress["recommendedConcurrency"]
        self.assertEqual(assessment["tests"][0]["stress"]["recommendedConcurrency"], recommended)
        self.assertEqual(assessment["categories"][0]["performance"]["recommendedConcurrency"], recommended)
        self.assertIn(f"推荐并发 {recommended}", html)
        self.assertNotIn("fixture-key", log_path.read_text(encoding="utf-8"))
        self.assertNotIn("fixture-key", html)
```

- [ ] **Step 2: Run the round-trip test and fix integration defects**

Run:

```bash
python3 -m unittest tests.test_model_capability_doctor_script.AdaptiveStressScriptTests.test_stress_log_round_trips_to_customer_report -v
```

Expected before integration fixes: any failure identifies an exact producer/consumer field mismatch. Align all layers to Task 4's single field schema; do not introduce aliases.

- [ ] **Step 3: Run complete automated verification**

Run:

```bash
bash -n model-capability-doctor.sh
python3 -m unittest discover -s tests -v
python3 -m py_compile \
  skills/creating-model-doctor-reports/scripts/model_doctor_log.py \
  skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py \
  skills/creating-model-doctor-reports/scripts/model_doctor_html.py \
  skills/creating-model-doctor-reports/scripts/model_doctor_report.py
python3 -m json.tool skills/creating-model-doctor-reports/references/assessment-schema.json >/dev/null
git diff --check
```

Expected: Bash syntax exits `0`; unittest reports zero failures/errors; Python compilation exits `0`; schema parsing succeeds; diff check is empty.

- [ ] **Step 4: Generate and inspect a deterministic report**

Use fake curl to generate a temporary v0.7.0 stress log and HTML. Verify programmatically that the HTML has no external `src`/`href`, contains every measured level once, marks one recommended row, and contains no `fixture-key`.

Serve the artifact with `python3 -m http.server` on an available local port. Inspect desktop and mobile widths with Playwright. Confirm the table does not overlap or clip text, the evidence toggle still works, recommendation is identifiable without color alone, dark mode is legible, and print CSS keeps metrics visible.

- [ ] **Step 5: Audit against the approved specification**

Check every acceptance criterion in `docs/superpowers/specs/2026-07-17-adaptive-concurrency-stress-test-design.md`. Confirm production defaults: 5-second warm-up, 30-second level measurement, 45-90 second baseline, max 64, 99% success, 2.0 P95 factor, and 3% plateau. Confirm test-only controls have no effect unless `MODEL_DOCTOR_STRESS_TEST_MODE=1`.

- [ ] **Step 6: Commit end-to-end verification**

```bash
git add tests/test_model_capability_doctor_script.py tests/test_model_doctor_html.py
git commit -m "test: verify adaptive stress report end to end"
```

- [ ] **Step 7: Show final history and status**

Run:

```bash
git log --oneline --decorate -8
git status --short
```

Expected: implementation commits are present; only the user's pre-existing logs, reports, and `.DS_Store` remain untracked.
