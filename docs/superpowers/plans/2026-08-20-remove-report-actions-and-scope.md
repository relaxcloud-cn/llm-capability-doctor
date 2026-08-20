# Remove Report Actions And Scope Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the sidebar action controls and the complete result-scope section from generated Model Doctor HTML reports and the current DeepSeek V4 Flash report.

**Architecture:** Keep `model_doctor_html.py` as the rendering source of truth. Remove the obsolete markup and navigation entry there, then delete only the JavaScript and CSS that exclusively supported those elements; assessment data and verdict logic remain unchanged.

**Tech Stack:** Python 3, `unittest`, self-contained HTML/CSS/JavaScript, Codex in-app browser.

---

### Task 1: Lock The Four-Section Report Contract

**Files:**
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_v6.py`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_html.py`
- Modify: `skills/creating-model-doctor-reports/assets/report.js`
- Modify: `skills/creating-model-doctor-reports/assets/report.css`

- [ ] **Step 1: Write the failing renderer test**

Add a test that renders the standard assessment and verifies the four retained sections in order while rejecting every deleted marker:

```python
def test_renderer_omits_sidebar_actions_and_result_scope(self) -> None:
    assessment = assemble_assessment(self._parsed(), self._reviews())
    html = render_report(assessment, ASSET_DIR)

    retained = ('id="conclusion"', 'id="issues"', 'id="run-info"', 'id="capabilities"')
    positions = tuple(html.find(marker) for marker in retained)
    self.assertTrue(all(position >= 0 for position in positions), positions)
    self.assertEqual(tuple(sorted(positions)), positions)
    for deleted in (
        'id="expand-all"', 'id="collapse-all"', 'id="print-report"',
        'href="#scope"', 'value="scope"', 'id="scope"',
        "展开全部", "收起全部", "打印报告", "结果适用范围",
    ):
        self.assertNotIn(deleted, html)
```

- [ ] **Step 2: Run the focused test and verify RED**

Run: `python3 -m unittest skills.creating-model-doctor-reports.tests.test_model_doctor_v6.ModelDoctorV6Tests.test_renderer_omits_sidebar_actions_and_result_scope -v`

Expected: `FAIL` because the current report contains `id="expand-all"` and the `scope` section.

- [ ] **Step 3: Implement the minimal renderer and asset removal**

Remove `SCOPE_BULLET_TEMPLATES`, `_scope_section`, the `scope` entry in `NAV_SECTIONS`, the sidebar action markup, and the `_scope_section(test_total)` render call. Remove the three obsolete button handlers from `report.js` and the `.sidebar-actions`, `.action-button`, and `.scope-list`-only rules from `report.css`. Update the renderer module docstring to list four sections.

- [ ] **Step 4: Run focused tests and verify GREEN**

Run: `python3 -m unittest skills.creating-model-doctor-reports.tests.test_model_doctor_v6.ModelDoctorV6Tests.test_renderer_omits_sidebar_actions_and_result_scope skills.creating-model-doctor-reports.tests.test_model_doctor_v6.ModelDoctorV6Tests.test_renderer_places_sections_in_approved_reading_order -v`

Expected: `OK` with `Ran 2 tests`.

- [ ] **Step 5: Commit the renderer change**

```bash
git add skills/creating-model-doctor-reports/tests/test_model_doctor_v6.py skills/creating-model-doctor-reports/scripts/model_doctor_html.py skills/creating-model-doctor-reports/assets/report.js skills/creating-model-doctor-reports/assets/report.css
git commit -m "feat: remove report actions and scope section"
```

### Task 2: Align The Report Skill Contract

**Files:**
- Modify: `skills/creating-model-doctor-reports/SKILL.md`
- Modify: `skills/creating-model-doctor-reports/tests/test_model_doctor_v6.py`

- [ ] **Step 1: Add a failing instruction-contract assertion**

Extend `test_skill_instructions_require_the_three_stage_flow` with:

```python
self.assertNotIn("结果适用范围", skill_text)
self.assertNotIn("展开全部", skill_text)
self.assertNotIn("收起全部", skill_text)
self.assertNotIn("打印报告", skill_text)
```

- [ ] **Step 2: Run the focused test and verify RED**

Run: `python3 -m unittest skills.creating-model-doctor-reports.tests.test_model_doctor_v6.ModelDoctorV6Tests.test_skill_instructions_require_the_three_stage_flow -v`

Expected: `FAIL` because the current workflow requires `结果适用范围`.

- [ ] **Step 3: Update the report instructions**

Change the fixed reading path and final section verification to the four retained sections: `总体结果`, `需要处理的问题`, `本次检测信息`, and `能力检查结果`. Do not change evidence, assessment, or verdict requirements.

- [ ] **Step 4: Run the focused test and full report suite**

Run: `python3 -m unittest discover -s skills/creating-model-doctor-reports/tests -p 'test_*.py' -v`

Expected: all tests pass with zero failures and zero errors.

- [ ] **Step 5: Commit the instruction alignment**

```bash
git add skills/creating-model-doctor-reports/SKILL.md skills/creating-model-doctor-reports/tests/test_model_doctor_v6.py
git commit -m "docs: align report skill with four-section layout"
```

### Task 3: Regenerate And Inspect The Current Report

**Files:**
- Regenerate: `deepseek-v4-flash-model-capability-report.html` from `deepseek-v4-flash-assessment.json`

- [ ] **Step 1: Render through the production renderer**

Run a Python command that imports `render_report`, loads `deepseek-v4-flash-assessment.json`, and writes the returned HTML to `deepseek-v4-flash-model-capability-report.html` with UTF-8 encoding.

- [ ] **Step 2: Verify deleted and retained markers**

Run a deterministic HTML check that rejects the three button IDs, the `scope` navigation target, and the `scope` section, then confirms the four retained sections occur once and in order.

Expected: exit code `0` and `report markers verified`.

- [ ] **Step 3: Inspect desktop and mobile layouts**

Open the regenerated local report in the in-app browser. Capture one desktop screenshot and one mobile screenshot. Confirm the sidebar/mobile navigation contains four entries, the report ends after capability results, and no controls overlap report content.

- [ ] **Step 4: Check repository state**

Run: `git status --short --branch`

Expected: only the known user-owned untracked files remain; the ignored regenerated report does not appear in Git status.
