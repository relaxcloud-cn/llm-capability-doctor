# Cross-Platform Terminal Progress Design

## Goal

Make the CLI terminal output readable during both 46-check collection and local self-analysis without changing detection requests, evidence.v4 logs, JSON/Markdown artifacts, or model PASS/FAIL authority.

## Output Contract

- Use plain UTF-8 text only. Do not use ANSI color, cursor movement, spinners, or box-drawing characters.
- Every collection line includes collection position, customer-facing category, and customer-facing test purpose:

  ```text
  [采集 14/46] 上下文能力 | 测试模型是否支持 8K 上下文长度
  ```

- The 057 internal check remains one of 46 checks, but each concurrent wave is displayed separately:

  ```text
  [采集 44/46 | 并发 1/4] 性能与稳定性 | 测试模型在 4 并发下能否正常响应
  ```

- The terminal-only names are independent from catalog/log/Markdown names. Existing evidence and artifact contracts remain unchanged.
- Every self-analysis batch emits a start line and completion line. Retry activity uses its existing bounded retry count and does not expose evidence text:

  ```text
  [自分析 03/12] 结构化结果 | 009-012 | 正在分析
  [自分析 03/12] 结构化结果 | 已完成
  [自分析 04/12] 上下文能力 | 重试 1/3
  ```

- Completion output is divided into collection summary, analysis summary, and artifact locations. Paths are made relative to the current working directory when possible; otherwise preserve the absolute path. The output names files rather than repeating long paths.

## Architecture

- Add terminal-only category/title accessors beside the static catalog. All 46 checks receive a customer-facing title; raw `TestCase.name` remains the audit name.
- Pass collection position and optional 057 wave information from `Runner::execute` into group execution so progress lines appear before each actual concurrent wave.
- Add a small analysis-progress reporter interface. Public `analyze` uses the terminal reporter; test-only `analyze_with_client` uses a no-op reporter. The orchestrator emits batch start, retry, and batch completion without changing analysis request payloads or decisions.
- Keep terminal rendering in a focused formatter module shared by runner, main, and the analysis reporter. It renders paths safely and has no network or model dependencies.

## Error Handling

- A collection error retains the last printed collection item and uses the existing error path.
- A failed analysis batch emits an explicit unavailable completion line; later batches continue as today.
- A cancellation emits the existing cancellation result and does not fabricate completed batches.

## Tests

- Verify representative terminal labels, all five context labels, and all four 057 wave labels.
- Verify terminal lines carry progress, category, and expanded purpose without ANSI escape bytes.
- Verify analysis event ordering for start, retry, completion, unavailable, and no-op test execution.
- Verify relative path rendering and absolute-path fallback.
- Run Rust full tests, CLI integration, Clippy, release build, Python tests, formatting, and diff checks.
