import SwiftUI

// 检测记录：按时间列出，可打开、可比对（条件一致时）。
struct HistoryView: View {
  @ObservedObject var store: Workbench
  var body: some View {
    VStack(alignment: .leading, spacing: 24) {
      HStack(alignment: .firstTextBaseline) {
        Text("检测记录").font(.system(size: 23, weight: .bold))
        Text("\(store.records.count)").font(.system(size: 15)).foregroundStyle(Theme.faint)
        Spacer()
      }
      HStack {
        Picker("服务范围", selection: $store.historyFilter) {
          Text("全部服务").tag("all")
          Text("当前服务").tag("current")
        }.labelsHidden().pickerStyle(.segmented).frame(width: 200)
        Spacer()
        HStack(spacing: 7) {
          Image(systemName: "magnifyingglass").foregroundStyle(Theme.faint)
          TextField("搜索模型或地址", text: $store.historySearch).textFieldStyle(.plain)
          if !store.historySearch.isEmpty {
            Button {
              store.historySearch = ""
            } label: {
              Image(systemName: "xmark.circle.fill").foregroundStyle(Theme.faint)
            }.buttonStyle(.plain)
          }
        }
        .font(Theme.bodyFont)
        .padding(.horizontal, 11).frame(width: 240, height: 34)
        .background(Theme.canvas, in: RoundedRectangle(cornerRadius: 8))
      }
      if store.records.isEmpty {
        VStack(alignment: .leading, spacing: 16) {
          Image(systemName: "clock.arrow.circlepath")
            .font(.system(size: 26, weight: .light))
            .foregroundStyle(Theme.faint)
            .frame(width: 56, height: 56)
            .background(Theme.unknownTint, in: Circle())
          Text("还没有检测记录").font(.system(size: 21, weight: .semibold))
          Text("完成第一次检测后，结果会显示在这里，随时可以回看。")
            .font(Theme.bodyFont).foregroundStyle(Theme.muted)
          Action(title: "立即测试", icon: "play.fill", disabled: store.running) {
            store.prepareRun()
          }
        }.padding(.vertical, 36)
      } else if store.visibleRecords.isEmpty {
        VStack(alignment: .leading, spacing: 14) {
          Text("没有找到匹配记录").font(.system(size: 18, weight: .semibold))
          Text("试试其他模型名称，或清除当前筛选。")
            .font(Theme.bodyFont).foregroundStyle(Theme.muted)
          Action(title: "清除筛选", icon: "xmark", primary: false) {
            store.historySearch = ""
            store.historyFilter = "all"
          }
        }.padding(.vertical, 36)
      } else {
        VStack(spacing: 0) {
          HStack {
            Text("模型服务")
              .padding(.leading, 30).frame(width: 240, alignment: .leading)
            Text("使用判断与限制").frame(maxWidth: .infinity, alignment: .leading)
            Text("检测时间").frame(width: 130, alignment: .trailing)
          }
          .font(.system(size: 10.5)).foregroundStyle(Theme.faint).padding(.bottom, 11)
          ForEach(store.visibleRecords) { record in
            HStack(spacing: 12) {
              Toggle(
                "选择记录 \(record.readableID)",
                isOn: Binding(
                  get: { store.historySelection.contains(record.id) },
                  set: { _ in store.toggleHistory(record.id) }
                )
              ).labelsHidden().toggleStyle(.checkbox)
              .disabled(
                store.historySelection.count == 2 && !store.historySelection.contains(record.id))
              Button {
                store.openRecord(record)
              } label: {
                HStack(alignment: .top, spacing: 18) {
                  VStack(alignment: .leading, spacing: 6) {
                    Text(record.service.model)
                      .font(.system(size: 12.5, weight: .semibold)).lineLimit(2)
                      .foregroundStyle(Theme.ink)
                    Text(record.service.host)
                      .font(Theme.captionFont).foregroundStyle(Theme.faint).lineLimit(1)
                  }.frame(width: 190, alignment: .leading)
                  VStack(alignment: .leading, spacing: 7) {
                    Pill(text: record.title, style: record.style)
                    Text(record.limitation)
                      .font(Theme.captionFont).foregroundStyle(Theme.muted).lineLimit(2)
                  }.frame(maxWidth: .infinity, alignment: .leading)
                  VStack(alignment: .trailing, spacing: 7) {
                    Text(record.date.formatted(date: .numeric, time: .shortened))
                    Text(
                      record.hasCurrentEvidence
                        ? "\(record.completed.count)/\(record.modules.count) 个模块完成" : "旧版记录")
                  }
                  .font(Theme.captionFont).foregroundStyle(Theme.faint)
                  .frame(width: 130, alignment: .trailing)
                }
                .contentShape(Rectangle())
              }
              .buttonStyle(.plain)
              .help("打开这次检测结果")
            }
            .padding(.vertical, 16).padding(.horizontal, 3)
            .background(
              store.historySelection.contains(record.id) ? Theme.accent.opacity(0.04) : .clear
            )
            .overlay(alignment: .bottom) { Rectangle().fill(Theme.line).frame(height: 1) }
          }
        }
        HStack(spacing: 14) {
          VStack(alignment: .leading, spacing: 5) {
            Text("已选 \(store.historySelection.count) / 2 条记录")
              .font(.system(size: 12.5, weight: .medium))
            Text(store.comparisonBlocker ?? "测试条件一致，可以比较分类表现。")
              .font(Theme.captionFont).foregroundStyle(Theme.muted)
              .fixedSize(horizontal: false, vertical: true)
          }
          Spacer()
          if !store.historySelection.isEmpty {
            IconButton(symbol: "xmark", help: "取消选择") { store.historySelection = [] }
          }
          Action(
            title: "对比结果", icon: "arrow.left.arrow.right", disabled: store.comparisonBlocker != nil
          ) {
            store.showHistoryComparison = true
          }
        }.padding(.top, 6)
      }
    }
    .sheet(isPresented: $store.showHistoryComparison) { HistoryComparison(store: store) }
  }
}

// 两次检测的对照：同条件下逐模块比。
struct HistoryComparison: View {
  @ObservedObject var store: Workbench
  var body: some View {
    VStack(alignment: .leading, spacing: 20) {
      HStack {
        Text("两次检测对比").font(.system(size: 20, weight: .bold))
        Spacer()
        IconButton(symbol: "xmark", help: "关闭对比") { store.showHistoryComparison = false }
      }
      Pill(text: "样本、规则与运行条件一致", style: FindingState.pass.style, icon: "checkmark.seal")
      ScrollView {
        VStack(alignment: .leading, spacing: 0) {
          HStack(alignment: .top, spacing: 24) {
            ForEach(store.comparedRecords) { record in
              VStack(alignment: .leading, spacing: 9) {
                Text(record.service.model)
                  .font(.system(size: 15, weight: .bold)).lineLimit(2)
                Text(record.date.formatted(date: .numeric, time: .shortened))
                  .font(Theme.captionFont).foregroundStyle(Theme.faint)
                Pill(text: record.title, style: record.style)
                Text(record.explanation)
                  .font(Theme.captionFont).foregroundStyle(Theme.muted)
                  .fixedSize(horizontal: false, vertical: true)
                Action(title: "查看这次依据", icon: "doc.text.magnifyingglass", primary: false) {
                  store.showHistoryComparison = false
                  store.openRecord(record)
                }
              }.frame(maxWidth: .infinity, alignment: .leading)
            }
          }.padding(.bottom, 20)
          ForEach(CheckModule.testModules) { module in
            VStack(alignment: .leading, spacing: 11) {
              Label(module.title, systemImage: module.symbol)
                .font(.system(size: 12.5, weight: .semibold))
              HStack(alignment: .top, spacing: 24) {
                ForEach(store.comparedRecords) { record in
                  Text(record.brief(module))
                    .font(Theme.captionFont).foregroundStyle(Theme.muted)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
              }
            }
            .padding(.vertical, 14)
            .overlay(alignment: .top) { Rectangle().fill(Theme.line).frame(height: 1) }
          }
        }
      }.frame(maxHeight: 540)
    }
    .padding(28).frame(width: 780).background(.white).foregroundStyle(Theme.ink)
  }
}
