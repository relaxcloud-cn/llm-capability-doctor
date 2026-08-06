# Customer-Facing General Capability Verdict Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every complete 46-check Model Doctor report lead with a deterministic customer-facing verdict of 通用能力通过、通用能力有条件通过, or 通用能力未通过, while incomplete historical logs display 通用能力未评定.

**Architecture:** Add one pure verdict module that owns the 31 core IDs, 15 enhanced IDs, fixed labels, counts, and statements. The assessment assembler injects its output, the validator independently recomputes it, and the HTML renderer only formats this structured result above the existing evidence-backed facts and issue summary. Skill-authored reviews remain unable to choose or override the verdict.

**Tech Stack:** Python 3 standard library, JSON Schema 2020-12, HTML/CSS, `unittest`, Model Doctor Skill validation scripts.

---

## Working Constraints

- Work in `/Users/libolun/.config/superpowers/worktrees/llm-capability-doctor/single-full-collection` on `codex/single-full-collection`, never on `main`.
- The first-byte timing label is already fixed by commit `ecdbc2b`; add verdict tests in a new test file to keep the feature isolated.
- Current baseline is 56 passing Skill tests.
- Do not change the collector, evidence.v1/v2 log formats, the 46 retained checks, or per-check PASS/FAIL semantics.
- Preserve v1 partial-log compatibility and v2 strict 46-check parsing.

### Task 1: Add the deterministic verdict engine

**Files:**
- Create: `skills/creating-model-doctor-reports/scripts/model_doctor_general_verdict.py`
- Create: `skills/creating-model-doctor-reports/tests/test_model_doctor_general_verdict.py`
- Modify: `docs/superpowers/specs/2026-08-06-customer-facing-general-capability-verdict-design.md`

- [ ] **Step 1: Write failing partition and verdict tests**

Create tests that import `RETAINED_TEST_IDS` and assert:

```python
self.assertEqual(CORE_TEST_IDS & ENHANCED_TEST_IDS, frozenset())
self.assertEqual(CORE_TEST_IDS | ENHANCED_TEST_IDS, frozenset(RETAINED_TEST_IDS))
self.assertEqual(len(CORE_TEST_IDS), 31)
self.assertEqual(len(ENHANCED_TEST_IDS), 15)
```

Cover these exact outcomes:

```python
all_pass -> PASS / 通用能力通过 / 46 collected / 46 passed
only_060_fails -> CONDITIONAL_PASS / 通用能力有条件通过 / core 31/31 / enhanced 14/15
only_001_fails -> FAIL / 通用能力未通过 / core 30/31
001_and_060_fail -> FAIL
only_001_present -> NOT_ASSESSED / 通用能力未评定 / 1 collected
```

Run:

```bash
python3 -m unittest skills/creating-model-doctor-reports/tests/test_model_doctor_general_verdict.py -v
```

Expected: `ImportError` or missing-module failure.

- [ ] **Step 2: Implement one pure derivation function**

Create constants with the approved partition and this public API:

```python
def derive_general_verdict(statuses: Mapping[str, str]) -> dict:
    """Return the complete deterministic generalVerdict object."""
```

The returned object has exactly:

```python
{
    "level",
    "label",
    "collectedTests",
    "passedTests",
    "totalTests",
    "passedCoreTests",
    "totalCoreTests",
    "passedEnhancedTests",
    "totalEnhancedTests",
    "statement",
}
```

Use fixed totals 46, 31, and 15. When the collected ID set is a strict subset of all 46 IDs, return `NOT_ASSESSED` before applying PASS/FAIL gates. Reject unknown IDs and unknown status values with `ValueError`.

Use these exact templates:

```text
本轮固定 46 项检测全部通过，因此判定通用能力通过。
本轮固定 46 项检测通过 {passed} 项，31 项基础必过项全部通过；{failed_enhanced} 项增强能力存在限制，因此判定通用能力有条件通过。
本轮固定 46 项检测通过 {passed} 项，其中 {failed_core} 项基础必过能力未满足，因此判定通用能力未通过。
本轮仅采集 {collected}/46 项，证据不足以生成通用能力等级，因此本轮通用能力未评定。
```

- [ ] **Step 3: Run the focused tests**

Run the Task 1 command again.

Expected: all verdict tests pass.

- [ ] **Step 4: Commit only Task 1-owned files**

```bash
git add skills/creating-model-doctor-reports/scripts/model_doctor_general_verdict.py \
  skills/creating-model-doctor-reports/tests/test_model_doctor_general_verdict.py \
  docs/superpowers/specs/2026-08-06-customer-facing-general-capability-verdict-design.md \
  docs/superpowers/plans/2026-08-06-customer-facing-general-capability-verdict.md
git commit -m "feat: define deterministic general capability verdict"
```

### Task 2: Make assessment.v6 generate and enforce the verdict

**Files:**
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py`
- Modify: `skills/creating-model-doctor-reports/references/assessment-schema.json`
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_general_verdict.py`

- [ ] **Step 1: Write failing assembly and tamper tests**

Build complete 46-item parsed/reviews fixtures by reusing the public fixture helpers or constructing a minimal item for every retained ID. Assert that:

```python
assessment["capabilitySummary"]["generalVerdict"] == derive_general_verdict(statuses)
validate_assessment(assessment) == []
```

Parametrize tampering over `level`, `label`, each count, and `statement`; each change must produce an error containing `generalVerdict does not match test statuses`.

Also assert reviews.v2 rejects a Skill-authored `generalVerdict` as an extra field, and a partial v1 assessment receives `NOT_ASSESSED`.

Run the focused test file. Expected: missing `generalVerdict` failures.

- [ ] **Step 2: Separate review and assessment summary fields**

In `model_doctor_assessment.py`:

```python
REVIEW_CAPABILITY_SUMMARY_FIELDS = {
    "headline", "verifiedFacts", "issues", "scopeBoundary"
}
ASSESSMENT_CAPABILITY_SUMMARY_FIELDS = (
    REVIEW_CAPABILITY_SUMMARY_FIELDS | {"generalVerdict"}
)
```

`validate_reviews` must use the review-only set. `assemble_assessment` must deep-copy the authored summary and inject:

```python
statuses = {item["testId"]: item["reviewedStatus"] for item in items}
capability_summary["generalVerdict"] = derive_general_verdict(statuses)
```

`_validate_assessment_summary` must allow the assessment set, recompute from unique valid test IDs, and reject any non-identical object. Malformed or duplicate assessment test IDs must keep producing validation errors without crashing.

- [ ] **Step 3: Extend assessment-schema.json**

Require `generalVerdict` in `$defs.capabilitySummary`. Add a closed `$defs.generalVerdict` object with:

```text
level enum: PASS, CONDITIONAL_PASS, FAIL, NOT_ASSESSED
label enum: 通用能力通过, 通用能力有条件通过, 通用能力未通过, 通用能力未评定
collectedTests/passedTests: integer 0..46
totalTests: const 46
passedCoreTests: integer 0..31
totalCoreTests: const 31
passedEnhancedTests: integer 0..15
totalEnhancedTests: const 15
statement: non-empty string
additionalProperties: false
```

- [ ] **Step 4: Run focused tests and schema validation**

```bash
python3 -m unittest skills/creating-model-doctor-reports/tests/test_model_doctor_general_verdict.py -v
python3 -m json.tool skills/creating-model-doctor-reports/references/assessment-schema.json >/dev/null
```

Expected: both commands exit 0.

- [ ] **Step 5: Commit Task 2**

```bash
git add skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py \
  skills/creating-model-doctor-reports/references/assessment-schema.json \
  skills/creating-model-doctor-reports/tests/test_model_doctor_general_verdict.py
git commit -m "feat: enforce general verdict in assessments"
```

### Task 3: Put the conclusion first in the customer HTML

**Files:**
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_html.py`
- Modify: `skills/creating-model-doctor-reports/assets/report.css`
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_general_verdict.py`

- [ ] **Step 1: Write failing renderer-order and escaping tests**

Assert this exact semantic order within `.final-conclusion`:

```text
最终结论
综合结论：{label}
{fixed statement}
{four fact-strip values}
{headline}
{issues, if any}
{scopeBoundary}
```

Cover all four level labels. Inject HTML metacharacters into every new displayed string and assert they are escaped. For partial logs assert the first fact reads `已采集 1/46` instead of `1/46 通过`.

- [ ] **Step 2: Render a stable fixed template**

Add helpers that render:

```html
<div class="general-verdict general-verdict-CONDITIONAL_PASS">
  <p class="general-verdict-label">综合结论：通用能力有条件通过</p>
  <p class="general-verdict-statement">...</p>
</div>
<dl class="capability-fact-strip">...</dl>
```

The four fact-strip labels/values are:

```text
检测结果 -> 40/46 通过 (or 已采集 N/46 for NOT_ASSESSED)
接口协议 -> verifiedFacts.interfaceProtocol.protocolLabel, else 未确认
上下文 -> verifiedFacts.contextWindow.highestVerifiedTier, else 未确认
并发 -> {highestVerifiedConcurrentRequests} 并发, else 未确认
```

Then render the existing evidence-backed detailed facts, headline, issues, and boundary. Do not infer model capability from URL, model name, or issue prose.

- [ ] **Step 3: Style desktop, mobile, dark, and print states**

Use restrained green/amber/red/gray status accents, stable `minmax(0, 1fr)` grid tracks, 8px or smaller radius, zero letter spacing, and no nested cards. On `max-width: 640px`, fact rows wrap without horizontal overflow. In print, preserve verdict colors as readable borders/text and prevent the verdict heading/statement from splitting.

Keep the existing `TTFB` label from commit `ecdbc2b` unchanged.

- [ ] **Step 4: Run renderer and full Skill tests**

```bash
python3 -m unittest skills/creating-model-doctor-reports/tests/test_model_doctor_general_verdict.py -v
python3 -m unittest discover -s skills/creating-model-doctor-reports/tests -p 'test_*.py' -v
```

Expected: all tests pass, including the pre-existing TTFB expectation.

- [ ] **Step 5: Commit only implementation-owned changes**

```bash
git add skills/creating-model-doctor-reports/scripts/model_doctor_html.py \
  skills/creating-model-doctor-reports/assets/report.css \
  skills/creating-model-doctor-reports/tests/test_model_doctor_general_verdict.py
git commit -m "feat: lead reports with customer verdict"
```

Do not modify `test_model_doctor_v6.py` unless an existing fixture must explicitly include the new required assessment field.

### Task 4: Make the Skill contract forbid authored verdicts

**Files:**
- Modify: `skills/creating-model-doctor-reports/SKILL.md`
- Modify: `skills/creating-model-doctor-reports/references/evaluation-rules.md`
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_general_verdict.py`

- [ ] **Step 1: Write failing documentation contract tests**

Assert the Skill and rules explicitly require:

```text
总体等级由 assemble_assessment 程序生成
reviews.v2 不得填写 generalVerdict
31 项基础必过项 / 15 项增强能力项
PASS / CONDITIONAL_PASS / FAIL / NOT_ASSESSED semantics
fixed customer-facing statement templates
no project READY/BLOCKED inference
```

- [ ] **Step 2: Update SKILL.md workflow**

Keep the three-stage parse -> semantic reviews -> validated assessment/report flow. In the reviews instructions, state that the author supplies evidence-backed `verifiedFacts`, headline, issues, and scope only. In the assembly/output instructions, state that the program injects and validates `generalVerdict`; reviewers must never choose it.

- [ ] **Step 3: Update evaluation-rules.md**

Document the exact core/enhanced partition, deterministic gates, partial-v1 `NOT_ASSESSED`, and exact fixed statements. Keep the current evidence sufficiency, dependency grouping, no-hard-limit, and reasoning-boundary rules unchanged.

- [ ] **Step 4: Validate the Skill**

```bash
python3 -m unittest skills/creating-model-doctor-reports/tests/test_model_doctor_general_verdict.py -v
python3 /Users/libolun/.codex/skills/.system/skill-creator/scripts/quick_validate.py \
  skills/creating-model-doctor-reports
```

Expected: tests pass and validator prints `Skill is valid!`.

- [ ] **Step 5: Commit Task 4**

```bash
git add skills/creating-model-doctor-reports/SKILL.md \
  skills/creating-model-doctor-reports/references/evaluation-rules.md \
  skills/creating-model-doctor-reports/tests/test_model_doctor_general_verdict.py
git commit -m "docs: define customer verdict contract"
```

### Task 5: Regenerate and visually verify the Doubao report

**Files:**
- Read: `/Users/libolun/model-doctor-output/doubao-1-5-pro-32k-250115-model-doctor-20260806-100857.log`
- Create: `/Users/libolun/model-doctor-output/doubao-1-5-pro-32k-250115-assessment-v6.json`
- Create: `/Users/libolun/model-doctor-output/doubao-1-5-pro-32k-250115-model-capability-report-v6.html`

- [ ] **Step 1: Run all automated verification**

```bash
python3 -m unittest discover -s skills/creating-model-doctor-reports/tests -p 'test_*.py' -v
python3 /Users/libolun/.codex/skills/.system/skill-creator/scripts/quick_validate.py \
  skills/creating-model-doctor-reports
cargo test --locked
```

Expected: all commands exit 0. If `cargo test` is blocked by unrelated dependency/network state, record the exact failure and still require all Skill tests to pass.

- [ ] **Step 2: Rebuild reviews.v2 from the existing evidence-backed findings**

Parse the source log with `model_doctor_log.py`. Reuse the prior 40 PASS / 6 FAIL semantic reviews only after validating every evidence reference against the new parsed JSON. Populate verified facts from direct evidence for OpenAI Chat Completions, the highest passing 32K approximate context tier, and the highest verified 32-request concurrency wave.

Run:

```bash
python3 skills/creating-model-doctor-reports/scripts/model_doctor_report.py validate \
  --parsed "$TMPDIR/parsed.json" --reviews "$TMPDIR/reviews.json" \
  --output /Users/libolun/model-doctor-output/doubao-1-5-pro-32k-250115-assessment-v6.json
python3 skills/creating-model-doctor-reports/scripts/model_doctor_report.py render \
  --assessment /Users/libolun/model-doctor-output/doubao-1-5-pro-32k-250115-assessment-v6.json \
  --output /Users/libolun/model-doctor-output/doubao-1-5-pro-32k-250115-model-capability-report-v6.html
chmod 600 /Users/libolun/model-doctor-output/doubao-1-5-pro-32k-250115-assessment-v6.json \
  /Users/libolun/model-doctor-output/doubao-1-5-pro-32k-250115-model-capability-report-v6.html
```

Never overwrite the prior v5 artifacts.

- [ ] **Step 3: Assert the accepted customer verdict**

Read the generated JSON and assert:

```text
passedTests = 40
passedCoreTests = 31
passedEnhancedTests = 9
level = CONDITIONAL_PASS
label = 通用能力有条件通过
```

Assert the HTML contains one `最终结论`, the exact fixed statement, `40/46 通过`, `OPENAI_CHAT_COMPLETIONS`'s customer label, the verified 32K tier, and `32 并发`. Assert the API key does not appear in either artifact.

- [ ] **Step 4: Perform browser visual QA**

Use the `browser:control-in-app-browser` Skill. Open the generated HTML, capture desktop 1440x900 and mobile 390x844 screenshots, and verify:

```text
verdict is visible in the first viewport
no text overlap or horizontal scrolling
fact strip wraps cleanly on mobile
details still expand
no external requests or blank sections
```

Fix and repeat if any check fails.

- [ ] **Step 5: Reinstall the updated project Skill and verify the installed copy**

Use the `skill-installer` instructions to reinstall from the worktree's `skills/creating-model-doctor-reports` directory without touching backup directories. Run quick validation against `/Users/libolun/.codex/skills/creating-model-doctor-reports` and confirm its `SKILL.md`, scripts, references, assets, and tests match the worktree version.

- [ ] **Step 6: Final verification and branch handoff**

Run `git status --short` and `git diff --check`; no unexplained dirty files are allowed. Then use `superpowers:verification-before-completion` and `superpowers:finishing-a-development-branch` before reporting completion.
