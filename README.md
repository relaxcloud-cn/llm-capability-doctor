# AgentCheck

模型接入检测工具：5 组接口检测，3 类真实 OMP 文件任务，每类执行两次。Rust 负责检测、判定、证据和报告，Electron 展示相同结果。

当前可本机试用 **macOS arm64 单文件候选版**。Windows、Linux、Responses 真实模型、官方基线和正式签名尚未完成验收；当前结果不能当作跨平台正式交付。进展和验收缺口见 [实现状态](docs/implementation-status.md)。冻结要求见 [最终计划](docs/agent-check-final-plan-2026-10-09.md)。

0.4.0 / `agentcheck/6` / 结果格式 v2：检测注册定义、原始观察、证据有效性、要求满足情况与接入总评分离。每个正式通过或不通过的要求都引用有路径、大小和 SHA-256 的证据；检测器、运行时、配置问题及证据冲突保留为未确认。CLI、GUI 和 HTML 读取同一份裁决，报告提供逐项证据入口和兼容条件；旧结果不按新规则重新判定。

并发仍按 1、2、4、8、16、32… 翻倍探测，平均完整响应时间 ≤ 10 秒、P95 ≤ 20 秒（少于 20 个请求用最慢耗时）；单请求硬超时 180 秒，不二分、不自动重试。首次不合格即停止，但内容错误、网络或普通服务错误不再冒充容量边界，也不给出据此推导的并发建议。结构化内容可兼容解析唯一 JSON 代码块；兼容条件明确保留，不宣称支持原生严格 Schema。

扩展项通过 `Registration` 提供版本化要求、执行器和证据验证器，再交给 `engine::run_with_registry`。通用判定器不按检测名称分支；进度和展示读取本次注册计划。新检测仍须定义可验证的专用判据，并提供正反例。

客户操作说明见 [使用手册](docs/customer/使用手册.md)。

## 运行

```sh
./dist/agent-check --config /absolute/path/model.json
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

每次检测保存独立目录，包含 `result.json`、`report.html`、`records.zip`、`evidence-index.json` 和脱敏原始记录。macOS 默认目录为 `~/Library/Application Support/AgentCheck/results`，可用 `--report-dir` 指定。缓存默认在 `~/Library/Caches/AgentCheck`，客户无需安装 Node、Bun、Python 或 OMP。

退出码：通过为 `0`，已确认失败为 `1`，未完成或启动错误为 `2`，用户取消为 `130`。GUI 启动交接成功时入口退出码 `0` 仅表示交接成功，检测结论以窗口和保存结果为准。

## 不访问模型的本机自检

```sh
./dist/agent-check --offline-self-test
./dist/agent-check --offline-self-test --gui
```

第一条校验、释放内置组件并核对 OMP 版本；第二条另外打开可交互 GUI，显示本机历史，不需要模型配置，也不会自动发起模型请求。可加 `--report-dir` 指向现有记录目录。

## 开发与打包

```sh
ulimit -n 4096 && cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
./scripts/build-macos.sh
python3 scripts/verify-runtime.py dist/agent-check --output .cache/runtime-verification.json
```

构建侧需要 Rust、Node/npm、Python 和获取固定组件的网络权限；这些不是客户运行依赖。构建脚本目前支持本机 macOS arm64，输出候选版及 SHA-256。未签名、公证及未完成的目标平台验收不会被脚本伪装成通过。
