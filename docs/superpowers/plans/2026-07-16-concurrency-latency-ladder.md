# Concurrency Latency Ladder Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace test `057`'s single 8-concurrency success check with an auditable 4/8/16/32 concurrency ladder that reports semantic-success response-latency distributions without imposing a latency SLA.

**Architecture:** Keep worker synchronization, exact-output validation, percentile calculation, and result aggregation in the existing Bash detector. Extend the shared Python fake-curl fixture for deterministic integration tests, and make the assessment layer display test `057`'s detector summary instead of its final request metric. Publish this post-`0.5.0` contract as `0.6.0` while retaining all historical report mappings.

**Tech Stack:** Bash 3-compatible shell, curl timing metrics, awk/sed, Python 3 standard library, unittest

---

## File Map

- Modify `model-capability-doctor.sh`: publish `0.6.0`, rename `057`, run synchronized waves, validate exact output, calculate nearest-rank percentiles, and aggregate four waves.
- Modify `tests/helpers/fake_model_curl.py`: add deterministic success and partial-failure concurrency scenarios.
- Modify `tests/test_model_capability_doctor_script.py`: verify all 60 request audits, parser linkage, statistics, failure handling, and continuation.
- Modify `skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py`: add the `0.6.0` map and prefer `057`'s detected summary.
- Modify `tests/test_model_doctor_assessment.py`: cover the new version and raw-observation selection.
- Modify `skills/creating-model-doctor-reports/references/evaluation-rules.md`: make `0.6.0` current and document concurrency interpretation.
- Modify `tests/test_model_doctor_skill.py`: lock the current and historical rule text.
- Modify `docs/superpowers/specs/2026-07-16-concurrency-latency-ladder-design.md`: record the version adjustment caused by the intervening `0.5.0` merge.

### Task 1: Lock the Detector Contract

**Files:**
- Modify: `tests/helpers/fake_model_curl.py`
- Modify: `tests/test_model_capability_doctor_script.py`
- Test: `tests/test_model_capability_doctor_script.py`

- [ ] **Step 1: Update version and catalog expectations**

In `test_help_declares_version_catalog_size_and_default_timeout`, change both
version assertions to `0.6.0`. In the catalog test, add:

```python
self.assertEqual(names["057"], "4-32 并发响应时间")
```

- [ ] **Step 2: Let `run_fixture` return parsed evidence when requested**

Add the report scripts to the import path in
`tests/test_model_capability_doctor_script.py`:

```python
import sys

REPORT_SCRIPT_DIR = ROOT / "skills" / "creating-model-doctor-reports" / "scripts"
sys.path.insert(0, str(REPORT_SCRIPT_DIR))

from model_doctor_log import parse_log  # noqa: E402
```

Change the helper signature and return branch without changing existing callers:

```python
def run_fixture(self, scenario, only, *, include_parsed=False):
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
                "--model", "fixture-model", "--api-key", "fixture-key",
                "--only", only, "--log-file", str(log_path),
            ],
            cwd=ROOT,
            env=environment,
            capture_output=True,
            text=True,
            check=False,
        )
        log = log_path.read_text(encoding="utf-8")
        if include_parsed:
            return result, log, parse_log(log_path)
        return result, log
```

- [ ] **Step 3: Add concurrency behavior to the shared fake curl**

Import `re` in `tests/helpers/fake_model_curl.py`. After reading `scenario`,
initialize `time_total = "0.020"`. Before the fallback response branch, add:

```python
elif scenario in {"concurrency_ladder_exact", "concurrency_ladder_partial"} and (
    marker_match := re.search(r"MODEL_DOCTOR_057_C(4|8|16|32)_OK", request_body)
):
    request_match = re.search(r"test-057-c(4|8|16|32)-([0-9]+)$", output_path.stem)
    level = int(marker_match.group(1))
    sample = int(request_match.group(2)) if request_match else 0
    response = chat(marker_match.group(0))
    time_total = f"{sample / 1000:.3f}"
    if scenario == "concurrency_ladder_partial" and level == 4:
        response = chat("WRONG_OUTPUT")
    elif scenario == "concurrency_ladder_partial" and (level, sample) == (8, 3):
        response = chat("RATE_LIMITED")
        http_status = "429"
    elif scenario == "concurrency_ladder_partial" and (level, sample) == (16, 2):
        time_total = "malformed"
```

Change the final metrics write to use the variable:

```python
sys.stdout.write(
    f"{http_status}\t{time_total}\t{time_starttransfer}\t{len(response.encode('utf-8'))}"
)
```

- [ ] **Step 4: Add the successful-ladder integration test**

```python
def test_057_reports_all_four_concurrency_latency_waves(self):
    result, log, parsed = self.run_fixture(
        "concurrency_ladder_exact",
        "057",
        include_parsed=True,
    )

    self.assertEqual(result.returncode, 0, result.stderr)
    request_starts = re.findall(
        r"^========== REQUEST test-057-c(?:4|8|16|32)-[0-9]+ BEGIN ==========$",
        log,
        flags=re.MULTILINE,
    )
    self.assertEqual(len(request_starts), 60)
    self.assertEqual(len(parsed["tests"]["057"]["requestRefs"]), 60)
    self.assertIn("result: PASS", log)
    self.assertIn(
        "detected: "
        "c4:success=4/4,p50_ms=2,p95_ms=4,max_ms=4,rate_limited=0;"
        "c8:success=8/8,p50_ms=4,p95_ms=8,max_ms=8,rate_limited=0;"
        "c16:success=16/16,p50_ms=8,p95_ms=16,max_ms=16,rate_limited=0;"
        "c32:success=32/32,p50_ms=16,p95_ms=31,max_ms=32,rate_limited=0",
        log,
    )
    self.assertIn("request_count: 61", log)
```

- [ ] **Step 5: Add the failure and continuation integration test**

```python
def test_057_rejects_bad_samples_and_continues_through_c32(self):
    result, log, parsed = self.run_fixture(
        "concurrency_ladder_partial",
        "057",
        include_parsed=True,
    )

    self.assertEqual(result.returncode, 0, result.stderr)
    self.assertIn("result: FAIL", log)
    self.assertIn(
        "c4:success=0/4,p50_ms=not_available,p95_ms=not_available,"
        "max_ms=not_available,rate_limited=0",
        log,
    )
    self.assertIn("c8:success=7/8,p50_ms=5,p95_ms=8,max_ms=8,rate_limited=1", log)
    self.assertIn("c16:success=15/16,p50_ms=9,p95_ms=16,max_ms=16,rate_limited=0", log)
    self.assertIn("c32:success=32/32", log)
    self.assertIn("semantic_success=0", log)
    self.assertEqual(len(parsed["tests"]["057"]["requestRefs"]), 60)
```

- [ ] **Step 6: Run the focused tests and confirm RED**

```bash
python3 -m unittest tests.test_model_capability_doctor_script -v
```

Expected: failures identify version `0.5.0`, old name `8 并发性能`, one eight-request
wave, missing percentile fields, and HTTP-only success counting.

- [ ] **Step 7: Commit the failing contract tests**

```bash
git add tests/helpers/fake_model_curl.py tests/test_model_capability_doctor_script.py
git commit -m "test: define concurrency latency ladder contract"
```

### Task 2: Implement the Four-Wave Detector

**Files:**
- Modify: `model-capability-doctor.sh`
- Test: `tests/test_model_capability_doctor_script.py`

- [ ] **Step 1: Publish the new contract**

Set `SCRIPT_VERSION="0.6.0"` and change catalog row `057` to:

```text
057	性能与稳定性	4-32 并发响应时间
```

- [ ] **Step 2: Add a nearest-rank helper after `millis_from_seconds`**

```bash
nearest_rank_from_sorted_file() {
  local file="$1"
  local percentile="$2"
  awk -v percentile="$percentile" '
    { values[NR] = $1 }
    END {
      if (NR == 0) exit
      rank = int((NR * percentile + 99) / 100)
      print values[rank]
    }
  ' "$file"
}
```

- [ ] **Step 3: Synchronize each wave**

Add `local gate_file="$4"` to `parallel_curl_worker`. Immediately before its
start timestamp, wait on the file:

```bash
while [[ ! -e "$gate_file" ]]; do
  sleep 0.01
done
```

In `run_parallel_batch`, create a per-wave gate before the launch loop:

```bash
local gate_file="$RUN_TMP_DIR/test-${id}-${label}.gate"
rm -f "$gate_file"
```

Pass it to every worker and release all workers after the launch loop:

```bash
parallel_curl_worker "$body" "$prefix" "$DETECTED_AUTH_MODE" "$gate_file" &
pids="$pids $!"
# After all workers are forked:
: >"$gate_file"
```

- [ ] **Step 4: Validate semantics and collect successful milliseconds**

In `run_parallel_batch`, uppercase the label for the marker and initialize files:

```bash
local upper_label="" visible="" latency_ms=""
local semantic_success=0
local times_file="$RUN_TMP_DIR/test-${id}-${label}.times"
local sorted_file="${times_file}.sorted"
upper_label="$(printf '%s' "$label" | tr '[:lower:]' '[:upper:]')"
marker="MODEL_DOCTOR_${id}_${upper_label}_OK"
: >"$evidence"
: >"$times_file"
```

Replace HTTP-only success counting with:

```bash
visible="$(extract_visible_text "${prefix}.body" 2>/dev/null | trim_text)"
semantic_success=0
latency_ms="not_available"
if [[ "$time_total" =~ ^[0-9]+([.][0-9]+)?$ ]]; then
  latency_ms="$(millis_from_seconds "$time_total")"
fi
if [[ "$curl_exit" == "0" && "$http" =~ ^2[0-9][0-9]$ && "$visible" == "$marker" && "$latency_ms" != "not_available" ]]; then
  semantic_success=1
  successes=$((successes + 1))
  printf '%s\n' "$latency_ms" >>"$times_file"
fi
[[ "$http" == "429" ]] && rate_limited=$((rate_limited + 1))
```

Write the following evidence line before each response body:

```bash
echo "concurrency=${concurrency} request_index=${index} http_status=${http:-000} curl_exit=${curl_exit} semantic_success=${semantic_success} time_total_ms=${latency_ms}"
```

After all requests are audited, calculate batch globals:

```bash
sort -n "$times_file" >"$sorted_file"
REQUEST_COUNT=$((REQUEST_COUNT + concurrency))
BATCH_SUCCESS_COUNT="$successes"
BATCH_RATE_LIMITED="$rate_limited"
BATCH_P50_MS="$(nearest_rank_from_sorted_file "$sorted_file" 50)"
BATCH_P95_MS="$(nearest_rank_from_sorted_file "$sorted_file" 95)"
BATCH_MAX_MS="$(tail -n 1 "$sorted_file")"
BATCH_EVIDENCE_FILE="$evidence"
```

- [ ] **Step 5: Aggregate cases 4, 8, 16, and 32 under test `057`**

Add `concurrency`, `separator`, `segment`, and `combined_evidence` locals to
`run_core_performance_test`. Replace branch `057` with:

```bash
057)
  if [[ "$DETECTED_PROTOCOL" == "unknown" ]]; then
    status="UNDETERMINED"
    conclusion="未知协议，无法执行并发响应时间检测"
    detected="c4:not_available;c8:not_available;c16:not_available;c32:not_available"
    evidence_file="$PROTOCOL_PROBE_RESPONSE_FILE"
  else
    combined_evidence="$RUN_TMP_DIR/test-${id}.concurrency-ladder"
    : >"$combined_evidence"
    detected=""
    conclusion=""
    separator=""
    for concurrency in 4 8 16 32; do
      run_parallel_batch "$id" "$concurrency" "c${concurrency}"
      cat "$BATCH_EVIDENCE_FILE" >>"$combined_evidence"
      segment="c${concurrency}:success=${BATCH_SUCCESS_COUNT}/${concurrency},p50_ms=${BATCH_P50_MS:-not_available},p95_ms=${BATCH_P95_MS:-not_available},max_ms=${BATCH_MAX_MS:-not_available},rate_limited=${BATCH_RATE_LIMITED}"
      detected="${detected}${separator}${segment}"
      conclusion="${conclusion}${separator}${concurrency} 并发成功 ${BATCH_SUCCESS_COUNT}/${concurrency}，P50 ${BATCH_P50_MS:-不可用}ms，P95 ${BATCH_P95_MS:-不可用}ms，最大 ${BATCH_MAX_MS:-不可用}ms，限流 ${BATCH_RATE_LIMITED}"
      separator=";"
      if [[ "$BATCH_SUCCESS_COUNT" != "$concurrency" ]]; then
        status="FAIL"
      fi
    done
    evidence_file="$combined_evidence"
  fi
  ;;
```

Initialize the new batch globals with the existing performance state:

```bash
BATCH_SUCCESS_COUNT=0
BATCH_RATE_LIMITED=0
BATCH_P50_MS=""
BATCH_P95_MS=""
BATCH_MAX_MS=""
BATCH_EVIDENCE_FILE=""
```

- [ ] **Step 6: Run focused verification**

```bash
bash -n model-capability-doctor.sh
python3 -m unittest tests.test_model_capability_doctor_script -v
```

Expected: valid shell syntax and all detector tests pass.

- [ ] **Step 7: Commit**

```bash
git add model-capability-doctor.sh
git commit -m "feat: measure latency across concurrency ladder"
```

### Task 3: Lock and Implement the Report Contract

**Files:**
- Modify: `tests/test_model_doctor_assessment.py`
- Modify: `tests/test_model_doctor_skill.py`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py`
- Modify: `skills/creating-model-doctor-reports/references/evaluation-rules.md`
- Test: `tests/test_model_doctor_assessment.py`
- Test: `tests/test_model_doctor_skill.py`
- Test: `tests/test_model_doctor_html.py`

- [ ] **Step 1: Write failing version and observation tests**

In `test_current_and_historical_62_item_versions_share_the_gate_mapping`, add the
`0.6.0` assertion so the complete body is:

```python
def test_current_and_historical_62_item_versions_share_the_gate_mapping(self):
    mappings = getattr(assessment_module, "CATALOG_GATE_LEVELS", {})

    self.assertEqual(mappings["0.6.0"], assessment_module.CURRENT_CATALOG_GATE_LEVELS)
    self.assertEqual(mappings["0.5.0"], assessment_module.CURRENT_CATALOG_GATE_LEVELS)
    self.assertEqual(mappings["0.4.0"], assessment_module.CURRENT_CATALOG_GATE_LEVELS)
```

Change the current wrong-gate validation test to use `0.6.0`. Add this
historical 62-item validation test and retain the existing `0.3.0` test:

```python
def test_validate_reviews_rejects_wrong_gate_for_historical_62_item_catalogs(self):
    for version in ("0.5.0", "0.4.0"):
        with self.subTest(version=version):
            self.parsed["run"]["script_version"] = version
            review = valid_review(gate="observation")

            errors = validate_reviews(self.parsed, {"001": review})

            self.assertTrue(any("001" in error and "critical" in error for error in errors))
```

Then add the observation test:

```python
def test_concurrency_ladder_uses_detected_summary_as_raw_observation(self):
    summary = (
        "c4:success=4/4,p50_ms=100,p95_ms=140,max_ms=140,rate_limited=0;"
        "c8:success=8/8,p50_ms=120,p95_ms=190,max_ms=190,rate_limited=0;"
        "c16:success=16/16,p50_ms=150,p95_ms=240,max_ms=250,rate_limited=0;"
        "c32:success=32/32,p50_ms=210,p95_ms=390,max_ms=420,rate_limited=0"
    )
    self.parsed["run"]["script_version"] = "0.6.0"
    test = self.parsed["tests"].pop("001")
    request = self.parsed["requests"].pop("test-001")
    test.update({
        "id": "057",
        "name": "4-32 并发响应时间",
        "category": "性能与稳定性",
        "detected": summary,
        "requestRefs": ["test-057-c4-1"],
    })
    request["request_id"] = "test-057-c4-1"
    self.parsed["tests"]["057"] = test
    self.parsed["requests"]["test-057-c4-1"] = request
    review = valid_review(test_id="057", gate="important")
    review["evidenceRefs"] = ["request:test-057-c4-1"]

    assessment = assemble_assessment(self.parsed, {"057": review})

    self.assertEqual(assessment["tests"][0]["rawObservation"], summary)
```

- [ ] **Step 2: Update the skill-rule assertions before implementation**

Expect these phrases in `tests/test_model_doctor_skill.py`:

```python
self.assertIn("当前 v0.6.0 的 62 项目录使用固定优先级", text)
self.assertIn("历史 v0.5.0", text)
self.assertIn("历史 v0.4.0", text)
self.assertIn("4、8、16、32", text)
self.assertIn("nearest-rank P95", text)
self.assertIn("完整响应延迟", text)
self.assertIn("不构成 SLA", text)
```

- [ ] **Step 3: Run report tests and confirm RED**

```bash
python3 -m unittest tests.test_model_doctor_assessment tests.test_model_doctor_skill -v
```

Expected: `0.6.0` has no gate mapping, test `057` shows its final request metric,
and the rules still name `0.5.0` as current.

- [ ] **Step 4: Implement version and observation selection**

Extend the map:

```python
CATALOG_GATE_LEVELS = {
    "0.3.0": V0_3_CATALOG_GATE_LEVELS,
    "0.4.0": CURRENT_CATALOG_GATE_LEVELS,
    "0.5.0": CURRENT_CATALOG_GATE_LEVELS,
    "0.6.0": CURRENT_CATALOG_GATE_LEVELS,
}
```

At the top of `_raw_observation`, add:

```python
if str(test.get("id")) == "057" and test.get("detected"):
    return str(test["detected"])
```

Leave other test IDs unchanged.

- [ ] **Step 5: Update evaluation rules**

Make `0.6.0` the current map. State that historical `0.5.0` and `0.4.0` use the
same 62-item gate mapping, and retain the `0.3.0` map. Append this paragraph to
the performance section:

```markdown
For test `057` in v0.6.0, review all 4、8、16、32 concurrent waves. Treat
`time_total` as complete-response latency, not TTFB, TTFT, throughput, or token
generation speed. Report semantic-success count, rate-limit count, P50,
nearest-rank P95, and maximum for every wave. The four short waves are a
capacity and latency snapshot and do not constitute an SLA or sustained-load
claim. A 2xx response with the wrong exact marker is a failed sample.
```

- [ ] **Step 6: Run focused report verification**

```bash
python3 -m unittest \
  tests.test_model_doctor_assessment \
  tests.test_model_doctor_skill \
  tests.test_model_doctor_html \
  -v
```

Expected: all tests pass. The HTML renderer needs no change because it already
escapes and displays `rawObservation`.

- [ ] **Step 7: Commit**

```bash
git add \
  tests/test_model_doctor_assessment.py \
  tests/test_model_doctor_skill.py \
  skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py \
  skills/creating-model-doctor-reports/references/evaluation-rules.md
git commit -m "feat: report concurrency latency ladder"
```

### Task 4: Verify the Complete Feature

**Files:**
- Verify: all files listed above

- [ ] **Step 1: Run syntax and catalog checks**

```bash
bash -n model-capability-doctor.sh
./model-capability-doctor.sh --help
./model-capability-doctor.sh --list-tests | awk -F '\t' 'END { print NR, $1, $3 }'
```

Expected help version: `0.6.0`. Expected catalog summary:

```text
62 062 英文安全词可用性
```

- [ ] **Step 2: Run the complete suite**

```bash
python3 -m unittest discover -s tests -v
```

Expected: all tests pass.

- [ ] **Step 3: Check whitespace, stale current text, and scope**

```bash
git diff --check
rg -n 'SCRIPT_VERSION="0\.5\.0"|057[[:space:]]+性能与稳定性[[:space:]]+8 并发性能|当前 v0\.5\.0' \
  model-capability-doctor.sh tests skills/creating-model-doctor-reports
git status --short
```

Expected: no whitespace errors or stale current-contract matches. Status may
also show the user's pre-existing untracked `gpt-5.5-*` artifacts; do not stage
or modify them.
