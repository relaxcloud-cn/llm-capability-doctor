# Issue #39：结构化输出支持测试设计

版本：`specification-structured-output/v1`  
关联：[#16](https://github.com/relaxcloud-cn/llm-capability-doctor/issues/16)  
交接：[#25](https://github.com/relaxcloud-cn/llm-capability-doctor/issues/25)

本文把“模型生成内容是合法 JSON”和“模型生成内容符合指定结构”作为两个独立检测项。两项都只检测结构，不评价字段内容的业务正确性、信息抽取质量或推理能力。

## 1. 固定请求和模式

所有样例使用同一模型、同一短输入、`temperature=0`、非流式 Chat Completions。每个样例初测 1 次，取得有效但不合规响应时追加最多 2 次同条件复核；认证、网络、限流和超时等执行异常最多追加 2 次重试。首次有效失败始终保留。

固定的最小业务无关内容：

```text
请把下面固定资料表示为 JSON：城市为“杭州”，温度为 23，是否下雨为 false，标签为 ["沿海", "春季"]。
```

模式矩阵：

| 模式 | 请求字段 | 结论边界 |
| --- | --- | --- |
| 普通文本提示 | 只要求返回 JSON，不设置结构化参数 | 只用于观察模型是否自行生成可解析 JSON |
| JSON 模式 | `response_format: {"type":"json_object"}` | 只说明该模式下内容能否保持合法 JSON，不扩大到 schema 约束 |
| 指定结构 | `response_format: {"type":"json_schema","json_schema":{...}}` | 只说明给定 schema 子集是否被遵守；服务拒绝该模式是 `unsupported` |

固定 schema：根对象只允许 `city:string`、`temperature:number`、`raining:boolean`、`tags:array[string]` 四个字段，四字段均必填，`city` 非空，`temperature` 范围 `[-80, 70]`，`tags` 最多 4 项。测试不宣称支持任意 JSON Schema 关键字。

## 2. 合法 JSON 检测

| 样例 | 模式 | 生成内容要求 | 通过 |
| --- | --- | --- | --- |
| J01 | 普通文本提示 | 直接返回对象 | 原始 assistant 内容可由标准 JSON 解析器解析 |
| J02 | `json_object` | 返回对象，不带说明文字 | 原始内容可解析为 JSON，值内容不参与评分 |
| J03 | `json_object` | 返回数组 | 原始内容可解析为 JSON，根类型可以是任意 JSON 值 |
| J04 | `json_object` | 服务明确拒绝 `response_format` | 记录 `unsupported`，不写成 JSON 生成失败 |

“响应外层是 JSON”不满足本项；必须从 `choices[0].message.content` 取模型生成内容并直接解析。禁止去代码围栏、补逗号、补括号、自动转换类型或截去前后文本后再判通过；修补结果可以作为诊断字段，但不能替代原始判定。

## 3. 指定结构检测

| 样例 | 变体 | 预期 |
| --- | --- | --- |
| S01 | 完整对象 | JSON 可解析且四个必填字段、类型、范围和数组元素全部满足 |
| S02 | 缺少 `tags` | JSON 可能合法，但结构项失败，定位 `tags` |
| S03 | `temperature` 为字符串 | JSON 可能合法，但类型失败，定位 `temperature` |
| S04 | `city` 为空 | JSON 可能合法，但非空约束失败，定位 `city` |
| S05 | `tags` 含数字元素 | JSON 可能合法，但数组元素类型失败，定位 `tags[]` |
| S06 | `temperature` 越界 | JSON 可能合法，但范围失败，定位 `temperature` |
| S07 | 额外字段 `source` | 仅在 schema 声明禁止额外字段且服务返回该字段时失败；未声明时不擅自失败 |
| S08 | 指定 schema 被服务拒绝 | `unsupported`，不计入结构失败分母 |

指定结构通过必须先满足原始 JSON 可解析，再满足冻结 schema 子集。合法 JSON 与结构失败分别产生两个结果，不允许用“JSON 通过”覆盖结构失败。

## 4. 状态判定

| 状态 | 条件 |
| --- | --- |
| `accepted` | 服务接受结构化请求，但没有足够响应证据判断内容或约束生效 |
| `effective` | 固定 schema 请求返回的原始内容满足全部约束，并且约束模式与普通提示/JSON 模式的条件有清晰区分 |
| `unsupported` | 服务明确拒绝或声明不支持所请求模式；不能由网络错误推导 |
| `failed` | 得到有效生成内容，但原始 JSON 不可解析，或可解析但不符合指定结构 |
| `inconclusive` | 超时、限流、响应截断、内容缺失或证据不足，无法完成判定 |

成功 HTTP 状态、响应外层 JSON、服务返回 `finish_reason` 或请求参数被回显，都不能单独证明模型生成内容通过。

## 5. 证据结构

每个样例保存：模式、`response_format` 原文、schema 指纹、输入指纹、初测/复核/重试序号、HTTP 状态、脱敏原始响应、`message.content` 原文、原始解析错误、结构路径错误、修补诊断（若有）、最终状态和限制。API key 不进入证据。

## 6. 受控验收案例

| 案例 | 服务行为 | 预期 |
| --- | --- | --- |
| R01 | 外层响应 JSON 正常，但 content 为 `{"city":}` | 合法 JSON 失败；不能被外层 JSON 掩盖 |
| R02 | content 为合法 JSON，但缺少必填字段 | 合法 JSON 通过，指定结构失败 |
| R03 | content 为完整合法 schema 对象 | 两项分别通过；结构项可记录 `effective` |
| R04 | content 被代码围栏包裹 | 原始合法 JSON 失败，修补只作为诊断 |
| R05 | schema 模式明确返回 unsupported | 指定结构为 `unsupported`，不归因模型生成失败 |
| R06 | 认证/超时/截断 | 对应项为 `inconclusive`，保留执行证据 |

## 7. 验收标准

| AC | 检查 | 预期 |
| --- | --- | --- |
| AC-01 | 核对普通提示、JSON 模式、指定 schema | 三种模式条件和结论边界清晰，不互相扩大 |
| AC-02 | R01、R04 和外层 JSON 正常案例 | 只解析 `message.content`，不把外层 JSON 当通过 |
| AC-03 | R02、S03-S07 | 合法性与指定结构分开，字段路径错误可追溯 |
| AC-04 | R05、R06 | unsupported、inconclusive、failed 不混淆 |
| AC-05 | 复核、重试和原始证据检查 | 原始不合规不被修补或后续成功覆盖，凭据不泄露 |
| AC-06 | 未参与起草者复核 | 无需猜测 schema、模式、判定顺序或停止条件 |

本设计是 #16 的正式结构化输出输入，不代表任何真实模型已通过测试；实施和真实服务验收必须按本版本保存实际证据。
