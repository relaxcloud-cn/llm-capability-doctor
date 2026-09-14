import Foundation

struct StructureDifference: Codable, Identifiable {
  var line: Int
  var path: String
  var description: String
  var requirement: String
  var id: Int { line }
}

struct BaselineResponse: Codable, Identifiable {
  var id: String
  var phase: String
  var actual: [String]
  var reference: [String]
  var differences: [StructureDifference] = []
  var unavailable: String? = nil
  var status: String {
    if let unavailable { return unavailable }
    return differences.isEmpty ? "未发现字段或结构差异" : "存在字段或结构差异"
  }
  var raw: String { actual.filter { !$0.isEmpty }.joined(separator: "\n") }
}

struct BaselineItem: Codable, Identifiable {
  var id: String
  var group: String
  var title: String
  var path: String
  var requirement: String
  var responses: [BaselineResponse]
  var hasDifference: Bool { responses.contains { !$0.differences.isEmpty } }
  static let groups = ["普通回复", "用量与附加信息", "函数工具调用", "流式输出", "错误返回"]
  static func forRecord(_ record: RunRecord) -> [BaselineItem] {
    var items = all
    for index in items.indices where ["BC01", "BC09"].contains(items[index].id) {
      for responseIndex in items[index].responses.indices {
        let response = items[index].responses[responseIndex]
        guard
          var object = (try? JSONSerialization.jsonObject(with: Data(response.raw.utf8)))
            as? [String: Any],
          let reference = try? JSONSerialization.jsonObject(
            with: Data(response.reference.joined(separator: "\n").utf8))
        else { continue }
        object["model"] = record.responseModel ?? record.service.model
        // Normalize both envelopes to keep fields aligned without comparing their values.
        let options: JSONSerialization.WritingOptions = [
          .prettyPrinted, .sortedKeys, .withoutEscapingSlashes,
        ]
        guard let left = try? JSONSerialization.data(withJSONObject: object, options: options),
          let right = try? JSONSerialization.data(withJSONObject: reference, options: options)
        else { continue }
        items[index].responses[responseIndex].actual = lines(String(decoding: left, as: UTF8.self))
        items[index].responses[responseIndex].reference = lines(
          String(decoding: right, as: UTF8.self))
      }
    }
    return items
  }
  static func lines(_ value: String) -> [String] { value.components(separatedBy: "\n") }
  static func sample(
    _ id: String, _ actual: String, _ reference: String, phase: String = "完整响应",
    differences: [StructureDifference] = []
  ) -> BaselineResponse {
    .init(
      id: id, phase: phase, actual: lines(actual), reference: lines(reference),
      differences: differences)
  }
  static let all: [BaselineItem] = [
    .init(
      id: "BC01", group: groups[0], title: "响应外层", path: "$",
      requirement: "id、object、created、model、choices 为必需字段。这里只对照字段与结构，不校验每个值。",
      responses: [
        sample(
          "B01-1",
          """
          {
            "id": "chat-demo-001",
            "object": "chat.completion",
            "created": 1788825600,
            "model": "agent-prod",
            "choices": []
          }
          """,
          """
          {
            "id": "chat-reference",
            "object": "chat.completion",
            "created": 1700000000,
            "model": "reference-model",
            "choices": []
          }
          """)
      ]),
    .init(
      id: "BC02", group: groups[0], title: "候选回复", path: "choices[]",
      requirement: "每个候选包含序号、回复消息、结束原因和概率信息；概率信息必须存在但可以为空。",
      responses: [
        sample(
          "B02-1",
          """
          {
            "index": 0,
            "message": {"role": "assistant", "content": "完成", "refusal": null},
            "finish_reason": "stop",
            "logprobs": null
          }
          """,
          """
          {
            "index": 0,
            "message": {"role": "assistant", "content": "Hello", "refusal": null},
            "finish_reason": "stop",
            "logprobs": null
          }
          """)
      ]),
    .init(
      id: "BC03", group: groups[0], title: "回复消息", path: "choices[].message",
      requirement: "role、content、refusal 必需；content 和 refusal 可为 null。此处展示选定的普通回复分支。",
      responses: [
        sample(
          "B03-1",
          """
          {
            "role": "assistant",
            "content": "查询完成",
            "refusal": null
          }
          """,
          """
          {
            "role": "assistant",
            "content": "Hello",
            "refusal": null
          }
          """)
      ]),
    .init(
      id: "BC04", group: groups[1], title: "用量统计", path: "usage",
      requirement: "usage 本身可选；返回该对象时，prompt_tokens、completion_tokens、total_tokens 必需。",
      responses: [
        sample(
          "B04-1",
          """
          {
            "prompt_tokens": 128,

            "total_tokens": 380
          }
          """,
          """
          {
            "prompt_tokens": 128,
            "completion_tokens": 252,
            "total_tokens": 380
          }
          """,
          differences: [
            .init(
              line: 2, path: "usage.completion_tokens",
              description: "缺失：实际 usage 没有 completion_tokens 字段。",
              requirement: "父对象 usage 已返回时，此字段必需。红色仅记录结构差异。")
          ]),
        sample(
          "B04-2",
          """
          {
            "prompt_tokens": 160,
            "completion_tokens": 64,
            "total_tokens": 224
          }
          """,
          """
          {
            "prompt_tokens": 128,
            "completion_tokens": 252,
            "total_tokens": 380
          }
          """),
      ]),
    .init(
      id: "BC05", group: groups[1], title: "细分用量", path: "usage.*_tokens_details",
      requirement: "两个细分对象及其所选子字段均为可选。缺少可选字段也展示差异，但不能按失败处理。",
      responses: [
        sample(
          "B05-1",
          """
          {
            "prompt_tokens_details": {
              "cached_tokens": 0
            },
            "completion_tokens_details": {

            }
          }
          """,
          """
          {
            "prompt_tokens_details": {
              "cached_tokens": 0
            },
            "completion_tokens_details": {
              "reasoning_tokens": 0
            }
          }
          """,
          differences: [
            .init(
              line: 5, path: "usage.completion_tokens_details.reasoning_tokens",
              description: "缺失：参考示例的 reasoning_tokens 未返回。",
              requirement: "这是可选字段。缺失不构成检测失败，也不改变可用性结论。")
          ])
      ]),
    .init(
      id: "BC06", group: groups[1], title: "附加信息", path: "$",
      requirement:
        "metadata、service_tier、system_fingerprint 是可选附加字段；system_fingerprint 已弃用。扩展字段仅标记新增。",
      responses: [
        sample(
          "B06-1",
          """
          {
            "metadata": {"project": "demo"},
            "service_tier": "default",
            "system_fingerprint": null,
            "gateway_trace": "demo-17"
          }
          """,
          """
          {
            "metadata": {"project": "reference"},
            "service_tier": "default",
            "system_fingerprint": null

          }
          """,
          differences: [
            .init(
              line: 4, path: "gateway_trace", description: "新增：该扩展字段不在本次参考片段中。",
              requirement: "仅标记扩展位置，不推断违反协议。其他未展示字段不由本片段判定。")
          ])
      ]),
    .init(
      id: "BC07", group: groups[2], title: "工具调用结构", path: "choices[].message.tool_calls",
      requirement: "观察到 function 工具分支时，对照 tool_calls 数组及其中 id、type、function 结构。",
      responses: [
        sample(
          "B07-1",
          """
          {
            "toolCalls": [{
              "id": "call-demo-1",
              "type": "function",
              "function": {"name": "lookup", "arguments": "{}"}
            }]
          }
          """,
          """
          {
            "tool_calls": [{
              "id": "call-reference",
              "type": "function",
              "function": {"name": "lookup", "arguments": "{}"}
            }]
          }
          """,
          differences: [
            .init(
              line: 1, path: "choices[].message.tool_calls",
              description: "字段名：实际 toolCalls，参考 tool_calls。",
              requirement: "本次选定函数工具调用分支以 tool_calls 承载调用数组。")
          ])
      ]),
    .init(
      id: "BC08", group: groups[2], title: "函数名称与参数", path: "tool_calls[].function",
      requirement: "name 和 arguments 必需，arguments 承载字符串。此处不检查函数参数的业务正确性。",
      responses: [
        sample(
          "B08-1",
          """
          {
            "name": "lookup_order",
            "arguments": {"id": "17"}
          }
          """,
          #"""
          {
            "name": "lookup_order",
            "arguments": "{\"id\":\"17\"}"
          }
          """#,
          differences: [
            .init(
              line: 2, path: "tool_calls[].function.arguments", description: "结构：实际是对象，参考承载结构为字符串。",
              requirement: "只对照承载类型，不把字符串内部的业务内容作为基线差异。")
          ])
      ]),
    .init(
      id: "BC09", group: groups[3], title: "分块外层", path: "chunk",
      requirement: "每个 chunk 的 id、object、created、model、choices 必需。其他可选附加字段按所选快照核对。",
      responses: [
        sample(
          "B09-1",
          """
          {
            "id": "stream-demo-1",
            "object": "chat.completion.chunk",
            "created": 1788825600,
            "model": "agent-prod",
            "choices": []
          }
          """,
          """
          {
            "id": "stream-reference",
            "object": "chat.completion.chunk",
            "created": 1700000000,
            "model": "reference-model",
            "choices": []
          }
          """, phase: "用量尾块")
      ]),
    .init(
      id: "BC10", group: groups[3], title: "增量候选", path: "chunk.choices[]",
      requirement: "候选包含序号、增量内容和结束原因；概率信息可选。用量尾块可以没有候选回复。",
      responses: [
        sample(
          "B10-1",
          """
          {
            "index": 0,
            "delta": {"content": "结果"},
            "finish_reason": null
          }
          """,
          """
          {
            "index": 0,
            "delta": {"content": "Hello"},
            "finish_reason": null
          }
          """, phase: "文本增量"),
        sample("B10-2", "{\n  \"choices\": []\n}", "{\n  \"choices\": []\n}", phase: "用量尾块"),
      ]),
    .init(
      id: "BC11", group: groups[3], title: "消息增量", path: "choices[].delta",
      requirement: "role、content、refusal 均可选；后续增量不必重复首块字段，结束时空 delta 有效。",
      responses: [
        sample(
          "B11-1", "{\n  \"role\": \"assistant\",\n  \"content\": \"\"\n}",
          "{\n  \"role\": \"assistant\",\n  \"content\": \"\"\n}", phase: "首块"),
        sample(
          "B11-2", "{\n  \"content\": \"完成\"\n}", "{\n  \"content\": \"Hello\"\n}", phase: "文本增量"),
        sample("B11-3", "{}", "{}", phase: "结束块"),
      ]),
    .init(
      id: "BC12", group: groups[3], title: "工具调用增量", path: "delta.tool_calls[]",
      requirement: "index 必需；id、type、function 可选。名称、参数可分块返回，不要求每块都是完整调用。",
      responses: [
        sample(
          "B12-1",
          """
          {
            "index": 0,
            "id": "call-demo-1",
            "type": "function",
            "function": {"name": "lookup"}
          }
          """,
          """
          {
            "index": 0,
            "id": "call-reference",
            "type": "function",
            "function": {"name": "lookup"}
          }
          """, phase: "工具首块"),
        sample(
          "B12-2", "{\n  \"index\": 0,\n  \"function\": {\"arguments\": \"17}\"}\n}",
          "{\n  \"index\": 0,\n  \"function\": {\"arguments\": \"17}\"}\n}", phase: "参数增量"),
      ]),
    .init(
      id: "BC13", group: groups[3], title: "流式用量返回", path: "chunk.usage / choices",
      requirement: "本组请求开启 include_usage。普通块 usage 可为 null，用量尾块提供计数且 choices 为空；不把结束标记当作 JSON 字段。",
      responses: [
        sample("B13-1", "{\n  \"usage\": null\n}", "{\n  \"usage\": null\n}", phase: "普通块"),
        sample(
          "B13-2",
          """
          {
            "choices": [],
            "usage": {
              "prompt_tokens": 128,
              "completion_tokens": 252,
              "total_tokens": 380
            }
          }
          """,
          """
          {
            "choices": [],
            "usage": {
              "prompt_tokens": 64,
              "completion_tokens": 32,
              "total_tokens": 96
            }
          }
          """, phase: "用量尾块"),
      ]),
    .init(
      id: "BC14", group: groups[4], title: "错误对象", path: "error",
      requirement:
        "只对照本次选定的 429 / 503 结构化错误对象。类型、消息、参数、错误码必须存在，参数与错误码可以为空；不外推到所有网关错误。",
      responses: [
        sample(
          "B14-1",
          """
          {
            "type": "rate_limit_error",
            "message": "演示错误响应",
            "param": null,
            "code": null
          }
          """,
          """
          {
            "type": "rate_limit_error",
            "message": "Reference error",
            "param": null,
            "code": null
          }
          """, phase: "429 结构化错误"),
        .init(
          id: "B14-2", phase: "网关 HTML 返回", actual: ["<html>gateway unavailable</html>"],
          reference: [], unavailable: "无适用基线"),
        .init(id: "B14-3", phase: "请求未获得响应", actual: [], reference: [], unavailable: "未获得响应"),
        .init(id: "B14-4", phase: "503 场景未执行", actual: [], reference: [], unavailable: "尚未检测"),
      ]),
  ]
}
