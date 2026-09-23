import Combine
import Foundation

enum CheckModule: String, CaseIterable, Codable, Identifiable {
  case info, parameters, functions, performance, agent, comparison
  static var testModules: [CheckModule] { allCases.filter { $0 != .info } }
  var id: String { rawValue }
  var title: String {
    switch self {
    case .info: return "模型接入信息"
    case .parameters: return "模型规格实测"
    case .functions: return "模型能力跑分"
    case .performance: return "模型性能实测"
    case .agent: return "智能体实测"
    case .comparison: return "模型基线对比"
    }
  }
  var subtitle: String {
    switch self {
    case .info: return "确认这次测的是哪套服务"
    case .parameters: return "实际支持什么，支持到哪里"
    case .functions: return "各类任务的强项与短板"
    case .performance: return "等待、并发与持续运行表现"
    case .agent: return "能否遵守规则并完成连续任务"
    case .comparison: return "实际返回与参考结构差在哪里"
    }
  }
  // 这个类别到底测了哪些东西——用于结果目录行，主题词 + 数字 + 清单，一行立住与其他类别的分界。
  var scope: String {
    switch self {
    case .info:
      return "本次接入的服务、接口与响应来源信息"
    case .parameters:
      return "五项基础规格实测——上下文容量、工具调用、结构化输出、多轮消息、流式响应"
    case .functions:
      return "六类模型能力逐项得分——文本理解、信息提取、工具选择、多轮承接、长材料利用、逻辑计算"
    case .performance:
      return "五维性能实测——首字等待、生成速度、并发承载、持续运行、长文本负载"
    case .agent:
      return "八项任务行为实测——规则遵循、工具运用、错误恢复、权限边界、真实交付"
    case .comparison:
      return "十四维结构对照——返回数据结构与官方规范示例逐项比对，只比结构、不比内容"
    }
  }
  var symbol: String {
    switch self {
    case .info: return "cpu"
    case .parameters: return "slider.horizontal.3"
    case .functions: return "wrench.and.screwdriver"
    case .performance: return "speedometer"
    case .agent: return "terminal"
    case .comparison: return "arrow.left.arrow.right"
    }
  }
  // 侧栏大纲与首页清单使用的短名：上下文已经表明是模型检测，不再重复「模型」前缀。
  var shortTitle: String {
    switch self {
    case .info: return "接入信息"
    case .parameters: return "规格实测"
    case .functions: return "能力跑分"
    case .performance: return "性能实测"
    case .agent: return "智能体实测"
    case .comparison: return "基线对比"
    }
  }
  // 首页体检清单用的一行内容简介（比 scope 短，不重复模块名）。
  var coverDesc: String {
    switch self {
    case .info: return "本次接入的服务、接口与响应来源"
    case .parameters: return "上下文容量、工具调用、结构化输出、多轮消息、流式响应"
    case .functions: return "文本理解、信息提取、工具选择、多轮承接、长材料、逻辑计算"
    case .performance: return "首字等待、生成速度、并发承载、持续运行、长文本负载"
    case .agent: return "规则遵循、工具运用、错误恢复、权限边界、真实交付"
    case .comparison: return "返回数据结构与官方规范示例逐项对照"
    }
  }
}

enum Outcome: String, CaseIterable, Codable, Identifiable {
  case usable, limited, blocked, inconclusive
  var id: String { rawValue }
  var title: String {
    switch self {
    case .usable: return "可以正常使用"
    case .limited: return "可以使用，但有使用限制"
    case .blocked: return "目前不能正常使用"
    case .inconclusive: return "证据不足，暂不能判断"
    }
  }
}

struct Service: Codable, Equatable {
  var url: String
  var model: String
  var host: String { URL(string: url)?.host ?? url }
  var displayURL: String {
    guard var parts = URLComponents(string: url) else { return "地址无法解析" }
    parts.user = nil
    parts.password = nil
    parts.query = nil
    parts.fragment = nil
    return parts.string ?? host
  }
}

struct TestContext: Codable, Equatable {
  var rules = "产品设计 v0.6 · 演示规则 3"
  var samples = "演示样本集 3"
  var platform = "受控任务约束 · 演示"
  var environment = "固定隔离工作区 · 演示环境 1"
  var parameters = "固定演示参数组 A"
  var baseline = "ab3dcfbbad92dba6c0a3e6e1e6b83e7b32e4d3b52a87ea868ee2835c069ad147"
}

struct RunRecord: Codable, Identifiable {
  var id = UUID()
  var date = Date()
  var service: Service
  var modules: [CheckModule]
  var completed: [CheckModule]
  var outcome: Outcome
  var stopped: Bool
  var presentationVersion: Int? = nil
  var context: TestContext? = nil
  var agentMode: String? = nil
  var responseModel: String? = nil
  var reportJSON: String? = nil
  var moduleStates: [String: String]? = nil
  var hasCurrentEvidence: Bool { presentationVersion == 3 || reportJSON != nil }
  var hasConfirmedBlocker: Bool {
    hasCurrentEvidence && completed.contains(.agent) && outcome == .blocked
  }
  var title: String {
    if !hasCurrentEvidence { return "旧版结果，需重新检查" }
    if hasConfirmedBlocker { return Outcome.blocked.title }
    if stopped || !completed.contains(.agent) { return Outcome.inconclusive.title }
    return outcome.title
  }
  var scope: String { "本次选择 \(modules.filter { $0 != .info }.count) 个检测模块" }
}

struct LocalSnapshot: Codable {
  var service: Service?
  var records: [RunRecord]
}

struct ProgressItem: Identifiable {
  let id: String
  let name: String
  var completed: Int
  let total: Int
  var state: String
}

enum Screen: Equatable {
  case home, comparison, history, result
  case module(CheckModule)
}

@MainActor
final class Workbench: ObservableObject {
  @Published var service: Service?
  @Published var records: [RunRecord] = []
  @Published var screen: Screen = .home
  @Published var configuring = true
  @Published var draftURL = ""
  @Published var draftModel = ""
  @Published var draftKey = ""
  @Published var connectionScenario = "success"
  @Published var connectionError: String?
  @Published var connecting = false
  @Published var showSettings = false
  @Published var showDemoControls = false
  @Published var showConfirmation = false
  @Published var showStopConfirmation = false
  @Published var selectedModules: Set<CheckModule> = []
  @Published var budgetSeconds = 60
  @Published var outcome: Outcome = .limited
  @Published var agentMode = "standard"
  @Published var responseNameDiff = false
  @Published var catalogSelection = ""
  @Published var catalogFilter = "all"
  @Published var running = false
  @Published var elapsed = 0
  @Published var completed: [CheckModule] = []
  @Published var activeModules: [CheckModule] = []
  @Published var moduleStates: [String: String] = [:]
  @Published var progressMessage = ""
  @Published var detailIndex = 0
  @Published var detailTotal: Int?
  @Published var detailID: String?
  @Published var progressItems: [ProgressItem] = []
  @Published var moduleProgressSnapshots: [String: [ProgressItem]] = [:]
  @Published var expandedModuleIDs: Set<String> = []
  @Published var realRunError: String?
  @Published var selectedRecordID: UUID?
  // 历史回看的只读查看器：打开它不改写工作台当前记录。
  @Published var viewedRecord: RunRecord?
  @Published var toast: String?
  @Published var persistenceError: String?
  @Published var evidenceExpanded = false
  @Published var agentSelection = 4
  @Published var agentFilter = "all"
  @Published var agentRun = 0
  @Published var agentSample = ""
  @Published var agentTab = "general"
  @Published var baselineSelection = "BC04"
  @Published var baselineDifferencesOnly = false
  @Published var baselineRecord = 0
  @Published var baselinePhase = 0
  @Published var baselineEmpty = "normal"
  @Published var baselineMark = -1
  @Published var showOriginal = false
  @Published var historyFilter = "all"
  @Published var historySearch = ""
  @Published var historySelection: Set<UUID> = []
  @Published var showHistoryComparison = false
  private var task: Task<Void, Never>?
  private let storageURL: URL?
  private var runningService: Service?
  private var runningOutcome: Outcome = .limited
  private var runningMode = "standard"
  private var runningResponseDiff = false
  private let launchConfiguration: LaunchConfiguration?
  private var realProcess: Process?
  private var progressTask: Task<Void, Never>?
  private var realRunDirectory: URL?
  private var realOutputURL: URL?
  private var realProgressURL: URL?
  private var realFinalizing = false
  private var sessionKey: String?

  static func uiModule(_ backendID: String) -> CheckModule? {
    switch backendID {
    case "specification": return .parameters
    case "capability": return .functions
    case "performance": return .performance
    case "agent": return .agent
    case "baseline": return .comparison
    default: return nil
    }
  }

  /// 报告目录名按启动时间生成，多次检测互不覆盖。
  static func reportDirectoryName(_ date: Date = Date()) -> String {
    let formatter = DateFormatter()
    formatter.dateFormat = "yyyyMMdd-HHmmss"
    return formatter.string(from: date)
  }

  static func backendModule(_ module: CheckModule) -> String? {
    switch module {
    case .info: return nil
    case .parameters: return "specification"
    case .functions: return "capability"
    case .performance: return "performance"
    case .agent: return "agent"
    case .comparison: return "baseline"
    }
  }

  init(storageURL: URL? = nil, launchConfiguration: LaunchConfiguration? = nil) {
    self.storageURL = storageURL
    self.launchConfiguration = launchConfiguration
    if let launchConfiguration {
      draftURL = launchConfiguration.endpoint
      draftModel = launchConfiguration.model
      selectedModules = Set(
        launchConfiguration.modules?.compactMap(CheckModule.fromBackend) ?? CheckModule.testModules)
    }
    if let url = storageURL, let data = try? Data(contentsOf: url) {
      do {
        let snapshot = try JSONDecoder().decode(LocalSnapshot.self, from: data)
        service = snapshot.service
        records = snapshot.records
        configuring = service == nil
      } catch {
        persistenceError = "本地记录读取失败，原文件未修改。"
      }
    }
  }

  var latestForService: RunRecord? { records.first { $0.service == service } }
  var currentRecord: RunRecord? {
    if let id = selectedRecordID { return records.first { $0.id == id } }
    return latestForService
  }
  var visibleRecords: [RunRecord] {
    records.filter {
      (historyFilter != "current" || $0.service == service)
        && (historySearch.isEmpty
          || $0.service.model.localizedCaseInsensitiveContains(historySearch)
          || $0.service.host.localizedCaseInsensitiveContains(historySearch))
    }
  }
  var comparedRecords: [RunRecord] { records.filter { historySelection.contains($0.id) } }
  var comparisonBlocker: String? {
    let pair = comparedRecords
    guard pair.count == 2 else { return "选择两次记录后才能比较。" }
    guard pair.allSatisfy({ !$0.stopped && $0.hasCurrentEvidence }) else {
      return "仅能比较完成的同版本演示记录；中止或旧版记录可单独查看。"
    }
    guard Set(pair[0].modules) == Set(pair[1].modules) else { return "两次检查范围不同，不能直接比较结论。" }
    guard let left = pair[0].context, let right = pair[1].context else { return "缺少测试条件，暂不能比较。" }
    guard left == right else { return "样本、规则、平台、参数、运行环境或基线不同，暂不能直接比较。" }
    return nil
  }
  func navigate(_ destination: Screen, retainRecord: Bool = false) {
    guard !connecting else { return }
    if !retainRecord { selectedRecordID = nil }
    evidenceExpanded = false
    screen = destination
    if service != nil { cancelConfiguration() }
  }
  func showModule(_ module: CheckModule) {
    if module == .info {
      // 接入信息不是检测项目：入口已移除，兜底回到报告页（报告抬头即接入事实）
      cancelConfiguration()
      catalogSelection = ""
      screen = currentRecord != nil ? .result : .home
      return
    }
    catalogSelection = ""
    catalogFilter = "all"
    if module == .agent {
      agentFilter = "all"
      agentTab = "general"
      agentSample = ""
      agentRun = 0
      let findings = currentRecord?.agentFindings ?? []
      agentSelection =
        findings.first { $0.state == .fail }?.id ?? findings.first { $0.state != .pass }?.id ?? 0
    }
    if module == .comparison {
      baselineSelection = "BC04"
      baselineRecord = 0
      baselineMark = -1
      showOriginal = false
    }
    navigate(module == .comparison ? .comparison : .module(module), retainRecord: true)
  }
  func toggleHistory(_ id: UUID) {
    if historySelection.contains(id) {
      historySelection.remove(id)
    } else if historySelection.count < 2 {
      historySelection.insert(id)
    }
  }
  var progress: Double {
    guard !activeModules.isEmpty else { return 0 }
    let completedFraction = Double(completed.count) / Double(activeModules.count)
    guard completed.count < activeModules.count,
      let detailTotal,
      detailTotal > 0
    else { return min(completedFraction, 1) }
    let detailFraction = Double(detailIndex) / Double(detailTotal)
    return min((Double(completed.count) + detailFraction) / Double(activeModules.count), 1)
  }
  var activeModule: CheckModule? {
    guard running, completed.count < activeModules.count else { return nil }
    return activeModules[completed.count]
  }
  var currentItemName: String {
    guard let detailID else { return "" }
    if let exact = progressItems.first(where: { $0.id == detailID }) { return exact.name }
    let prefix = detailID.split(separator: "-").first.map(String.init) ?? detailID
    return progressItems.first(where: { $0.id == prefix })?.name ?? detailID
  }
  // 大项展开即小项：当前模块直接用实时小项；其余模块用完成快照或等待模板。
  func moduleItems(_ module: CheckModule) -> [ProgressItem] {
    if module == activeModule && !progressItems.isEmpty { return progressItems }
    guard let backendID = module.backendID else { return [] }
    return moduleProgressSnapshots[backendID] ?? Self.progressItems(for: backendID)
  }
  func isModuleExpanded(_ module: CheckModule) -> Bool {
    if module == activeModule { return true }
    guard let backendID = module.backendID else { return false }
    return expandedModuleIDs.contains(backendID)
  }
  func toggleModuleExpanded(_ module: CheckModule) {
    guard let backendID = module.backendID else { return }
    if expandedModuleIDs.contains(backendID) {
      expandedModuleIDs.remove(backendID)
    } else {
      expandedModuleIDs.insert(backendID)
    }
  }
  var currentItemIndex: Int { detailIndex }
  var currentItemTotal: Int { detailTotal ?? 0 }
  var localStatus: String {
    persistenceError ?? (storageURL == nil ? "临时演示会话" : "记录保存在本机")
  }
  var isRealMode: Bool { launchConfiguration != nil }

  static func validationError(url: String, model: String, key: String) -> String? {
    guard let parts = URLComponents(string: url.trimmingCharacters(in: .whitespacesAndNewlines)),
      ["https", "http"].contains(parts.scheme?.lowercased() ?? ""),
      let host = parts.host, !host.isEmpty,
      parts.user == nil, parts.password == nil, parts.query == nil, parts.fragment == nil
    else {
      return "请输入 http:// 或 https:// 开头的服务地址，不要在地址中放入凭据或查询参数。"
    }
    guard !model.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return "请输入模型名称。" }
    guard !key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
      return "请输入 API Key；体验原型可使用示例凭据。"
    }
    return nil
  }

  func fillExample() {
    draftURL = "https://gateway.example.invalid/v1"
    draftModel = "agent-prod"
    draftKey = "demo-key-not-a-real-secret"
    connectionError = nil
  }

  func editService() {
    guard !running else { return }
    draftURL = service?.url ?? ""
    draftModel = service?.model ?? ""
    draftKey = ""
    sessionKey = nil
    connectionError = nil
    configuring = true
    screen = .home
  }

  func cancelConfiguration() {
    guard !connecting, service != nil else { return }
    draftKey = ""
    connectionError = nil
    if launchConfiguration == nil { sessionKey = nil }
    configuring = false
  }

  func connect() async {
    guard !connecting, !running else { return }
    let effectiveKey = draftKey.isEmpty
      ? (ProcessInfo.processInfo.environment["MODEL_API_KEY"] ?? "")
      : draftKey
    connectionError = Self.validationError(url: draftURL, model: draftModel, key: effectiveKey)
    guard connectionError == nil else { return }
    sessionKey = effectiveKey
    connecting = true
    let candidate = Service(
      url: draftURL.trimmingCharacters(in: .whitespacesAndNewlines),
      model: draftModel.trimmingCharacters(in: .whitespacesAndNewlines))
    let scenario = connectionScenario
    // The key is inherited by the real CLI child and is never persisted by the GUI.
    draftKey = ""
    if launchConfiguration == nil {
      try? await Task.sleep(nanoseconds: 650_000_000)
    }
    connecting = false
    if scenario == "auth" {
      connectionError = "API Key 未通过验证。请核对密钥与这套服务是否匹配，再重新输入。"
      return
    }
    if scenario == "timeout" {
      connectionError = "连接超时。请核对服务地址及网络访问条件后重试，模型能力尚未检测。"
      return
    }
    service = candidate
    configuring = false
    screen = .home
    selectedRecordID = nil
    if selectedModules.isEmpty, launchConfiguration != nil {
      selectedModules = Set(CheckModule.testModules)
    }
    persist()
    if launchConfiguration != nil {
      startRun(automatic: false)
    }
  }

  func prepareRun(module: CheckModule? = nil) {
    guard service != nil, !running else { return }
    if module == .info {
      showModule(.info)
      return
    }
    selectedModules = module.map { [$0] } ?? []
    showConfirmation = true
  }
  func prepareMissing() {
    guard let record = currentRecord, record.service == service, !running else { return }
    selectedModules = Set(record.modules).subtracting(record.completed).subtracting([.info])
    if selectedModules.isEmpty { selectedModules = [.agent] }
    showConfirmation = true
  }

  func startRun(automatic: Bool = true) {
    let runnable = selectedModules.subtracting([.info])
    guard let service, !runnable.isEmpty, !running else { return }
    task?.cancel()
    activeModules = CheckModule.testModules.filter { runnable.contains($0) }
    completed = []
    elapsed = 0
    moduleStates = [:]
    progressMessage = ""
    detailIndex = 0
    detailTotal = nil
    detailID = nil
    progressItems = []
    moduleProgressSnapshots = [:]
    expandedModuleIDs = []
    runningService = service
    runningMode = outcome == .limited ? agentMode : "standard"
    runningOutcome = runningMode == "pending-review" ? .inconclusive : outcome
    runningResponseDiff = responseNameDiff
    running = true
    selectedRecordID = nil
    showConfirmation = false
    screen = .home
    if launchConfiguration != nil {
      startRealRun(service: service, modules: activeModules)
      return
    }
    if automatic {
      task = Task { [weak self] in
        while !Task.isCancelled {
          do { try await Task.sleep(nanoseconds: 1_000_000_000) } catch { return }
          guard let self, self.running else { return }
          self.advance()
        }
      }
    }
  }

  func advance() {
    guard running else { return }
    elapsed += 1
    completed = Array(activeModules.prefix(min(elapsed / 2, activeModules.count)))
    if completed.count == activeModules.count {
      finish(stopped: false)
    } else if elapsed >= budgetSeconds {
      finish(stopped: true)
    }
  }

  func finish(stopped: Bool) {
    if launchConfiguration != nil && realProcess != nil {
      if stopped { realProcess?.terminate() }
      Task { @MainActor [weak self] in
        await self?.finishRealRun(stopped: stopped)
      }
      return
    }
    guard running, let service = runningService else { return }
    task?.cancel()
    task = nil
    running = false
    showStopConfirmation = false
    let record = RunRecord(
      service: service, modules: activeModules, completed: completed, outcome: runningOutcome,
      stopped: stopped, presentationVersion: 3, context: TestContext(), agentMode: runningMode,
      responseModel: completed.isEmpty
        ? nil : (runningResponseDiff ? "service-deployment-alias" : service.model))
    records.insert(record, at: 0)
    selectedRecordID = record.id
    screen = .result
    persist()
  }

  private func startRealRun(service: Service, modules: [CheckModule]) {
    guard let launchConfiguration else { return }
    realRunError = nil
    progressMessage = "正在准备检测"
    let directory = FileManager.default.temporaryDirectory
      .appendingPathComponent("agentcheck-\(UUID().uuidString)", isDirectory: true)
    do {
      try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    } catch {
      realRunError = "无法创建检测临时目录：\(error.localizedDescription)"
      running = false
      return
    }
    let outputURL = launchConfiguration.outputPath.map { URL(fileURLWithPath: $0) }
      ?? directory.appendingPathComponent("report.json")
    let progressURL = directory.appendingPathComponent("progress.jsonl")
    realRunDirectory = directory
    realOutputURL = outputURL
    realProgressURL = progressURL
    let process = Process()
    process.executableURL = URL(fileURLWithPath: launchConfiguration.cliPath)
    var arguments = [
      "--url", service.url,
      "--model", service.model,
      "--no-gui",
      "--format", "json",
      "--output", outputURL.path,
      "--progress-file", progressURL.path,
      "--timeout-seconds", String(launchConfiguration.timeoutSeconds),
    ]
    let backendModules = ["ingress"] + modules.compactMap(Self.backendModule)
    // 未显式指定时把报告写到应用数据目录（不能放上面的临时目录，跑完会被清理）。
    let fallbackRoot = storageURL?.deletingLastPathComponent()
      ?? FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("AgentCheckPrototype")
    let reportDirectory = launchConfiguration.reportDirectory
      ?? fallbackRoot
        .appendingPathComponent("reports")
        .appendingPathComponent(Self.reportDirectoryName())
        .path
    arguments += ["--report-dir", reportDirectory]
    if let htmlPath = launchConfiguration.htmlPath {
      arguments += ["--html", htmlPath]
    }
    if !backendModules.isEmpty {
      arguments += ["--modules", backendModules.joined(separator: ",")]
    }
    if let stopAfter = launchConfiguration.stopAfter {
      if backendModules.contains(stopAfter) { arguments += ["--stop-after", stopAfter] }
    }
    if let reportMode = launchConfiguration.reportMode {
      arguments += ["--mode", reportMode]
    }
    process.arguments = arguments
    var environment = ProcessInfo.processInfo.environment
    if let sessionKey { environment["MODEL_API_KEY"] = sessionKey }
    process.environment = environment
    process.terminationHandler = { [weak self] process in
      Task { @MainActor [weak self] in
        guard let self else { return }
        await self.finishRealRun(stopped: process.terminationStatus != 0)
      }
    }
    do {
      try process.run()
      realProcess = process
      progressTask = Task { [weak self] in
        await self?.consumeProgress(at: progressURL)
      }
    } catch {
      realRunError = "启动检测进程失败：\(error.localizedDescription)"
      running = false
      realRunDirectory = nil
    }
  }

  // 只消费到最后一处换行：没写完的半行留到下一次再读，事件不会丢。
  private func processProgressData(_ data: Data, offset: Int) -> Int {
    guard data.count > offset else { return offset }
    let chunk = data.subdata(in: offset..<data.count)
    guard let lastNewline = chunk.lastIndex(of: UInt8(ascii: "\n")) else { return offset }
    let complete = chunk[chunk.startIndex...lastNewline]
    let lines = String(decoding: complete, as: UTF8.self).split(separator: "\n")
    for line in lines {
      guard let event = try? JSONDecoder().decode(ProgressEvent.self, from: Data(line.utf8)) else {
        continue
      }
      handleProgress(event)
    }
    return offset + complete.count
  }

  private func consumeProgress(at url: URL) async {
    var offset = 0
    while !Task.isCancelled && running {
      if let data = try? Data(contentsOf: url) {
        offset = processProgressData(data, offset: offset)
      }
      try? await Task.sleep(nanoseconds: 150_000_000)
    }
  }

  func handleProgress(_ event: ProgressEvent) {
    progressMessage = Self.customerProgressMessage(event.message)
    if event.phase == "module_started" {
      guard let moduleID = event.moduleID, Self.uiModule(moduleID) != nil else { return }
      // 优先使用事件携带的小项清单（CLI 单源下发）；旧版 CLI 无该字段时退回本地模板。
      var items =
        event.items?.map {
          ProgressItem(id: $0.id, name: $0.name, completed: 0, total: $0.total, state: "等待中")
        } ?? Self.progressItems(for: moduleID)
      if !items.isEmpty { items[0].state = "进行中" }
      progressItems = items
      moduleProgressSnapshots[moduleID] = items
      expandedModuleIDs.insert(moduleID)
      detailIndex = 0
      detailTotal = nil
      detailID = nil
      elapsed = max(elapsed, completed.count * 2)
    } else if event.phase == "module_progress", let moduleID = event.moduleID {
      guard Self.uiModule(moduleID) != nil else { return }
      detailIndex = event.detailIndex ?? detailIndex
      detailTotal = event.detailTotal ?? detailTotal
      detailID = event.detailID
      updateProgressItems(moduleID: moduleID, event: event)
    } else if event.phase == "module_completed", let moduleID = event.moduleID {
      if let module = CheckModule.fromBackend(moduleID), !completed.contains(module) {
        completed.append(module)
        detailIndex = 0
        detailTotal = nil
        detailID = nil
      }
      moduleStates[moduleID] = event.state ?? "unknown"
      progressItems = progressItems.map { item in
        var item = item
        item.completed = item.total
        item.state = event.state == "pass" ? "已完成" : "已结束"
        return item
      }
      moduleProgressSnapshots[moduleID] = progressItems
      expandedModuleIDs.remove(moduleID)
      elapsed = max(elapsed, completed.count * 2)
    }
  }

  private func updateProgressItems(moduleID: String, event: ProgressEvent) {
    guard let detailID = event.detailID else { return }
    let itemID = progressItems.contains(where: { $0.id == detailID })
      ? detailID
      : detailID.split(separator: "-").first.map(String.init) ?? detailID
    let index = event.detailIndex ?? 0
    var completedBefore = 0
    progressItems = progressItems.map { item in
      var item = item
      if item.id == itemID || (index > completedBefore && index <= completedBefore + item.total) {
        // 编号命中的小项之外，样本位置落在哪项就推进哪项（detail_id 滞后时不至于停住）
        item.completed = min(item.total, max(0, index - completedBefore))
        item.state = item.completed >= item.total ? "已完成" : "进行中"
      } else if completedBefore + item.total <= index {
        // 进度位置已越过该小项：直接按全部完成收尾，漏掉中间事件也不停在等待中
        item.completed = item.total
        item.state = "已完成"
      } else if item.completed > 0 && item.state == "进行中" {
        item.state = "已完成"
      }
      completedBefore += item.total
      return item
    }
  }

  private static func progressItems(for moduleID: String) -> [ProgressItem] {
    let names: [(String, String, Int)]
    switch moduleID {
    case "specification":
      names = [("S01", "协议可接受上限", 4), ("S04", "工具调用", 5), ("S05", "结构化输出", 2), ("S06", "消息与多轮输入", 4), ("S07", "流式输出", 2)]
    case "capability":
      names = [("C01", "文本理解与指令执行", 44), ("C02", "信息提取与结构化填写", 20), ("C03", "工具选择与参数填写", 20), ("C04", "多轮对话与条件承接", 20), ("C05", "长材料理解与信息利用", 20), ("C06", "逻辑推理与计算", 20)]
    case "performance":
      names = [("P01", "首字响应时间", 2), ("P02", "完整响应时间", 1), ("P03", "并发处理能力", 4), ("P04", "持续运行稳定性", 1), ("P05", "长文本负载", 7)]
    case "agent":
      names = [("T1-A", "遵守任务规则", 1), ("T1-B", "处理外部注入", 1), ("T2-A", "选择正确工具", 1), ("T2-B", "校验路径与参数", 1), ("T3-A", "使用工具返回驱动下一步", 1), ("T3-B", "处理工具返回的信息缺失", 1), ("T4-A", "跨轮次保留状态", 1), ("T4-B", "跨轮次响应条件变化", 1), ("T5-A", "处理可恢复工具失败", 1), ("T5-B", "处理信息不足与提前结束", 1)]
    case "baseline":
      return BaselineItem.all.map {
        ProgressItem(id: $0.id, name: $0.title, completed: 0, total: 1, state: "等待中")
      }
    default:
      names = []
    }
    return names.map { ProgressItem(id: $0.0, name: $0.1, completed: 0, total: $0.2, state: "等待中") }
  }

  private static func customerProgressMessage(_ message: String) -> String {
    [
      "specification": "模型规格实测",
      "capability": "模型能力跑分",
      "performance": "模型性能实测",
      "baseline": "模型基线对比",
      "ingress": "模型接入信息",
      "agent": "智能体实测",
    ].reduce(message) { result, item in
      result.replacingOccurrences(of: item.key, with: item.value)
    }
  }

  @MainActor
  private func finishRealRun(stopped: Bool) async {
    guard realProcess != nil, !realFinalizing else { return }
    realFinalizing = true
    // 进程退出和轮询之间存在竞态：退出前最后写入的事件可能还没被读到。
    // 这里把进度文件整体重放一遍（handleProgress 幂等），确保不丢尾部事件。
    if let url = realProgressURL {
      realProgressURL = nil
      if let data = try? Data(contentsOf: url) {
        _ = processProgressData(data, offset: 0)
      }
    }
    progressTask?.cancel()
    progressTask = nil
    let reportData = realOutputURL.flatMap { try? Data(contentsOf: $0) }
    let reportText = reportData.map { String(decoding: $0, as: UTF8.self) }
    let reportObject = reportData.flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] }
    let overall = reportObject?["overall"] as? String
    let outcome = Outcome(rawValue: overall ?? "inconclusive") ?? .inconclusive
    let responseModel = ((reportObject?["record"] as? [String: Any])?["target"] as? [String: Any])?["model"] as? String
    guard let service = runningService ?? self.service else { return }
    let record = RunRecord(
      service: service,
      modules: activeModules,
      completed: completed,
      outcome: stopped ? .inconclusive : outcome,
      stopped: stopped,
      presentationVersion: 4,
      context: nil,
      agentMode: "real",
      responseModel: responseModel,
      reportJSON: reportText,
      moduleStates: moduleStates
    )
    running = false
    realProcess = nil
    realFinalizing = false
    realRunError = stopped && reportText == nil ? "检测未生成完整报告，已保留已完成模块状态。" : nil
    selectedRecordID = record.id
    records.insert(record, at: 0)
    screen = .result
    progressMessage = stopped ? "检测已停止" : "检测已完成"
    persist()
    if let realRunDirectory { try? FileManager.default.removeItem(at: realRunDirectory) }
    realRunDirectory = nil
    realOutputURL = nil
    realProgressURL = nil
  }

  func openRecord(_ record: RunRecord) {
    evidenceExpanded = false
    selectedRecordID = record.id
    screen = .result
  }

  // 历史回看：进只读查看器。「检测报告」与各模块页永远属于当前这一次检测，
  // 不允许被历史记录切换来切换去。
  func reviewRecord(_ record: RunRecord) {
    viewedRecord = record
  }

  // 回到当前这一次检测的报告页（跑完检测的落地页，侧栏不设常驻入口）。
  func showCurrentReport() {
    selectedRecordID = nil
    screen = .result
  }

  // 联调入口：把磁盘上的 CLI run.json 载入为一条记录，验证真实报告渲染。
  func loadReportFile(_ url: URL) {
    guard let data = try? Data(contentsOf: url),
      let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
      let recordObject = object["record"] as? [String: Any]
    else {
      realRunError = "无法读取报告文件：\(url.lastPathComponent)"
      return
    }
    let configuration = object["configuration"] as? [String: Any] ?? [:]
    let target = recordObject["target"] as? [String: Any] ?? [:]
    let service = Service(
      url: configuration["url"] as? String ?? target["endpointFingerprint"] as? String
        ?? "已导入报告",
      model: target["model"] as? String ?? configuration["model"] as? String ?? "未知模型")
    var states: [String: String] = [:]
    var completed: [CheckModule] = []
    for result in recordObject["moduleResults"] as? [[String: Any]] ?? [] {
      guard let id = result["moduleId"] as? String ?? result["module_id"] as? String else {
        continue
      }
      let state = result["state"] as? String ?? "unknown"
      states[id] = state
      if let module = CheckModule.fromBackend(id), state != "not_selected",
        !completed.contains(module)
      {
        completed.append(module)
      }
    }
    let selected = (object["selected_modules"] as? [String] ?? [])
      .compactMap(CheckModule.fromBackend)
    let overall = object["overall"] as? String
    let record = RunRecord(
      service: service,
      modules: selected.isEmpty ? completed : selected,
      completed: completed,
      outcome: Outcome(rawValue: overall ?? "inconclusive") ?? .inconclusive,
      stopped: false,
      presentationVersion: 4,
      context: nil,
      agentMode: "real",
      responseModel: (recordObject["serviceReturnedModel"] as? [String: Any])?["modelId"]
        as? String,
      reportJSON: String(decoding: data, as: UTF8.self),
      moduleStates: states)
    records.insert(record, at: 0)
    selectedRecordID = record.id
    screen = .result
  }

  func persist() {
    guard let url = storageURL, persistenceError == nil else { return }
    do {
      try FileManager.default.createDirectory(
        at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
      let data = try JSONEncoder().encode(LocalSnapshot(service: service, records: records))
      try data.write(to: url, options: .atomic)
    } catch {
      persistenceError = "记录保存失败，本次结果仍可在当前窗口查看。"
    }
  }
}
