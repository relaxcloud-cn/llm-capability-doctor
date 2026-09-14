import SwiftUI

// 接入信息摘要：默认只给四项关键信息，其余按需展开。
struct AccessSummary: View {
  @ObservedObject var store: Workbench
  var record: RunRecord?
  private var service: Service? { record?.service ?? store.service }
  private var observed: RunRecord? { record ?? store.latestForService }
  private var state: String {
    record != nil ? "已结束" : store.running ? "检测中" : observed == nil ? "待检测" : "已结束"
  }
  var body: some View {
    VStack(alignment: .leading, spacing: 14) {
      HStack {
        Label("模型接入信息", systemImage: "cpu")
          .font(.system(size: 14, weight: .semibold))
        Spacer()
        if record == nil {
          TextButton(title: "更换配置") { store.editService() }
            .disabled(store.running)
        }
        TextButton(title: store.accessExpanded ? "收起详情" : "展开详情") {
          store.accessExpanded.toggle()
        }
      }
      HStack(alignment: .top, spacing: 28) {
        VStack(alignment: .leading, spacing: 0) {
          KVRow(label: "模型名称", value: service?.model ?? "尚未配置")
          KVRow(label: "接入地址", value: service?.displayURL ?? "尚未填写")
        }.frame(maxWidth: .infinity, alignment: .leading)
        VStack(alignment: .leading, spacing: 0) {
          KVRow(label: "检测状态", value: state)
          KVRow(
            label: record == nil ? "最近结果" : "本次记录",
            value: observed.map {
              $0.date.formatted(date: .abbreviated, time: .shortened)
            } ?? "暂无记录")
        }.frame(maxWidth: .infinity, alignment: .leading)
      }
      if store.accessExpanded {
        VStack(spacing: 0) {
          KVRow(label: "接口方式", value: "OpenAI Chat Completions", note: "来源：本次接入配置")
          KVRow(
            label: "响应模型名", value: observed?.responseModel ?? "尚无可用响应记录",
            note: "来自响应的 model 字段；不据此确认后端模型身份。")
          if let observed, let name = observed.responseModel, name != observed.service.model {
            KVRow(
              label: "名称差异", value: "配置名称与响应名称不同",
              note: "保留两侧原值，不推断为假模型或不可用。")
          }
          KVRow(
            label: "运行版本", value: record?.isRealReport == true ? "AgentCheck 桌面端 · macOS" : "AgentCheck 交互原型 · macOS",
            note: "本地运行信息，不是服务端部署版本。")
          KVRow(
            label: "测试条件", value: observed?.context?.environment ?? "尚未建立测试记录",
            note: observed?.context?.parameters ?? "测试开始后与记录一起保存")
        }.padding(.top, 2)
      }
    }
    .padding(.vertical, 8)
    .id("access")
  }
}
