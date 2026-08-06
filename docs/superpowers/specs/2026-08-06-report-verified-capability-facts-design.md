# Model Doctor 报告已验证能力事实设计

## 目标

让 `creating-model-doctor-reports` Skill 生成的每份新报告在“最终结论”中固定回答三件事：

1. 接口采用哪一种请求与响应格式，是已识别的常见协议还是自定义格式；
2. 本轮证据支持到什么上下文范围，以及第一个失败档位；
3. 本轮最高验证到多少并发，以及该数字的证据边界。

这些值必须来自日志中的可观察证据。最高已测值不能写成模型或服务的真实硬上限，字符近似档位不能替代原生 Token 计数。

## 契约版本

这是对评审输入和 assessment 输出的必填字段扩展，因此采用新契约：

- `llm-capability-doctor.reviews.v2`
- `llm-capability-doctor.assessment.v6`

证据日志契约保持不变，继续接受已支持的 `evidence.v1 / collector 0.9.0` 与 `evidence.v2 / collector 0.10.0`。旧 assessment 文件仍是历史产物，不做原地升级或重写。

## 数据模型

在 `capabilitySummary` 中新增必填对象 `verifiedFacts`，包含三个必填子对象。

### interfaceProtocol

- `evidenceState`: `VERIFIED`、`INCONCLUSIVE` 或 `NOT_COLLECTED`
- `family`: `OPENAI_CHAT_COMPLETIONS`、`OPENAI_RESPONSES`、`ANTHROPIC_MESSAGES`、`GEMINI_GENERATE_CONTENT`、`OLLAMA_CHAT`、`CUSTOM` 或 `UNKNOWN`
- `requestFormat`: 客户可读的请求结构说明
- `responseFormat`: 客户可读的响应结构说明
- `statement`: 一句结论，明确是否属于已识别协议或自定义格式
- `evidenceRefs`: 支撑结论的请求证据引用
- `boundary`: 不扩大到提供方身份或未采集协议的边界

### contextWindow

- `evidenceState`: `VERIFIED`、`INCONCLUSIVE` 或 `NOT_COLLECTED`
- `highestVerifiedTier`: 最高通过的检测档位；未采集时为 `null`
- `highestVerifiedInputTokens`: 最高通过请求中提供方原生报告的输入 Token；不可观察时为 `null`
- `firstFailedTier`: 更高档位首次失败的检测档位；没有更高失败或未采集时为 `null`
- `firstFailedInputTokens`: 首次失败请求中原生报告的输入 Token；不可观察时为 `null`
- `statement`: 同时说明最高通过与首个失败证据
- `evidenceRefs`: 支撑最高通过和首个失败结论的请求引用
- `boundary`: 明确最高已验证值不是硬上限，字符数不等于精确 Token 数

### concurrency

- `evidenceState`: `VERIFIED`、`INCONCLUSIVE` 或 `NOT_COLLECTED`
- `highestVerifiedConcurrentRequests`: 所有样本均满足契约且无缺样、限流时的最高波次；无法确认时为 `null`
- `statement`: 说明各执行波次和最高通过波次
- `evidenceRefs`: 至少覆盖最高波次，并由报告中的完整请求证据支撑全部波次
- `boundary`: 明确这是短时并发验证，不是服务硬上限、持续负载结论或 SLA

## 状态与校验规则

`VERIFIED` 要求相应已验证值非空且 `evidenceRefs` 非空。`NOT_COLLECTED` 要求数值字段为 `null`、引用为空，并在 statement 中明确未采集。`INCONCLUSIVE` 用于已有请求但证据不足以形成所需结论；引用必须非空，数值字段只填写被证据直接支持的部分。历史自定义日志若缺少中间上下文档位，`firstFailedTier` 只能表示“已采集的更高档位中第一个失败项”，不能暗示未采集的中间档位已经通过或失败。

所有引用必须存在于 parsed evidence。协议事实优先引用检测项 002；上下文事实只引用 014-018 的请求；并发事实只引用 057 的请求。校验器拒绝缺字段、未知枚举、矛盾状态、无效引用和扩大为硬上限的措辞。

对 v0.10 evidence.v2，三个事实都应有对应检测项。历史 v0.9 onsite/custom 日志若缺少某类检测项，必须输出 `NOT_COLLECTED`，不能从未关联请求或模型名称猜测。

## 评审流程

逐项 PASS/FAIL 固定后，Skill 再生成 `verifiedFacts`：

1. 从协议探测的实际请求与响应结构分类接口格式；未匹配五种已知协议时只能写 `CUSTOM` 或 `UNKNOWN`。
2. 从 014-018 找出最高通过档和第一个更高失败档；只有响应原生 usage 字段才能进入 Token 数值字段。
3. 从 057 检查每个已执行波次的全部请求；只有该波次全样本语义正确、指标有效且无限流时才能成为最高已验证并发。
4. 最后编写现有 headline 和失败 issues。`verifiedFacts` 不改变任何单项 PASS/FAIL，也不替代失败审计。

## HTML 呈现

“最终结论”保持位于“检测信息”和能力域结果表之间。该区域按以下顺序显示：

1. “接口协议格式”：协议家族、请求格式、响应格式和边界；
2. “上下文能力”：最高已验证值、首个失败档位和边界；
3. “并发能力”：最高已验证并发和短时测试边界；
4. 现有 headline、失败问题列表与 scope boundary。

使用一个紧凑的事实表或定义列表，不增加嵌套卡片。动态文本统一 HTML 转义；打印样式必须完整显示三项事实。

## 错误处理

- reviews 缺少任一事实对象：validation 失败，不渲染报告。
- `VERIFIED` 没有可验证值或证据引用：validation 失败。
- 引用不存在或引用了错误检测域：validation 失败。
- 日志未采集对应项目：输出 `NOT_COLLECTED`，而不是让 Skill 猜测。
- 发现“真实最大”“硬上限”“一定支持更高档”等无边界肯定句：validation 失败并要求改写为“最高已验证”或“至少支持”。“不是硬上限”“真实上限未测试”这类否定边界必须允许。

## 测试

先增加失败测试，再实现：

- reviews.v2 缺少 `verifiedFacts` 时拒绝；
- 三个事实对象的状态、枚举、空值组合和证据归属校验；
- assessment.v6 保留三个事实且 schema 声明全部必填字段；
- HTML 最终结论包含协议、上下文和并发三行，并位于能力域表之前；
- `NOT_COLLECTED` 能稳定渲染且不产生猜测值；
- 动态字段经过 HTML 转义，打印样式不隐藏事实；
- 现有 FAIL 审计、issue 分组、凭证脱敏与离线 HTML 测试继续通过；
- 用一份完整 evidence.v2 fixture 验证 OpenAI Chat、上下文最高通过/首失败、32 并发能够写入报告。

## 非目标

- 不修改采集器检测项、并发波次或上下文探针。
- 不从模型名称、产品文档或 URL 猜测能力上限。
- 不自动给出项目可上线、不可上线或 READY/BLOCKED 判断。
- 不把短时并发结果解释为 RPM、TPM、持续吞吐或 SLA。
