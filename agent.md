# AgentCheck 编码代理约定

本文件约束所有在本仓库工作的编码代理（Claude Code、Codex 等）。

## 编译产物命名（必须遵守）

CLI 的编译产物必须命名为 **`agent-check`**（Windows 为 `agent-check.exe`）。

- `Cargo.toml` 的 `[[bin]]` 已配置为直接产出 `agent-check`，开发构建产物在 `target/release/agent-check`。
- `scripts/package-release.sh` 打包单文件发行版时同样输出 `agent-check`（内置 omp 与运行时）。
- 新增构建、打包、交付脚本时，产物名必须是 `agent-check`，不得引入其他命名（如 `llm-capability-doctor`）。
- crate/lib 的内部名仍是 `llm-capability-doctor`，仅产物可执行文件命名受本约定约束。

## dynamic 报告模式的运行时依赖

`--mode dynamic` 依赖 OhMyPi（可执行文件 `omp`）做逐模块 AI 分析。非打包构建需满足任一条件：

- 将 `omp` 可执行文件放在与 CLI 相同目录（`target/release/omp`）；
- 设置环境变量 `OMP_BIN` 指向 `omp` 的绝对路径。

## 提交规范

提交信息使用中文，格式 `feat: 描述` / `fix: 描述`，正文说明改动动机与要点。
