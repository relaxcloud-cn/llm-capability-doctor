# Skill-Only Assessment v2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Skill's semantic review the only formal verdict in assessment JSON, customer HTML, summaries, rules, and documentation.

**Architecture:** Keep log parsing as lossless temporary evidence collection, but stop copying the parsed script result or original test object into final artifacts. Upgrade the canonical assessment schema to v2, derive every aggregate from `reviewedStatus`, and render only that status and its conclusion.

**Tech Stack:** Python 3 standard library, `unittest`, JSON Schema document, self-contained HTML/CSS/JavaScript.

---

### Task 1: Upgrade the canonical assessment contract to v2

**Files:**
- Modify: `tests/test_model_doctor_assessment.py`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py`
- Modify: `skills/creating-model-doctor-reports/references/assessment-schema.json`

- [ ] **Step 1: Write failing v2 contract tests**

Replace the test that expects both verdicts with a Skill-only contract and add an explicit schema-version assertion:

```python
def test_assemble_assessment_uses_reviewed_status_as_only_formal_verdict(self):
    parsed = parse_log(FIXTURES / "minimal.log")
    review = valid_review(status="FAIL")

    result = assemble_assessment(parsed, {"001": review})
    item = result["tests"][0]

    self.assertEqual(result["schemaVersion"], "llm-capability-doctor.assessment.v2")
    self.assertEqual(item["reviewedStatus"], "FAIL")
    for removed in ("originalStatus", "discrepancy", "originalTest"):
        self.assertNotIn(removed, item)
    self.assertEqual(result["overall"]["counts"]["FAIL"], 1)
```

Add this schema contract test:

```python
def test_assessment_schema_declares_skill_only_v2_contract(self):
    schema_path = ROOT / "skills" / "creating-model-doctor-reports" / "references" / "assessment-schema.json"
    schema = json.loads(schema_path.read_text(encoding="utf-8"))
    test_schema = schema["properties"]["tests"]["items"]

    self.assertEqual(schema["$id"], "llm-capability-doctor.assessment.v2")
    self.assertEqual(
        schema["properties"]["schemaVersion"]["const"],
        "llm-capability-doctor.assessment.v2",
    )
    self.assertIn("reviewedStatus", test_schema["required"])
    for removed in ("originalStatus", "discrepancy", "originalTest"):
        self.assertNotIn(removed, test_schema["required"])
        self.assertNotIn(removed, test_schema.get("properties", {}))
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
python3 -m unittest tests.test_model_doctor_assessment -v
```

Expected: FAIL because the current schema version is v1 and assembled tests still contain `originalStatus`, `discrepancy`, and `originalTest`.

- [ ] **Step 3: Implement the minimal v2 assessment**

In `model_doctor_assessment.py`, set:

```python
ASSESSMENT_SCHEMA_VERSION = "llm-capability-doctor.assessment.v2"
```

Build each assessment test without reading or copying the parsed result:

```python
items.append(
    {
        "testId": test_id,
        "category": test.get("category", "Unclassified"),
        "name": test.get("name", f"Test {test_id}"),
        "gateLevel": review["gateLevel"],
        "reviewedStatus": review["reviewedStatus"],
        "confidence": review["confidence"],
        "conclusion": review["conclusion"],
        "logic": review["logic"],
        "rawObservation": _raw_observation(test, requests),
        "metrics": [request.get("metrics", {}) for request in requests],
        "evidenceRefs": review["evidenceRefs"],
        "evidenceExcerpts": review["evidenceExcerpts"],
        "limitations": review["limitations"],
        "retestInstructions": review["retestInstructions"],
        "requests": requests,
    }
)
```

Update `assessment-schema.json` `$id`, schema-version constant, and required test properties to v2. Remove `originalStatus` from the required list.

- [ ] **Step 4: Run focused and full assessment tests**

Run:

```bash
python3 -m unittest tests.test_model_doctor_assessment -v
```

Expected: all assessment tests PASS.

- [ ] **Step 5: Commit Task 1**

```bash
git add tests/test_model_doctor_assessment.py \
  skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py \
  skills/creating-model-doctor-reports/references/assessment-schema.json
git commit -m "升级仅保留Skill判定的评估v2"
```

### Task 2: Remove script verdicts from summaries, HTML, rules, and documentation

**Files:**
- Modify: `tests/test_model_doctor_log.py`
- Modify: `tests/test_model_doctor_html.py`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_report.py`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_html.py`
- Modify: `skills/creating-model-doctor-reports/assets/report.css`
- Modify: `skills/creating-model-doctor-reports/SKILL.md`
- Modify: `skills/creating-model-doctor-reports/references/evaluation-rules.md`
- Modify: `README.md`

- [ ] **Step 1: Write failing summary and HTML tests**

In the CLI summary test, assert:

```python
self.assertNotIn("originalStatusCounts", summary_value)
```

Replace the discrepancy HTML test with:

```python
def test_summary_row_uses_only_reviewed_status_and_conclusion(self):
    value = assessment("FAIL")
    html = render_report(value, ASSET_DIR)

    self.assertIn('class="result-group" data-status="FAIL"', html)
    self.assertIn('<span class="status status-FAIL">失败</span>', html)
    for removed in ("判定发生变化", "脚本原判", "原始判断", "discrepancy-note"):
        self.assertNotIn(removed, html)
```

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```bash
python3 -m unittest \
  tests.test_model_doctor_log.ModelDoctorCliLogTests.test_parse_summary_and_packet_commands \
  tests.test_model_doctor_html.ModelDoctorHtmlTests.test_summary_row_uses_only_reviewed_status_and_conclusion -v
```

Expected: FAIL because the summary still emits `originalStatusCounts` and HTML still renders the discrepancy marker.

- [ ] **Step 3: Remove runtime discrepancy behavior**

Delete `originalStatusCounts` from `_summary()` in `model_doctor_report.py`.

In `_test_row_group()` remove the `discrepancy` lookup and note construction. Render the conclusion directly:

```python
f'<span>{_e(item.get("conclusion"))}</span>'
```

Delete `.discrepancy-note` selectors from `report.css`.

- [ ] **Step 4: Make Skill rules and public documentation Skill-only**

Change the Skill overview to preserve every discovered test, full redacted inputs/outputs, and the Skill-reviewed result, without promising the script result.

Replace the two discrepancy-oriented cross-cutting rules with:

```markdown
1. Treat `reviewedStatus` as the only formal assessment verdict.
8. Judge observable evidence independently; never use a parsed script result as assessment ground truth or copy it into final artifacts.
```

Update the README artifact and report descriptions so they describe the canonical Skill assessment, reviewed status, five expandable evidence sections, and no discrepancy filter or original verdict.

- [ ] **Step 5: Run focused tests, complete tests, and source scan**

Run:

```bash
python3 -m unittest tests.test_model_doctor_log tests.test_model_doctor_html -v
python3 -m unittest discover -s tests -v
rg -n "originalStatusCounts|判定发生变化|discrepancy-note|originalStatus|originalTest" \
  skills/creating-model-doctor-reports README.md tests
```

Expected: all tests PASS. The source scan may match negative assertions in tests and removal language in the schema design, but must not match production code, Skill instructions, CSS, or README.

- [ ] **Step 6: Commit Task 2**

```bash
git add README.md tests/test_model_doctor_log.py tests/test_model_doctor_html.py \
  skills/creating-model-doctor-reports/SKILL.md \
  skills/creating-model-doctor-reports/references/evaluation-rules.md \
  skills/creating-model-doctor-reports/scripts/model_doctor_report.py \
  skills/creating-model-doctor-reports/scripts/model_doctor_html.py \
  skills/creating-model-doctor-reports/assets/report.css
git commit -m "移除最终产物中的脚本原判"
```

### Task 3: Regenerate and verify a real v2 customer report

**Files:**
- Read only: `/Users/libolun/projects/llm-capability-doctor/gpt-5.5-model-doctor.log`
- Read only: `/Users/libolun/Desktop/testllm/gpt-5.5-assessment.json`
- Create: timestamped files under `/Users/libolun/Desktop/testllm/`

- [ ] **Step 1: Reconstruct semantic reviews without modifying prior artifacts**

Parse the source log to a temporary directory. Build temporary `reviews.json` by selecting the existing semantic review fields from each v1 assessment test: `testId`, `reviewedStatus`, `confidence`, `gateLevel`, `conclusion`, `logic`, `evidenceRefs`, `evidenceExcerpts`, `limitations`, and `retestInstructions`.

Do not copy `originalStatus`, `discrepancy`, or `originalTest`. Do not write into the repository or overwrite the source assessment.

- [ ] **Step 2: Validate and render with the official CLI**

Run `validate`, then `render` with new timestamped v2 JSON and HTML paths under `/Users/libolun/Desktop/testllm/`.

Expected: the CLI prints two new absolute output paths and leaves every existing artifact unchanged.

- [ ] **Step 3: Verify static artifact invariants**

Assert that the new assessment has schema version v2 and recursively contains none of the removed keys. Assert that the HTML contains two tables, 113 result groups, 113 evidence rows, 165 request inputs, 165 request outputs, and no external `src` or `href` resource.

- [ ] **Step 4: Validate the Skill and complete regression suite**

Run:

```bash
python3 -m unittest discover -s tests -v
/tmp/model-doctor-skill-validator/bin/python \
  /Users/libolun/.codex/skills/.system/skill-creator/scripts/quick_validate.py \
  skills/creating-model-doctor-reports
node --check skills/creating-model-doctor-reports/assets/report.js
python3 -m py_compile skills/creating-model-doctor-reports/scripts/*.py
git diff --check
```

Expected: zero failures, `Skill is valid!`, and no syntax or whitespace errors.

- [ ] **Step 5: Browser-verify the new report**

Serve `/Users/libolun/Desktop/testllm` locally. In the in-app browser verify desktop 736px layout, one-row expansion, the five approved evidence sections, independent expansion, 390px mobile layout, no horizontal overflow, and no console errors. Confirm no row displays a script verdict or change marker.

- [ ] **Step 6: Record the final implementation state**

Update the implementation checklist, confirm the worktree is clean, and report the v2 JSON path, HTML path, localhost URL, test count, and integration branch.
