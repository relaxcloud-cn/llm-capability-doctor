import AppKit
import SwiftUI

struct WorkbenchView: View {
  @ObservedObject var store: Workbench
  private var title: String {
    if store.configuring { return store.service == nil ? "添加模型服务" : "更换模型服务" }
    switch store.screen {
    case .home: return store.running ? "检测进行中" : "首页"
    case .result: return "检测结果"
    case .history: return "检测记录"
    case .comparison: return CheckModule.comparison.title
    case .module(let module): return module.title
    }
  }
  private var showsRecord: Bool {
    !store.configuring && store.screen != .home && store.screen != .history
  }
  var body: some View {
    HStack(spacing: 0) {
      sidebar
      Rectangle().fill(Theme.line).frame(width: 1)
      VStack(spacing: 0) {
        toolbar
        ScrollViewReader { reader in
          ScrollView {
            Color.clear.frame(height: 0).id("top")
            Group {
              if store.configuring {
                ConnectionView(store: store)
              } else {
                switch store.screen {
                case .home:
                  if store.running { ProgressScreen(store: store) } else { HomeView(store: store) }
                case .result: ResultView(store: store)
                case .comparison: ComparisonView(store: store)
                case .history: HistoryView(store: store)
                case .module(let module): ModuleView(store: store, module: module)
                }
              }
            }.frame(maxWidth: 1150, alignment: .leading).padding(.horizontal, 32).padding(.vertical, 28)
              .frame(maxWidth: .infinity, alignment: .topLeading)
          }
          .onChange(of: store.screen) { _, _ in reader.scrollTo("top", anchor: .top) }
          .onChange(of: store.configuring) { _, _ in reader.scrollTo("top", anchor: .top) }
          .onChange(of: store.selectedRecordID) { _, _ in reader.scrollTo("top", anchor: .top) }
          .onChange(of: store.accessRequested) { _, _ in reader.scrollTo("access", anchor: .top) }
        }.background(.white)
        if let toast = store.toast ?? store.realRunError ?? store.persistenceError {
          HStack(spacing: 8) {
            Image(
              systemName: store.persistenceError == nil && store.realRunError == nil
                ? "checkmark.circle.fill" : "exclamationmark.circle.fill")
            Text(toast).fixedSize(horizontal: false, vertical: true)
            Spacer()
            Button {
              store.toast = nil
            } label: {
              Image(systemName: "xmark")
            }.buttonStyle(.plain)
          }
          .font(.system(size: 12))
          .padding(.horizontal, 24).padding(.vertical, 6)
          .background(Theme.canvas)
        }
      }
    }.font(Theme.bodyFont).foregroundStyle(Theme.ink).preferredColorScheme(.light)
      .sheet(isPresented: $store.showConfirmation) { ConfirmationView(store: store) }
      .sheet(isPresented: $store.showSettings) { SettingsView(store: store) }
      .sheet(isPresented: $store.showDemoControls) { DemoControlsView(store: store) }
      .alert("停止本次检测？", isPresented: $store.showStopConfirmation) {
        Button("继续检测", role: .cancel) {}
        Button("停止并保留结果", role: .destructive) { store.finish(stopped: true) }
      } message: {
        Text("已完成的项目会保存，未完成的项目不计为模型失败。")
      }
  }

  // MARK: 侧边栏

  private var sidebar: some View {
    VStack(alignment: .leading, spacing: 0) {
      HStack(spacing: 10) {
        Image(systemName: "cross.case.fill")
          .font(.system(size: 21))
          .foregroundStyle(.white)
          .frame(width: 34, height: 34)
          .background(Theme.accent, in: RoundedRectangle(cornerRadius: 8.5))
        VStack(alignment: .leading, spacing: 1.5) {
          Text("AgentCheck").font(.system(size: 15, weight: .semibold))
          Text("模型服务体检").font(.system(size: 10.5)).foregroundStyle(Theme.faint)
        }
      }
      .padding(.horizontal, 18).frame(height: 64)

      Button {
        store.editService()
      } label: {
        HStack(spacing: 9) {
          Image(systemName: "cpu")
            .font(.system(size: 13))
            .foregroundStyle(Theme.muted)
            .frame(width: 30, height: 30)
            .background(Theme.canvas, in: RoundedRectangle(cornerRadius: 7))
          VStack(alignment: .leading, spacing: 3) {
            Text(store.service?.model ?? "添加模型服务")
              .font(.system(size: 12, weight: .medium))
              .lineLimit(1)
            Text(store.service?.host ?? "尚未配置")
              .font(.system(size: 10.5)).foregroundStyle(Theme.faint)
              .lineLimit(1)
          }
          Spacer(minLength: 0)
          Image(systemName: "chevron.up.chevron.down")
            .font(.system(size: 8.5)).foregroundStyle(Theme.faint)
        }
        .padding(8)
        .background(.white, in: RoundedRectangle(cornerRadius: 10))
        .overlay(RoundedRectangle(cornerRadius: 10).stroke(Theme.line))
      }
      .buttonStyle(.plain)
      .padding(.horizontal, 12).padding(.bottom, 20)
      .disabled(store.running || store.connecting)
      .help("更换当前模型服务")

      navGroup("总览") {
        nav("首页", icon: "square.grid.2x2", screen: .home)
        nav("检测记录", icon: "clock.arrow.circlepath", screen: .history)
      }
      navGroup("检测结果") {
        nav("检测结果", icon: "chart.bar.doc.horizontal", screen: .result, retain: true)
        ForEach(CheckModule.allCases) { module in
          let target: Screen = module == .comparison ? .comparison : .module(module)
          nav(module.title, icon: module.symbol, screen: target, module: module)
        }
      }
      Spacer(minLength: 20)

      if store.running {
        Button {
          store.navigate(.home)
        } label: {
          HStack(spacing: 8) {
            ProgressView().controlSize(.small)
            Text("检测中 · \(Int((store.progress * 100).rounded()))%")
              .contentTransition(.numericText())
              .animation(.easeOut(duration: 0.42), value: store.progress)
            Spacer()
            Image(systemName: "arrow.up.right")
          }
          .font(.system(size: 11))
          .padding(10)
          .background(Theme.accent.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
        }
        .buttonStyle(.plain)
        .foregroundStyle(Theme.accent)
        .padding(.horizontal, 12).padding(.bottom, 10)
      }

      HStack {
        Spacer()
        IconButton(symbol: "gearshape", help: "设置") { store.showSettings = true }
      }
      .padding(.horizontal, 16)
      Label(store.localStatus, systemImage: "internaldrive")
        .font(.system(size: 10))
        .foregroundStyle(Theme.faint)
        .padding(.horizontal, 18).padding(.top, 10).padding(.bottom, 16)
    }
    .frame(width: 224)
    .frame(maxHeight: .infinity)
    .background(Theme.canvas)
  }

  private func navGroup(_ name: String, @ViewBuilder content: () -> some View) -> some View {
    VStack(alignment: .leading, spacing: 0) {
      Text(name)
        .font(.system(size: 10.5, weight: .semibold))
        .foregroundStyle(Theme.faint)
        .padding(.horizontal, 20).padding(.bottom, 7)
      content().padding(.bottom, 14)
    }
  }

  private func nav(
    _ name: String, icon: String, screen: Screen, module: CheckModule? = nil, retain: Bool = false
  ) -> some View {
    let selected = !store.configuring && store.screen == screen
    return Button {
      if let module {
        store.showModule(module)
      } else {
        store.navigate(screen, retainRecord: retain)
      }
    } label: {
      HStack(spacing: 10) {
        Image(systemName: icon)
          .font(.system(size: 12.5))
          .frame(width: 17)
          .foregroundStyle(selected ? Theme.accent : Theme.muted)
        Text(name)
          .font(.system(size: 12.5, weight: selected ? .semibold : .regular))
          .foregroundStyle(selected ? Theme.accent : Theme.ink)
          .lineLimit(1)
        Spacer(minLength: 0)
      }
      .padding(.horizontal, 10)
      .frame(height: 32)
      .background(
        selected ? Theme.accent.opacity(0.09) : .clear,
        in: RoundedRectangle(cornerRadius: 8)
      )
      .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
    .padding(.horizontal, 10)
    .accessibilityIdentifier("nav-\(module?.rawValue ?? name)")
    .disabled(store.connecting || (store.service == nil && screen != .home))
  }

  // MARK: 工具栏

  private var toolbar: some View {
    HStack(spacing: 12) {
      Text(title).font(.system(size: 14, weight: .semibold)).fixedSize()
      if showsRecord, let record = store.currentRecord {
        Rectangle().fill(Theme.line).frame(width: 1, height: 16)
        Menu {
          ForEach(store.records) { entry in
            Button(
              "\(entry.service.model) · \(entry.date.formatted(date: .numeric, time: .shortened))"
            ) {
              store.selectedRecordID = entry.id
              store.evidenceExpanded = false
              store.showOriginal = false
            }
          }
        } label: {
          HStack(spacing: 5) {
            Text(
              "\(record.date.formatted(date: .numeric, time: .shortened)) · \(record.service.model)"
            )
            .font(.system(size: 11.5)).lineLimit(1)
            Image(systemName: "chevron.up.chevron.down").font(.system(size: 8))
          }
          .foregroundStyle(Theme.muted)
        }
        .menuStyle(.borderlessButton)
        .fixedSize()
        .help("切换检测记录")
        if record.service != store.service {
          Pill(text: "历史配置", style: Outcome.limited.style, icon: "clock")
        }
      }
      Spacer(minLength: 8)
      if showsRecord, let record = store.currentRecord {
        IconButton(symbol: "square.and.arrow.up", help: "导出当前检测记录") {
          exportRecord(record, store: store)
        }
      }
      if !store.configuring && !store.running {
        Action(title: "新建检测", icon: "plus") { store.prepareRun() }.accessibilityIdentifier(
          "new-detection")
      }
    }
    .padding(.horizontal, 24)
    .frame(height: 56)
    .background(.white)
    .overlay(alignment: .bottom) { Rectangle().fill(Theme.line).frame(height: 1) }
  }
}

// 未检测模块的空状态：说明原因，并给出下一步。
struct EmptyModule: View {
  @ObservedObject var store: Workbench
  var module: CheckModule
  var record: RunRecord?
  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      Image(systemName: module.symbol)
        .font(.system(size: 28, weight: .light))
        .foregroundStyle(Theme.faint)
        .frame(width: 56, height: 56)
        .background(Theme.unknownTint, in: Circle())
      Text(title).font(.system(size: 21, weight: .semibold)).fixedSize(
        horizontal: false, vertical: true)
      Text(reason).font(.system(size: 13)).foregroundStyle(Theme.muted)
        .fixedSize(horizontal: false, vertical: true)
      Action(title: module.actionTitle, icon: "play.fill", disabled: store.running) {
        store.prepareRun(module: module)
      }
      if record?.service != nil && record?.service != store.service {
        Text("新检测将使用当前服务：\(store.service?.model ?? "")")
          .font(Theme.captionFont).foregroundStyle(Theme.faint)
      }
    }
    .padding(.vertical, 40)
    .frame(maxWidth: 520, alignment: .leading)
  }
  private var title: String {
    if let record, !record.hasCurrentEvidence { return "这条旧记录缺少完整证据" }
    if record?.stopped == true { return "检测已停止，这项没有完成" }
    if let record, !record.modules.contains(module) { return "本次没有检测这一项" }
    return "还没有\(module.title)结果"
  }
  private var reason: String {
    if record?.hasCurrentEvidence == false { return "原有摘要仍然保留，重新检测后可以查看完整结果与执行记录。" }
    return "\(module.subtitle)。尚未检测不代表模型不支持。"
  }
}

@MainActor
func exportRecord(_ record: RunRecord, store: Workbench) {
  let panel = NSSavePanel()
  panel.nameFieldStringValue = "agent-check-record-\(record.id.uuidString.prefix(8)).json"
  panel.title = "导出检测记录"
  guard panel.runModal() == .OK, let url = panel.url else { return }
  do {
    if let reportJSON = record.reportJSON {
      try Data(reportJSON.utf8).write(to: url, options: .atomic)
      store.toast = "已导出真实检测报告"
      return
    }
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    try encoder.encode(ReportExport(record: record)).write(to: url, options: .atomic)
    store.toast = "已导出检测记录"
  } catch { store.toast = "导出失败，请检查保存位置权限后重试。" }
}
