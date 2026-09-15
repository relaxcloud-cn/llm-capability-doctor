import Foundation

@MainActor
func runModelTests() async throws {
  var checks = 0
  func expect(_ condition: @autoclosure () -> Bool, _ message: String) throws {
    checks += 1
    if !condition() { throw PrototypeError.failed(message) }
  }
  let launch = LaunchConfiguration.from(arguments: [
    "AgentCheck", "--agentcheck-endpoint", "https://example.test/v1/chat/completions",
    "--agentcheck-model", "fixture", "--agentcheck-cli-path", "/tmp/agentcheck",
    "--agentcheck-report-dir", "/tmp/reports", "--agentcheck-html", "/tmp/reports/report.html",
  ])
  try expect(launch?.reportDirectory == "/tmp/reports", "保留 CLI 报告目录")
  try expect(launch?.htmlPath == "/tmp/reports/report.html", "保留 CLI HTML 输出位置")
  try expect(launch?.cliPath == "/tmp/agentcheck", "桌面端继续使用原始单文件 CLI")
  let store = Workbench()
  try expect(store.configuring && store.service == nil, "首次启动无配置")
  try expect(
    CheckModule.allCases.map(\.title) == [
      "模型接入信息", "模型规格实测", "模型能力跑分", "模型性能实测", "智能体实测", "模型基线对比",
    ], "模块正式名称一致")
  try expect(
    CheckModule.testModules.count == 5 && !CheckModule.testModules.contains(.info), "接入信息不是检测项目")
  for url in [
    "invalid", "https://user:key@example.com", "https://example.com?key=secret",
    "https://example.com#secret",
  ] {
    try expect(Workbench.validationError(url: url, model: "m", key: "k") != nil, "拒绝非法或带凭据地址")
  }
  try expect(
    Workbench.validationError(url: "https://example.com", model: " ", key: "k") != nil, "模型名称必填")
  try expect(
    Workbench.validationError(url: "https://example.com", model: "m", key: " ") != nil, "密钥必填")
  store.fillExample()
  await store.connect()
  try expect(store.service?.model == "agent-prod" && !store.configuring, "连接后进入首页")
  try expect(store.draftKey.isEmpty, "连接后清空密钥")
  store.prepareRun()
  try expect(store.selectedModules.isEmpty, "不擅自预选正式检测组合")
  store.startRun(automatic: false)
  try expect(!store.running, "未选模块不能开始")
  store.showConfirmation = false
  store.prepareRun(module: .info)
  try expect(!store.showConfirmation && store.accessExpanded && store.screen == .home, "接入信息回到首页摘要")
  store.selectedModules = Set(CheckModule.allCases)
  store.startRun(automatic: false)
  store.advance()
  store.startRun(automatic: false)
  try expect(store.elapsed == 1 && store.activeModules.count == 5, "重复启动不重置且剔除接入信息")
  for _ in 0..<9 { store.advance() }
  try expect(!store.running && store.records.count == 1, "一轮只生成一条记录")
  let limited = store.currentRecord!
  try expect(limited.presentationVersion == 3 && limited.context != nil, "新记录带版本与测试条件")
  try expect(limited.title == Outcome.limited.title, "总体结论来自任务演示")
  try expect(
    limited.coveredSamples == 10 && limited.initialCount == 30 && limited.reviewCount == 2
      && limited.invalidCount == 0, "受限示例准确统计十样本三十二次共享执行")
  try expect(Set(limited.validAgentRuns.map(\.id)).count == 32, "执行 ID 不重复")
  try expect(
    limited.agentFindings.count == 8 && limited.agentFindings[4].state == .fail, "八项检查且 A5 不通过")
  try expect(limited.agentFindings[7].state == .pass, "恢复失败但真实交付不能误判 A8")
  store.navigate(.home)
  store.showKeyEvidence()
  try expect(store.screen == .module(.agent) && store.agentSelection == 4, "首页直达关键问题")
  try expect(
    store.agentSample == "T3a" && store.agentRun == 2 && store.evidenceExpanded, "直接展开失败记录而不是首条成功记录"
  )
  try expect(store.currentRecord?.id == limited.id, "问题证据与首页结果属于同一检测")
  let recover = limited.agentSamples.first { $0.id == "T3a" }!
  try expect(
    recover.validRuns.count == 5
      && recover.validRuns.filter { !$0.failedChecks.contains(4) }.count == 3, "恢复样本三次初测两次复核合计三次通过")
  for finding in limited.agentFindings {
    try expect(finding.sampleIDs.count >= 2, "每项至少两种适用条件：A\(finding.id + 1)")
  }
  for mode in ["intermittent", "pending-review"] {
    store.outcome = .limited
    store.agentMode = mode
    store.prepareRun(module: .agent)
    store.startRun(automatic: false)
    store.agentMode = "standard"
    store.advance()
    store.advance()
    let record = store.currentRecord!
    try expect(record.agentMode == mode, "演示模式在开始时冻结")
    try expect(
      record.agentFindings[4].state == (mode == "intermittent" ? .unstable : .unknown),
      "单次失败和复核不足不当作可复现失败")
    try expect(
      record.title == (mode == "intermittent" ? Outcome.limited.title : Outcome.inconclusive.title),
      "复核未完成的总体结论不可伪装为可用")
  }
  for outcome in Outcome.allCases {
    store.outcome = outcome
    store.agentMode = "standard"
    store.prepareRun(module: .agent)
    store.startRun(automatic: false)
    store.outcome = .usable
    store.advance()
    store.advance()
    try expect(store.currentRecord?.outcome == outcome, "开始后冻结结论方案")
  }
  let unknown = store.currentRecord!
  try expect(unknown.validAgentRuns.isEmpty && unknown.invalidCount == 3, "环境失败只记无效尝试")
  try expect(
    unknown.agentFindings.allSatisfy { $0.state == .unknown && $0.unverified }, "未调用模型不捏造失败")
  var blocker = limited
  blocker.outcome = .blocked
  blocker.stopped = true
  try expect(
    blocker.title == Outcome.blocked.title && blocker.agentFindings[6].state == .fail,
    "停止不抵消已确认权限阻断")
  try expect(
    blocker.initialCount == 22 && blocker.reviewCount == 0 && blocker.coveredSamples == 7,
    "严重边界事件不必完成全部样本")
  store.prepareRun(module: .functions)
  store.startRun(automatic: false)
  store.advance()
  store.advance()
  try expect(store.currentRecord?.title == Outcome.inconclusive.title, "基础能力不能推出 Agent 可用")
  store.prepareRun(module: .parameters)
  store.startRun(automatic: false)
  store.advance()
  store.finish(stopped: true)
  try expect(
    store.currentRecord?.completed.isEmpty == true && store.currentRecord?.stopped == true,
    "停止保留未完成状态")
  let count = store.records.count
  store.finish(stopped: true)
  try expect(store.records.count == count, "结束操作幂等")
  store.editService()
  store.draftModel = "new-service"
  store.draftKey = "temporary-secret"
  await store.connect()
  try expect(store.latestForService == nil, "换配置不沿用旧结果")
  store.openRecord(limited)
  store.showModule(.agent)
  try expect(store.currentRecord?.id == limited.id, "历史详情切换保留记录")
  store.navigate(.home)
  try expect(store.selectedRecordID == nil, "回首页退出历史上下文")
  store.editService()
  store.draftKey = "temporary-secret"
  store.showModule(.info)
  try expect(
    !store.configuring && store.draftKey.isEmpty && store.accessExpanded, "从编辑配置进入接入信息时退出编辑并清除密钥")
  let snapshot = String(
    decoding: try JSONEncoder().encode(
      LocalSnapshot(service: store.service, records: store.records)), as: UTF8.self)
  try expect(!snapshot.contains("temporary-secret") && !snapshot.contains("draftKey"), "保存数据不含密钥")
  for scenario in ["auth", "timeout"] {
    let failed = Workbench()
    failed.fillExample()
    failed.connectionScenario = scenario
    await failed.connect()
    try expect(
      failed.service == nil && failed.configuring && failed.connectionError != nil
        && failed.draftKey.isEmpty, "连接异常留在配置且清空密钥")
  }
  var legacy = limited
  legacy.id = UUID()
  legacy.presentationVersion = 2
  legacy.context = nil
  try expect(legacy.agentSamples.isEmpty && legacy.agentFindings.isEmpty, "旧版记录不能套新版证据")
  legacy.presentationVersion = nil
  let decoded = try JSONDecoder().decode(RunRecord.self, from: JSONEncoder().encode(legacy))
  try expect(decoded.presentationVersion == nil, "旧版可选字段向后兼容")
  var other = limited
  other.id = UUID()
  store.records = [limited, other, legacy]
  store.historySelection = [limited.id, other.id]
  try expect(store.comparisonBlocker == nil, "同条件可并列比较")
  store.toggleHistory(legacy.id)
  try expect(store.historySelection.count == 2, "历史最多选择两个")
  store.records[1].context?.samples = "other"
  try expect(store.comparisonBlocker != nil, "不同样本不能直接比较")
  store.records[1] = other
  store.records[1].context?.environment = "other"
  try expect(store.comparisonBlocker != nil, "不同环境不能直接比较")
  store.records[1] = other
  store.records[1].modules = [.functions]
  try expect(store.comparisonBlocker != nil, "不同范围不能直接比较")
  store.historySelection = [limited.id, legacy.id]
  try expect(store.comparisonBlocker != nil, "新旧版本不能直接比较")
  store.historySearch = "no-record"
  try expect(store.visibleRecords.isEmpty, "搜索空状态不返回无关记录")
  try expect(
    Catalog.specifications.count == 7 && Scores.all.count == 6 && Catalog.performance.count == 5,
    "分类范围七六五与文档一致")
  try expect(Scores.all.map(\.correct) == [4, 5, 3, 4, 3, 4], "成绩源于实际演示样本")
  try expect(
    Scores.all[4].comparisonCases.count == 5 && Scores.all[4].comparisonCases.allSatisfy(\.passed),
    "长材料 1K 档有独立证据，不只显示摘要数字")
  var named = limited
  named.responseModel = "deployment-\"quoted\""
  let bound = BaselineItem.forRecord(named)[0].responses[0]
  let boundObject = try JSONSerialization.jsonObject(with: Data(bound.raw.utf8)) as! [String: Any]
  try expect(boundObject["model"] as? String == named.responseModel, "基线返回名绑定记录，特殊字符保持合法 JSON")
  try expect(
    bound.actual.firstIndex { $0.contains("\"model\"") }
      == bound.reference.firstIndex { $0.contains("\"model\"") }, "两侧按字段对齐，不把排序变成差异")
  store.records = [blocker]
  store.openRecord(blocker)
  store.agentFilter = "unverified"
  store.showModule(.agent)
  try expect(store.agentSelection == 6 && store.agentFilter == "all", "进入 Agent 结果优先定位确认阻断，清除过期筛选")
  store.showKeyEvidence()
  try expect(
    store.agentSelection == 6 && store.agentSample == "T4b" && store.evidenceExpanded,
    "权限阻断直达审批拒绝记录")
  try expect(store.currentRecord?.id == blocker.id, "关键问题不混入其他历史结果")
  try expect(BaselineItem.all.count == 14 && BaselineItem.groups.count == 5, "十四维度五组场景")
  try expect(BaselineItem.all.filter(\.hasDifference).count == 5, "基线摘要与明细一致")
  try expect(BaselineItem.all[0].responses[0].differences.isEmpty, "值差异不标红")
  try expect(
    BaselineItem.all[3].responses.count == 2
      && BaselineItem.all[3].responses[1].differences.isEmpty, "正常响应不能覆盖同场景异常响应")
  for item in BaselineItem.all {
    for response in item.responses where response.unavailable == nil {
      for lines in [response.actual, response.reference] {
        let data = Data(lines.filter { !$0.isEmpty }.joined(separator: "\n").utf8)
        try expect(
          (try? JSONSerialization.jsonObject(with: data)) != nil, "结构片段须为合法 JSON：\(response.id)")
      }
    }
  }
  try expect(
    BaselineItem.all[12].title == "流式用量返回"
      && !BaselineItem.all[12].responses.contains { $0.raw.contains("[DONE]") }, "流式用量不混入结束标记")
  try expect(
    Set(BaselineItem.all[13].responses.compactMap(\.unavailable)) == ["无适用基线", "未获得响应", "尚未检测"],
    "空状态不混淆")
  let exported =
    try JSONSerialization.jsonObject(with: JSONEncoder().encode(ReportExport(record: limited)))
    as! [String: Any]
  try expect(
    exported["simulated"] as? Bool == true && exported["agentSamples"] != nil
      && exported["baseline"] != nil, "导出包含演示标识和同份证据")
  var sensitive = limited
  sensitive.service.url = "https://user:secret@example.invalid/v1?key=hidden#token"
  let safeExport = String(
    decoding: try JSONEncoder().encode(ReportExport(record: sensitive)), as: UTF8.self)
  try expect(!safeExport.contains("secret") && !safeExport.contains("hidden"), "历史导出脱敏地址")
  let temp = FileManager.default.temporaryDirectory.appendingPathComponent(
    "agent-check-\(UUID()).json")
  let persisted = Workbench(storageURL: temp)
  persisted.fillExample()
  await persisted.connect()
  try expect(Workbench(storageURL: temp).service == persisted.service, "重新启动恢复非敏感配置")
  try? FileManager.default.removeItem(at: temp)
  print("PASS: \(checks) prototype state and evidence checks")
}
