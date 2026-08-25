# 大模型能力诊断工具

## 检测范围

工具共执行 46 项检测，覆盖模型接入、能力适配和稳定性验证。

| 检测方向 | 覆盖内容 |
| --- | --- |
| 接口与协议 | URL 可达性、协议识别、鉴权、同步与流式生成、流结束、Token usage、错误可观测性 |
| 结构化结果与上下文 | JSON 输出、字段类型、嵌套数据、证据引用、8K 至 128K 上下文、多轮修正记忆 |
| 指令、推理与工具调用 | 精确输出、格式约束、信息抽取、摘要、Thinking、逻辑推理、单工具/串行/并行工具调用及失败恢复 |
| 性能、稳定性与护栏 | 首字节与完整响应延迟、重复成功率、P50/P95、4 至 32 并发、中文与英文安全业务词可用性 |

## 执行命令

进入解压后的程序目录，将命令中的接口地址、模型名称和 API Key 替换为实际值后执行。

### Windows x86_64（CMD）

```cmd
if not exist "model-doctor-output" mkdir "model-doctor-output"
.\model-capability-doctor-v0.12.0-windows-x86_64.exe --url "https://model.example/v1/chat/completions" --model "your-model-name" --api-key "your-api-key" --log-file ".\model-doctor-output\model-doctor.log"
```

### macOS ARM64

```bash
chmod +x ./model-capability-doctor-v0.12.0-macos-arm64
mkdir -p ./model-doctor-output
./model-capability-doctor-v0.12.0-macos-arm64 --url 'https://model.example/v1/chat/completions' --model 'your-model-name' --api-key 'your-api-key' --log-file './model-doctor-output/model-doctor.log'
```

### Linux x86_64

```bash
chmod +x ./model-capability-doctor-v0.12.0-linux-x86_64
mkdir -p ./model-doctor-output
./model-capability-doctor-v0.12.0-linux-x86_64 --url 'https://model.example/v1/chat/completions' --model 'your-model-name' --api-key 'your-api-key' --log-file './model-doctor-output/model-doctor.log'
```

### Linux ARM64

```bash
chmod +x ./model-capability-doctor-v0.12.0-linux-arm64
mkdir -p ./model-doctor-output
./model-capability-doctor-v0.12.0-linux-arm64 --url 'https://model.example/v1/chat/completions' --model 'your-model-name' --api-key 'your-api-key' --log-file './model-doctor-output/model-doctor.log'
```

## 输出结果
执行命令默认完成 46 项模型能力检测、OpenCodex v2.7.42 兼容性检测和被测模型自分析，并输出：

1.以.log为后缀的文件：该文件记录了检测项的所有curl请求的输入和输出，用来给技术人员分析使用的。

2.以.md为后缀的文件：本次检测的自分析报告，用来直接查看检测结论，适合给客户看。

3.`model-doctor-opencodex-v2742.md`：OpenCodex v2.7.42 的 `openai-chat`、`anthropic` 和 `google` adapter 兼容性报告。

## OpenCodex v2.7.42 模型输出兼容性

每次执行都会自动检测同一个模型地址返回的数据能否被 OpenCodex v2.7.42 的 `openai-chat`、`anthropic` 和 `google` adapter 读取并转交给 Codex，不需要额外传入开关。

```bash
./model-capability-doctor-v0.12.0-macos-arm64 \
  --url 'https://model.example/v1/chat/completions' \
  --model 'your-model-name' \
  --api-key 'your-api-key' \
  --log-file './model-doctor-output/model-doctor.log'
```

检测会自动依次发送三种原生请求格式，不需要指定 adapter。每种格式的结果只有“通过”或“不通过”；不通过时，报告会列出 OpenCodex 规则编号、要求的返回字段、实际返回路径和值、以及对工具调用或完成事件的影响。

输出目录会新增：

```text
model-doctor-opencodex-v2742.json
model-doctor-opencodex-v2742.md
```

该阶段的规则固定为 OpenCodex `v2.7.42`，客户环境不需要安装 OpenCodex、Node.js、Bun、Docker，也不会联网下载规则。
