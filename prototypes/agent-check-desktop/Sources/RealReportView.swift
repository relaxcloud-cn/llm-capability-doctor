import SwiftUI

struct RealModuleView: View {
  @ObservedObject var store: Workbench
  var module: CheckModule
  var record: RunRecord

  private var backendID: String { module.backendID ?? "ingress" }
  private var state: String { record.moduleStates?[backendID] ?? "unverified" }
  private var stateStyle: StatusStyle {
    switch state {
    case "pass": return FindingState.pass.style
    case "fail": return FindingState.fail.style
    case "unsupported", "inconclusive", "invalid_execution": return FindingState.unstable.style
    default: return FindingState.unknown.style
    }
  }
  private var reportObject: [String: Any]? {
    guard let reportJSON = record.reportJSON,
      let data = reportJSON.data(using: .utf8)
    else { return nil }
    return try? JSONSerialization.jsonObject(with: data) as? [String: Any]
  }
  private var evidenceCount: Int {
    ((reportObject?["record"] as? [String: Any])?["evidence"] as? [[String: Any]])?.count ?? 0
  }
  private var reason: String {
    let results = ((reportObject?["record"] as? [String: Any])?["module_results"] as? [[String: Any]]) ?? []
    return results.first { ($0["module_id"] as? String) == backendID }?["reason"] as? String
      ?? "该模块已完成真实执行；详细事件和证据见导出报告。"
  }

  var body: some View {
    VStack(alignment: .leading, spacing: 24) {
      VerdictBanner(
        style: stateStyle,
        title: "\(module.title)：\(label)",
        detail: reason,
        meta: "真实 Rust CLI 执行 · \(evidenceCount) 条统一证据"
      ) {
        EmptyView()
      }
      VStack(alignment: .leading, spacing: 0) {
        SectionHeader(title: "本次真实结果", caption: "GUI 只展示 Rust CLI 生成的事实，不重新解释结论")
        KVRow(label: "检测项目", value: module.title)
        KVRow(label: "执行状态", value: label)
        KVRow(label: "证据条目", value: "\(evidenceCount) 条")
        KVRow(label: "总体结论", value: record.outcome.title)
        if let limitations = reportObject?["limitations"] as? [String], !limitations.isEmpty {
          KVRow(label: "限制", value: limitations.joined(separator: "；"))
        }
      }
      .padding(18)
      .background(.white, in: RoundedRectangle(cornerRadius: Theme.radiusCard))
      .overlay(RoundedRectangle(cornerRadius: Theme.radiusCard).stroke(Theme.line))
      Text("完整请求、响应摘要、事件、证据引用和未验证范围请使用右上角导出报告查看。")
        .font(Theme.captionFont)
        .foregroundStyle(Theme.faint)
        .fixedSize(horizontal: false, vertical: true)
    }
  }

  private var label: String {
    switch state {
    case "pass": return "通过"
    case "fail": return "失败"
    case "unsupported": return "不支持"
    case "invalid_execution": return "执行无效"
    case "inconclusive": return "待确认"
    case "not_selected": return "未选择"
    default: return "未验证"
    }
  }
}
