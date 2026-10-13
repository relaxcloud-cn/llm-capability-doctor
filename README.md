# AgentCheck

模型接入检测工具：5 组接口检测，3 类真实 OMP 文件任务，每类执行两次。Rust 负责检测、判定、证据和报告，Electron 展示相同结果。

当前可本机试用 **macOS arm64 单文件候选版**。Windows、Linux、Responses 真实模型、官方基线和正式签名尚未完成验收；当前结果不能当作跨平台正式交付。进展和验收缺口见 [实现状态](docs/implementation-status.md)。冻结要求见 [最终计划](docs/agent-check-final-plan-2026-10-09.md)。

0.4.6 / `agentcheck/10` / 结果格式 v2：检测注册定义、原始观察、证据有效性、要求满足情况与接入总评分离。每个正式通过或不通过的要求都引用有路径、大小和 SHA-256 的证据；检测器、运行时、配置问题及证据冲突保留为未确认。CLI、GUI 和 HTML 读取同一份裁决，报告提供逐项证据入口和兼容条件；旧结果不按新规则重新判定。

并发使用固定普通短文本请求，只核对协议响应有效、正常结束且有非空正文；不要求 JSON、固定措辞或原样回显，也不评价正文内容准确性。仍按 1、2、4、8、16、32… 翻倍探测，平均完整响应时间 ≤ 10 秒、P95 ≤ 20 秒（少于 20 个请求用最慢耗时）；单请求硬超时 180 秒，不二分、不自动重试。限流、硬超时或响应过慢可支持上一合格档位的粗略建议；空回复、截断、拒绝、未请求的工具调用、协议/网络异常不冒充容量边界。每次请求通过本地 HTTP 证据编号关联，不要求模型回显标识。并发规则版本为 2，旧 JSON 回显记录保留原规则与原结论，不按新标准补判。

JSON 内容与格式仍由独立的结构化输出项核验；该项允许兼容解析唯一 JSON 代码块，兼容条件明确保留，不宣称支持原生严格 Schema。

直接工具调用检测不以服务端严格工具模式为前提：Chat Completions 不发送工具 `strict`，Responses 显式发送 `strict: false`，避免其默认归一化启用严格模式。参数定义、工具选择、参数内容、调用关联和返回结果仍逐项核对；独立结构化输出项的原生严格约束不变。单次输出预算为 20000 tokens，完整请求超时为 600 秒，首轮流式请求与后续往返均适用。工具项总阶段预算为 1200 秒，避免仍被旧的 360 秒上限截断；其他检测的单请求预算仍为 180 秒，原有阶段和资源保护不变。HTTP 证据记录该请求配置的超时值。

扩展项通过 `Registration` 提供版本化要求、执行器和证据验证器，再交给 `engine::run_with_registry`。通用判定器不按检测名称分支；进度和展示读取本次注册计划。新检测仍须定义可验证的专用判据，并提供正反例。

正式报告按确认的三章节版式生成：检测结论、检测结果与基线对照、使用建议。主表由实际检查数量和状态生成，证据收进页尾附件；无真实基线时不展示空的参考列。HTML 核心内容静态可读，筛选、打印和 ZIP 下载不改动检测判定。新结果保存检测程序版本，旧结果缺失版本时明确显示未记录。

桌面 GUI 在正常结束并保存报告后自动打开 HTML；macOS/Windows 本机交互式 CLI 也会请求系统默认应用打开。JSON、SSH、CI、非交互终端及主动取消时不弹窗；`--no-open-report` 可关闭此行为。打开失败保留检测结果和手动查看入口。PDF 仍从 HTML 的“打印 / 保存 PDF”按需导出，不自动生成或打开。

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
node --test tests/desktop_report.test.cjs
./scripts/build-macos.sh
python3 scripts/verify-runtime.py dist/agent-check --output .cache/runtime-verification.json
```

构建侧需要 Rust、Node/npm、Python 和获取固定组件的网络权限；这些不是客户运行依赖。构建脚本目前支持本机 macOS arm64，输出候选版及 SHA-256。未签名、公证及未完成的目标平台验收不会被脚本伪装成通过。
