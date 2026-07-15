# LLM Capability Doctor

`llm-capability-doctor` 是一个单文件大模型能力体检工具。用户提供完整模型 URL、模型名称和 API Key 后，脚本通过真实请求检测模型的通用能力，并生成可复盘的审计日志。

运行时不依赖 Docker、Python、Node.js 或 `jq`，只需要 Bash、`curl` 和常见系统文本命令。

## 检测范围

脚本当前包含 113 个检测项：

- 100 项通用能力：接口协议、8K/16K 长输出、生成控制、指令遵循、结构化输出、文本处理、上下文、Thinking、工具调用、静态代码推理、性能和并发稳定性。
- 12 项护栏与词汇可用性：越权请求、合法防御分析、恶意分析、漏洞、告警、恶意软件、攻击、威胁、payload、exploit 等词汇下的能力保持。
- 1 项长结果完整性：调查阶段、证据定义和 `evidenceRefs` 引用覆盖。

工具不计算总分，也不检测多模态、闭卷知识或某个具体业务系统的专项能力。

## 支持的常见协议

- OpenAI Chat Completions
- OpenAI Responses 常见信封
- Anthropic Messages
- Gemini GenerateContent
- Ollama Chat

脚本原样使用用户提供的完整 URL，不补全、不裁剪也不改写路径。无法识别的响应协议会保留完整输入输出，并将不能可靠解释的项目标记为 `UNDETERMINED`。

## 快速开始

```bash
chmod +x model-capability-doctor.sh

read -rsp '请输入 API Key: ' MODEL_API_KEY
echo
export MODEL_API_KEY

./model-capability-doctor.sh \
  --url 'https://model.example/v1/chat/completions' \
  --model 'your-model-name' \
  --timeout 180 \
  --log-file './model-doctor.log'

unset MODEL_API_KEY
```

推荐通过 `MODEL_API_KEY` 环境变量输入 Key，避免凭据进入 Shell 历史。脚本也兼容下面的形式，但不推荐在共享服务器上使用：

```bash
./model-capability-doctor.sh \
  --url 'https://model.example/v1/chat/completions' \
  --model 'your-model-name' \
  --api-key 'your-api-key'
```

## 选择检测项

查看全部检测项：

```bash
./model-capability-doctor.sh --list-tests
```

只检测 8K、16K 长输出以及调查阶段和证据引用完整性：

```bash
./model-capability-doctor.sh \
  --url 'https://model.example/v1/chat/completions' \
  --model 'your-model-name' \
  --only '011,012,113' \
  --timeout 180 \
  --log-file './long-output.log'
```

`--only` 接受逗号分隔的三位检测项 ID。省略该参数时执行全部 113 项。

## 输出与日志

终端逐项输出结论性结果：

```text
检测项 067：单工具调用
检测结果：通过
检测结论：返回 get_weather 工具调用及 city=Beijing
```

日志默认命名为 `model-doctor-YYYYMMDD-HHMMSS.log`，权限强制设置为 `0600`。每次真实网络请求都会保存：

- 脱敏且可复现的完整 curl 命令
- 完整请求 JSON
- curl 退出码、HTTP 状态、总耗时、首字节耗时和下载字节数
- 完整响应头
- 完整 curl stderr
- 完整响应正文
- 对应检测项的预期、检测值和最终结论

协议探测、参数回退、对照实验、多轮工具调用、重复请求和每一个并发请求都会生成独立的 `REQUEST ... BEGIN/END` 审计块。

API Key 不会明文写入日志。curl 命令中的鉴权值使用 `${MODEL_API_KEY}` 占位符，URL 查询凭据、响应中回显的 Key、`Authorization`、`Set-Cookie` 等敏感字段会被脱敏。

日志包含完整提示词、上下文和响应，完整运行时文件可能较大，也可能包含业务数据。不要把运行日志直接提交到公开仓库。

## 状态含义

| 状态 | 含义 |
| --- | --- |
| `PASS` | 请求成功且结果满足检测预期 |
| `FAIL` | 接口接受请求，但输出不符合预期 |
| `UNSUPPORTED` | 接口明确拒绝该参数或能力 |
| `UNDETERMINED` | 收到响应，但纯 Shell 无法可靠解释 |
| `SKIPPED` | 前置能力不满足，未继续执行 |
| `ERROR` | 网络、超时或其他执行错误 |

能力检测失败不会中止后续项目。完整运行结束通常返回 `0`；参数错误返回 `2`；缺少运行依赖或无法创建安全日志时返回 `1`。

## 注意事项

- 8K/16K 输出、32K/64K 上下文、重复请求和并发测试会产生真实 token 消耗。
- 默认单请求超时为 30 秒。长输出或慢速模型建议设置 `--timeout 180` 或更高。
- 上下文长度由字符负载近似构造，不等同于厂商 tokenizer 的精确 token 数。
- 性能结果同时受到模型、网关、网络、限流策略和当前 Key 配额影响。
- 纯 Shell 检查可以核对常见 JSON 信封、约定字段和引用关系，但不等同于完整 JSON Schema 校验。
