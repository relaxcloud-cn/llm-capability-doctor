# AgentCheck

模型接入检测工具：5 组接口检测，3 类真实 OMP 文件任务，每类执行两次。Rust 负责检测、判定、证据和报告，Electron 展示相同结果。

当前可本机试用 **macOS arm64 单文件候选版**。Windows、Linux、Responses 真实模型、官方基线和正式签名尚未完成验收；当前结果不能当作跨平台正式交付。进展和验收缺口见 [实现状态](docs/implementation-status.md)。冻结要求见 [最终计划](docs/agent-check-final-plan-2026-10-09.md)。

0.2.0 / `agentcheck/2`：单次请求超时为 180 秒；并发按 1、2、4、8、16、32 逐档探测短请求能力，不要求 token 明细，也不自动重试掩盖限流。到达 32 只表示至少支持 32。结构化输出的唯一 JSON 代码块若数据正确则兼容通过，并保留原生格式未满足提示。新规则来自用户 2026-10-10 的明确修订，旧检测记录不重写。

客户操作说明见 [使用手册](docs/customer/使用手册.md)。

## 运行

```sh
./dist/agent-check-macos-arm64 --config /absolute/path/model.json
```

默认先检查连接，桌面可用时自动打开 GUI 并继续同一次检测。强制终端使用 `--no-gui`；`--json` 自动使用终端，最终 JSON 写 stdout，进度写 stderr；`--no-color` 关闭颜色。

配置文件示例，密钥通过环境变量引用：

```json
{
  "base_url": "https://model.example.com/v1",
  "model": "your-model",
  "api_key_env": "MODEL_API_KEY",
  "protocol": "chat-completions"
}
```

也支持仓库外的私有配置文件中提供 `api_key`。不要提交含密钥的配置。`protocol` 可为 `chat-completions` 或 `responses`，完整接口地址与显式协议不一致时会拒绝运行。

每次检测保存独立目录，包含 `result.json`、`report.html`、`records.zip` 和脱敏原始记录。macOS 默认目录为 `~/Library/Application Support/AgentCheck/results`，可用 `--report-dir` 指定。缓存默认在 `~/Library/Caches/AgentCheck`，客户无需安装 Node、Bun、Python 或 OMP。

退出码：通过为 `0`，已确认失败为 `1`，未完成或启动错误为 `2`，用户取消为 `130`。GUI 启动交接成功时入口退出码 `0` 仅表示交接成功，检测结论以窗口和保存结果为准。

## 不访问模型的本机自检

```sh
./dist/agent-check-macos-arm64 --offline-self-test
./dist/agent-check-macos-arm64 --offline-self-test --gui
```

第一条校验、释放内置组件并核对 OMP 版本；第二条另外打开可交互 GUI，显示本机历史，不需要模型配置，也不会自动发起模型请求。可加 `--report-dir` 指向现有记录目录。

## 开发与打包

```sh
cargo test
cargo clippy --all-targets -- -D warnings
./scripts/build-macos.sh
python3 scripts/verify-runtime.py dist/agent-check-macos-arm64 --output .cache/runtime-verification.json
```

构建侧需要 Rust、Node/npm、Python 和获取固定组件的网络权限；这些不是客户运行依赖。构建脚本目前支持本机 macOS arm64，输出候选版及 SHA-256。未签名、公证及未完成的目标平台验收不会被脚本伪装成通过。
