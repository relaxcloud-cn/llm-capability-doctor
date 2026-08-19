# OpenCodex 数据格式兼容性设计

## 结论

这些规则可以直接落到 Doctor，而且不需要启动 OpenCodex 或 ClawOps。

Doctor 继续直连客户模型接口，只把现有检测项改成 OpenCodex 真实会消费的
请求、响应、流事件和工具调用格式；报告再根据这些检测项给出一个独立的
“OpenCodex 数据格式兼容性”结论。

这个结论只回答：模型接口返回的数据能否被 OpenCodex 解析和继续工具循环。
它不判断网络、鉴权、Provider 配置、部署状态或 ClawOps 整体可上线性。

## 合同来源

首个兼容性配置固定为：

```text
opencodex-2.7.42-data-format
```

规则来自 `@bitkyc08/opencodex@2.7.42` 的以下源码：

- `src/adapters/openai-chat.ts`
- `src/adapters/anthropic.ts`
- `src/adapters/google.ts`
- `src/adapters/openai-responses.ts`
- `src/responses/schema.ts` 与 `src/responses/parser.ts`
- `src/types.ts` 与 `src/bridge.ts`

后续 OpenCodex 升级时新增兼容性配置，不能静默改变旧报告的判定含义。

## 必须满足的数据格式

### 响应与流结束

| 协议 | 同步响应核心结构 | 流式成功结束信号 |
| --- | --- | --- |
| OpenAI Chat | `choices[0].message`；文本位于 `message.content` | `[DONE]`、非空 `finish_reason`，或最终 `usage` 帧 |
| OpenAI Responses | 非空响应 ID 与 `output` 数组；文本来自原生 output-text 项 | `response.completed` |
| Anthropic Messages | `type:"message"` 与 `content` 数组；文本来自 `text` block | `message_stop`，或非空 `message_delta.delta.stop_reason` |
| Google GenerateContent | `candidates[0].content.parts` | 非空 `finishReason`，或有效 `usageMetadata` |

所有用于判定的流式响应都必须是完整 SSE record；除 Chat 的 `[DONE]` 外，
`data` 必须是合法 JSON。Chat 按 OpenCodex 2.7.42 的实际实现只消费
`data: ` 前缀；Google 消费 `data:`；Anthropic 同时支持 SSE `event` 与 JSON
中的 `type`。Chat、Responses 和 Google 的畸形数据帧判失败；Anthropic 按源码
丢弃畸形帧，但剩余有效帧仍必须组成完整内容并正常结束。只看到 EOF 不算正常
结束。`response.failed`、`response.incomplete`、协议错误事件、残缺 SSE 或
错误字段类型都判失败。

结束信号分两类处理，避免把“看到了字段”误当成 OpenCodex 已完成：

- Chat `[DONE]`、Responses 的 failed/incomplete 和显式错误事件会立即结束读取。
- Responses 的 `response.completed`、Chat 的 `finish_reason/usage`、Anthropic 的
  `message_stop/stop_reason`、Google 的 `finishReason/usageMetadata` 都必须继续读到
  正常 EOF。如果随后出现错误或连接直到 timeout 都不关闭，仍然失败。

EOF 前没有空行或换行的最后一个完整 `data:` record 仍须解析，行为与
OpenCodex 的 SSE decoder 一致。

### 工具调用

| 协议 | 调用结构 | 调用 ID | 参数 |
| --- | --- | --- | --- |
| OpenAI Chat | `message.tool_calls[]` 或按 `index` 拼接的流式 delta | 非空 `id` | `function.arguments` 拼接后必须是 JSON object |
| OpenAI Responses | `type:"function_call"` 的 output item 或对应流事件 | 非空 `call_id` | `arguments` 必须是可解析为 object 的 JSON string |
| Anthropic Messages | `type:"tool_use"` block 或 `input_json_delta.partial_json` | 非空 `id` | `input` 或拼接后的 partial JSON 必须是 object |
| Google GenerateContent | part 中的 `functionCall` | 上游 ID 可省略且不作为关联 ID；OpenCodex 总是生成新的内部 ID | `functionCall.args` 必须是 object |

参数解析后还必须满足 Doctor 发出的 JSON Schema，包括必填字段、类型、枚举、
嵌套结构，以及协议支持时的 `additionalProperties:false`。

流式工具检测按协议原生关联键重组调用：Chat 使用 `index`（再回退到 `id`），
Responses 使用 `output_index/item_id`，Anthropic 使用当前 content block 与
`tool_use.id`，Google 使用 candidate part 中的完整 `functionCall`。

### 工具命名空间与结果关联

检测 041 和 047 使用 namespace `doctor` 下的 `get_weather`，并使用无 namespace
的 `get_time` 作为候选或第二步工具：

- OpenAI Responses 发送原生 `type:"namespace"` 工具；返回调用必须是
  `namespace:"doctor"` 与 `name:"get_weather"`。
- OpenAI Chat、Anthropic 和 Google 发送 OpenCodex 实际暴露给模型的扁平名称
  `doctor__get_weather`；模型必须原样返回。
- 工具结果必须使用原调用的关联值：`call_id`、`tool_call_id`、
  `tool_use_id`，或 Google 的 `functionCall.id/functionResponse.id`。
- Google 无论上游是否返回 ID，Doctor 都按 OpenCodex 行为生成新的本地 ID，并把
  同一 ID 同时写回重放的 `functionCall` 和 `functionResponse`；不能要求保留
  上游 ID。

## 落到现有检测项

不增加检测项 ID，也不增加新的 CLI 模式。

| 检测项 | 修改后的职责 |
| --- | --- |
| 002 | 按根路径和字段类型识别协议，不能再递归搜索同名 key；Ollama 仍可被 Doctor 识别，但不属于本兼容性配置支持的四类协议。 |
| 004 | 校验完整同步 envelope，只从模型可见文本路径读取 marker。 |
| 005 | 校验真实 SSE，按协议顺序拼接文本 delta。 |
| 006 | marker 完整且出现上表中的协议原生结束信号。 |
| 040 | 改为 `stream:true`；重组一个工具调用，要求正确名称、协议要求的调用 ID、合法且匹配 Schema 的参数，以及正常工具流结束。 |
| 041 | 从 weather/time 中选择带 namespace 的 weather 工具，并保留原生或扁平名称。 |
| 043 | 严格检查必填参数、类型、枚举和额外字段。 |
| 045 | 改为 `stream:true`，按协议重组两个并行工具调用及参数，且工具流必须正常结束；保留增强能力检测，但不作为 OpenCodex 格式硬门槛。 |
| 047 | 第一轮 namespace 工具调用、工具结果和第二轮 time 调用必须使用正确的关联 ID。 |

OpenCodex 数据格式兼容性的 8 个必过项是 002、004、005、006、040、041、043、
047。检测 045 仍验证流式并行调用，但 OpenCodex 允许 Provider 禁用并行调用，
Spark 兼容路径也会强制关闭它，因此 045 只能保持“通用能力”的增强项，不能因
模型选择串行工具就判定数据格式不兼容。

## 代码改动

1. `src/protocol/mod.rs`
   收紧同步响应识别，只校验协议定义的根路径、数组和字段类型。

2. `src/protocol/stream.rs`（新增）与 `src/http.rs`
   增加增量 SSE inspector。它缓存跨 chunk 的残帧，用结构化 JSON 解析结束事件，
   先把完整事件写入原始 evidence；只对上文的立即结束事件停止读取，EOF fallback
   必须等正常 EOF。没有结束事件时仍由现有 timeout 和 cancellation 处理。

3. `src/protocol/tools.rs`
   修改 040、041、045、047 的 namespace、stream 和 follow-up 构造；Google
   follow-up 每次都生成 OpenCodex 同款本地调用 ID。

4. `src/runner.rs`
   只对 Google 流式请求做确定性 URL 处理：
   `:generateContent` 改成 `:streamGenerateContent?alt=sse`；已经是
   `:streamGenerateContent` 时只补 `alt=sse`；其他自定义路径保持不变。
   README 同步说明这是“不改写完整 URL”规则的唯一例外。

5. `skills/creating-model-doctor-reports/`
   收紧 002、004、005、006、040、041、043、045、047 的 v3 评估规则，更新
   assessment schema、派生逻辑、HTML 和测试。

Collector 仍只记录原始请求、响应和指标，不在 Rust 进程里给模型打 PASS/FAIL。

## 结果与版本

现有检测语义发生变化，必须换证据合同，防止旧日志被误当成新证据：

```text
script_version: 0.11.0
log_schema: llm-capability-doctor.evidence.v3
compatibility_profile: opencodex-2.7.42-data-format
```

报告输出升级为 `llm-capability-doctor.assessment.v7`，在
`capabilitySummary.openCodexCompatibility` 中加入程序派生结果：

```json
{
  "profile": "opencodex-2.7.42-data-format",
  "level": "PASS",
  "label": "OpenCodex 数据格式兼容",
  "protocolFamily": "OPENAI_CHAT_COMPLETIONS",
  "requiredTestIds": ["002", "004", "005", "006", "040", "041", "043", "047"],
  "failedTestIds": [],
  "statement": "本轮八项必需数据格式检查全部通过，因此判定符合 OpenCodex 2.7.42 数据格式合同。",
  "scopeBoundary": "仅判断本轮模型端数据格式，不覆盖鉴权、网络、部署或 ClawOps 运行环境。"
}
```

判定规则固定为：

- `PASS`：v3 声明正确 profile、协议属于四类支持协议、8 项全部 PASS。
- `FAIL`：v3 的协议不支持/未知，或任一必过项 FAIL。
- `NOT_ASSESSED`：v1/v2 没有采集 namespace 与 040 流式工具合同，不能倒推兼容性。

固定标签分别为“OpenCodex 数据格式兼容”“OpenCodex 数据格式不兼容”
和“OpenCodex 数据格式未评定”。FAIL 文案列出失败检测项；协议不支持时单独写明
协议原因；旧日志不制造失败项，只说明未采集。

Parser 继续接受 v0.9/v1 和 v0.10/v2，并新增 v0.11/v3 精确组合；v3 缺失或
声明未知 profile 是日志合同错误，不是模型能力失败。v1/v2 保持原检测规则，
报告中的 OpenCodex 结果为 `NOT_ASSESSED`。

Reviews 合同保持 `llm-capability-doctor.reviews.v2`，因为兼容性对象由程序派生，
不由评审者填写。Evidence v3 与 v2 一样必须包含完整 46 项，且不得重新引入
`collection_profile`。

HTML 顺序为：检测信息、OpenCodex 数据格式兼容性、现有通用能力结论、能力域表、
逐项证据。兼容性区不使用 READY/BLOCKED，也不宣称 ClawOps 已可部署。

## 不纳入兼容性门槛

- Token usage 检测 007。
- Thinking/reasoning 检测 033-036。
- 裸 JSON 文本、固定回答措辞等内容能力。
- URL 白名单、鉴权头、网络、TLS、Provider catalog、部署和 ClawOps runtime。
- 模型商业身份与供应商真实性。

这些检测可继续存在于 Doctor 的通用能力报告，但不会影响 OpenCodex 数据格式结论。

## 验证

1. Rust 单测覆盖四类同步 envelope、跨 chunk SSE、全部成功结束信号、错误事件、
   残帧和无结束 EOF。
2. 合同测试覆盖 Responses 原生 namespace、其他协议扁平名称、040 流式单调用、
   045 流式双调用、Schema 校验、047 ID 关联和 Google 重建 ID。
3. HTTP 测试覆盖记录完整终止帧后结束、无终止帧 timeout，以及 Google URL 转换。
4. Python 测试覆盖 v3 PASS/FAIL、Ollama/UNKNOWN、v1/v2 `NOT_ASSESSED`、
   assessment v7 和 HTML 位置。
5. 最终运行 Rust fmt、Clippy（warnings denied）、全部 Rust/Python 测试和 release build。

全部测试只使用本地 fixture 和静态日志，不启动 OpenCodex/ClawOps，也不访问客户接口。

## 验收标准

- 同一份 v0.11 日志明确记录它检测的 OpenCodex 源码合同版本。
- 8 个必过检测项能区分 envelope 错误、流截断、工具参数错误、namespace 丢失、
  流式工具拼接错误和结果 ID 错配。
- Google 的流式端点与始终重建内部工具 ID 的行为和 OpenCodex 一致。
- v3 报告稳定生成唯一的 OpenCodex 数据格式结论；旧日志仍可出报告但显示未评定。
- 整个流程不需要 OpenCodex 或 ClawOps 运行环境。
