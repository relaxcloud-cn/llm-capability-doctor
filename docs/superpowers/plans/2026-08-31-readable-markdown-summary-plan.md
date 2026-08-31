# Markdown Report Readability Optimization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make category-level Markdown conclusions summarize all distinct failure causes clearly and remove per-request failure details from the concurrency section.

**Architecture:** Keep evidence, JSON, detection decisions, and `ConcurrencyFailure` data unchanged. Add a Markdown-only normalization and aggregation helper in `src/analysis/orchestrator.rs`; render one counted sentence per category and keep the existing detailed sections for traceability.

**Tech Stack:** Rust 2024, `cargo test`, Markdown string rendering, existing `AnalysisTestResult`, `ConcurrencyWave`, and `markdown_cell` helpers.

---

### Task 1: Add regression tests for readable category summaries

**Files:**
- Modify: `src/analysis/orchestrator.rs` tests near `markdown_fail_explains_each_non_pass_item`

- [ ] **Step 1: Add a failing category aggregation test**

Add a test that marks three Tool-call results as failures with two repeated transport errors and one automatic-tool-choice error, then asserts the category row contains counts and one occurrence of each normalized cause while excluding URLs and internal fields:

```rust
#[test]
fn markdown_category_failure_summary_aggregates_distinct_causes() {
    let mut results = crate::catalog::CATALOG
        .iter()
        .map(|test| available_markdown_result(test.id, CandidateStatus::Pass, None))
        .collect::<Vec<_>>();
    for (id, cause) in [
        ("040", "请求发生 transport_error：tls handshake eof，url=https://example.test"),
        ("041", "stderrExcerpt=transport_error，request_id=test-041，tls handshake eof"),
        ("043", "BadRequestError: auto tool choice requires --enable-auto-tool-choice"),
    ] {
        let result = results.iter_mut().find(|result| result.test_id == id).unwrap();
        result.candidate_status = Some(CandidateStatus::Fail);
        result.validated_status = Some(ValidatedStatus::Fail);
        result.failure_cause = Some(cause.into());
    }

    let markdown = render_markdown(&markdown_fixture(results));
    let category_row = markdown
        .lines()
        .find(|line| line.starts_with("| 工具调用 |"))
        .unwrap();

    assert!(category_row.contains("11 项，8 项通过、3 项未通过"));
    assert!(category_row.contains("请求连接中断或未返回有效响应（2 项）"));
    assert!(category_row.contains("接口不支持当前自动工具选择配置（1 项）"));
    assert!(!category_row.contains("https://example.test"));
    assert!(!category_row.contains("request_id"));
    assert!(!category_row.contains("transportOutcome"));
}
```

- [ ] **Step 2: Run the focused test and verify it fails**

Run: `cargo test markdown_category_failure_summary_aggregates_distinct_causes -- --nocapture`

Expected: FAIL because the current renderer concatenates raw `failureCause` text and does not output counted normalized causes.

### Task 2: Implement category aggregation and hide concurrency failure rows

**Files:**
- Modify: `src/analysis/orchestrator.rs:1006-1030,1307-1402`

- [ ] **Step 1: Add Markdown-only failure normalization helpers**

Add helpers beside `non_pass_reason`:

```rust
fn normalized_category_failure(reason: &str) -> String {
    let lower = reason.to_ascii_lowercase();
    if lower.contains("transport_error")
        || lower.contains("tls handshake eof")
        || lower.contains("timeout")
        || reason.contains("超时")
        || reason.contains("连接中断")
        || reason.contains("空响应")
        || reason.contains("未获得任何响应")
    {
        return "请求连接中断或未返回有效响应".into();
    }
    if lower.contains("auto tool choice") || reason.contains("自动工具选择") {
        return "接口不支持当前自动工具选择配置".into();
    }
    if lower.contains("tool loop")
        || lower.contains("tool envelope")
        || reason.contains("工具闭环")
        || reason.contains("结果关联")
    {
        return "工具调用流程未完成".into();
    }
    if lower.contains("passcriteria")
        || reason.contains("期望")
        || reason.contains("字段")
        || reason.contains("参数")
        || reason.contains("类型")
        || reason.contains("格式")
    {
        return "模型返回结果与检测要求不一致".into();
    }
    clean_category_failure(reason)
}

fn clean_category_failure(reason: &str) -> String {
    let mut cleaned = reason.replace("stderrExcerpt", "");
    cleaned = cleaned.replace("responseBodyExcerpt", "");
    cleaned = cleaned.replace("request_id", "请求");
    if let Some((prefix, _)) = cleaned.split_once("url (") {
        cleaned = prefix.trim_end_matches([' ', ':', '(', '"']).to_owned();
    }
    let first_sentence = cleaned
        .split(['。', '.', ';', '；'])
        .next()
        .unwrap_or(cleaned.as_str())
        .trim();
    let bounded: String = first_sentence.chars().take(96).collect();
    if bounded.is_empty() {
        "存在未通过项".into()
    } else {
        bounded
    }
}
```

- [ ] **Step 2: Aggregate and render all distinct causes with counts**

Add a `category_failure_summary` helper that iterates the category's non-PASS tests, normalizes each reason, counts equal strings in a `BTreeMap<String, usize>`, sorts by descending count then cause text, and returns:

```rust
format!(
    "{total} 项，{passed} 项通过、{failed} 项未通过。主要问题包括：{}。",
    causes.join("；")
)
```

Each cause is formatted as `原因（N 项）`. Replace the current `non_pass.iter().map(non_pass_reason).join("；")` branch in `render_category_summary`; leave the all-PASS branch unchanged.

- [ ] **Step 3: Remove per-request concurrency failure rendering**

In `render_concurrency_summary`, retain the heading and summary table, then append one blank line and return. Delete the local `failures` collection and the loop that emits `失败请求` and `request_id` lines. Do not remove `ConcurrencyFailure` from the data model or JSON serialization.

- [ ] **Step 4: Run the focused renderer tests**

Run: `cargo test markdown_ -- --nocapture`

Expected: the new category test passes; the existing concurrency test is the only expected failure until its obsolete detailed-failure assertions are updated in Task 3.

### Task 3: Update concurrency assertions and run the full regression suite

**Files:**
- Modify: `src/analysis/orchestrator.rs` test `markdown_concurrency_summary_reports_each_wave_and_failures`

- [ ] **Step 1: Replace obsolete failure-detail assertions**

Keep the four summary-row assertions and replace the two detailed failure assertions with:

```rust
assert!(!markdown.contains("失败请求："));
assert!(!markdown.contains("request_id=test-057-c8-3"));
assert!(!markdown.contains("request_id=test-057-c32-9"));
assert!(!markdown.contains("HTTP 500: upstream unavailable"));
assert!(!markdown.contains("timeout"));
```

- [ ] **Step 2: Run all tests**

Run: `cargo test`

Expected: all unit and integration tests pass, including the existing Markdown, JSON, evidence, and output-path tests.

- [ ] **Step 3: Inspect the final diff**

Run: `git diff --check && git diff --stat && git status --short --branch`

Expected: only `src/analysis/orchestrator.rs` and the tracked implementation-plan/spec files are changed by this task; unrelated untracked user files remain untouched.

- [ ] **Step 4: Commit the implementation**

```bash
git add src/analysis/orchestrator.rs
git commit -m "feat: simplify markdown failure summaries"
```
