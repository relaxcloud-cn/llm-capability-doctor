# Issue #36：工具调用支持测试设计

版本：`specification-tool-calling/v1`  
关联：[#16](https://github.com/relaxcloud-cn/llm-capability-doctor/issues/16)  
交接：[#25](https://github.com/relaxcloud-cn/llm-capability-doctor/issues/25)

本文冻结 S04 的输入、固定工具、判定和证据要求。它只回答“模型能否生成符合要求的非流式工具调用”，不执行工具，不测试工具结果回传、续答、自主规划或 Agent 任务完成。

## 1. 固定请求

每个样例均使用 Chat Completions 非流式请求：`temperature=0`、`stream=false`、`tool_choice` 按样例固定。除当前样例字段外，不改变模型、消息、工具定义和其他设置。

固定工具定义：

```json
{
  "type": "function",
  "function": {
    "name": "lookup_weather",
    "description": "查询城市天气",
    "parameters": {
      "type": "object",
      "properties": {
        "city": {"type": "string"},
        "days": {"type": "integer", "minimum": 1, "maximum": 7},
        "unit": {"type": "string", "enum": ["celsius", "fahrenheit"]},
        "include_alert": {"type": "boolean"},
        "locations": {"type": "array", "items": {"type": "string"}}
      },
      "required": ["city", "days", "unit", "include_alert", "locations"],
      "additionalProperties": false
    }
  }
}
```

第二个工具固定为 `lookup_air_quality`，只保留 `city: string` 和 `index: integer` 两个必填字段，用于验证不同工具之间的名称和参数不串位。

## 2. 样例矩阵

每种样例固定 3 组参数，每组初测 1 次，共 27 个初测请求。有效响应不符合规则时，对同一输入和设置追加最多 2 次复核；超时、限流、连接中断等执行异常最多追加 2 次重试。复核和重试分开记录。

| 样例 | 调用形态 | 参数覆盖 | `tool_choice` | 通过条件 |
| --- | --- | --- | --- | --- |
| T01 | 单个 `lookup_weather` | 字符串 + 数字 | `required` | 只生成一个正确工具调用，所有必填字段存在且类型和值正确 |
| T02 | 单个 `lookup_weather` | 布尔值 | `required` | `include_alert` 为布尔值且其余参数不丢失 |
| T03 | 单个 `lookup_weather` | 枚举 + 数组 | `required` | `unit` 只能为允许枚举，`locations` 为字符串数组 |
| T04 | 单个 `lookup_weather` | 嵌套对象 | `required` | 嵌套字段结构、必填项和类型完整 |
| T05 | 单个工具禁止调用 | 普通文本输入 | `none` | 不返回工具调用；若返回调用则有效失败 |
| T06 | 同一工具调用两次 | 两组不同参数 | `required` | 恰好两次 `lookup_weather`，参数按调用顺序对应 |
| T07 | 两个不同工具各调用一次 | 天气 + 空气质量 | `required` | 恰好各一次，工具名与各自参数完全对应 |
| T08 | 强制指定工具 | `lookup_weather` | 指定函数 | 只返回指定工具，不能替换为普通文字或另一工具 |
| T09 | 缺少必填字段 | 故意省略 `locations` | `required` | 模型仍按题目生成；缺字段、错类型和普通文字均为失败 |

## 3. 判定顺序

1. 先确认 HTTP 成功、响应可解析、`choices[0].message` 存在。
2. 再确认 `tool_calls` 容器、调用数量、函数名和参数 JSON 可解析。
3. 最后按 JSON Schema 检查必填字段、类型、枚举、数组元素和禁止额外字段。
4. 只要某一层失败，保留失败层级和原始脱敏响应；后续复核成功不能覆盖首次有效失败。

结果映射：

- `pass`：本样例所有必要条件满足。
- `fail`：取得有效响应，但调用数量、名称、参数或 schema 不满足。
- `inconclusive`：没有足够有效响应，例如认证失败、超时、限流耗尽或响应截断。
- `unsupported`：服务明确拒绝工具字段或明确声明不支持该能力；环境错误不能写成 unsupported。

## 4. 证据结构

每个样例至少保存：`sample_id`、请求设置指纹、工具定义指纹、初测/复核/重试序号、HTTP 状态、脱敏原始响应、解析后的调用列表、判定路径、失败字段路径和最终状态。API key 不得进入请求快照、错误文本或报告。

## 5. 验收标准

| AC | 检查 | 预期 |
| --- | --- | --- |
| AC-01 | 固定工具和 9 类样例各 3 组 | 生成 27 个初测输入，版本和指纹稳定 |
| AC-02 | 正确单调用、同工具双调用、不同工具双调用 | 数量、名称、参数对应关系分别可判定 |
| AC-03 | 字符串、数字、布尔、枚举、嵌套对象、数组 | 类型和 schema 违反能定位到字段路径 |
| AC-04 | `none`、指定工具、服务拒绝和网络异常 | 控制参数、unsupported、inconclusive 不混淆 |
| AC-05 | 有效失败后复核成功、执行异常重试 | 首次有效失败保留，复核/重试独立留证 |
| AC-06 | 序列化和脱敏检查 | 样例 ID、规则版本、证据引用可回溯且不含凭据 |

## 6. 交接约束

本设计补齐 #16 中 S04 的正式输入和判定，不新增测试次数或扩大 S04 边界。#25 接入时必须保留“工具生成”和“工具执行/Agent 闭环”的模块边界；流式工具增量由 #40 继续细化。
