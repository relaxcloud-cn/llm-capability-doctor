import AppKit
import SwiftUI

struct ComparisonView: View {
  @ObservedObject var store: Workbench
  var body: some View { ModuleView(store: store, module: .comparison) }
}

// 基线对比：左侧实际返回，右侧官方规范，只对照结构不对照答案。
struct BaselineDetails: View {
  @ObservedObject var store: Workbench
  @State private var specExpanded = false
  var record: RunRecord
  private var items: [BaselineItem] { BaselineItem.forRecord(record) }
  private var item: BaselineItem {
    items.first { $0.id == store.baselineSelection } ?? items[3]
  }
  private var response: BaselineResponse {
    item.responses[min(max(0, store.baselineRecord), item.responses.count - 1)]
  }
  private var mark: StructureDifference? {
    response.differences.first { $0.line == store.baselineMark } ?? response.differences.first
  }
  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      HStack(alignment: .firstTextBaseline) {
        VStack(alignment: .leading, spacing: 4) {
          Text("结构对照：你的服务 vs 官方规范")
            .font(.system(size: 20, weight: .bold))
          Text("只对照字段与结构，不比较内容答案；结构差异不等于模型不可用。")
            .font(Theme.captionFont).foregroundStyle(Theme.muted)
        }
        Spacer()
        Text("14 个维度 · 5 项含差异")
          .font(Theme.captionFont).foregroundStyle(Theme.faint)
      }
      HStack(alignment: .top, spacing: 18) {
        VStack(alignment: .leading, spacing: 10) {
          ForEach(BaselineItem.groups, id: \.self) { group in
            VStack(alignment: .leading, spacing: 1) {
              Text(group)
                .font(.system(size: 10.5, weight: .semibold))
                .foregroundStyle(Theme.faint)
                .padding(.horizontal, 9)
              ForEach(items.filter { $0.group == group }) { entry in
                Button {
                  store.baselineSelection = entry.id
                  store.baselineRecord = 0
                  store.baselineMark = -1
                  store.showOriginal = false
                } label: {
                  HStack(spacing: 6) {
                    Text(entry.title)
                      .font(.system(size: 11.5))
                      .lineLimit(1)
                      .foregroundStyle(Theme.ink)
                    Spacer(minLength: 1)
                    if entry.hasDifference {
                      Circle().fill(Theme.blockedBar).frame(width: 5, height: 5)
                    }
                  }
                  .padding(.horizontal, 9).frame(height: 30)
                  .background(
                    item.id == entry.id ? Theme.accent.opacity(0.06) : .clear,
                    in: RoundedRectangle(cornerRadius: 7))
                  .overlay(
                    RoundedRectangle(cornerRadius: 7)
                      .stroke(item.id == entry.id ? Theme.accent.opacity(0.4) : .clear))
                }.buttonStyle(.plain)
              }
            }
          }
        }.frame(width: 158)
        VStack(alignment: .leading, spacing: 13) {
          HStack {
            SectionHeader(title: item.title)
            Toggle("仅看差异", isOn: $store.baselineDifferencesOnly)
              .toggleStyle(.checkbox)
              .font(Theme.captionFont)
              .fixedSize()
          }
          HStack {
            Picker("响应记录", selection: $store.baselineRecord) {
              ForEach(Array(item.responses.enumerated()), id: \.offset) { index, value in
                Text("\(value.id) · \(value.phase)").tag(index)
              }
            }.labelsHidden().frame(maxWidth: 310).id("\(item.id)-\(response.id)")
            Spacer()
            Text("\(item.responses.count) 条记录")
              .font(Theme.captionFont).foregroundStyle(Theme.faint)
          }
          .onChange(of: store.baselineRecord) { _, _ in store.baselineMark = -1 }
          if response.differences.isEmpty {
            Pill(text: response.status, style: FindingState.pass.style, icon: "checkmark")
          } else {
            Pill(text: response.status, style: FindingState.fail.style, icon: "exclamationmark")
          }
          if let state = response.unavailable {
            VStack(alignment: .leading, spacing: 8) {
              Text(state).font(.system(size: 16, weight: .semibold))
              Text(emptyReason(state))
                .font(Theme.captionFont).foregroundStyle(Theme.muted)
                .fixedSize(horizontal: false, vertical: true)
            }.padding(.vertical, 10)
            if !response.raw.isEmpty {
              Text(response.raw)
                .font(.system(size: 12, design: .monospaced))
                .textSelection(.enabled)
            }
          } else if store.baselineDifferencesOnly && response.differences.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
              Text("这条记录没有字段或结构差异")
                .font(.system(size: 15, weight: .semibold))
              Text("两侧具体值不同不标红；同一场景的其他记录仍保留各自结果。")
                .font(Theme.captionFont).foregroundStyle(Theme.muted)
              TextButton(title: "查看全部内容") { store.baselineDifferencesOnly = false }
            }.padding(.vertical, 8)
          } else {
            HStack(alignment: .top, spacing: 2) {
              codePane("你的服务", subtitle: record.service.model, actual: true)
              codePane("官方规范", subtitle: "OpenAI 参考示例 · 2026-09-08", actual: false)
            }
            .background(Theme.line)
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(Theme.line))
            .clipShape(RoundedRectangle(cornerRadius: 8))
          }
          if let mark {
            VStack(alignment: .leading, spacing: 6) {
              Label(
                mark.path,
                systemImage: "exclamationmark.circle"
              )
              .font(.system(size: 11, weight: .semibold, design: .monospaced))
              .foregroundStyle(Theme.blocked)
              .textSelection(.enabled)
              Text(mark.description).font(Theme.bodyFont)
              Text(mark.requirement)
                .font(Theme.captionFont).foregroundStyle(Theme.muted)
            }
            .padding(13)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Theme.blockedTint.opacity(0.6), in: RoundedRectangle(cornerRadius: 9))
          }
          Fold(title: "字段要求与参考快照", icon: "doc.badge.clock", expanded: $specExpanded) {
            Text(item.path)
              .font(.system(size: 11, design: .monospaced))
              .textSelection(.enabled)
            Text(item.requirement).font(Theme.bodyFont)
            Text(
              "OpenAI OpenAPI Specification · 核对日期 2026-09-08\n来源：openai/openai-openapi · openapi.json\n规范格式 3.1.0 · 文档信息版本 2.3.0（不是接口版本）"
            ).font(Theme.captionFont).foregroundStyle(Theme.faint)
            Text("SHA-256 \(record.context?.baseline ?? "缺失")")
              .font(.system(size: 10, design: .monospaced))
              .textSelection(.enabled)
          }
          Fold(title: "原始响应片段", icon: "curlybraces", expanded: $store.showOriginal) {
            Text("\(record.service.displayURL) · \(response.id) · \(response.phase)")
              .font(Theme.captionFont).foregroundStyle(Theme.faint)
            Text(
              "原始局部片段（演示）；对齐视图中的空白占位不写入响应。独立结构样例，不借作 Agent 任务证据。"
            ).font(Theme.captionFont).foregroundStyle(Theme.faint)
            Text(response.raw.isEmpty ? "没有响应正文" : response.raw)
              .foregroundStyle(Theme.ink)
              .font(.system(size: 11.5, design: .monospaced))
              .textSelection(.enabled)
              .frame(maxWidth: .infinity, alignment: .leading)
          }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
      }
      Text("红色 = 字段或结构差异，不等于模型不可用。异常响应中未执行的分支仍标记为未验证。")
        .font(Theme.captionFont).foregroundStyle(Theme.faint)
    }
  }

  private func codePane(_ title: String, subtitle: String, actual: Bool) -> some View {
    let lines = actual ? response.actual : response.reference
    let indices = (0..<max(response.actual.count, response.reference.count)).filter { index in
      !store.baselineDifferencesOnly || response.differences.contains { $0.line == index }
    }
    return VStack(alignment: .leading, spacing: 0) {
      HStack(alignment: .top) {
        VStack(alignment: .leading, spacing: 4) {
          Text(title).font(.system(size: 13, weight: .semibold))
          Text(subtitle)
            .font(.system(size: 10)).foregroundStyle(Theme.faint)
            .lineLimit(1).truncationMode(.middle)
            .help(subtitle)
        }
        Spacer(minLength: 3)
        Button {
          NSPasteboard.general.clearContents()
          NSPasteboard.general.setString(
            lines.filter { !$0.isEmpty }.joined(separator: "\n"), forType: .string)
          store.toast = "已复制\(title)的局部片段"
        } label: {
          Image(systemName: "doc.on.doc")
        }
        .buttonStyle(.plain)
        .foregroundStyle(Theme.muted)
        .help("复制完整局部片段")
      }
      .padding(12).frame(minHeight: 58)
      .background(Theme.canvas)
      ScrollView([.horizontal, .vertical]) {
        VStack(alignment: .leading, spacing: 0) {
          ForEach(indices, id: \.self) { index in
            let changed = response.differences.contains { $0.line == index }
            let line = index < lines.count ? lines[index] : ""
            Button {
              if changed { store.baselineMark = index }
            } label: {
              HStack(spacing: 9) {
                Text("\(index + 1)")
                  .foregroundStyle(Theme.faint)
                  .frame(width: 20, alignment: .trailing)
                  .monospacedDigit()
                highlighted(line, changed: changed, actual: actual)
                  .fixedSize()
                Spacer(minLength: 12)
              }
              .font(.system(size: 11.5, design: .monospaced))
              .foregroundStyle(Theme.ink)
              .padding(.horizontal, 9)
              .frame(minWidth: 268, minHeight: 27, alignment: .leading)
              .background(changed ? Theme.blockedTint : .white)
            }
            .buttonStyle(.plain)
            .help(changed ? "查看此处差异" : "字段值不参与标红")
          }
        }.padding(.vertical, 8)
      }.frame(height: CGFloat(min(indices.count, 9) * 27 + 22))
    }
    .frame(maxWidth: .infinity, alignment: .topLeading)
    .background(.white)
  }
  private func highlighted(_ line: String, changed: Bool, actual: Bool) -> Text {
    if line.isEmpty {
      return Text(actual ? "缺失" : "无此字段").foregroundColor(Theme.blocked)
    }
    guard changed, let colon = line.firstIndex(of: ":") else { return Text(line) }
    return Text(String(line[..<colon])).foregroundColor(Theme.blocked)
      + Text(String(line[colon...])).foregroundColor(Theme.ink)
  }
  private func emptyReason(_ state: String) -> String {
    if state == "无适用基线" {
      return "收到网关 HTML，而当前基线是指定错误分支的 JSON 对象；不强行套用，也不显示为通过。"
    }
    if state == "未获得响应" { return "没有响应正文，无法进行结构对照；这条记录不代表模型结构不符合要求。" }
    return "此场景尚未执行，没有可对照数据。"
  }
}
