# LLM Capability Doctor

`llm-capability-doctor` 是一个单文件大模型能力体检工具。用户提供完整模型 URL、模型名称和 API Key 后，脚本通过真实请求检查模型是否具备 ClawOps 所需的核心通用能力，并生成可复盘的完整请求/响应日志。

运行时不依赖 Docker、Python、Node.js 或 `jq`，只需要 Bash、`curl` 和常见系统文本命令。

## 检测范围

脚本包含 62 个连续编号的核心检测项，不保留旧版 113 项兼容目录：

| ID | 能力范围 | 检测重点 |
| --- | --- | --- |
| `001-008` | 接口与协议 | URL、协议识别、鉴权、模型名称、同步/流式、usage、错误正文 |
| `009-013` | 结构化结果 | 裸 JSON、字段类型、嵌套结构、Result 核心字段、阶段与证据引用 |
| `014-015` | 长输出结果 | 8K/16K 可见 Result JSON、Token、正文长度、收尾标记和结构完整性 |
| `016-022` | 指令与文本 | 精确输出、组合格式、修正优先级、抽取、分类、摘要、去重 |
| `023-031` | 上下文 | 8K/32K/64K 级字符近似、首中尾召回、跨段关联、干扰、多轮修正 |
| `032-039` | Thinking 与推理 | 参数、low/high 档、reasoning token、答案分离、流事件、计算、逻辑、规划复核 |
| `040-050` | 工具调用 | 选择、参数 Schema、嵌套参数、并行、Call ID、串行、结果回传、失败恢复、大目录 |
| `051-058` | 性能与稳定性 | 总延迟、首字节、首 Token 近似、重复成功率、P50/P95、8 并发、持续负载 |
| `059-062` | 护栏与词汇 | 越权请求拦截、合法防御任务、安全词不过度拒答、中英文一致性 |

查看完整目录：

```bash
./model-capability-doctor.sh --list-tests
```

工具不计算总分，也不检测多模态、闭卷知识或简单代码选择题。

## 支持的常见协议

- OpenAI Chat Completions
- OpenAI Responses 常见信封
- Anthropic Messages
- Gemini GenerateContent
- Ollama Chat

脚本原样使用用户提供的完整 URL，不补全、不裁剪也不改写路径。协议探测会针对同一个完整 URL 尝试常见请求信封和鉴权头；无法识别的响应会保留完整输入输出，相关能力标记为 `UNDETERMINED`。

OpenAI Chat 和 Responses 支持标准工具结果回传闭环。其他协议能够检测基础工具调用；如果纯 Shell 无法安全构造带原始 Call ID 的后续回传，则明确标记为 `UNDETERMINED`，不会伪造通过。

## 快速开始

```bash
chmod +x model-capability-doctor.sh

read -rsp '请输入 API Key: ' MODEL_API_KEY
echo
export MODEL_API_KEY

./model-capability-doctor.sh \
  --url 'https://model.example/v1/chat/completions' \
  --model 'your-model-name' \
  --timeout 300 \
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

## 定向复测

只复测长输出：

```bash
./model-capability-doctor.sh \
  --url 'https://model.example/v1/chat/completions' \
  --model 'your-model-name' \
  --only '014,015' \
  --timeout 300 \
  --log-file './long-output.log'
```

只复测标准工具闭环：

```bash
./model-capability-doctor.sh \
  --url 'https://model.example/v1/chat/completions' \
  --model 'your-model-name' \
  --only '047,048,049' \
  --log-file './tool-closure.log'
```

`--only` 接受逗号分隔的三位检测项 ID。省略时执行全部 62 项。

## 输出与日志

终端逐项输出结论性结果：

```text
检测项 047：串行工具调用
检测结果：通过
检测结论：标准工具结果回传后继续调用 get_time
```

日志默认命名为 `model-doctor-YYYYMMDD-HHMMSS.log`，权限强制设置为 `0600`。每次真实请求都会保存：

- 脱敏且可复现的完整 curl 命令
- 完整请求 JSON
- curl 退出码、HTTP 状态、总耗时、首字节耗时和下载字节数
- 完整响应头
- 完整 curl stderr
- 完整响应正文
- 检测项预期、检测值和结论

协议探测、参数对照、多轮工具回传、重复请求和每一个并发请求都会生成独立的 `REQUEST ... BEGIN/END` 审计块。

API Key 不会明文写入日志。curl 命令使用 `${MODEL_API_KEY}` 占位符，URL 查询凭据、响应回显 Key、`Authorization`、`Set-Cookie` 等敏感内容会被脱敏。

日志包含完整提示词、上下文和模型响应，文件可能较大，也可能包含业务数据。不要把运行日志直接提交到公开仓库。

## 状态含义

| 状态 | 含义 |
| --- | --- |
| `PASS` | 请求成功，且日志中的实际证据满足该检测项要求 |
| `FAIL` | 接口接受请求，但模型可见答案或工具行为不满足要求 |
| `UNSUPPORTED` | 接口明确拒绝该参数或能力 |
| `UNDETERMINED` | 已收到证据，但纯 Shell 或当前协议无法可靠完成判定 |
| `SKIPPED` | 前置能力不满足，未继续执行 |
| `ERROR` | 网络、超时或其他执行错误，不能解释为模型能力失败 |

能力检测失败不会中止后续项目。完整运行结束通常返回 `0`；参数错误返回 `2`；缺少运行依赖或无法创建安全日志时返回 `1`。

## 判定边界

- 上下文档位使用字符负载近似：约 32K、128K、256K 字符分别代表 8K、32K、64K 级上下文压力，不等同于供应商 tokenizer 的精确 Token 数。
- 首 Token 时间使用流式请求的首个响应字节近似；代理缓冲可能影响该值。
- P50/P95 来自 5 次真实成功请求的端到端延迟排序，是本次运行快照，不是服务 SLA。
- 8K/16K 长输出同时检查可见输出 Token、可见正文字节、完成标记、Result 字段、三个调查阶段、六个证据及其引用。默认 30 秒通常不足，建议使用 `--timeout 300`。
- 结构化检测针对已识别响应信封提取模型可见答案，并验证该项要求的具体结构；它不是任意厂商 JSON Schema 引擎。
- 性能结果同时受到模型、网关、网络、限流策略和当前 Key 配额影响。
- curl-only 脚本可以验证模型 HTTP 接口和标准工具协议，但不能独立认证完整 Codex app-server、MCP、namespace 工具、skills 和 approvals 运行链；ClawOps 上线前仍应运行产品内的模型连接诊断。
