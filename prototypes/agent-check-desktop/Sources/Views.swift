import AppKit
import SwiftUI

struct WorkbenchView: View {
  @ObservedObject var store: Workbench
  private var title: String {
    if store.configuring { return store.service == nil ? "添加模型服务" : "更换模型服务" }
    switch store.screen {
    case .home: return store.running ? "检测进行中" : "工作台"
    case .result: return "检测报告"
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

        }.background(.white)
          .sheet(item: $store.viewedRecord) { record in
            HistoryRecordViewer(store: store, record: record)
          }
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
      .padding(.horizontal, 12).padding(.bottom, 10)
      .disabled(store.running || store.connecting)
      .help("更换当前模型服务")

      Button {
        store.prepareRun()
      } label: {
        HStack(spacing: 6) {
          Image(systemName: "plus").font(.system(size: 11, weight: .bold))
          Text("新建检测")
        }
        .font(.system(size: 12.5, weight: .semibold))
        .foregroundStyle(.white)
        .frame(maxWidth: .infinity).frame(height: 34)
        .background(Theme.accent, in: RoundedRectangle(cornerRadius: 8))
        .shadow(color: Theme.accent.opacity(0.3), radius: 2, y: 1)
      }
      .buttonStyle(.plain)
      .padding(.horizontal, 12).padding(.bottom, 16)
      .disabled(store.running || store.connecting || store.service == nil)
      .accessibilityIdentifier("new-detection")

      VStack(alignment: .leading, spacing: 0) {
        navItem("工作台", icon: "square.grid.2x2", screen: .home)
        navItem(
          "检测记录", icon: "clock.arrow.circlepath", screen: .history,
          badge: store.records.isEmpty ? nil : store.records.count,
          disabled: store.records.isEmpty)
      }

      // 上次体检状态卡（检测运行时让位给进度面板）
      if store.running {
        runPanel
      } else if let latest = store.latestForService {
        Rectangle().fill(Theme.line).frame(height: 1)
          .padding(.horizontal, 14).padding(.top, 14).padding(.bottom, 14)
        lastCheckCard(latest)
      }
      Spacer(minLength: 20)


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

  // 上次体检状态卡：侧栏随时回答「当前服务现在什么状态」。
  // 只反映当前服务最近一次，不是历史列表；检测运行时让位给进度面板。
  private func lastCheckCard(_ record: RunRecord) -> some View {
    VStack(alignment: .leading, spacing: 0) {
      HStack {
        Text("上次体检").font(.system(size: 10.5, weight: .semibold)).foregroundStyle(Theme.faint)
        Spacer()
        Text(record.date.formatted(.dateTime.month().day().hour().minute()))
          .font(.system(size: 10.5)).foregroundStyle(Theme.faint)
      }
      .padding(.horizontal, 16).padding(.top, 15).padding(.bottom, 9)
      HStack(spacing: 7) {
        Image(systemName: verdictIcon(record.outcome))
          .font(.system(size: 9, weight: .bold)).foregroundStyle(.white)
          .frame(width: 16, height: 16)
          .background(outcomeSolidColor(record.outcome), in: Circle())
        Text(record.title)
          .font(.system(size: 12, weight: .semibold))
          .foregroundStyle(outcomeSolidColor(record.outcome))
          .lineLimit(1)
      }
      .padding(.horizontal, 16).padding(.bottom, 12)
      VStack(spacing: 9) {
        ForEach(CheckModule.testModules) { module in
          moduleStatusRow(record, module: module)
        }
      }
      .padding(.horizontal, 16).padding(.bottom, 12)
      Button {
        store.showCurrentReport()
      } label: {
        HStack(spacing: 4) {
          Text("查看当前报告")
          Image(systemName: "arrow.right").font(.system(size: 9, weight: .semibold))
        }
        .font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.accent)
      }
      .buttonStyle(.plain)
      .padding(.horizontal, 16).padding(.bottom, 15)
    }
    .background(.white, in: RoundedRectangle(cornerRadius: 11))
    .overlay(RoundedRectangle(cornerRadius: 11).stroke(Theme.line))
    .padding(.horizontal, 12)
  }

  private func moduleStatusRow(_ record: RunRecord, module: CheckModule) -> some View {
    let dot = record.navDot(module)
    return HStack(spacing: 8) {
      navDotView(dot, dark: false, size: 7)
      Text(module.shortTitle)
        .font(.system(size: 11.5)).foregroundStyle(Theme.muted)
      Spacer()
      Text(dotStatusWord(dot))
        .font(.system(size: 10.5)).foregroundStyle(Theme.faint)
    }
    .padding(.vertical, 1)
  }

  // 运行中：进度面板占住状态卡的位置，点它回工作台看详情。
  private var runPanel: some View {
    Button {
      store.navigate(.home)
    } label: {
      VStack(alignment: .leading, spacing: 7) {
        HStack {
          Text(store.progressMessage.isEmpty ? "检测中" : store.progressMessage)
            .font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.accent)
            .lineLimit(1)
          Spacer()
          Text("\(Int((store.progress * 100).rounded()))%")
            .font(.system(size: 11, weight: .semibold)).foregroundStyle(Theme.accent)
            .monospacedDigit()
        }
        GeometryReader { proxy in
          ZStack(alignment: .leading) {
            Capsule().fill(Theme.accent.opacity(0.14))
            Capsule().fill(Theme.accent)
              .frame(width: max(4, proxy.size.width * store.progress))
          }
        }
        .frame(height: 4)
      }
      .padding(11)
      .background(Theme.accent.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
      .animation(.easeOut(duration: 0.42), value: store.progress)
    }
    .buttonStyle(.plain)
    .padding(.horizontal, 12).padding(.top, 12)
  }

  private func verdictIcon(_ outcome: Outcome) -> String {
    switch outcome {
    case .usable: return "checkmark"
    case .limited: return "exclamationmark"
    case .blocked: return "xmark"
    case .inconclusive: return "questionmark"
    }
  }
  private func outcomeSolidColor(_ outcome: Outcome) -> Color {
    switch outcome {
    case .usable: return Theme.pass
    case .limited: return Theme.limited
    case .blocked: return Theme.blocked
    case .inconclusive: return Theme.unknown
    }
  }
  private func dotStatusWord(_ dot: NavDot) -> String {
    switch dot {
    case .pass: return "已完成"
    case .warn: return "有限制"
    case .fail: return "未通过"
    case .info: return "接入"
    case .none: return "未检测"
    }
  }

  private func navItem(
    _ name: String, icon: String, screen: Screen, badge: Int? = nil, note: String? = nil,
    disabled: Bool = false, retain: Bool = false
  ) -> some View {
    let selected = !store.configuring && store.screen == screen
    return HStack(spacing: 0) {
      RoundedRectangle(cornerRadius: 1.5)
        .fill(selected ? Theme.accent : .clear)
        .frame(width: 3, height: 18)
      Button {
        store.navigate(screen, retainRecord: retain)
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
          if let note {
            Text(note).font(.system(size: 10.5)).foregroundStyle(Theme.faint)
          }
          if let badge {
            Text("\(badge)")
              .font(.system(size: 10, weight: .semibold))
              .foregroundStyle(Theme.muted)
              .padding(.horizontal, 5).frame(height: 16).frame(minWidth: 18)
              .background(Theme.line.opacity(0.7), in: Capsule())
          }
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
      .accessibilityIdentifier("nav-\(name)")
      .disabled(store.connecting || disabled || (store.service == nil && screen != .home))
      .opacity(store.connecting || disabled || (store.service == nil && screen != .home) ? 0.45 : 1)
    }
    .padding(.leading, 7).padding(.trailing, 10)
  }

  // MARK: 工具栏（面包屑 + 页面动作）

  private var toolbar: some View {
    HStack(spacing: 12) {
      // 配置页没有路径概念，直接给标题；其余页面用面包屑标出「我在哪、怎么回去」。
      if store.configuring {
        Text(title).font(.system(size: 14, weight: .semibold))
      } else {
        crumbs
        if showsRecord, let record = store.currentRecord {
          recordChip(record)
        }
      }
      Spacer(minLength: 8)
      if !store.configuring {
        toolbarActions
      }
    }
    .padding(.horizontal, 24)
    .frame(height: 56)
    .background(.white)
    .overlay(alignment: .bottom) { Rectangle().fill(Theme.line).frame(height: 1) }
  }

  // 面包屑：工作台 › 检测报告 › 模块。侧栏没有报告入口后，模块页靠它回退。
  @ViewBuilder private var crumbs: some View {
    HStack(spacing: 8) {
      if store.screen == .home {
        crumbCurrent("工作台")
      } else {
        crumbLink("工作台") { store.navigate(.home) }
        crumbSeparator
        switch store.screen {
        case .history:
          crumbCurrent("检测记录")
        case .result:
          crumbCurrent("检测报告")
        case .comparison:
          crumbLink("检测报告") { store.showCurrentReport() }
          crumbSeparator
          crumbCurrent(CheckModule.comparison.title)
        case .module(let module):
          crumbLink("检测报告") { store.showCurrentReport() }
          crumbSeparator
          crumbCurrent(module.title)
        case .home:
          EmptyView()
        }
      }
    }
  }

  private func crumbLink(_ text: String, action: @escaping () -> Void) -> some View {
    Button(action: action) {
      Text(text).font(.system(size: 13)).foregroundStyle(Theme.muted)
    }
    .buttonStyle(.plain)
    .help("回到\(text)")
  }

  private func crumbCurrent(_ text: String) -> some View {
    Text(text).font(.system(size: 14, weight: .semibold)).foregroundStyle(Theme.ink)
  }

  private var crumbSeparator: some View {
    Text("›").font(.system(size: 11, weight: .semibold)).foregroundStyle(Color(white: 0.76))
  }

  // 只读的记录标识：说明正在看哪次检测。切换记录走「检测记录」页打开，
  // 不在报告页顶部悄悄换掉整份内容。
  private func recordChip(_ record: RunRecord) -> some View {
    HStack(spacing: 7) {
      Circle().fill(outcomeSolidColor(record.outcome)).frame(width: 8, height: 8)
      Text(
        "\(record.service.model) · \(record.date.formatted(date: .numeric, time: .shortened))"
      )
      .font(.system(size: 11.5)).foregroundStyle(Theme.muted).lineLimit(1)
    }
    .padding(.horizontal, 11).padding(.vertical, 5)
    .background(.white, in: RoundedRectangle(cornerRadius: 8))
    .overlay(RoundedRectangle(cornerRadius: 8).stroke(Theme.line))
    .fixedSize()
    .help("正在查看这次检测；换记录请到「检测记录」页打开")
  }

  // 每页的动作组：工具栏同时是「这页能干什么」的清单。
  @ViewBuilder private var toolbarActions: some View {
    switch store.screen {
    case .history:
      toolbarButton("新建检测", icon: "plus", style: .primary, disabled: store.running || store.service == nil) {
        store.prepareRun()
      }
    case .result:
      if let record = store.currentRecord {
        // 中途停止且还是当前服务：给补测入口，和报告页主行动一致
        let resumable = record.stopped && record.service == store.service
        toolbarButton(
          resumable ? "补测未完成项" : "重新检测",
          icon: resumable ? "play.fill" : "arrow.clockwise",
          style: .ghost, disabled: store.running
        ) {
          if resumable {
            store.selectedRecordID = record.id
            store.prepareMissing()
          } else {
            store.prepareRun()
          }
        }
        toolbarButton("导出报告", icon: "square.and.arrow.up", style: .accent) {
          exportRecord(record, store: store)
        }
      }
    case .module(let module):
      if let record = store.currentRecord {
        toolbarButton("单独复测", icon: "arrow.clockwise", style: .ghost, disabled: store.running) {
          store.prepareRun(module: module)
        }
        toolbarButton("导出报告", icon: "square.and.arrow.up", style: .accent) {
          exportRecord(record, store: store)
        }
      }
    case .comparison:
      if let record = store.currentRecord {
        toolbarButton("导出报告", icon: "square.and.arrow.up", style: .accent) {
          exportRecord(record, store: store)
        }
      }
    case .home:
      EmptyView()
    }
  }

  private enum ToolbarButtonStyle { case primary, ghost, accent }

  private func toolbarButton(
    _ title: String, icon: String, style: ToolbarButtonStyle, disabled: Bool = false,
    action: @escaping () -> Void
  ) -> some View {
    Button(action: action) {
      HStack(spacing: 5) {
        Image(systemName: icon).font(.system(size: 10.5, weight: .semibold))
        Text(title).font(.system(size: 12, weight: .medium))
      }
      .padding(.horizontal, 13).frame(height: 30)
      .background(
        style == .primary ? Theme.accent : style == .accent ? Theme.infoTint : .white,
        in: RoundedRectangle(cornerRadius: 8))
      .overlay(
        RoundedRectangle(cornerRadius: 8)
          .stroke(
            style == .primary ? Theme.accent : style == .accent ? Theme.accent.opacity(0.35) : Theme.line))
      .foregroundStyle(
        style == .primary ? .white : style == .accent ? Theme.accent : Theme.muted)
    }
    .buttonStyle(.plain)
    .disabled(disabled)
    .opacity(disabled ? 0.45 : 1)
    .help(title)
  }
}

// 状态点：空心圆=未测/未选，实心=对应状态色。深色底用 darkColor。
func navDotView(_ dot: NavDot, dark: Bool, size: CGFloat = 8) -> some View {
  Group {
    if dot == .none {
      Circle().stroke(
        dark ? Color.white.opacity(0.28) : Color(red: 0.765, green: 0.792, blue: 0.827),
        lineWidth: 1.5)
    } else {
      Circle().fill(dark ? dot.darkColor : dot.color)
    }
  }
  .frame(width: size, height: size)
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
  panel.title = "导出检测报告"

  // HTML 是主交付物（单文件、双击可开）；JSON 留给调试和对接。
  let format = NSPopUpButton(frame: NSRect(x: 0, y: 0, width: 240, height: 26))
  format.addItems(withTitles: ["HTML 报告（推荐）", "JSON 数据"])
  let accessory = NSView(frame: NSRect(x: 0, y: 0, width: 260, height: 30))
  accessory.addSubview(format)
  panel.accessoryView = accessory

  let stampFormatter = DateFormatter()
  stampFormatter.dateFormat = "yyyyMMdd-HHmm"
  let stamp = stampFormatter.string(from: record.date)
  let base = "agentcheck-\(exportSafe(record.service.model))"
  func defaultName(_ ext: String) -> String { "\(base)-\(stamp).\(ext)" }
  panel.nameFieldStringValue = defaultName("html")

  let wantsHTML = { format.indexOfSelectedItem == 0 }
  // 切格式时同步文件名后缀
  let nameUpdater = ExportNameUpdater { panel.nameFieldStringValue = defaultName(wantsHTML() ? "html" : "json") }
  format.target = nameUpdater
  format.action = #selector(ExportNameUpdater.pick)

  guard panel.runModal() == .OK, var url = panel.url else { return }
  let ext = wantsHTML() ? "html" : "json"
  if url.pathExtension.lowercased() != ext {
    url = url.deletingPathExtension().appendingPathExtension(ext)
  }
  do {
    if wantsHTML() {
      try Data(renderReportHTML(record: record).utf8).write(to: url, options: .atomic)
      store.toast = "已导出 HTML 报告（单文件，双击即可打开）"
    } else if let reportJSON = record.reportJSON {
      try Data(reportJSON.utf8).write(to: url, options: .atomic)
      store.toast = "已导出真实检测报告 JSON"
    } else {
      let encoder = JSONEncoder()
      encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
      try encoder.encode(ReportExport(record: record)).write(to: url, options: .atomic)
      store.toast = "已导出检测记录 JSON"
    }
  } catch { store.toast = "导出失败，请检查保存位置权限后重试。" }
}

private func exportSafe(_ model: String) -> String {
  String(
    model.map { character in
      character.isLetter || character.isNumber || character == "-" || character == "."
        ? character : "-"
    })
}

// 保存面板的 target-action 桥：切导出格式时同步默认文件名后缀。
private final class ExportNameUpdater: NSObject {
  private let apply: () -> Void
  init(apply: @escaping () -> Void) { self.apply = apply }
  @objc func pick() { apply() }
}
