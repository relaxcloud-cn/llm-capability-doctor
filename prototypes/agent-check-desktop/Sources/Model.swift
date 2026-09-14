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
      return "七项基础规格实测——上下文与输出长度、常用参数、工具调用、结构化输出、多轮消息、流式响应"
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
  @Published var accessExpanded = false
  @Published var accessRequested = false
  @Published var catalogSelection = ""
  @Published var catalogFilter = "all"
  @Published var running = false
  @Published var elapsed = 0
  @Published var completed: [CheckModule] = []
  @Published var activeModules: [CheckModule] = []
  @Published var moduleStates: [String: String] = [:]
  @Published var progressMessage = ""
  @Published var realRunError: String?
  @Published var selectedRecordID: UUID?
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
      cancelConfiguration()
      accessExpanded = true
      accessRequested.toggle()
      if selectedRecordID != nil { screen = .result } else { screen = .home }
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
    return min(Double(elapsed) / Double(activeModules.count * 2), 1)
  }
  var activeModule: CheckModule? {
    guard running, completed.count < activeModules.count else { return nil }
    return activeModules[completed.count]
  }
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
    if !backendModules.isEmpty {
      arguments += ["--modules", backendModules.joined(separator: ",")]
    }
    if let stopAfter = launchConfiguration.stopAfter {
      if backendModules.contains(stopAfter) { arguments += ["--stop-after", stopAfter] }
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

  private func consumeProgress(at url: URL) async {
    var offset = 0
    while !Task.isCancelled && running {
      if let data = try? Data(contentsOf: url), data.count > offset {
        let newData = data.subdata(in: offset..<data.count)
        offset = data.count
        let lines = String(decoding: newData, as: UTF8.self).split(separator: "\n")
        for line in lines {
          guard let event = try? JSONDecoder().decode(ProgressEvent.self, from: Data(line.utf8)) else {
            continue
          }
          handleProgress(event)
        }
      }
      try? await Task.sleep(nanoseconds: 150_000_000)
    }
  }

  private func handleProgress(_ event: ProgressEvent) {
    progressMessage = Self.customerProgressMessage(event.message)
    if event.phase == "module_started" {
      elapsed = max(elapsed, event.index * 2)
    } else if event.phase == "module_completed", let moduleID = event.moduleID {
      if let module = CheckModule.fromBackend(moduleID), !completed.contains(module) {
        completed.append(module)
      }
      moduleStates[moduleID] = event.state ?? "unknown"
      elapsed = max(elapsed, event.index * 2)
    }
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
