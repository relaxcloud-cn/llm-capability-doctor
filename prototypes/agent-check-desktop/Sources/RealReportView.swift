import SwiftUI

private struct RealTaskReport: Identifiable {
  let id: String
  let kind: String
  let status: String
  let verdict: String
  let request: String
  let response: String
  let evidenceJSON: String
}

struct RealModuleView: View {
  @ObservedObject var store: Workbench
  var module: CheckModule
  var record: RunRecord

  private var backendID: String { module.backendID ?? "ingress" }
  private var reportObject: [String: Any]? {
    guard let reportJSON = record.reportJSON,
      let data = reportJSON.data(using: .utf8)
    else { return nil }
    return try? JSONSerialization.jsonObject(with: data) as? [String: Any]
  }
  private var recordObject: [String: Any] {
    reportObject?["record"] as? [String: Any] ?? [:]
  }
  private var state: String {
    record.moduleStates?[backendID] ?? moduleResult?["state"] as? String ?? "unverified"
  }
  private var stateStyle: StatusStyle {
    switch state {
    case "pass": return FindingState.pass.style
    case "fail": return FindingState.fail.style
    case "unsupported", "inconclusive", "invalid_execution": return FindingState.unstable.style
    default: return FindingState.unknown.style
    }
  }
  private var moduleResult: [String: Any]? {
    let results = recordObject["moduleResults"] as? [[String: Any]] ?? []
    return results.first { ($0["moduleId"] as? String) == backendID }
      ?? results.first { ($0["module_id"] as? String) == backendID }
  }
  private var reason: String {
    moduleResult?["reason"] as? String ?? "该模块已完成真实执行；详细任务证据见下方。"
  }
  private var tasks: [RealTaskReport] {
    let verdicts = verdictsByTask
    return evidenceItems.enumerated().map { index, item in
      let itemID = item["id"] as? String ?? "任务 \(index + 1)"
      let payload = item["payload"] as? [String: Any] ?? [:]
      let nested = payload["payload"] as? [String: Any]
      let sampleID = item["sample_id"] as? String
        ?? payload["sample_id"] as? String
        ?? nested?["sample_id"] as? String
        ?? itemID
      return RealTaskReport(
        id: sampleID,
        kind: item["kind"] as? String ?? payload["kind"] as? String ?? "检测任务",
        status: statusText(from: item),
        verdict: verdicts[sampleID] ?? label,
        request: requestText(from: item),
        response: responseText(from: item),
        evidenceJSON: prettyJSON(item)
      )
    }
  }
  private var evidenceItems: [[String: Any]] {
    let all = recordObject["evidence"] as? [[String: Any]] ?? []
    return all.flatMap { item -> [[String: Any]] in
      let id = item["id"] as? String ?? ""
      let payload = item["payload"] as? [String: Any] ?? [:]
      guard payload["module"] as? String == backendID || id.contains("\(backendID)") else {
        return []
      }
      let nestedPayload = payload["payload"] as? [String: Any]
      if let nested = nestedPayload?["evidence"] as? [[String: Any]], !nested.isEmpty {
        return nested.map { evidence in
          var copy = evidence
          copy["kind"] = item["kind"] ?? "检测任务"
          return copy
        }
      }
      return [item]
    }
  }
  private var verdictsByTask: [String: String] {
    let all = recordObject["evidence"] as? [[String: Any]] ?? []
    for item in all {
      let payload = item["payload"] as? [String: Any] ?? [:]
      guard payload["module"] as? String == backendID,
        let modulePayload = payload["payload"] as? [String: Any]
      else { continue }
      if let scorecard = modulePayload["scorecard"] as? [String: Any],
        let observations = scorecard["observations"] as? [[String: Any]]
      {
        return Dictionary(uniqueKeysWithValues: observations.compactMap { observation in
          guard let id = observation["sample_id"] as? String,
            let result = observation["label"] as? String
          else { return nil }
          return (id, result)
        })
      }
      if let report = modulePayload["report"] as? [String: Any],
        let rows = report["rows"] as? [[String: Any]]
      {
        return Dictionary(uniqueKeysWithValues: rows.compactMap { row in
          guard let id = row["sample_id"] as? String,
            let result = row["result"] as? String
          else { return nil }
          return (id, result)
        })
      }
    }
    return [:]
  }

  var body: some View {
    VStack(alignment: .leading, spacing: 24) {
      VerdictBanner(
        style: stateStyle,
        title: "\(module.title)：\(label)",
        detail: reason,
        meta: "真实执行 · \(tasks.count) 个任务报告"
      )

      VStack(alignment: .leading, spacing: 0) {
        SectionHeader(title: "任务报告", caption: "每条记录来自 CLI 的实际请求、响应和判定证据")
        if tasks.isEmpty {
          Text("没有找到该模块的任务级证据。")
            .font(Theme.bodyFont)
            .foregroundStyle(Theme.faint)
            .padding(.vertical, 18)
        } else {
          LazyVStack(spacing: 0) {
            ForEach(tasks) { task in
              RealTaskRow(task: task)
              if task.id != tasks.last?.id { Divider() }
            }
          }
        }
      }
      .padding(18)
      .background(.white, in: RoundedRectangle(cornerRadius: Theme.radiusCard))
      .overlay(RoundedRectangle(cornerRadius: Theme.radiusCard).stroke(Theme.line))
    }
  }

  private var label: String {
    switch state {
    case "pass": return "通过"
    case "fail": return "失败"
    case "unsupported": return "不支持"
    case "invalid_execution": return "本次检测未完成"
    case "inconclusive": return "证据不足，暂不能判断"
    case "not_selected": return "未选择"
    default: return "尚未检测"
    }
  }

  private func requestText(from item: [String: Any]) -> String {
    let payload = item["payload"] as? [String: Any] ?? item
    let nested = payload["payload"] as? [String: Any] ?? payload
    let request = (nested["request"] as? [String: Any]) ?? (payload["request"] as? [String: Any])
    return request?["prompt"] as? String ?? "未记录请求内容"
  }

  private func responseText(from item: [String: Any]) -> String {
    let payload = item["payload"] as? [String: Any] ?? item
    let nested = payload["payload"] as? [String: Any] ?? payload
    let response = (nested["response"] as? [String: Any]) ?? (payload["response"] as? [String: Any])
    if let stream = response?["stream"] as? [String: Any],
      let content = stream["content"] as? String, !content.isEmpty
    { return content }
    if let body = response?["body"] as? [String: Any],
      let choices = body["choices"] as? [[String: Any]],
      let message = choices.first?["message"] as? [String: Any],
      let content = message["content"] as? String, !content.isEmpty
    { return content }
    if let error = response?["error"] as? String, !error.isEmpty { return error }
    return "未记录文本响应；请展开原始证据查看。"
  }

  private func statusText(from item: [String: Any]) -> String {
    let payload = item["payload"] as? [String: Any] ?? item
    let nested = payload["payload"] as? [String: Any] ?? payload
    let response = (nested["response"] as? [String: Any]) ?? (payload["response"] as? [String: Any])
    if let error = response?["error"] as? String, !error.isEmpty { return "请求异常" }
    if let status = response?["status"] as? Int {
      return status >= 200 && status < 300 ? "请求成功" : "HTTP \(status)"
    }
    return "已记录"
  }

  private func prettyJSON(_ value: [String: Any]) -> String {
    guard JSONSerialization.isValidJSONObject(value),
      let data = try? JSONSerialization.data(
        withJSONObject: value, options: [.prettyPrinted, .sortedKeys])
    else { return "无法展示原始证据" }
    return String(decoding: data, as: UTF8.self)
  }
}

private struct RealTaskRow: View {
  let task: RealTaskReport
  @State private var expanded = false

  var body: some View {
    DisclosureGroup(isExpanded: $expanded) {
      VStack(alignment: .leading, spacing: 14) {
        EvidenceField(title: "检测输入", value: task.request)
        EvidenceField(title: "模型响应", value: task.response)
        EvidenceField(title: "原始证据", value: task.evidenceJSON, monospaced: true)
      }
      .padding(.top, 12)
    } label: {
      HStack(spacing: 12) {
        Image(systemName: expanded ? "chevron.down.circle" : "chevron.right.circle")
          .foregroundStyle(Theme.accent)
        VStack(alignment: .leading, spacing: 4) {
          Text(task.id).font(.system(size: 12.5, weight: .semibold)).foregroundStyle(Theme.ink)
          Text(task.kind).font(Theme.captionFont).foregroundStyle(Theme.faint)
        }
        Spacer()
        Pill(text: task.status, style: Theme.informative, icon: "network")
        Pill(text: task.verdict, style: verdictStyle(task.verdict))
      }
      .contentShape(Rectangle())
    }
    .padding(.vertical, 13)
  }

  private func verdictStyle(_ verdict: String) -> StatusStyle {
    switch verdict.lowercased() {
    case "correct", "accepted", "pass": return FindingState.pass.style
    case "wrong", "failed", "fail": return FindingState.fail.style
    default: return FindingState.unstable.style
    }
  }
}

private struct EvidenceField: View {
  let title: String
  let value: String
  var monospaced = false

  var body: some View {
    VStack(alignment: .leading, spacing: 6) {
      Text(title).font(.system(size: 11.5, weight: .semibold)).foregroundStyle(Theme.muted)
      Text(value)
        .font(monospaced ? .system(size: 10, design: .monospaced) : Theme.bodyFont)
        .foregroundStyle(Theme.ink)
        .textSelection(.enabled)
        .fixedSize(horizontal: false, vertical: true)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(10)
        .background(Theme.canvas, in: RoundedRectangle(cornerRadius: 6))
    }
  }
}
