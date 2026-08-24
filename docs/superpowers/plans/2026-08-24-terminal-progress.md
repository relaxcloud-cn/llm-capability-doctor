# Terminal Progress Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render readable, cross-platform plain-text progress for collection and self-analysis, with customer-facing check titles, visible 4/8/16/32 concurrency waves, batch-level analysis progress, retry notices, and short result paths.

**Architecture:** Create a pure `terminal` module that owns display labels, line formatting, and relative-path rendering. `Runner` calls it before each check and before each 057 concurrent group. The self-analysis orchestrator emits progress events to the public CLI path while its existing test helper uses a no-op callback; no event changes collection, analysis packets, or final statuses.

**Tech Stack:** Rust, Tokio, existing catalog/check plans, Cargo test/Clippy.

---

### Task 1: Create pure terminal display helpers

**Files:**
- Create: `src/terminal.rs`
- Modify: `src/lib.rs`

- [x] **Step 1: Write failing unit tests for labels and safe text output**

Add tests that assert:

```rust
assert_eq!(display_category("上下文"), "上下文能力");
assert_eq!(display_title("014"), "测试模型是否支持 8K 上下文长度");
assert_eq!(display_title("018"), "测试模型是否支持 128K 上下文长度");
assert_eq!(display_title("057", Some(32)), "测试模型在 32 并发下能否正常响应");
assert_eq!(collection_line(14, 46, "上下文能力", "测试模型是否支持 8K 上下文长度", None), "[采集 14/46] 上下文能力 | 测试模型是否支持 8K 上下文长度");
```

Assert every formatted line has no `\x1b` escape byte.

- [x] **Step 2: Run the focused test and confirm it fails**

Run: `cargo test terminal::tests --lib`

Expected: compilation failure because the `terminal` module and formatting functions do not exist.

- [x] **Step 3: Implement terminal labels and formatter functions**

Add `pub mod terminal;` to `src/lib.rs`. In `src/terminal.rs`, implement these pure functions:

```rust
pub fn display_category(category: &str) -> &str;
pub fn display_title(test_id: &str, concurrency: Option<usize>) -> String;
pub fn collection_line(position: usize, total: usize, category: &str, title: &str, wave: Option<(usize, usize)>) -> String;
pub fn analysis_start_line(index: usize, total: usize, categories: &[String], test_ids: &[String]) -> String;
pub fn analysis_retry_line(index: usize, total: usize, retry: usize, max_retries: usize) -> String;
pub fn analysis_finish_line(index: usize, total: usize, categories: &[String], unavailable: bool) -> String;
pub fn display_path(path: &Path) -> String;
```

Use the following terminal-only titles. They never replace `TestCase.name` in audit logs or artifacts:

```text
001 HTTP 接口连通性与可观察响应
002 模型接口协议识别与响应结构
003 鉴权有效性与模型名称接受
004 同步文本生成响应
005 流式文本生成响应
006 流式响应正常结束
007 用量统计字段返回
008 异常请求错误信息可观测性
009 严格裸 JSON 输出
010 JSON 必填字段与数据类型
011 JSON 嵌套对象、数组与空值
012 结构化结果核心字段
013 调查阶段与证据引用结构
014 测试模型是否支持 8K 上下文长度
015 测试模型是否支持 16K 上下文长度
016 测试模型是否支持 32K 上下文长度
017 测试模型是否支持 64K 上下文长度
018 测试模型是否支持 128K 上下文长度
019 精确文本输出一致性
020 多行组合格式约束
022 多字段信息抽取
024 限长摘要关键要点保留
031 多轮修正后的上下文记忆
033 推理档位参数接受度
035 推理内容与最终答案分离
036 流式推理事件与最终答案分离
038 逻辑关系与时间顺序推理
040 单工具调用结构
041 多工具候选下的正确工具选择
042 无需工具时不发起工具调用
043 工具必填参数、数据类型与枚举值
044 工具嵌套参数结构
045 并行工具调用能力
046 官方工具协议结构与结果闭环
047 连续工具调用与结果关联
048 工具结果内容保留
049 工具调用失败后的恢复
050 大工具目录下的工具选择
052 非流式首字节响应时间
053 流式首字节响应时间
054 完整响应时间
055 重复请求成功率
056 P50/P95 响应时间
057 测试模型在 X 并发下能否正常响应
059 中文安全业务词与业务字段可用性
060 英文安全业务词与业务字段可用性
```

Use `X` only when `concurrency` is `Some(4 | 8 | 16 | 32)`; otherwise use `4/8/16/32 并发响应表现`. `display_path` returns `./relative/path` when the path is below the current directory and returns the original absolute path otherwise.

- [x] **Step 4: Run focused tests and commit**

Run: `cargo test terminal::tests --lib`

Expected: PASS.

### Task 2: Render collection progress and split 057 waves

**Files:**
- Modify: `src/runner.rs`
- Test: `src/runner/tests.rs`

- [x] **Step 1: Write failing collection-progress tests**

Add a pure `collection_progress_line` helper test for position 14/context and position 44/wave 4 of 4. Add a plan-level assertion that test 057 still contains four `RequestGroup::Concurrent` groups of lengths 4, 8, 16, 32.

- [x] **Step 2: Run the focused test and confirm it fails**

Run: `cargo test runner::tests::collection_progress --lib`

Expected: failure because `Runner` has no collection position/wave rendering seam.

- [x] **Step 3: Add collection progress rendering before real work**

Change `Runner::execute` to enumerate `self.selected.clone()` and print one line using `terminal::collection_line` before each non-057 check. Change `execute_check` and `execute_groups` to receive `collection_position` and the `TestCase` reference. For test 057, print a line before each `RequestGroup::Concurrent` with `wave = Some((group_index + 1, 4))` and `display_title("057", Some(requests.len()))`; do not print an additional generic 057 line.

Do not print request bodies, model output, credentials, or per-request network details. Do not alter `TestManifest`, request IDs, group execution ordering, or evidence logging.

- [x] **Step 4: Run runner and performance tests**

Run: `cargo test runner::tests checks::performance::tests::check_057_requests_any_short_non_empty_response --lib`

Expected: PASS.

### Task 3: Emit self-analysis batch progress and retries

**Files:**
- Modify: `src/analysis/orchestrator.rs`
- Modify: `src/main.rs`
- Test: `src/analysis/orchestrator.rs`

- [x] **Step 1: Write failing event-order tests**

Introduce test-only collection of progress messages for one batch that first returns `ClientError::Transport("timeout")` then a valid envelope. Assert this exact event order:

```text
[自分析 01/12] 接口与协议 | 001-004 | 正在分析
[自分析 01/12] 接口与协议 | 重试 1/3
[自分析 01/12] 接口与协议 | 已完成
```

Add a separate unrecoverable batch assertion ending in `分析不可用`.

- [x] **Step 2: Run the focused tests and confirm failure**

Run: `cargo test analysis::orchestrator::tests::analysis_progress --lib`

Expected: failure because progress events are not emitted.

- [x] **Step 3: Add a callback-based progress seam**

Define a private `AnalysisProgressEvent` enum for batch start, retry, and finish. Add `analyze_with_client_and_progress` that accepts `&mut dyn FnMut(AnalysisProgressEvent)`. Keep `analyze_with_client` as a no-op-wrapper for existing tests. Make public `analyze` use a terminal callback that prints formatter lines. Pass the callback into `process_batch` and `analyze_with_transport_retries`; emit retry before each bounded backoff. Calculate batch categories in catalog order from the `EvidencePacket` category values and render test IDs as compressed ranges such as `001-004` or joined values where IDs are not contiguous.

- [x] **Step 4: Run orchestrator tests**

Run: `cargo test analysis::orchestrator::tests --lib`

Expected: PASS.

### Task 4: Render concise completion summaries

**Files:**
- Modify: `src/main.rs`
- Modify: `README.md`
- Test: `src/terminal.rs`

- [x] **Step 1: Write failing path and summary tests**

Test `display_path` with a child path under `current_dir` and an unrelated absolute path. Assert the child result starts with `./` and the unrelated path remains absolute.

- [x] **Step 2: Run the focused test and confirm failure**

Run: `cargo test terminal::tests::display_path --lib`

Expected: failure until the formatter is added in Task 1; this step remains documented as the path regression guard.

- [x] **Step 3: Replace main completion printing**

Use plain separators and these sections:

```text
========== 采集完成 ==========
耗时：89 秒 | 请求：115 | 检测项：46
输出目录：./model-doctor-output
日志：model-doctor.log

========== 自分析完成 ==========
可用：46（PASS 45 / FAIL 1）| 不可用：0
JSON：model-doctor-self-analysis.json
结果：model-doctor-self-analysis.md
```

Derive the output directory from `outcome.log_path.parent()` and use `display_path` for it. Keep cancellation wording and error behavior intact.

- [x] **Step 4: Update README and run CLI integration test**

Document the two-stage terminal progress. Run: `cargo test --test cli_self_analysis`

Expected: PASS; default non-self-analysis runs must still not print self-analysis output.

### Task 5: Verify and deliver

**Files:**
- Modify: `docs/superpowers/plans/2026-08-24-terminal-progress.md`

- [x] **Step 1: Run complete verification**

Run `cargo fmt --check`, `git diff --check`, `cargo test --all-targets`, `cargo clippy --all-targets -- -D warnings`, `cargo build --release`, and `python3 -m unittest discover -s skills/creating-model-doctor-reports/tests -p 'test_*.py'`.

- [x] **Step 2: Inspect output-oriented diff and mark plan complete**

Confirm the diff has no ANSI escape construction, no credentials/evidence-body printing, preserves 46 catalog manifests and 057's four waves, and preserves silent test helper behavior.

- [ ] **Step 3: Commit and build macOS ARM64 CLI**

Force-add the ignored plan file; commit with `feat: improve terminal progress output`. Run `cargo build --release --target aarch64-apple-darwin`, copy the binary to `dist/model-capability-doctor-v0.12.0-macos-arm64`, then verify `file`, SHA-256, and `--help`.
