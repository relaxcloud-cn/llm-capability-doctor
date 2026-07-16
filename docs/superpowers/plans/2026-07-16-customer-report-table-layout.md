# Customer Report Table Layout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the dashboard-style customer HTML with the legacy 736px two-table layout while adding independently expandable evidence rows containing purpose, method, pass criteria, request input, and request output.

**Architecture:** Keep `assessment.json`, semantic review rules, and the Shell collector unchanged. Refactor only the deterministic renderer and its CSS/JS assets: Python emits two semantic tables and stable row/detail IDs, CSS reproduces the legacy visual system and responsive behavior, and a small offline script toggles hidden detail rows with accessible state.

**Tech Stack:** Python 3.9 standard library, semantic HTML, CSS media queries, vanilla JavaScript, `unittest`, Codex in-app browser.

---

## File Map

- `skills/creating-model-doctor-reports/scripts/model_doctor_html.py`: render the two tables, summary rows, hidden evidence rows, and redacted turn evidence.
- `skills/creating-model-doctor-reports/assets/report.css`: reproduce the legacy narrow table layout, dark/light colors, mobile stacking, code overflow, and print behavior.
- `skills/creating-model-doctor-reports/assets/report.js`: toggle one result row, its arrow, hidden detail, and `aria-expanded` state without using `innerHTML`.
- `tests/test_model_doctor_html.py`: enforce structure, content mapping, safety, styling, interaction contracts, and collision-safe CLI rendering.
- `docs/superpowers/specs/2026-07-16-customer-report-table-layout-design.md`: approved source of requirements; do not change unless implementation exposes a contradiction.

### Task 1: Lock the legacy two-table HTML contract

**Files:**
- Modify: `tests/test_model_doctor_html.py`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_html.py`

- [ ] **Step 1: Replace the dashboard information-architecture test with a failing two-table test**

Add this contract to `ModelDoctorHtmlTests`:

```python
def test_render_report_uses_legacy_two_table_information_architecture(self):
    html = render_report(assessment(), ASSET_DIR)

    self.assertEqual(html.count("<table"), 2)
    self.assertIn("能力域总结", html)
    self.assertIn("逐项检测结果", html)
    self.assertIn("<th>能力域</th><th>状态</th><th>关键数据</th><th>最终结论</th>", html)
    self.assertIn("<th>编号</th><th>检测项</th><th>检测结果</th><th>检测结论</th>", html)
    for removed in ("status-counts", "filter-bar", "method-grid", "integrity-grid"):
        self.assertNotIn(removed, html)
```

- [ ] **Step 2: Add a failing row/detail pairing test**

```python
def test_each_result_row_has_one_hidden_accessible_detail_row(self):
    html = render_report(assessment(), ASSET_DIR)

    self.assertIn('class="result-row" data-detail-id="test-detail-001"', html)
    self.assertIn('class="row-toggle" aria-expanded="false" aria-controls="test-detail-001"', html)
    self.assertIn('id="test-detail-001" class="evidence-row" hidden', html)
    self.assertIn('<td colspan="4">', html)
```

- [ ] **Step 3: Run the focused test and verify RED**

Run:

```bash
python3 -m unittest tests.test_model_doctor_html.ModelDoctorHtmlTests.test_render_report_uses_legacy_two_table_information_architecture tests.test_model_doctor_html.ModelDoctorHtmlTests.test_each_result_row_has_one_hidden_accessible_detail_row -v
```

Expected: FAIL because the renderer still emits dashboard sections and `<details>` cards rather than two tables and paired rows.

- [ ] **Step 4: Replace dashboard helpers with table-focused helpers**

In `model_doctor_html.py`:

1. Keep `_e`, `_list`, `_observed_protocol`, CSP, status labels, and `render_report` validation.
2. Remove `_verdict_statement`, `_decision_items`, `_counts`, `_filters`, `_test_groups`, and `_test_detail`.
3. Add category labels and these helpers:

```python
CATEGORY_STATUS_LABELS = {
    "PASS": "通过",
    "FAIL": "未通过",
    "CONDITIONAL": "需复测",
}


def _category_key_data(category: dict) -> str:
    counts = category.get("counts", {})
    total = sum(int(value) for value in counts.values())
    parts = [f"{int(counts.get('PASS', 0))}/{total} 通过"]
    if category.get("criticalFailures"):
        parts.append("硬门禁失败：" + ", ".join(category["criticalFailures"]))
    if category.get("unknowns"):
        parts.append("待补证：" + ", ".join(category["unknowns"]))
    return "；".join(parts)


def _category_conclusion(category: dict) -> str:
    status = category.get("status")
    if status == "PASS":
        return "本次证据未发现该能力域的已确认问题。"
    if category.get("criticalFailures"):
        return "存在硬门禁失败，需先处理对应检测项。"
    return "存在失败、错误或证据不足项，请查看逐项检测证据。"
```

4. Render category rows with exact four-column labels and escaped values.
5. Render every test as one `<tbody class="result-group">` containing:
   - `<tr class="result-row" data-detail-id="test-detail-{id}">`
   - one button with `aria-expanded="false"` and `aria-controls`
   - `<tr id="test-detail-{id}" class="evidence-row" hidden>`
6. Use reviewed status and conclusion in the summary row. Put category and `rawObservation` in small secondary text matching the legacy report.
7. Replace `render_report` body with two `<table>` elements and no dashboard sections.

- [ ] **Step 5: Run the focused tests and verify GREEN**

Run the Step 3 command.

Expected: both tests PASS.

- [ ] **Step 6: Run the renderer test module**

Run:

```bash
python3 -m unittest tests.test_model_doctor_html -v
```

Expected: remaining old dashboard assertions fail. Update only assertions that conflict with the approved design; retain offline safety, non-mutation, count, and CLI coverage.

- [ ] **Step 7: Commit the semantic table structure**

```bash
git add tests/test_model_doctor_html.py skills/creating-model-doctor-reports/scripts/model_doctor_html.py
git commit -m '将客户报告改为旧版双表结构'
```

### Task 2: Render only the five approved evidence sections

**Files:**
- Modify: `tests/test_model_doctor_html.py`
- Modify: `skills/creating-model-doctor-reports/scripts/model_doctor_html.py`

- [ ] **Step 1: Write a failing exact-content test**

```python
def test_expanded_row_contains_only_approved_logic_and_io_sections(self):
    html = render_report(assessment(), ASSET_DIR)

    for expected in ("检测目的", "检测方法", "通过条件", "请求输入", "请求输出"):
        self.assertIn(expected, html)
    for removed in ("失败条件", "能力边界", "判定证据", "证据引用", "复测建议"):
        self.assertNotIn(removed, html)
```

- [ ] **Step 2: Write a failing multi-turn ordering test**

Build from the minimal assessment without mutating shared fixtures:

```python
def test_expanded_row_pairs_every_turn_input_and_output_in_order(self):
    value = assessment()
    second = deepcopy(value["tests"][0]["requests"][0])
    second["request_id"] = "test-001-follow"
    second["requestBody"] = '{"turn":2,"input":"follow-up"}'
    second["responseBody"] = '{"turn":2,"output":"done"}'
    value["tests"][0]["requests"].append(second)

    html = render_report(value, ASSET_DIR)

    self.assertLess(html.index("Turn 1"), html.index("Turn 2"))
    self.assertLess(html.index("Reply only OK"), html.index("follow-up"))
    self.assertLess(html.index('content&quot;:&quot;OK'), html.index("done"))
```

- [ ] **Step 3: Run both tests and verify RED**

Run:

```bash
python3 -m unittest tests.test_model_doctor_html.ModelDoctorHtmlTests.test_expanded_row_contains_only_approved_logic_and_io_sections tests.test_model_doctor_html.ModelDoctorHtmlTests.test_expanded_row_pairs_every_turn_input_and_output_in_order -v
```

Expected: FAIL because the current detail renderer includes extra review sections and uses the old `模型输出` label.

- [ ] **Step 4: Implement the compact evidence renderer**

Replace `_request_timeline` with a helper that renders each request in order:

```python
def _request_evidence(requests: List[dict]) -> str:
    if not requests:
        return '<p class="empty-evidence">日志未包含可关联的完整请求块。</p>'
    turns = []
    for index, request in enumerate(requests, start=1):
        metrics = request.get("metrics", {})
        meta = " · ".join(
            value for value in (
                str(request.get("request_id") or f"Turn {index}"),
                f"HTTP {metrics.get('http_status')}" if metrics.get("http_status") else "",
                f"{metrics.get('time_total')}s" if metrics.get("time_total") else "",
                f"{metrics.get('size_download')} bytes" if metrics.get("size_download") else "",
            ) if value
        )
        turns.append(
            '<section class="turn-evidence">'
            f'<h4>Turn {index}<span>{_e(meta)}</span></h4>'
            '<h5>请求输入</h5>'
            f'<pre><code>{_e(request.get("requestBody"))}</code></pre>'
            '<h5>请求输出</h5>'
            f'<pre><code>{_e(request.get("responseBody"))}</code></pre>'
            '</section>'
        )
    return "".join(turns)
```

The evidence row must contain exactly:

```python
'<section class="logic-item"><h4>检测目的</h4>'
f'<p>{_e(logic.get("purpose"))}</p></section>'
'<section class="logic-item"><h4>检测方法</h4>'
f'<p>{_e(logic.get("method"))}</p></section>'
'<section class="logic-item pass-criteria"><h4>通过条件</h4>'
f'{_list(logic.get("passCriteria", []))}</section>'
f'{_request_evidence(item.get("requests", []))}'
```

- [ ] **Step 5: Run the focused tests and verify GREEN**

Run the Step 3 command.

Expected: both tests PASS.

- [ ] **Step 6: Re-run hostile-output and non-mutation tests**

```bash
python3 -m unittest tests.test_model_doctor_html.ModelDoctorHtmlTests.test_render_report_is_offline_and_renders_hostile_output_as_text tests.test_model_doctor_html.ModelDoctorHtmlTests.test_render_report_does_not_mutate_assessment -v
```

Expected: PASS; hostile output is escaped and the assessment object remains unchanged.

- [ ] **Step 7: Commit the approved evidence surface**

```bash
git add tests/test_model_doctor_html.py skills/creating-model-doctor-reports/scripts/model_doctor_html.py
git commit -m '精简逐项展开的检测逻辑与输入输出'
```

### Task 3: Reproduce legacy styling and accessible row toggles

**Files:**
- Modify: `tests/test_model_doctor_html.py`
- Modify: `skills/creating-model-doctor-reports/assets/report.css`
- Modify: `skills/creating-model-doctor-reports/assets/report.js`

- [ ] **Step 1: Write failing CSS contract assertions**

```python
def test_report_css_matches_legacy_narrow_table_and_responsive_contract(self):
    html = render_report(assessment(), ASSET_DIR)

    for expected in (
        "max-width: 736px",
        "border-collapse: collapse",
        "prefers-color-scheme: dark",
        "@media (max-width: 640px)",
        ".evidence-row[hidden]",
        "overflow-wrap: anywhere",
    ):
        self.assertIn(expected, html)
    self.assertNotIn("max-width: 1440px", html)
    self.assertNotIn("position: sticky", html)
```

- [ ] **Step 2: Write failing JavaScript contract assertions**

```python
def test_report_script_toggles_detail_hidden_state_and_aria_without_inner_html(self):
    html = render_report(assessment(), ASSET_DIR)

    for expected in (
        '.querySelectorAll(".result-row")',
        'button.setAttribute("aria-expanded"',
        "detail.hidden = !expanded",
        'row.classList.toggle("is-expanded"',
    ):
        self.assertIn(expected, html)
    self.assertNotIn("innerHTML", html)
```

- [ ] **Step 3: Run the two tests and verify RED**

Run:

```bash
python3 -m unittest tests.test_model_doctor_html.ModelDoctorHtmlTests.test_report_css_matches_legacy_narrow_table_and_responsive_contract tests.test_model_doctor_html.ModelDoctorHtmlTests.test_report_script_toggles_detail_hidden_state_and_aria_without_inner_html -v
```

Expected: FAIL because current CSS is dashboard-oriented and current JavaScript targets filters and `<details>`.

- [ ] **Step 4: Replace `report.css` with the legacy visual system**

Implement these exact layout rules:

```css
:root {
  color-scheme: light dark;
  --page: #ffffff;
  --text: #202124;
  --muted: #8b8f94;
  --header: #e7e7e8;
  --line: #d8d8d8;
  --detail: #f7f7f8;
  --pass: #202124;
  --warn: #7a5b00;
  --fail: #a61b1b;
}

body { margin: 0; padding: 16px; background: var(--page); color: var(--text); }
.report-shell { width: 100%; max-width: 736px; margin: 0 auto; }
table { width: 100%; border-collapse: collapse; table-layout: fixed; }
.summary-table { margin-bottom: 42px; }
.summary-table col:nth-child(1) { width: 17%; }
.summary-table col:nth-child(2) { width: 14%; }
.summary-table col:nth-child(3) { width: 29%; }
.summary-table col:nth-child(4) { width: 40%; }
.results-table col:nth-child(1) { width: 10%; }
.results-table col:nth-child(2) { width: 30%; }
.results-table col:nth-child(3) { width: 25%; }
.results-table col:nth-child(4) { width: 35%; }
th, td { padding: 13px 12px; border-bottom: 1px solid var(--line); text-align: left; vertical-align: top; }
.evidence-row[hidden] { display: none; }
pre { max-height: 360px; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; }
```

Use the old report's four-column proportions. Style the toggle as a small chevron icon with no rounded text container. Use neutral status text with amber/red only for non-pass results. Do not add cards inside the detail row.

Add `@media (prefers-color-scheme: dark)` variables matching the old report's dark gray table, and `@media (max-width: 640px)` rules that hide the table header and stack summary cells with `data-label` pseudo-labels. Keep evidence detail single-column on mobile. Add print rules that hide toggle buttons and preserve the current hidden/open state.

- [ ] **Step 5: Replace `report.js` with independent row toggling**

Use this behavior without `innerHTML`:

```javascript
(() => {
  "use strict";

  document.querySelectorAll(".result-row").forEach((row) => {
    row.addEventListener("click", () => {
      const detail = document.getElementById(row.dataset.detailId);
      const button = row.querySelector(".row-toggle");
      if (!detail || !button) return;
      const expanded = button.getAttribute("aria-expanded") !== "true";
      button.setAttribute("aria-expanded", expanded ? "true" : "false");
      detail.hidden = !expanded;
      row.classList.toggle("is-expanded", expanded);
    });
  });
})();
```

Because the button is inside the row, keyboard activation emits a click that bubbles to the row and uses the same code path.

- [ ] **Step 6: Run focused tests and verify GREEN**

Run the Step 3 command.

Expected: both tests PASS.

- [ ] **Step 7: Run the complete HTML test module**

```bash
python3 -m unittest tests.test_model_doctor_html -v
```

Expected: all HTML and CLI rendering tests PASS.

- [ ] **Step 8: Commit style and interaction**

```bash
git add tests/test_model_doctor_html.py skills/creating-model-doctor-reports/assets/report.css skills/creating-model-doctor-reports/assets/report.js
git commit -m '复刻旧版报告样式并支持行内展开'
```

### Task 4: Regenerate the real report and verify in browser

**Files:**
- No tracked source changes expected
- Generate outside Git: `/Users/libolun/Desktop/testllm/gpt-5.5-customer-readiness-report-<timestamp>.html`

- [ ] **Step 1: Run the complete automated suite**

```bash
bash -n model-capability-doctor.sh
python3 -m unittest discover -s tests -p 'test_*.py' -v
/tmp/model-doctor-skill-validator/bin/python \
  /Users/libolun/.codex/skills/.system/skill-creator/scripts/quick_validate.py \
  skills/creating-model-doctor-reports
python3 -m py_compile skills/creating-model-doctor-reports/scripts/*.py
git diff --check
```

Expected: collector syntax passes, all tests pass, Skill is valid, Python compiles, and diff check is clean.

- [ ] **Step 2: Render the existing real assessment with the updated renderer**

Use the existing validated assessment as input and write a non-overwriting timestamped HTML beside it:

```bash
PYTHONPATH=skills/creating-model-doctor-reports/scripts python3 -c '
import json
from datetime import datetime
from pathlib import Path
from model_doctor_html import render_report
assessment_path = Path("/Users/libolun/Desktop/testllm/gpt-5.5-assessment.json")
assessment = json.loads(assessment_path.read_text(encoding="utf-8"))
stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
output = assessment_path.with_name(f"gpt-5.5-customer-readiness-report-{stamp}.html")
output.write_text(render_report(assessment, Path("skills/creating-model-doctor-reports/assets")), encoding="utf-8")
print(output.resolve())
'
```

Expected: a new absolute HTML path; the existing old report remains unchanged.

- [ ] **Step 3: Run static real-report assertions**

For the emitted path, verify:

```bash
test "$(rg -o '<table' "$REPORT" | wc -l | tr -d ' ')" -eq 2
test "$(rg -o 'class="result-row"' "$REPORT" | wc -l | tr -d ' ')" -eq 113
test "$(rg -o 'class="evidence-row" hidden' "$REPORT" | wc -l | tr -d ' ')" -eq 113
test -z "$(rg '(?:src|href)="https?://' "$REPORT")"
```

Expected: two tables, 113 summary rows, 113 hidden evidence rows, and no external resources.

- [ ] **Step 4: Verify desktop layout and interaction in the Codex in-app browser**

Open the new localhost URL. At the normal desktop viewport:

- compare width, typography, table header, row spacing, borders, and dark/light palette to `gpt-5.5-customer-readiness-report.html`;
- confirm the first table has no toggles;
- click item `001`, confirm its arrow and `aria-expanded` update and the five sections appear;
- open a second item and confirm item `001` remains open;
- inspect browser console logs for errors.

Expected: visual parity with the old table and independent row expansion with no console errors.

- [ ] **Step 5: Verify mobile and long-evidence behavior**

Use the browser viewport capability at approximately 390x844, reload, and inspect:

- no horizontal page overflow;
- summary cells stack with visible labels;
- long request/response code blocks scroll internally;
- headings, code, and following rows do not overlap.

Reset the viewport override afterward.

Expected: usable single-column mobile details and no incoherent overlap.

- [ ] **Step 6: Verify repository and installation state**

```bash
git status --short
git log --oneline -8
test -L "$HOME/.codex/skills/creating-model-doctor-reports"
test -f "$HOME/.codex/skills/creating-model-doctor-reports/SKILL.md"
```

Expected: only the user's pre-existing untracked `gpt-5.5-model-doctor.log` may appear; implementation files are committed; installed Skill resolves through the main project path after integration.

## Execution Notes

- Preserve `/Users/libolun/projects/llm-capability-doctor/gpt-5.5-model-doctor.log`; it is a user-generated untracked artifact.
- Do not overwrite either existing customer report under `/Users/libolun/Desktop/testllm`.
- Do not alter semantic statuses, gates, conclusions, the assessment schema, parser, or collector.
- Execute inline unless the user explicitly authorizes subagents; current collaboration rules do not permit implicit delegation.
