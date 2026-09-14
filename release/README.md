# llm-capability-doctor 首版 CLI

## 支持边界

发行包只声明构建时确认的目标三元组，不默认覆盖其他操作系统或 CPU 架构。包内二进制可直接运行；运行时需要能够访问目标 Chat Completions 服务。

macOS 发行包另外包含 `AgentCheck.app`。在有桌面的 macOS 环境直接运行 CLI 时，会先打开原生桌面端；桌面端负责配置确认、启动检测、显示模块进度、查看结果和导出报告。`--no-gui`、SSH、CI 或无桌面环境继续使用纯 CLI。

## 前置条件

- 可执行目标平台的发行包。
- 已授权的 Chat Completions 服务 URL 和模型名。
- API 密钥通过 `MODEL_API_KEY` 环境变量提供。
- 服务返回 OpenAI 风格 `choices` 响应；不兼容协议会保留为协议失败或不可测量。

## 启动

```bash
export MODEL_API_KEY='授权测试密钥'
./llm-capability-doctor \
  --url 'https://service.example.test/v1/chat/completions' \
  --model 'model-id' \
  --format json \
  --output ./agentcheck-run.json
```

在 macOS 桌面环境中，上面的命令会启动 AgentCheck 桌面端；如果需要无人值守或直接生成 JSON，追加 `--no-gui`。

默认执行六个正式模块。专项运行示例：

```bash
./llm-capability-doctor --url "$MODEL_URL" --model "$MODEL_ID" \
  --modules ingress,baseline --no-gui --format json --output ./selected.json
```

`--stop-after MODULE` 会保留停止前事实，后续模块为 `unverified`。桌面端的导出内容直接来自同一次 Rust CLI 检测，不重新解释模块结论。

完整评估流程（发行包已包含对应平台的 OhMyPi `omp`）：

```bash
./llm-capability-doctor \
  --url "$MODEL_URL" \
  --model "$MODEL_ID" \
  --no-gui \
  --report-dir ./agentcheck-report \
  --html ./agentcheck-report/report.html
```

该流程会为每个模块保存 `module-input/*.input.json`，CLI 会自动生成本次运行的 OhMyPi 模型配置，让 OhMyPi 使用同一个客户模型，再生成 `module-report/*.report.json` 和 `report.html`。也可以通过 `OMP_BIN` 指定外部 OhMyPi 路径。OhMyPi 不可用或输出不符合约定时，模块报告保留为 `inconclusive`。

## 结果边界

报告会区分 `pass`、`fail`、`inconclusive`、`invalid_execution` 和 `unverified`。Agent 场景没有观察到工具、权限或产物事实时不会判定通过；性能请求错误不进入正常速度指标；结构基线差异不直接推导整体可用性。Issue #34 的可靠性门禁仍为 `insufficient_evidence`，本包不声称生产可靠性。

## 完整性

使用包内 `SHA256SUMS` 校验文件，使用 `VERSION` 核对版本；校验通过后再执行 `--version`。
