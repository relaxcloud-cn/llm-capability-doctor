# Hide Freeform Final Summary Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the red-boxed freeform summary from customer HTML reports without weakening assessment audit data.

**Architecture:** Treat `capabilitySummary.headline`, `issues`, and `scopeBoundary` as non-rendered assessment metadata. Simplify the final-conclusion renderer to fixed customer verdict and verified facts, then remove its unreachable CSS.

**Tech Stack:** Python 3, `unittest`, self-contained HTML/CSS, JSON assessment v6

---

### Task 1: Lock the HTML contract

**Files:**
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_v6.py`
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_general_verdict.py`

- [ ] **Step 1: Write the failing test**

Add a renderer test with unique freeform markers and assert that headline, issue title/statement/boundary/reference, and scope are absent while `general-verdict` and `verified-facts` are present. Update the fixed-order test so its expected sequence ends after verified facts.

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
cd skills/creating-model-doctor-reports/tests
python3 -m unittest test_model_doctor_v6.ModelDoctorV6Tests.test_final_conclusion_omits_freeform_summary_content
```

Expected: FAIL because the current renderer emits the freeform markers.

### Task 2: Simplify report rendering

**Files:**
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_html.py`
- Modify: `skills/creating-model-doctor-reports/assets/report.css`

- [ ] **Step 1: Write minimal implementation**

Make `_capability_summary` return only the section heading, `_general_verdict`, and `_verified_facts`. Delete CSS rules used only by the removed elements.

- [ ] **Step 2: Run focused renderer tests and verify GREEN**

Run:

```bash
python3 -m unittest skills.creating-model-doctor-reports.tests.test_model_doctor_v6 skills.creating-model-doctor-reports.tests.test_model_doctor_general_verdict
```

Expected: all tests pass.

### Task 3: Update and deploy the skill

**Files:**
- Modify: `skills/creating-model-doctor-reports/SKILL.md`
- Modify: `/Users/libolun/model-doctor-output/doubao-1-5-pro-32k-250115-model-capability-report-v6.html`
- Synchronize: `/Users/libolun/.codex/skills/creating-model-doctor-reports`

- [ ] **Step 1: Update the report contract**

State that HTML ends after fixed verified facts while JSON retains and validates headline, issues, and scope for audit.

- [ ] **Step 2: Run full verification**

Run all project and skill tests plus `quick_validate.py`; expected result is zero failures.

- [ ] **Step 3: Regenerate and visually inspect the report**

Render from the existing structured evidence, synchronize the active skill, and verify at desktop and mobile widths that the capability-domain table follows the verified facts with no freeform block.

- [ ] **Step 4: Commit**

Commit the scoped repository changes with message `feat: hide freeform final summary from reports`.
