# AgentCheck 首版最终发布评审记录

日期：2026-09-11  
集成基线：`lbl/agent-check`  
基线提交：`79e8139227a27adfc2520b82e9d8edd0279f3476`  
产品版本：`0.1.0`  
发布状态：`release-candidate`

## 1. 发行包

在干净的 `lbl/agent-check` 工作树执行：

```bash
RELEASE_OUTPUT_DIR=./dist ./scripts/package-release.sh
tar -xzf ./dist/llm-capability-doctor-v0.1.0-<target>.tar.gz -C /tmp/agentcheck-release
sha256sum -c SHA256SUMS
./llm-capability-doctor --version
```

本次实际目标为 `aarch64-apple-darwin`，包 SHA-256 为：

```text
16ce7c4d39b26b8077e07daeefc4b55615d41893ce00e111f7dbf7a33cda75b9
```

解包后的二进制输出 `llm-capability-doctor 0.1.0`，`--help` 可识别 URL、模型、模块选择、停止、输出格式和 GUI 回退参数。包只包含二进制、README、VERSION、BUILD 和 SHA256SUMS，不包含源码、`target/` 或凭据。

## 2. 真实服务专项端到端

使用解包后的发行二进制、授权 Chat Completions 服务和模型 `doubao-1-5-pro-32k-250115` 执行：

```bash
MODEL_API_KEY='[通过环境变量注入]' ./llm-capability-doctor \
  --url 'https://ark.cn-beijing.volces.com/api/v3/chat/completions' \
  --model 'doubao-1-5-pro-32k-250115' \
  --modules ingress,agent,baseline \
  --no-gui --format json --output ./packaged-e2e.json --timeout-seconds 60
```

实际结果：生命周期 `completed`，总体结论 `limited`，证据记录 13 条，模块状态为 `ingress=pass`、`agent=fail`、`baseline=pass`；Agent 10 个场景和基线 14 个场景均产生报告对象，模块证据引用无缺失，API key 未进入 JSON。该结果是当前服务配置下的专项实测，不泛化为模型或生产可靠性结论。

## 3. 默认全测编排

使用同一发行二进制对本地受控 Chat Completions fixture 执行省略 `--modules` 的默认流程，命令退出码为 `0`。实际结果：生命周期 `completed`，六个正式模块均有结果入口，统一记录包含 2,695 条证据，其中 2,679 条为性能样本；报告可写入 JSON。fixture 仅验证编排、状态、性能矩阵展开和证据写入，不用于证明模型能力。

真实外部服务没有执行完整性能矩阵，以避免未经客户明确授权的长时/高请求量负载。真实服务专项和本地默认全测分别覆盖真实性与完整编排边界。

## 4. GUI、CLI 与记录一致性

以下回归覆盖有桌面、无桌面、CI 和 GUI 启动失败：

```bash
cargo test --locked gui::tests::desktop_detection_keeps_linux_headless_and_ci_on_cli
cargo test --locked gui::tests::gui_failure_returns_a_cli_fallback_result
cargo test --locked cli::tests::selected_modules_are_saved_and_unselected_modules_are_explicit
```

GUI 使用同一 `CliRunReport`，无桌面、CI 或启动失败时保留 CLI 结果，不改写模型结论。发行包真实专项使用 `--no-gui` 成功完成并取得 JSON 报告。

## 5. 质量门禁与限制

基线上的验证命令：

```bash
cargo fmt --all
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
cargo build --release --locked
```

当前 #34 的独立校准门禁为 `insufficient_evidence`：受控保留集未发现已知误判，但样本规模、真实客户工作流和人工争议复核不足以证明统计可靠性或生产泛化。因此本版本只能交付为可安装、可运行、可取得报告和证据的 release candidate，不宣称生产可靠性、最大用户数、费用或完整业务覆盖。

本记录不包含 API key、客户敏感数据或完整原始响应；真实报告保存在运行环境的受控路径中，公开交付只保留脱敏后的状态、范围和证据索引。
