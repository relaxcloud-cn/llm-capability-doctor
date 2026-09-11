# Chat Completions 官方结构基线与 BC01-BC14 差异规则

版本：1.0
整理日期：2026-09-11
关联任务：[#20](https://github.com/relaxcloud-cn/llm-capability-doctor/issues/20)
交接任务：[#29](https://github.com/relaxcloud-cn/llm-capability-doctor/issues/29)

本文冻结第一阶段的官方参考结构、变体选择和差异判定。范围只包含 Chat Completions 输出结构；不混入 Responses API、SDK 兼容性、业务任务成功或 Agent 可用性。本文的字段值、文本内容、动态 ID 和合法数组顺序均不是结构差异。

## 1. 固定来源与阅读边界

官方参考入口：<https://developers.openai.com/api/reference/chat>，读取日期：2026-09-11。该页面用于核对 Chat Completions 的响应对象、流式 chunk、消息、工具调用、usage 和错误对象。

本仓库把“来源 URL + 读取日期 + 本文版本 + 结构目录”作为第一阶段的规范快照记录。在线页面更新不会自动改变本文；若字段范围发生变化，必须新增基线版本并重新验收。本文不复制官方完整页面，也不把某个 SDK 的类型定义当作规范来源。

## 2. 第一阶段回答的问题

只回答以下结构事实：

1. 字段是否存在；
2. 字段名称和嵌套位置是否一致；
3. 对象、数组和标量类型是否一致；
4. 流式字段属于哪个事件阶段；
5. 差异是否能追溯到本版本官方结构目录。

不回答字段值是否正确、文本是否相同、工具参数业务内容是否相同、数组顺序是否相同、流式分块数量是否相同，也不由结构差异直接推导服务不可用。

## 3. 字段状态和匹配语法

### 3.1 字段状态

结构目录给每个路径标注以下状态：

| 状态 | 含义 | 缺失或出现时的处理 |
| --- | --- | --- |
| `required` | 当前变体必须存在 | 缺失或类型不符为结构差异 |
| `optional` | 官方允许该字段在当前变体出现或不出现 | 出现/缺失不标红，但出现时类型必须符合 |
| `nullable` | 字段可为 `null`，也可按目录声明为对象/数组 | `null` 与声明类型均可接受；其他类型为差异 |
| `variant` | 只在特定请求或流式阶段适用 | 选择错误变体前先报告前提，不比较不适用字段 |
| `forbidden` | 当前明确分支不应出现 | 出现为结构差异；仅在目录有此声明时使用 |

字段是否出现是结构事实；字段里的字符串、数字、布尔值和业务对象内容不是结构事实。目录没有声明的额外字段先报告“未收录字段”，再由验收规则决定是否属于结构差异，不能由工具默认算法直接判定。

### 3.2 路径和数组

使用 JSON 路径，数组元素用 `[]` 表示形状位置，例如 `choices[].message.role`。比较数组时：

- 不比较数组长度、顺序或元素值；
- 只比较已出现元素的对象键、嵌套位置和类型；
- 空数组在目录已有元素 schema 时视为形状可接受；
- 空数组没有可观察元素 schema 时标记“元素形状未观察”，不标红；
- 数组元素类型从对象变为标量，或对象键层级变化，属于结构差异。

### 3.3 值、空值和不可解析响应

相同 key 的值不同不标红。`null` 与缺失只有在目录把字段标为 `required` 或 `required + nullable` 时按规则处理：必需字段缺失为差异；允许缺失的可选字段不因缺失标红；可空字段的 `null` 不因值为空标红。

无法取得 JSON、响应被截断或事件无法按协议解析时，结果为 `inconclusive`，不是结构差异。测试客户端必须保留原始响应，不能用部分文本拼出一个参考对象。

## 4. BC01-BC14 参考变体目录

### 4.1 普通回复

| 编号 | 变体 | 适用请求 | 关键路径 |
| --- | --- | --- | --- |
| BC01 | 响应外层 | 非流式普通文本或工具响应 | `object`, `id`, `created`, `model`, `choices[]`, `usage` |
| BC02 | 候选结果容器 | BC01 的 `choices[]` | `choices[].index`, `choices[].message`, `choices[].finish_reason`, `choices[].logprobs` |
| BC03 | 回复消息 | 普通文本消息 | `choices[].message.role`, `content`, `refusal`, `tool_calls`, `function_call` |

BC01 的 `object` 应与非流式 Chat Completions 对象分支一致；BC02 只检查候选容器的键和类型；BC03 的工具字段按是否选择工具的变体判断，不把工具字段缺失直接写成普通文本结构失败。

### 4.2 用量与附加信息

| 编号 | 变体 | 适用请求 | 关键路径 |
| --- | --- | --- | --- |
| BC04 | 用量汇总 | 响应带 usage 的非流式请求 | `usage.prompt_tokens`, `usage.completion_tokens`, `usage.total_tokens` |
| BC05 | 用量明细 | usage 带明细的响应 | `usage.prompt_tokens_details`, `cached_tokens`, `usage.completion_tokens_details`, `reasoning_tokens` |
| BC06 | 服务附加信息 | 服务返回这些字段的响应 | `system_fingerprint`, `service_tier`, `request_id`, `metadata` 等目录字段 |

BC04-BC06 是附加变体，不要求每个服务都返回所有可选字段。服务没有 usage 时记录“未提供 usage”，不把可选字段缺失标为结构失败；如果服务返回字段，则按目录验证名称、位置和类型。

### 4.3 函数工具调用

| 编号 | 变体 | 适用请求 | 关键路径 |
| --- | --- | --- | --- |
| BC07 | 函数调用容器 | 非流式响应包含工具调用 | `choices[].message.tool_calls[]`, `id`, `type`, `function` |
| BC08 | 函数名称与参数承载 | BC07 的 function 对象 | `tool_calls[].function.name`, `tool_calls[].function.arguments` |

BC08 只比较 `name` 和 `arguments` 的字段路径及标量类型。arguments 字符串内部的业务值、空格、键顺序和 JSON 语义不属于本阶段结构差异；是否可解析和参数是否正确由 #16/#17 处理。

### 4.4 流式输出

| 编号 | 变体 | 适用事件 | 关键路径 |
| --- | --- | --- | --- |
| BC09 | 分块外层 | `stream: true` 的每个 chunk | `object`, `id`, `created`, `model`, `choices[]`, `usage` |
| BC10 | 分块候选容器 | chunk 的候选数组 | `choices[].index`, `choices[].delta`, `choices[].finish_reason`, `choices[].logprobs` |
| BC11 | 消息增量 | 文本/角色/拒答增量 | `choices[].delta.role`, `content`, `refusal` |
| BC12 | 函数调用增量 | 工具调用增量事件 | `delta.tool_calls[].index`, `id`, `type`, `function.name`, `function.arguments` |
| BC13 | 流式用量返回 | 终止或约定 usage chunk | `usage.prompt_tokens`, `completion_tokens`, `total_tokens` 及明细 |

流式比较按事件阶段选择变体，不把第一块、内容块、工具参数块和终止块拼成一个“必然完整标准”。同一字段在不同阶段的缺失必须按阶段目录判断。合法的分块数量、参数片段长度和到达顺序变化不标红；事件缺少正常终止或无法组装属于 #16 流式能力判定，不由本基线单独推导整体不可用。

### 4.5 异常响应

| 编号 | 变体 | 适用响应 | 关键路径 |
| --- | --- | --- | --- |
| BC14 | JSON 错误对象 | HTTP 错误且 body 可解析为 JSON | `error`, `error.message`, `error.type`, `error.param`, `error.code` |

BC14 比较错误 envelope、字段名称、嵌套位置和字段类型。错误消息、业务描述、动态 code 值和 HTTP 状态值不在结构红线中；HTTP 状态和具体错误原因另存为运行事实。

## 5. 变体选择与参考快照

测试请求必须先记录适用条件，再选择参考变体：

| 请求/响应条件 | 选择的变体 |
| --- | --- |
| 非流式普通文本 | BC01-BC03；若返回 usage/附加字段，再加入 BC04-BC06 |
| 非流式工具调用 | BC01-BC03、BC07-BC08；usage/附加字段按实际加入 |
| 流式普通文本 | BC09-BC11；有 usage 终止事件时加入 BC13 |
| 流式工具调用 | BC09-BC12；有 usage 终止事件时加入 BC13 |
| 可解析错误 JSON | BC14；不与成功响应变体混比 |

参考快照记录至少包含：来源 URL、读取日期、本文版本、请求模式、适用变体、字段目录版本和派生样例指纹。原始响应、规范示例和派生对照必须分开标记；不能将某次服务响应反向写成官方标准。

## 6. 结构差异判定

### 6.1 差异类型

| 差异 | 结构结果 | 示例 |
| --- | --- | --- |
| 字段名变化 | `different` | `message.content` 改为 `message.text` |
| 承载位置变化 | `different` | `tool_calls[].function` 移到 `message.function` |
| 对象/数组形状变化 | `different` | `choices[]` 变为对象，或数组元素键层级改变 |
| 标量类型变化 | `different` | `usage.total_tokens` 从 number 变为 string |
| 相同 key 的值不同 | `same_structure` | `finish_reason` 值不同、文本不同、ID 不同 |
| 字段顺序不同 | `same_structure` | JSON 对象键顺序变化 |
| 合法数组顺序/数量不同 | `same_structure` | 两个工具调用换序或 chunk 数量不同 |
| 可选字段缺失 | `same_structure` 或 `not_observed` | 目录标为 optional 且服务未返回 |
| JSON 不可解析/响应截断 | `inconclusive` | 没有足够结构证据 |

### 6.2 展示规则

报告显示参考路径、实际路径、差异类型、适用变体和证据位置。只有 `different` 才进入结构差异清单；`same_structure` 不标红；`not_observed` 和 `inconclusive` 显示证据状态。

结构差异必须与以下结论分开：

- 合规或协议支持；
- 模型任务是否完成；
- Agent 工具链是否可用；
- 客户服务是否整体可用。

第一阶段不建立“红色数量越多越不可用”的总分，也不把动态值差异当作结构差异数量。

## 7. 纸面验收集

以下样例只验证比较规则，不是实际服务结果：

| 样例 | 变化 | 预期 |
| --- | --- | --- |
| V01 | `choices[0].message.content` 文本从 A 变为 B | `same_structure` |
| V02 | `tool_calls` 两个元素交换顺序 | `same_structure` |
| V03 | `message.tool_calls[].function.arguments` 内部 JSON 值变化 | `same_structure` |
| V04 | `choices[].message` 改为 `choices[].output` | `different` |
| V05 | `usage.total_tokens` 从 number 变为 string | `different` |
| V06 | optional `system_fingerprint` 未返回 | `same_structure` / `not_observed` |
| V07 | `choices[]` 从数组变为对象 | `different` |
| V08 | 数组为空但目录已有元素 schema | `same_structure` |
| V09 | 数组为空且目录没有元素 schema | `not_observed` |
| V10 | 流式文本 chunk 没有工具增量字段 | 按 BC11 判断，不套用 BC12 |
| V11 | 最终 chunk 有 usage，前序 chunk 没有 usage | 按 BC13 阶段规则判断 |
| V12 | 错误消息文字变化但 error envelope 相同 | `same_structure` |
| V13 | body 只有半截 JSON | `inconclusive` |
| V14 | HTTP 2xx 但响应字段整体不是 JSON | `inconclusive`，不是结构红线 |

## 8. 候选工具边界

候选比较器只能提供解析、路径枚举或差异定位能力；产品规则仍由本文决定。候选工具必须通过以下验证后才能进入 #29：

1. 能区分对象键、数组元素形状和标量类型；
2. 能配置忽略动态值、对象键顺序和合法数组顺序；
3. 能按流式阶段分别比较，不把全部 chunk 合并成假标准；
4. 能保留空数组、null、缺失字段和不可解析响应的证据状态；
5. 能输出原始路径和派生差异，而不是只有一个差异数量。

不能满足上述任一条件的工具只能作为辅助解析器，不能决定客户报告的红线。

## 9. #20 验收记录与交接

| AC | 检查动作 | 文档落点 | 通过证据 |
| --- | --- | --- | --- |
| AC-01 | 从 BC01-BC14 任取一项追溯来源、字段和场景 | 第 1、4、5 节 | 变体、路径、适用条件和版本可定位 |
| AC-02 | 只修改相同 key 的值或对象字段顺序 | 第 3、6、7 节 | 不标红，不比较工具参数业务值 |
| AC-03 | 修改字段名、位置或对象/数组结构 | 第 6、7 节 | 结构差异可定位且不推导整体可用性 |
| AC-04 | 检查流式阶段、可选字段和合法分支 | 第 4、5、6 节 | 按阶段选择变体，不拼假标准 |
| AC-05 | 检查空数组、null、标量类型和不可解析 JSON | 第 3、7 节 | 每类有固定预期，不由工具默认行为决定 |

独立验收记录格式固定为：

```text
AC 编号 -> 快照及版本 -> 输入样例/运行命令 -> 预期与实际
-> 差异证据位置 -> 验证结果 -> 修订提交
```

交接给 #29 的内容是：BC01-BC14 目录、变体选择条件、字段状态、路径匹配语法、差异类型、展示状态、纸面验收集和候选工具验收条件。本文不实现比较器，不完成真实服务重放。
