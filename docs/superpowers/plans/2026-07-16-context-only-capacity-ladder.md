# Context-Only Capacity Ladder Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace all generated long-output tests with a single 8K-128K input-context capacity ladder and publish a versioned 62-test catalog.

**Architecture:** Keep protocol request construction and evidence recording in the existing Bash script, but separate capacity-level context checks from positional context checks. Remove the long-output code path entirely, migrate downstream IDs once, and make the report validator select a fixed gate map by script version so `0.3.0` logs remain valid while `0.4.0` logs use the new catalog.

**Tech Stack:** Bash 3.2-compatible shell, Python 3 standard library `unittest`, Markdown documentation.

---

## File Map

- Modify `model-capability-doctor.sh`: publish catalog `0.4.0`, implement the capacity-context handler, remove long-output code, and migrate downstream IDs.
- Modify `tests/test_model_capability_doctor_script.py`: specify the new catalog, source contract, runtime context evidence, and shifted markers before implementation.
- Modify `skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py`: add version-specific `0.3.0` and `0.4.0` gate maps.
- Modify `tests/test_model_doctor_assessment.py`: specify both historical and current mappings and gate validation.
- Modify `skills/creating-model-doctor-reports/references/evaluation-rules.md`: document the current 62-item map and preserve the historical 65-item map.
- Modify `tests/test_model_doctor_skill.py`: enforce current and historical priority documentation.
- Modify `README.md`: document context-only targeted runs, 62 tests, and character/token boundaries.
- Modify `docs/superpowers/plans/2026-07-16-context-only-capacity-ladder.md`: check off executed plan steps.

### Task 1: Specify The Context-Only Shell Contract

**Files:**
- Modify: `tests/test_model_capability_doctor_script.py`
- Test: `tests/test_model_capability_doctor_script.py`

- [x] **Step 1: Replace the catalog and version expectations with the approved contract**

Update the existing contract tests to require:

```python
self.assertIn('SCRIPT_VERSION="0.4.0"', source)
self.assertIn("62-item core catalog", result.stdout)
self.assertEqual([row[0] for row in rows], [f"{value:03d}" for value in range(1, 63)])
self.assertEqual(
    {identifier: (categories[identifier], names[identifier]) for identifier in ("014", "015", "016", "017", "018")},
    {
        "014": ("上下文", "8K 级上下文（字符近似）"),
        "015": ("上下文", "16K 级上下文（字符近似）"),
        "016": ("上下文", "32K 级上下文（字符近似）"),
        "017": ("上下文", "64K 级上下文（字符近似）"),
        "018": ("上下文", "128K 级上下文（字符近似）"),
    },
)
```

Assert representative shifted IDs: `026=开头信息召回`, `032=Thinking 参数接受`, `040=单工具调用`, `051=冷请求总延迟`, and `059=越权请求护栏`.

- [x] **Step 2: Replace long-output source assertions with capacity-context assertions**

Require these exact source facts:

```python
for expected in (
    '014) echo 32000 ;;',
    '015) echo 64000 ;;',
    '016) echo 128000 ;;',
    '017) echo 256000 ;;',
    '018) echo 512000 ;;',
    '014|015|016|017|018) run_core_context_capacity_test',
):
    self.assertIn(expected, source)

for removed in (
    "long_output_body()",
    "perform_long_output_request()",
    "run_core_long_output_test()",
    "MODEL_DOCTOR_OUTPUT_",
    "LONG_OUTPUT_",
):
    self.assertNotIn(removed, source)
```

Update the shifted marker assertions to the new IDs, including `ctx_029_a`, `test-055-repeat-${index}`, and the absence of their `0.3.0` forms.

- [x] **Step 3: Add a fake-curl runtime test for one capacity request**

Run only test `014` against a fake curl that returns a protocol probe response for the first request and `CTX_014_OK` with `usage.prompt_tokens=12000` for the capacity request. Assert that the audit log contains:

```python
def test_context_capacity_request_records_characters_and_observed_input_tokens(self):
    with tempfile.TemporaryDirectory() as temporary_directory:
        directory = Path(temporary_directory)
        fake_curl = directory / "curl"
        log_path = directory / "capacity.log"
        fake_curl.write_text(
            """#!/usr/bin/env bash
if [[ "${1:-}" == "--version" ]]; then
  echo "curl mock"
  exit 0
fi
output_file=""
headers_file=""
request_body=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output) output_file="$2"; shift 2 ;;
    --dump-header) headers_file="$2"; shift 2 ;;
    --write-out) shift 2 ;;
    --data-binary) request_body="$2"; shift 2 ;;
    *) shift ;;
  esac
done
if [[ "$request_body" == *"CTX_014_OK"* ]]; then
  content="CTX_014_OK"
else
  content="MODEL_DOCTOR_PROTOCOL_OK"
fi
printf '{"object":"chat.completion","choices":[{"message":{"content":"%s"},"finish_reason":"stop"}],"usage":{"prompt_tokens":12000,"completion_tokens":8,"total_tokens":12008}}' "$content" >"$output_file"
printf 'HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n' >"$headers_file"
printf '200\t0.010\t0.005\t200'
""",
            encoding="utf-8",
        )
        fake_curl.chmod(0o755)
        environment = dict(os.environ)
        environment["PATH"] = f"{directory}:{environment['PATH']}"

        result = subprocess.run(
            [
                "bash",
                str(SCRIPT),
                "--url",
                "https://model.example/v1/chat/completions",
                "--model",
                "test-model",
                "--api-key",
                "test-key",
                "--only",
                "014",
                "--log-file",
                str(log_path),
            ],
            cwd=ROOT,
            env=environment,
            capture_output=True,
            text=True,
            check=False,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        log_text = log_path.read_text(encoding="utf-8")
        self.assertIn("test_id: 014", log_text)
        self.assertIn("request_chars=32000,input_tokens=12000", log_text)
        self.assertIn("CTX_014_OK", log_text)
        self.assertIn("字符负载", log_text)
        self.assertIn("输入 Token", log_text)
```

The fake curl must capture `--data-binary`, write HTTP 200 headers, and return protocol-shaped JSON without contacting an endpoint.

- [x] **Step 4: Run the focused tests and verify RED**

Run:

```bash
python3 -m unittest tests.test_model_capability_doctor_script -v
```

Expected: FAIL because the source still reports `0.3.0`, 65 tests, long-output handlers, and old IDs.

- [x] **Step 5: Commit the failing contract tests**

```bash
git add tests/test_model_capability_doctor_script.py
git commit -m "test: define context-only capacity catalog"
```

### Task 2: Implement The 62-Test Shell Catalog

**Files:**
- Modify: `model-capability-doctor.sh`
- Test: `tests/test_model_capability_doctor_script.py`

- [x] **Step 1: Publish the new version and catalog**

Set:

```bash
SCRIPT_VERSION="0.4.0"
```

Change help text to `62-item core catalog`. Replace tests `014-018` with the five `上下文` capacity levels, delete duplicate entries `026-028`, and shift current `029-065` entries backward by three IDs exactly as specified in the design.

- [x] **Step 2: Add input-token extraction and the uniform capacity handler**

Add a protocol-neutral extractor:

```bash
input_token_count() {
  local file="$1"
  grep -Eo '"(prompt_tokens|input_tokens|promptTokenCount|prompt_eval_count)"[[:space:]]*:[[:space:]]*[0-9]+' "$file" 2>/dev/null \
    | tail -n 1 | grep -Eo '[0-9]+$' || true
}

core_context_capacity_chars() {
  case "$1" in
    014) echo 32000 ;;
    015) echo 64000 ;;
    016) echo 128000 ;;
    017) echo 256000 ;;
    018) echo 512000 ;;
  esac
}
```

Implement `run_core_context_capacity_test` to generate filler, place `CTX_${id}_OK` at the end, perform one non-streaming request, extract visible text and input tokens, and record:

```bash
expected="CTX_${id}_OK"
detected="request_chars=${chars},input_tokens=${input_tokens:-unknown}"
```

Use `ERROR` for curl failures, `FAIL` for `400|413|422` and missing markers, `UNDETERMINED` for unknown protocols, and `PASS` only for exact marker recall. The success conclusion must report characters and observed input tokens separately.

- [x] **Step 3: Remove the long-output path**

Delete `long_output_body`, `output_token_count`, `response_is_truncated`, `perform_long_output_request`, `core_long_structure_complete`, `run_core_long_output_test`, `OUTPUT_8K_*`, and `LONG_OUTPUT_*`. Keep `normalize_json_text` because tool-result tests still use it.

- [x] **Step 4: Migrate downstream handlers and internal markers**

Shift all hard-coded IDs and dispatch ranges:

```bash
014|015|016|017|018) run_core_context_capacity_test "$id" "$category" "$name" ;;
019|020|021|022|023|024|025|037|038|039) run_core_text_test "$id" "$category" "$name" ;;
026|027|028|029|030|031) run_core_context_test "$id" "$category" "$name" ;;
032|033|034|035|036) run_core_thinking_test "$id" "$category" "$name" ;;
040|041|042|043|044|045|046|047|048|049|050) run_core_tool_test "$id" "$category" "$name" ;;
051|052|053|054|055|056|057|058) run_core_performance_test "$id" "$category" "$name" ;;
059|060|061|062) run_core_guardrail_test "$id" "$category" "$name" ;;
```

Update prompt markers, follow-up request IDs, repeated-sample IDs, and any test-specific branches by the same migration table. Do not change test semantics besides IDs.

- [x] **Step 5: Run focused tests and shell syntax verification**

Run:

```bash
bash -n model-capability-doctor.sh
python3 -m unittest tests.test_model_capability_doctor_script -v
```

Expected: shell syntax succeeds and all script contract tests pass.

- [x] **Step 6: Commit the shell implementation**

```bash
git add model-capability-doctor.sh
git commit -m "feat: replace long output with context capacity tests"
```

### Task 3: Version The Report Gate Maps

**Files:**
- Modify: `tests/test_model_doctor_assessment.py`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py`
- Test: `tests/test_model_doctor_assessment.py`

- [x] **Step 1: Write failing tests for `0.4.0` and historical `0.3.0` maps**

Specify the current map:

```python
def id_range(first, last):
    return {f"{value:03d}" for value in range(first, last + 1)}

all_ids = id_range(1, 62)
expected_critical = id_range(1, 6) | id_range(40, 50)
expected_important = id_range(14, 18) | id_range(32, 36) | {"057"}
expected_observation = all_ids - expected_critical - expected_important
```

Keep a separate assertion for the historical `0.3.0` map currently encoded by the tests. Add validation cases proving a wrong `0.4.0` gate and a wrong `0.3.0` gate are both rejected.

- [x] **Step 2: Run the focused assessment tests and verify RED**

```bash
python3 -m unittest tests.test_model_doctor_assessment -v
```

Expected: FAIL because the module has only one `0.3.0` map.

- [x] **Step 3: Implement version-specific maps**

Define:

```python
def _catalog_gate_levels(catalog_size: int, critical: set[str], important: set[str]) -> Dict[str, str]:
    all_ids = {f"{test_id:03d}" for test_id in range(1, catalog_size + 1)}
    observation = all_ids - critical - important
    return {
        **{test_id: "critical" for test_id in critical},
        **{test_id: "important" for test_id in important},
        **{test_id: "observation" for test_id in observation},
    }

V0_3_CATALOG_GATE_LEVELS = _catalog_gate_levels(
    65,
    {f"{test_id:03d}" for test_id in (*range(1, 7), *range(43, 54))},
    {f"{test_id:03d}" for test_id in (*range(26, 29), *range(35, 40), 60)},
)
CURRENT_CATALOG_GATE_LEVELS = _catalog_gate_levels(
    62,
    {f"{test_id:03d}" for test_id in (*range(1, 7), *range(40, 51))},
    {f"{test_id:03d}" for test_id in (*range(14, 19), *range(32, 37), 57)},
)
CATALOG_GATE_LEVELS = {
    "0.3.0": V0_3_CATALOG_GATE_LEVELS,
    "0.4.0": CURRENT_CATALOG_GATE_LEVELS,
}
```

In `validate_reviews`, select the map by `parsed["run"]["script_version"]` and enforce a gate only when that version has a fixed map. Preserve the current product-principle behavior for older or unknown versions.

- [x] **Step 4: Run assessment tests and verify GREEN**

```bash
python3 -m unittest tests.test_model_doctor_assessment -v
```

Expected: all assessment tests pass.

- [x] **Step 5: Commit the report mapping implementation**

```bash
git add tests/test_model_doctor_assessment.py skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py
git commit -m "feat: version model doctor gate mappings"
```

### Task 4: Update Skill Rules And User Documentation

**Files:**
- Modify: `tests/test_model_doctor_skill.py`
- Modify: `skills/creating-model-doctor-reports/references/evaluation-rules.md`
- Modify: `README.md`
- Test: `tests/test_model_doctor_skill.py`

- [x] **Step 1: Write failing skill-rule assertions**

Require the rules to state:

```python
self.assertIn("当前 v0.4.0 的 62 项目录使用固定优先级", text)
self.assertIn("`critical`：`001-006`、`040-050`", text)
self.assertIn("`important`：`014-018`、`032-036`、`057`", text)
self.assertIn("重要检测项共 28 项", text)
self.assertIn("次要检测项共 34 项", text)
self.assertIn("历史 v0.3.0", text)
```

- [x] **Step 2: Run the skill tests and verify RED**

```bash
python3 -m unittest tests.test_model_doctor_skill -v
```

Expected: FAIL because the rules still call `0.3.0` current and report 26/39.

- [x] **Step 3: Update rules and README**

Document the `0.4.0` map first, followed by the preserved `0.3.0` map. Replace README long-output examples with:

```bash
./model-capability-doctor.sh \
  --url 'https://model.example/v1/chat/completions' \
  --model 'your-model-name' \
  --only '014,015,016,017,018' \
  --log-file './context-capacity.log'
```

State that the payloads use 32K/64K/128K/256K/512K characters as approximations, observed input token counts are reported separately, successful recall is a lower bound, and the catalog has 62 tests. Remove recommendations specific to generated long output.

- [x] **Step 4: Run skill and script tests**

```bash
python3 -m unittest tests.test_model_doctor_skill tests.test_model_capability_doctor_script -v
```

Expected: all selected tests pass.

- [x] **Step 5: Commit documentation and rule changes**

```bash
git add README.md tests/test_model_doctor_skill.py skills/creating-model-doctor-reports/references/evaluation-rules.md
git commit -m "docs: describe context-only model capacity checks"
```

### Task 5: Full Verification

**Files:**
- Verify: all modified files

- [x] **Step 1: Run the complete automated suite**

```bash
python3 -m unittest discover -s tests -v
```

Expected: all tests pass with zero failures or errors.

- [x] **Step 2: Run static and contract checks**

```bash
bash -n model-capability-doctor.sh
./model-capability-doctor.sh --help
./model-capability-doctor.sh --list-tests
git diff --check
```

Expected: valid shell syntax, help reports `0.4.0`/62 tests, the catalog has exactly 62 unique rows, and no whitespace errors exist.

- [x] **Step 3: Confirm long-output code is absent**

```bash
rg -n 'long_output|LONG_OUTPUT|MODEL_DOCTOR_OUTPUT_|完整 Result JSON|长输出结果' model-capability-doctor.sh README.md skills/creating-model-doctor-reports tests
```

Expected: no active-code or current-documentation matches. Historical design and plan documents are intentionally excluded from this check.

- [x] **Step 4: Inspect the final branch diff and commits**

```bash
git status --short
git diff main...HEAD --stat
git log --oneline main..HEAD
```

Expected: only approved implementation, tests, rules, README, and this tracked plan differ from `main`; the branch contains focused commits for each task.
