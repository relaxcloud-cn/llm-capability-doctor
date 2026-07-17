# LLM Capability Doctor（大模型能力诊断工具）

面向智能体平台模型接入场景的可审计能力检查工具。当前版本为
`0.7.0`，采用严格的“采集证据”和“评判证据”两阶段架构。

## 职责边界

- Shell 脚本只负责执行 curl 并记录完整证据。
- Skill 是唯一的二元评判者。
- Shell 不比较答案、不判断能力、不输出测试状态、不汇总语义成功率。
- Skill 不再次调用被测接口，只评判 manifest 明确引用的请求。

脚本输出 `llm-capability-doctor.evidence.v1` 日志。Skill 输出
`llm-capability-doctor.assessment.v3` JSON 和离线 HTML 报告。

## 运行采集器

现场环境只需要 Bash、curl 和常见系统文本工具。推荐通过环境变量传递密钥：

1. 将 `model-capability-doctor.sh` 传入客户服务器，执行：

```bash
MODEL_API_KEY='your-secret' ./model-capability-doctor.sh \
  --url 'https://model.example/v1/chat/completions' \
  --model 'your-model-name' \
  --log-file './model-doctor-evidence.log'
```

2. 执行完成后，将客户现场的 `model-doctor-evidence.log` 拷贝出来。

脚本会记录：

- 每次 curl 的脱敏复现命令和完整请求体；
- curl 退出码、HTTP 状态、总耗时、TTFB 和响应字节数；
- 完整脱敏响应头、curl stderr 和响应体；
- 每个测试 manifest 及其有序 `request_refs`；
- 运行时长、curl 请求数和 manifest 数。

日志没有能力结论。HTTP 2xx、模型名称或单次成功响应都不会被脚本解释为
`PASS`。

### 采集部分测试

`--only` 只控制要采集的测试项，不改变职责边界：

```bash
MODEL_API_KEY='your-secret' ./model-capability-doctor.sh \
  --url 'https://model.example/v1/messages' \
  --model 'your-model-name' \
  --only '002,003,047,048,049,055,056' \
  --log-file './focused-evidence.log'
```

使用 `./model-capability-doctor.sh --list-tests` 查看 62 项目录。

## 生成报告

先在本地 Codex 安装报告 Skill：

```text
帮我安装 https://github.com/relaxcloud-cn/llm-capability-doctor/tree/main/skills/creating-model-doctor-reports 到本地
```

然后将 evidence-v1 日志交给 Skill：

```text
使用 model doctor report 技能分析 model-doctor-evidence.log
```

将 evidence-v1 日志交给 `creating-model-doctor-reports` Skill。Skill 会：

1. 严格解析 schema、计数、重复块和显式请求引用；
2. 逐项检查 manifest 引用的完整输入、输出和指标；
3. 为每个测试写一个 `PASS` 或 `FAIL`；
4. 生成 assessment v3 和自包含 HTML 报告。

测试和能力域只有 `PASS`、`FAIL`。总体只有 `READY`、`BLOCKED`。
缺少请求、关联链不完整、传输失败、超时、协议拒绝、响应格式错误或证据含糊
都必须明确评为 `FAIL`，并附可观察原因。重跑说明不会产生另一种状态。

## 检测范围

目录固定为 `001-062`，覆盖：

- 接口与协议；
- 结构化结果；
- 上下文容量与召回；
- 指令遵循与文本处理；
- Thinking 与推理；
- 工具调用和多轮工具结果回传；
- 性能与稳定性；
- 护栏与安全词汇。

工具链 `047-049` 会按 OpenAI Chat Completions、OpenAI Responses、
Anthropic Messages、Gemini GenerateContent 和 Ollama Chat 的协议要求，
把首轮真实 assistant 对象或关联 ID 带入第二次 curl。脚本只完成协议编排，
最终是否满足工具调用契约由 Skill 评判。

性能项保留原始逐请求指标：`055/056` 共享五次请求，`057` 记录
4、8、16、32 四个并发波次的全部 60 次请求，`058` 记录十次持续请求和一次
独立恢复请求。分位数、语义成功率和就绪结论由 Skill 计算。

## 破坏性变更

旧日志不受支持，必须使用 v0.7.0 重新采集。解析器不会推断、迁移或升级其他
日志格式；旧 assessment JSON 和旧 HTML 也不能作为 v3 输入继续处理。

这样可以保证一条清晰的证据链：curl 原始输入输出来自 Shell，所有能力结论
来自 Skill，不存在脚本结论与 Skill 结论冲突或“等待补证”的中间状态。
