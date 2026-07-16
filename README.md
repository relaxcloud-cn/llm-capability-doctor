# LLM Capability Doctor

`llm-capability-doctor` 是一套证据优先的大模型能力体检流程。用户提供完整模型 URL、模型名称和 API Key 后，单文件 Shell 采集器通过真实请求生成可复盘的完整请求/响应日志；Codex Skill 再对日志做语义复核，生成标准评估 JSON 和可交付客户的离线 HTML 报告。

Shell 采集阶段不依赖 Docker、Python、Node.js 或 `jq`，只需要 Bash、`curl` 和常见系统文本命令。报告阶段由 `$creating-model-doctor-reports` Skill 运行，不改变 Shell 脚本的采集职责。

## 检测范围

当前脚本包含 65 个连续编号的核心检测项，不在采集脚本中保留旧版 113 项目录；报告 Skill 仍能读取旧版完整日志，并保留其中发现的全部历史检测项：

| ID | 能力范围 | 检测重点 |
| --- | --- | --- |
| `001-008` | 接口与协议 | URL、协议识别、鉴权、模型名称、同步/流式、usage、错误正文 |
| `009-013` | 结构化结果 | 裸 JSON、字段类型、嵌套结构、Result 核心字段、阶段与证据引用 |
| `014-018` | 长输出结果 | 8K/16K/32K/64K/128K 可见 Result JSON、Token、正文长度、收尾标记和结构完整性 |
| `019-025` | 指令与文本 | 精确输出、组合格式、修正优先级、抽取、分类、摘要、去重 |
| `026-034` | 上下文 | 8K/32K/64K 级字符近似、首中尾召回、跨段关联、干扰、多轮修正 |
| `035-042` | Thinking 与推理 | 参数、low/high 档、reasoning token、答案分离、流事件、计算、逻辑、规划复核 |
| `043-053` | 工具调用 | 选择、参数 Schema、嵌套参数、并行、Call ID、串行、结果回传、失败恢复、大目录 |
| `054-061` | 性能与稳定性 | 总延迟、首字节、首 Token 近似、重复成功率、P50/P95、8 并发、持续负载 |
| `062-065` | 护栏与词汇 | 越权请求拦截、合法防御任务、安全词不过度拒答、中英文一致性 |

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
  --only '014,015,016,017,018' \
  --timeout 300 \
  --log-file './long-output.log'
```

只复测标准工具闭环：

```bash
./model-capability-doctor.sh \
  --url 'https://model.example/v1/chat/completions' \
  --model 'your-model-name' \
  --only '050,051,052' \
  --log-file './tool-closure.log'
```

`--only` 接受逗号分隔的三位检测项 ID。省略时执行全部 65 项。

## 输出与日志

终端只显示当前正在执行的检测项，完整结果和结论写入日志：

```text
正在执行检测项 050：串行工具调用
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

## 从日志生成客户报告

完整流程严格分为两阶段：

1. `model-capability-doctor.sh` 只负责执行请求并保存完整审计日志，不承担跨模型语义解析。
2. `$creating-model-doctor-reports` 把日志当作不可信证据读取，逐项复核输入设计、协议事实和模型输出，再生成报告。

把 `.log` 文件提供给 Codex，并使用下面的提示：

```text
使用 $creating-model-doctor-reports 分析这份 Model Doctor 日志并生成客户就绪报告。
```

Skill 默认把两个产物写到源日志所在目录：

- `<model-slug>-assessment.json`：v2 唯一标准评估结果，只使用 Skill 语义复核状态，并保留证据引用、门禁和总体结论。
- `<model-slug>-customer-readiness-report.html`：由评估 JSON 确定性渲染的自包含离线报告。

如果目标文件已经存在，生成器会添加时间戳后缀，绝不覆盖旧报告。源 `.log` 只读，分析前后通过 SHA-256 验证未被修改。

报告由“能力域总结”“重要检测项”和“次要检测项”三张表组成。重要检测项固定覆盖连通性、最大上下文、Thinking、工具调用和并发性能；其余项目作为次要检测项。逐项状态与结论全部来自 Skill 语义复核；点击任意检测项可展开检测目的、检测方法、通过条件、完整脱敏请求输入和请求输出。

Skill 对日志执行第二次凭据脱敏，但报告仍包含完整提示词、模型响应和可能的业务数据。向客户或第三方分发前必须复核内容，不应把 HTML 或 JSON 直接提交到公开仓库。

总体结论使用门禁而不是平均分：critical 失败为 `BLOCKED`；没有 critical 失败但 critical/important 项存在非 `PASS` 为 `CONDITIONAL`；所有 critical/important 项通过才是 `READY`。证据缺失、方法未真正覆盖检测目标或输出含义不明确时，必须使用 `UNDETERMINED` 并给出复测方式。

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
- 单次请求默认超时为 120 秒，可通过 `--timeout SECONDS` 覆盖。8K/16K/32K/64K/128K 长输出同时检查可见输出 Token、可见正文字节、完成标记、Result 字段、三个调查阶段、六个证据及其引用；慢模型仍建议使用 `--timeout 300`。
- 结构化检测针对已识别响应信封提取模型可见答案，并验证该项要求的具体结构；它不是任意厂商 JSON Schema 引擎。
- 性能结果同时受到模型、网关、网络、限流策略和当前 Key 配额影响。
- curl-only 脚本可以验证模型 HTTP 接口和标准工具协议，但不能独立认证完整 Codex app-server、MCP、namespace 工具、skills 和 approvals 运行链；ClawOps 上线前仍应运行产品内的模型连接诊断。
