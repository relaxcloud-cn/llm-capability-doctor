# Remove Reasoning Token And Disambiguate Checks Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Release one new Model Doctor contract that removes check 034 and makes checks 020 and 022 mechanically unambiguous.

**Architecture:** The Rust collector owns the evidence contract and request catalog; the Python report skill owns exact contract admission, assessment assembly, validation, and rendering. Both sides move together to `v0.12.0 / evidence.v4 / assessment.v9`, accept only the new evidence pair, and share one 46-check partition with 32 core and 14 enhanced checks.

**Tech Stack:** Rust, Bash reference collector, Python 3 standard library, JSON Schema, `unittest`, HTML/CSS.

---

### Task 1: Lock The New Collector Contract With Failing Tests

**Files:**
- Modify: `src/catalog.rs`
- Modify: `src/checks/content.rs`
- Modify: `src/checks/mod.rs`
- Modify: `src/audit.rs`
- Modify: `tests/cli.rs`

- [ ] **Step 1: Write failing Rust tests for the 46-check catalog and v4 contract**

Change the catalog invariant to require 46 unique IDs, assert that `034` is absent, assert the evidence schema is `llm-capability-doctor.evidence.v4`, and add prompt assertions requiring check 020 to describe literal ASCII vertical bars and check 022 to use `labels=URGENT,DATABASE` plus `excluded_label=NETWORK`.

- [ ] **Step 2: Run the focused Rust tests and verify RED**

Run: `cargo test catalog_has_46_unique_ids audit::tests checks::content::tests -- --nocapture`

Expected: FAIL because the catalog still has 47 checks, evidence is still v3, check 034 still exists, and the prompts still use the old ambiguous wording.

- [ ] **Step 3: Implement the minimal collector changes**

Set `Cargo.toml` to `0.12.0`, regenerate `Cargo.lock`, set `LOG_SCHEMA` to `llm-capability-doctor.evidence.v4`, remove `034` from the catalog/planner/dispatcher, and replace the prompts with these requirements:

```text
020: The two ASCII vertical bar characters "|" are literal output characters and must both be present.
022: Copy labels only from labels=URGENT,DATABASE; never copy excluded_label=NETWORK and never infer labels.
```

- [ ] **Step 4: Run the Rust suite and verify GREEN**

Run: `cargo test`

Expected: PASS with 46 unique checks and no route for `034`.

- [ ] **Step 5: Commit the collector contract**

```bash
git add Cargo.toml Cargo.lock src tests
git commit -m "feat: publish 46-check evidence v4 contract"
```

### Task 2: Make The Report Pipeline Accept Only Evidence V4

**Files:**
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_contracts.py`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_parser.py`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_assessment.py`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_opencodex_compatibility.py`
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_parser.py`
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_assessment.py`

- [ ] **Step 1: Write failing Python tests for strict v4 admission**

Add tests that accept only `("llm-capability-doctor.evidence.v4", "0.12.0")`, reject v1/v2/v3 pairs, require 46 observed checks, require 32 core and 14 enhanced checks, and verify `034` is not in any supported partition.

- [ ] **Step 2: Run the focused tests and verify RED**

Run: `python3 -m unittest skills.creating-model-doctor-reports.tests.test_model_doctor_parser skills.creating-model-doctor-reports.tests.test_model_doctor_assessment`

Expected: FAIL because the current code accepts v1-v3 and emits assessment v8.

- [ ] **Step 3: Implement strict v4 parsing and assessment v9**

Define only:

```python
V4_CONTRACT = ("llm-capability-doctor.evidence.v4", "0.12.0")
ASSESSMENT_SCHEMA = "llm-capability-doctor.assessment.v9"
```

Build `V4_TEST_IDS` from the current v3 IDs minus `034`, keep all 32 existing core IDs, remove `034` from enhanced IDs, and make parser/compatibility admission reject every non-v4 contract pair.

- [ ] **Step 4: Run the focused tests and verify GREEN**

Run: `python3 -m unittest skills.creating-model-doctor-reports.tests.test_model_doctor_parser skills.creating-model-doctor-reports.tests.test_model_doctor_assessment`

Expected: PASS with v4-only admission and assessment v9 output.

- [ ] **Step 5: Commit the report contract**

```bash
git add skills/creating-model-doctor-reports/scripts skills/creating-model-doctor-reports/tests
git commit -m "feat: require evidence v4 reports"
```

### Task 3: Update Evaluation, Schema, And Rendering

**Files:**
- Modify: `skills/creating-model-doctor-reports/references/evaluation-rules.md`
- Modify: `skills/creating-model-doctor-reports/references/assessment-schema.json`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_display.py`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_html.py`
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_rules.py`
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_schema.py`
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_html.py`

- [ ] **Step 1: Write failing tests for exact check 022 order and hidden check 034**

Require check 022 to pass only for `"labels":["URGENT","DATABASE"]`, fail extra/reordered labels, require rendered metadata to use assessment v9, and assert rendered HTML contains no check `034` or reasoning-token issue.

- [ ] **Step 2: Run the focused tests and verify RED**

Run: `python3 -m unittest skills.creating-model-doctor-reports.tests.test_model_doctor_rules skills.creating-model-doctor-reports.tests.test_model_doctor_schema skills.creating-model-doctor-reports.tests.test_model_doctor_html`

Expected: FAIL on the old unordered-label rule, assessment v8 schema, and visible check 034 mapping.

- [ ] **Step 3: Implement rules, schema, and renderer changes**

Remove rule/display entries for `034`, make 022 compare the ordered array exactly, change schema `$id` and `schema` const to assessment v9, constrain run input to evidence v4/v0.12.0, and update total/core/enhanced bounds to 46/32/14.

- [ ] **Step 4: Run the focused tests and verify GREEN**

Run: `python3 -m unittest skills.creating-model-doctor-reports.tests.test_model_doctor_rules skills.creating-model-doctor-reports.tests.test_model_doctor_schema skills.creating-model-doctor-reports.tests.test_model_doctor_html`

Expected: PASS with exact labels and no check 034 output path.

- [ ] **Step 5: Commit report behavior**

```bash
git add skills/creating-model-doctor-reports/references skills/creating-model-doctor-reports/scripts skills/creating-model-doctor-reports/tests
git commit -m "fix: remove reasoning token report check"
```

### Task 4: Align The Skill, README, And Reference Collector

**Files:**
- Modify: `skills/creating-model-doctor-reports/SKILL.md`
- Modify: `skills/creating-model-doctor-reports/tests/test_skill_instructions.py`
- Modify: `README.md`
- Modify: `model-capability-doctor.sh`

- [ ] **Step 1: Write failing documentation contract tests**

Assert that the skill names only v0.12.0/evidence.v4/assessment.v9, says 46 total/32 core/14 enhanced, and contains no active compatibility table or evaluation entry for check 034.

- [ ] **Step 2: Run the skill instruction tests and verify RED**

Run: `python3 -m unittest skills.creating-model-doctor-reports.tests.test_skill_instructions`

Expected: FAIL because the skill still documents v1-v3, 47 checks, assessment v8, and check 034.

- [ ] **Step 3: Update instructions and the reference shell collector**

Document only the new contract in `SKILL.md` and `README.md`. In the shell reference, remove the `034` case and route, change its total from 62 to 61, and apply the exact same unambiguous 020/022 prompts as the Rust collector.

- [ ] **Step 4: Run documentation and shell checks**

Run: `python3 -m unittest skills.creating-model-doctor-reports.tests.test_skill_instructions && bash -n model-capability-doctor.sh`

Expected: PASS and no shell syntax errors.

- [ ] **Step 5: Commit docs and reference implementation**

```bash
git add README.md model-capability-doctor.sh skills/creating-model-doctor-reports/SKILL.md skills/creating-model-doctor-reports/tests/test_skill_instructions.py
git commit -m "docs: document the 46-check contract"
```

### Task 5: Verify The Release End To End

**Files:**
- Modify only if verification exposes an in-scope defect.

- [ ] **Step 1: Run all automated tests**

Run: `cargo test && python3 -m unittest discover -s skills/creating-model-doctor-reports/tests -p 'test_*.py'`

Expected: PASS for the complete Rust and report-skill suites.

- [ ] **Step 2: Build the release collector**

Run: `cargo build --release`

Expected: PASS and create `target/release/model-capability-doctor` version 0.12.0.

- [ ] **Step 3: Verify the active source has no check 034 contract**

Run: `rg -n '034|reasoning token|reasoning_token' src README.md model-capability-doctor.sh skills/creating-model-doctor-reports --glob '!tests/fixtures/**'`

Expected: no active catalog, prompt, rule, display mapping, or instruction for check 034; negative regression tests may mention it explicitly.

- [ ] **Step 4: Verify the working tree and commit any verification fix**

Run: `git diff --check && git status --short`

Expected: no whitespace errors and no uncommitted tracked changes. If an in-scope verification fix was necessary, commit it with `git commit -m "test: verify evidence v4 release"`.
