import AppKit
import SwiftUI

struct ModuleView: View {
  @ObservedObject var store: Workbench
  var module: CheckModule
  var body: some View {
    VStack(alignment: .leading, spacing: 26) {
      if let record = store.currentRecord, record.completed.contains(module),
        record.hasCurrentEvidence
      {
        // 演示记录与真实报告共用同一套分组折叠骨架
        RealModuleView(module: module, record: record).id(record.id)
        Divider()
        HStack(spacing: 14) {
          Action(title: "单独复测", icon: "arrow.clockwise", primary: false, disabled: store.running) {
            store.prepareRun(module: module)
          }
          Text(
            record.service == store.service
              ? "新建一条检测记录，保留本次结果。" : "将使用当前服务：\(store.service?.model ?? "")"
          )
          .font(Theme.captionFont).foregroundStyle(Theme.faint).lineLimit(2)
          Spacer()
        }
      } else {
        EmptyModule(store: store, module: module, record: store.currentRecord)
      }
    }
  }
}
