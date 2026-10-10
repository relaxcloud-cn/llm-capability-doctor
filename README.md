# AgentCheck

从空分支重新实现的模型接入检测工具。产品与验收要求见 [最终计划](docs/agent-check-final-plan-2026-10-09.md)。

- 5 组接口检测与 3 类 OMP 任务（每类两次）。
- Rust 共用检测核心，macOS / Windows 使用 Electron GUI，Linux 使用 CLI。
- 报告、CLI、GUI 以 `docs/*-reference/` 的冻结原型为准。

当前开发进度与实测证据见 [实现状态](docs/implementation-status.md)。
