import AppKit
import SwiftUI

enum PrototypeError: Error { case failed(String) }

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
  var window: NSWindow!
  var store: Workbench!
  var automated = false

  func applicationDidFinishLaunching(_ notification: Notification) {
    let args = CommandLine.arguments
    automated = args.contains("--self-test") || args.contains("--capture")
    let launchConfiguration = LaunchConfiguration.from(arguments: args)
    let dataPath = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[
      0
    ].appendingPathComponent("AgentCheckPrototype/session.json")
    store = Workbench(
      storageURL: automated || args.contains("--review-all") ? nil : dataPath,
      launchConfiguration: launchConfiguration
    )
    window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 1180, height: 830),
      styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false
    )
    window.title = "AgentCheck — 模型服务体检"
    window.minSize = NSSize(width: 1000, height: 740)
    window.isReleasedWhenClosed = false
    window.delegate = self
    window.contentView = NSHostingView(rootView: WorkbenchView(store: store))
    window.center()
    installMenu()
    if automated {
      window.orderBack(nil)
    } else {
      window.makeKeyAndOrderFront(nil)
      NSApp.activate(ignoringOtherApps: true)
    }
    if args.contains("--review-all") {
      Task {
        store.fillExample()
        await store.connect()
        makeResult(.usable)
        store.records[0].date = Date().addingTimeInterval(-3600)
        makeResult(.limited)
        store.navigate(.home)
      }
    } else if args.contains("--self-test") {
      Task {
        do {
          try await runModelTests()
          NSApp.terminate(nil)
        } catch {
          fputs("FAIL: \(error)\n", stderr)
          exit(1)
        }
      }
    } else if let index = args.firstIndex(of: "--capture"), args.count > index + 1 {
      Task {
        do {
          try await captureScreens(URL(fileURLWithPath: args[index + 1], isDirectory: true))
          NSApp.terminate(nil)
        } catch {
          fputs("CAPTURE FAIL: \(error)\n", stderr)
          exit(1)
        }
      }
    } else if launchConfiguration != nil {
      Task {
        try? await Task.sleep(nanoseconds: 450_000_000)
        await store.connect()
      }
    }
  }
  func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
  func windowShouldClose(_ sender: NSWindow) -> Bool {
    guard store.running && !automated else { return true }
    let alert = NSAlert()
    alert.messageText = "停止检测并关闭窗口？"
    alert.informativeText = "已完成的演示项目会保存在历史记录中。"
    alert.addButton(withTitle: "继续检测")
    alert.addButton(withTitle: "停止并关闭")
    if alert.runModal() == .alertSecondButtonReturn {
      store.finish(stopped: true)
      return true
    }
    return false
  }
  func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
    windowShouldClose(window) ? .terminateNow : .terminateCancel
  }
  private func installMenu() {
    let menu = NSMenu()
    let appItem = NSMenuItem()
    let appMenu = NSMenu()
    appMenu.addItem(
      withTitle: "退出 AgentCheck", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q"
    )
    appItem.submenu = appMenu
    menu.addItem(appItem)
    let edit = NSMenuItem()
    let editMenu = NSMenu(title: "编辑")
    edit.title = "编辑"
    editMenu.addItem(withTitle: "撤销", action: Selector(("undo:")), keyEquivalent: "z")
    editMenu.addItem(withTitle: "剪切", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
    editMenu.addItem(withTitle: "复制", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
    editMenu.addItem(withTitle: "粘贴", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
    editMenu.addItem(withTitle: "全选", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
    edit.submenu = editMenu
    menu.addItem(edit)
    NSApp.mainMenu = menu
  }
  private func capture(_ name: String, folder: URL, bottom: Bool = false) async throws {
    try await Task.sleep(nanoseconds: 500_000_000)
    guard let view = (window.attachedSheet ?? window).contentView else {
      throw PrototypeError.failed("窗口内容缺失")
    }
    if let scroll = scrollView(in: view), let document = scroll.documentView {
      let end = max(0, document.bounds.height - scroll.contentSize.height)
      let y = document.isFlipped ? (bottom ? end : 0) : (bottom ? 0 : end)
      scroll.contentView.scroll(to: NSPoint(x: 0, y: y))
      scroll.reflectScrolledClipView(scroll.contentView)
      try await Task.sleep(nanoseconds: 200_000_000)
    }
    view.layoutSubtreeIfNeeded()
    // 整窗 cacheDisplay 会把超出视口的滚动内容二次合成（表现为内容重复）；
    // 超长页面退化为只截滚动视口，保证所见即所得；短页面保留整窗（含侧栏与工具栏）。
    var captureView: NSView = view
    if let scroll = scrollView(in: view), let document = scroll.documentView {
      if document.bounds.height > scroll.contentView.bounds.height + 1 {
        captureView = scroll.contentView
      }
    }
    let captureBounds = CGRect(origin: .zero, size: captureView.bounds.size)
    guard let bitmap = captureView.bitmapImageRepForCachingDisplay(in: captureBounds) else {
      throw PrototypeError.failed("无法创建截图")
    }
    captureView.cacheDisplay(in: captureBounds, to: bitmap)
    guard
      bitmap.pixelsWide > 500, bitmap.pixelsHigh > 400,
      let data = bitmap.representation(using: .png, properties: [:]),
      data.count > 50_000
    else { throw PrototypeError.failed("\(name) 截图疑似空白") }
    try data.write(to: folder.appendingPathComponent("\(name).png"))
    print("CAPTURE: \(name) \(bitmap.pixelsWide)x\(bitmap.pixelsHigh)")
  }
  private func scrollView(in view: NSView) -> NSScrollView? {
    if let scroll = view as? NSScrollView { return scroll }
    for child in view.subviews { if let scroll = scrollView(in: child) { return scroll } }
    return nil
  }
  private func makeResult(_ outcome: Outcome, mode: String = "standard") {
    store.outcome = outcome
    store.agentMode = mode
    store.prepareRun()
    store.selectedModules = Set(CheckModule.testModules)
    store.startRun(automatic: false)
    for _ in 0..<10 { store.advance() }
  }
  private func captureScreens(_ folder: URL) async throws {
    try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
    // 等待首帧合成完成，避免第一张窗口级截图空白。
    try await Task.sleep(nanoseconds: 1_200_000_000)
    try await capture("01-connection", folder: folder)
    store.fillExample()
    store.connectionScenario = "auth"
    await store.connect()
    try await capture("02-connection-error", folder: folder)
    store.fillExample()
    store.connectionScenario = "success"
    await store.connect()
    try await capture("03-home-empty", folder: folder)
    store.prepareRun()
    try await capture("04-confirmation-empty", folder: folder)
    store.selectedModules = Set(CheckModule.testModules)
    try await capture("05-confirmation-selected", folder: folder)
    store.startRun(automatic: false)
    for _ in 0..<5 { store.advance() }
    try await capture("06-running", folder: folder)
    for _ in 0..<5 { store.advance() }
    try await capture("07-result", folder: folder)
    store.navigate(.home)
    try await capture("08-home-complete", folder: folder)
    window.setContentSize(NSSize(width: 1000, height: 740))
    try await capture("compact-home-complete", folder: folder)
    window.setContentSize(NSSize(width: 1180, height: 830))
    store.showKeyEvidence()
    try await capture("overview-key-evidence", folder: folder)
    try await capture("overview-key-delivery", folder: folder, bottom: true)
    store.navigate(.home)
    store.accessExpanded = true
    try await capture("09-access-expanded", folder: folder)
    store.accessExpanded = false
    for module in CheckModule.testModules {
      store.showModule(module)
      try await capture("detail-\(module.rawValue)", folder: folder)
      window.setContentSize(NSSize(width: 1000, height: 740))
      try await capture("compact-\(module.rawValue)", folder: folder)
      window.setContentSize(NSSize(width: 1180, height: 830))
    }
    for (module, items) in [
      (CheckModule.parameters, Catalog.specifications), (.performance, Catalog.performance),
    ] {
      store.showModule(module)
      for item in items {
        store.catalogSelection = item.id
        store.evidenceExpanded = false
        try await capture("catalog-\(item.id)", folder: folder)
      }
      store.evidenceExpanded = true
      try await capture("evidence-\(module.rawValue)", folder: folder)
    }
    store.showModule(.functions)
    for category in Scores.all {
      store.catalogSelection = category.id
      try await capture("score-\(category.id)", folder: folder)
    }
    store.showModule(.agent)
    store.agentSelection = 4
    store.evidenceExpanded = true
    store.agentRun = 2
    try await capture("agent-initial-failure", folder: folder)
    store.agentRun = 4
    try await capture("agent-review-failure", folder: folder)
    try await capture("agent-review-delivery", folder: folder, bottom: true)
    store.agentSample = "T3b"
    store.agentRun = 0
    try await capture("agent-permanent-error", folder: folder)
    store.agentTab = "business"
    try await capture("agent-business-unverified", folder: folder)
    store.agentTab = "general"
    store.agentFilter = "unverified"
    try await capture("agent-empty-filter", folder: folder)
    store.agentFilter = "all"
    store.agentSample = ""
    store.showModule(.comparison)
    for item in BaselineItem.all {
      store.baselineSelection = item.id
      store.baselineRecord = 0
      store.showOriginal = false
      try await capture("baseline-\(item.id)", folder: folder)
    }
    for index in 1..<4 {
      store.baselineRecord = index
      try await capture("baseline-empty-\(index)", folder: folder)
    }
    store.baselineSelection = "BC04"
    store.baselineRecord = 1
    try await capture("baseline-second-response", folder: folder)
    store.baselineDifferencesOnly = true
    try await capture("baseline-no-differences", folder: folder)
    store.baselineRecord = 0
    try await capture("baseline-differences-only", folder: folder)
    store.showOriginal = true
    try await capture("baseline-original", folder: folder, bottom: true)
    store.showOriginal = false
    store.baselineDifferencesOnly = false
    store.baselineSelection = "BC13"
    store.baselineRecord = 1
    try await capture("baseline-usage-tail", folder: folder)
    store.showSettings = true
    try await capture("settings", folder: folder)
    store.showSettings = false
    try await Task.sleep(nanoseconds: 500_000_000)
    store.showDemoControls = true
    try await capture("demo-controls", folder: folder)
    store.showDemoControls = false
    try await Task.sleep(nanoseconds: 500_000_000)
    for outcome in [Outcome.usable, .blocked, .inconclusive] {
      makeResult(outcome)
      try await capture("result-\(outcome.rawValue)", folder: folder)
      store.showModule(.agent)
      store.agentSelection = outcome == .blocked ? 6 : 0
      store.evidenceExpanded = true
      try await capture("agent-\(outcome.rawValue)", folder: folder)
    }
    for mode in ["intermittent", "pending-review"] {
      makeResult(.limited, mode: mode)
      try await capture("result-\(mode)", folder: folder)
      store.showModule(.agent)
      store.agentSelection = 4
      store.evidenceExpanded = true
      try await capture("agent-\(mode)", folder: folder)
    }
    store.navigate(.history)
    store.historySelection = Set(store.records.prefix(2).map(\.id))
    try await capture("history", folder: folder)
    store.showHistoryComparison = true
    try await capture("history-comparison", folder: folder)
    store.showHistoryComparison = false
    try await Task.sleep(nanoseconds: 500_000_000)
    store.historySearch = "no-matching-model"
    try await capture("history-no-matches", folder: folder)
    store.historySearch = ""
    store.prepareRun(module: .parameters)
    store.startRun(automatic: false)
    store.advance()
    store.finish(stopped: true)
    try await capture("result-stopped", folder: folder)
    store.showModule(.parameters)
    try await capture("detail-stopped", folder: folder)
    store.records[0].presentationVersion = 2
    store.navigate(.result)
    try await capture("result-legacy", folder: folder)
    store.editService()
    store.draftModel = "customer-production-long-model-name-with-deployment-and-version-2026-09-08"
    store.draftKey = "demo-only"
    await store.connect()
    window.setContentSize(NSSize(width: 1000, height: 740))
    try await capture("long-model-name", folder: folder)
    store.navigate(.result)
    try await capture("result-empty", folder: folder)
    store.showModule(.agent)
    try await capture("agent-not-tested", folder: folder)
    makeResult(.limited)
    try await capture("long-model-result", folder: folder)
    store.showModule(.comparison)
    try await capture("long-model-baseline", folder: folder)
    store.records = []
    store.navigate(.history)
    try await capture("history-empty", folder: folder)
  }
}

MainActor.assumeIsolated {
  let app = NSApplication.shared
  app.setActivationPolicy(.regular)
  let delegate = AppDelegate()
  app.delegate = delegate
  app.run()
}
