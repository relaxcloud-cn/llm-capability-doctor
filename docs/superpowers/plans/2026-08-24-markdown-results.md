# Markdown 检测结果 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在客户侧自分析完成后，生成与 JSON 同目录的 Markdown 结果文件，包含 8 个大分类结论、46 项明细和非通过项证据详情。

**Architecture:** 在 `src/analysis/orchestrator.rs` 内基于已经构建的 `SelfAnalysisArtifact` 渲染 Markdown，分类和名称从 `src/catalog.rs` 读取，状态只使用已验证的模型结果和批次不可用状态。JSON 与 Markdown 共享同一份内存 artifact 和碰撞安全写入流程，避免二次判定或结论漂移。

**Tech Stack:** Rust, serde, existing atomic/private output helpers, Cargo tests.

---

### Task 1: Lock Markdown output contract with tests

**Files:**
- Modify: `src/analysis/orchestrator.rs`
- Modify: `README.md`

- [x] **Step 1: Add renderer tests before implementation**

Add these unit tests in `src/analysis/orchestrator.rs`:

- `markdown_all_pass_has_eight_satisfied_categories_and_46_rows`: render a fixture with all validated PASS results; assert eight `| 满足 |` category rows and 46 numbered detail rows.
- `markdown_fail_explains_each_non_pass_item`: render one FAIL with `failure_cause = Some("tool envelope incomplete")`; assert the category is `不满足` and the reason contains `046`, the catalog name, and the failure text.
- `markdown_unavailable_is_not_presented_as_model_fail`: render one unavailable result with `limitations = ["offline"]`; assert `ANALYSIS_UNAVAILABLE`, `分析不可用`, and `offline` appear in the category reason and detail section.
- `markdown_escapes_table_content`: render observations and causes containing `|` and newlines; assert the table has escaped pipes and no raw multiline cell.
- `markdown_and_json_use_distinct_collision_safe_paths`: create an existing JSON and Markdown sibling, then assert the next paths are timestamped and neither existing file is overwritten.

- [x] **Step 2: Run the focused tests and confirm failure**

Run `cargo test analysis::orchestrator::tests::markdown --lib`. Expected: compile/test failure because the renderer and Markdown output path do not exist yet.

- [x] **Step 3: Update README output list after the renderer contract is fixed**

Document `model-doctor-self-analysis.md` beside the existing JSON and state that it contains category conclusions plus failed/unavailable details.

### Task 2: Implement Markdown rendering and safe output

**Files:**
- Modify: `src/analysis/orchestrator.rs`

- [x] **Step 1: Add catalog lookup and Markdown escaping helpers**

Implement helpers with these signatures:

```rust
fn catalog_case(test_id: &str) -> Option<&'static crate::catalog::TestCase>;
fn markdown_cell(value: &str) -> String;
fn markdown_reason(result: &AnalysisTestResult) -> String;
fn result_status(result: &AnalysisTestResult) -> &'static str;
```

`markdown_cell` replaces backslashes/newlines and escapes `|`; `result_status` maps validated PASS to `PASS`, validated FAIL to `FAIL`, and unavailable state to `ANALYSIS_UNAVAILABLE`.

- [x] **Step 2: Add category and detail renderers**

Implement:

```rust
fn render_markdown(artifact: &SelfAnalysisArtifact) -> String;
fn render_category_summary(artifact: &SelfAnalysisArtifact, output: &mut String);
fn render_test_table(artifact: &SelfAnalysisArtifact, output: &mut String);
fn render_non_pass_details(artifact: &SelfAnalysisArtifact, output: &mut String);
```

Use catalog order for categories and tests. A category is `满足` only when every member has validated PASS; otherwise it is `不满足`. Reasons list every non-pass item as `ID 名称：failureCause` or `ID 名称：分析不可用（limitation）`. Details include status, decision source, failure cause, observations, refs, limitations, and validation notes only for non-pass results.

- [x] **Step 3: Write Markdown beside JSON using collision-safe private output**

Refactor the existing JSON path helper to accept an extension and add:

```rust
fn collision_safe_output_path(log_path: &Path, extension: &str) -> PathBuf;
fn write_output(log_path: &Path, extension: &str, bytes: &[u8]) -> Result<PathBuf, AnalysisError>;
```

Write UTF-8 Markdown through the same private temp file, hard-link, collision-safe process. Keep `AnalysisOutcome.path` as the JSON path for compatibility and add `markdown_path: PathBuf` so CLI can report both artifacts.

- [x] **Step 4: Render both files from the same artifact**

In `analyze_with_client`, serialize/write JSON and Markdown after the artifact is complete. Return both paths; do not make Markdown generation call the model or recompute PASS/FAIL.

### Task 3: Expose and verify the customer-facing result

**Files:**
- Modify: `src/main.rs`
- Modify: `README.md`

- [x] **Step 1: Print both artifact paths in the CLI summary**

Update `print_analysis_outcome` to print the JSON path and Markdown path while preserving existing counts and cancellation wording.

- [x] **Step 2: Update CLI/integration assertions**

Keep `default_cli_run_creates_no_self_analysis_artifact` unchanged for the no-flag path. Extend the orchestrator fixture assertions so both sibling files exist, JSON remains v2, Markdown contains the category table and 46 detail rows, and sensitive fixture values remain absent. Update `print_analysis_outcome` output assertions only if a CLI unit seam is introduced.

- [x] **Step 3: Run focused tests**

Run `cargo test analysis::orchestrator::tests::markdown analysis::orchestrator::tests --lib` and inspect generated Markdown from the fixture.

### Task 4: Verify and deliver

**Files:**
- Modify: `docs/superpowers/plans/2026-08-24-markdown-results.md`

- [x] **Step 1: Run the full verification suite**

Run `cargo fmt --check`, `git diff --check`, `cargo test --all-targets`, `cargo clippy --all-targets -- -D warnings`, `cargo build --release`, and `python3 -m unittest discover -s skills/creating-model-doctor-reports/tests -p 'test_*.py'`.

- [x] **Step 2: Mark the plan complete and inspect the diff**

Confirm the output path, status aggregation, reason rendering, escaping, collision handling, and no hard-gate logic are all represented in the final diff.

- [ ] **Step 3: Commit the implementation**

Use `git add -f` for ignored documentation paths and commit with `feat: generate markdown self-analysis results`.
