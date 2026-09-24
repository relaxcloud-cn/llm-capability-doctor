import SwiftUI

// 真实检测的模块结果页：先结论、按小项分组、证据沉底。
// 内部样本编号（context-1024、P02、real_specification_report）不直接露脸，
// 一律翻译成客户能懂的说法；原始数据保留在最底层的折叠里。

struct RealResultCatalog {
  struct Group {
    let id: String
    let name: String
    let passNote: String
    let failNote: String
    let how: String
    let judge: String
  }

  var groups: [Group]
  var unitName: String
  var sampleGroup: (String) -> String?
  var sampleName: (Int, String) -> String
  var desc: String
  var scope: String

  static func catalog(for backendID: String) -> RealResultCatalog? {
    switch backendID {
    case "specification": return specification
    case "capability": return capability
    case "performance": return performance
    case "agent": return agent
    case "baseline": return baseline
    default: return nil
    }
  }

  // 演示记录与真实报告共用同一骨架；分组名跟随演示数据自身的分类。
  static func demoCatalog(for backendID: String) -> RealResultCatalog? {
    switch backendID {
    case "specification", "capability": return catalog(for: backendID)
    case "performance": return demoPerformance
    case "agent": return demoAgent
    case "baseline": return demoBaseline
    default: return nil
    }
  }

  private static let demoPerformance: RealResultCatalog = {
    let groups = Catalog.performance.map {
      Group(
        id: $0.id, name: $0.title, passNote: $0.value, failNote: $0.summary,
        how: "实测「\($0.title)」：真实发起请求并记录测量值。",
        judge: "测量值在正常范围 → 正常；出现超时、错误或明显异常 → 未通过。")
    }
    return RealResultCatalog(
      groups: groups,
      unitName: "条",
      sampleGroup: { _ in nil },
      sampleName: { index, _ in "第 \(index) 条记录" },
      desc: performance.desc,
      scope: performance.scope)
  }()

  private static let demoAgent: RealResultCatalog = {
    let groups = AgentEvidence.definitions.map {
      Group(
        id: Self.demoAgentGroup($0.id), name: $0.title,
        passNote: "任务按预期完成", failNote: "任务没有按预期完成，展开可看卡在哪一步",
        how: "下发固定任务书「\($0.title)」，记录模型的完整执行过程与最终交付。",
        judge: "按任务预期完成 → 通过；未按预期完成 → 未通过；无有效执行 → 未判定（无效尝试不计为模型失败）。")
    }
    return RealResultCatalog(
      groups: groups,
      unitName: "个",
      sampleGroup: { _ in nil },
      sampleName: { _, _ in "完整任务过程" },
      desc: agent.desc,
      scope: agent.scope)
  }()

  // 演示基线的逐条单位是「响应记录」（23 条），不是结构项（14 个）。
  private static let demoBaseline: RealResultCatalog = RealResultCatalog(
    groups: baseline.groups,
    unitName: "条",
    sampleGroup: { _ in nil },
    sampleName: { index, _ in "第 \(index) 条记录" },
    desc: baseline.desc,
    scope: baseline.scope)

  // 演示任务编号 T1a / T1b -> 分组号 T1-A / T1-B
  static func demoAgentGroup(_ sampleID: String) -> String {
    let upper = sampleID.uppercased()
    guard upper.count == 3 else { return upper }
    return "\(upper.prefix(2))-\(upper.suffix(1))"
  }

  func groupID(for sampleID: String) -> String {
    sampleGroup(sampleID) ?? "_other"
  }

  private static func plain(_ known: [String: String], fallback: @escaping (Int) -> String)
    -> (Int, String) -> String
  {
    { index, sampleID in known[sampleID] ?? fallback(index) }
  }

  // ---------- 模型规格实测 ----------
  private static let specification: RealResultCatalog = {
    let groups = [
      Group(
        id: "S01", name: "协议可接受上限",
        passNote: "不同长度的请求都能被服务接受并正常回答",
        failNote: "服务拒绝了部分长度的请求；超长业务内容需要先拆分或截断",
        how: "向服务发送多个真实长度的输入（约 64K–512K token），观察每个档位是否被接受并完整返回。",
        judge: "全部档位被接受且回复完整 → 通过；任一档位被拒绝或回复截断 → 未通过；未取得有效回复 → 未判定。"),
      Group(
        id: "S04", name: "工具调用",
        passNote: "都能按格式返回工具调用",
        failNote: "有请求没有按格式返回工具调用；业务依赖工具调用的话需要先处理",
        how: "构造五种工具调用场景（填齐各类参数、禁止调用、同一工具连调两次、两个不同工具、强制指定），检查返回的 tool_calls。",
        judge: "tool_calls 结构完整、参数可解析 → 通过；未按协议格式返回 → 未通过；服务不支持工具调用 → 未判定。"),
      Group(
        id: "S05", name: "结构化输出",
        passNote: "JSON、按字段模板两种都能按格式输出",
        failNote: "部分格式要求没有满足；程序直接解析返回值的场景需要适配",
        how: "分别要求按 JSON 和按字段模板输出，验证返回是否严格符合声明格式。",
        judge: "返回可被对应格式解析 → 通过；格式不符或混入多余内容 → 未通过。"),
      Group(
        id: "S06", name: "消息与多轮输入",
        passNote: "单轮、多轮、系统提示都能正确处理",
        failNote: "部分消息形态处理不正确；多轮对话业务建议验证",
        how: "发送四种消息组合（system+user、带历史回引、多标记区分、含 tool 角色），检查多轮消息形态是否被正确处理。",
        judge: "各消息形态均被正确处理 → 通过；报错或语义错乱 → 未通过。"),
      Group(
        id: "S07", name: "流式输出",
        passNote: "流式返回能正常开始和正常结束",
        failNote: "流式返回未能正常开始或结束；流式场景需要先排查",
        how: "以 stream 模式请求文本与工具调用输出，观察分块流能否正常开始、正常结束。",
        judge: "流式正常开始且正常结束 → 通过；中途断流或协议错误 → 未通过；不支持流式 → 未判定。"),
    ]
    let membership: [String: String] = [
      "context-64k": "S01", "context-128k": "S01", "context-256k": "S01", "context-512k": "S01",
      "tools-all-types": "S04", "tools-none": "S04", "tools-same-twice": "S04",
      "tools-two-distinct": "S04", "tools-forced": "S04",
      "json": "S05", "schema": "S05",
      "M01": "S06", "M02": "S06", "M03": "S06", "M04": "S06",
      "stream-text": "S07", "stream-tool": "S07",
    ]
    let names: [String: String] = [
      "context-64k": "约 64K token 的真实长度输入",
      "context-128k": "约 128K token 的真实长度输入",
      "context-256k": "约 256K token 的真实长度输入",
      "context-512k": "约 512K token 的真实长度输入",
      "tools-all-types": "单次调用填齐各类参数",
      "tools-none": "禁止工具调用时直接回答",
      "tools-same-twice": "同一工具调用两次且参数不串",
      "tools-two-distinct": "两个不同工具各调一次",
      "tools-forced": "强制调用指定工具",
      "M01": "system+user 角色组合",
      "M02": "带历史回引暗号",
      "M03": "多条历史里区分目标标记",
      "M04": "含 tool 角色的消息序列",
      "json": "要求 JSON 回答",
      "schema": "要求按字段模板回答",
      "stream-text": "流式文本输出",
      "stream-tool": "流式工具调用输出",
    ]
    return RealResultCatalog(
      groups: groups,
      unitName: "项",
      sampleGroup: { membership[$0] },
      sampleName: plain(names, fallback: { "第 \($0) 项检查" }),
      desc: "检查接口的基本收发能力：不同长度的请求能否被接受、工具调用能否按格式返回、JSON 和字段模板能否按格式输出、单轮与多轮消息能否正确处理、流式返回能否正常开始和结束。",
      scope: "以上结论来自固定检查项，覆盖日常使用的请求形态；它说明「这些常规用法没问题」，不代表该模型的能力上限。")
  }()

  // ---------- 模型能力跑分 ----------
  private static let capability: RealResultCatalog = {
    let judge = "回答与参考答案一致 → 答对；不一致 → 答错；未作答或无法判分 → 未判定。"
    let groups = [
      Group(id: "C01", name: "文本理解与指令执行", passNote: "这类题做得稳", failNote: "错得较多，重要指令建议复核", how: "围绕文本理解与指令执行出固定题目，收集模型回答。", judge: judge),
      Group(id: "C02", name: "信息提取与结构化填写", passNote: "这类题做得稳", failNote: "错得较多，提取结果建议复核", how: "围绕信息提取与结构化填写出固定题目，收集模型回答。", judge: judge),
      Group(id: "C03", name: "工具选择与参数填写", passNote: "这类题做得稳", failNote: "近半数选择或参数出错，重点复核", how: "围绕工具选择与参数填写出固定题目，收集模型回答。", judge: judge),
      Group(id: "C04", name: "多轮对话与条件承接", passNote: "这类题做得稳", failNote: "错得较多，条件变化场景建议复核", how: "围绕多轮对话与条件承接出固定题目，收集模型回答。", judge: judge),
      Group(id: "C05", name: "长材料理解与信息利用", passNote: "这类题做得稳", failNote: "错得较多，长材料结论建议复核", how: "围绕长材料理解与信息利用出固定题目，收集模型回答。", judge: judge),
      Group(id: "C06", name: "逻辑推理与计算", passNote: "这类题做得稳", failNote: "错得最多；重要计算务必人工复核", how: "围绕逻辑推理与计算出固定题目，收集模型回答。", judge: judge),
    ]
    return RealResultCatalog(
      groups: groups,
      unitName: "题",
      sampleGroup: { id in id.split(separator: "-").first.map(String.init) },
      sampleName: { index, _ in "第 \(index) 题" },
      desc: "用固定题目逐题判分：文本理解与指令执行、信息提取与结构化填写、工具选择与参数填写、多轮对话与条件承接、长材料理解、逻辑推理与计算。",
      scope: "题目为固定题库，衡量「这类任务它做得稳不稳」，不是通用智力评分；同题不同次作答可能有小幅波动。")
  }()

  // ---------- 模型性能实测 ----------
  private static let performance: RealResultCatalog = {
    let judge = "测量值在正常范围且无错误 → 通过；出现超时、错误或明显异常 → 未通过；预热样本仅标注，不计入结论。"
    let groups = [
      Group(id: "P01", name: "首字响应时间", passNote: "从发出请求到第一个字返回的等待正常", failNote: "部分请求的首字等待异常", how: "实测从发出请求到收到第一个字的等待时间。", judge: judge),
      Group(id: "P02", name: "完整响应时间", passNote: "完整生成一段回答的耗时正常", failNote: "部分请求耗时异常", how: "实测从发出请求到回答完整生成的总耗时。", judge: judge),
      Group(id: "P03", name: "并发处理能力", passNote: "不同并发档位下请求都能完成", failNote: "部分并发档位出现失败", how: "在多个并发档位下同时发起请求，观察各档位完成情况。", judge: judge),
      Group(id: "P04", name: "持续运行稳定性", passNote: "持续运行期间没有出现失败或明显变慢", failNote: "持续运行期间出现失败或明显变慢", how: "持续发起一段时间的请求，观察是否出现失败或明显变慢。", judge: judge),
      Group(id: "P05", name: "长文本负载", passNote: "不同长度的输入材料下表现正常", failNote: "长输入场景下表现异常", how: "在不同长度的输入材料下实测响应表现。", judge: judge),
    ]
    return RealResultCatalog(
      groups: groups,
      unitName: "批",
      sampleGroup: { id in id.split(separator: "-").first.map(String.init) },
      sampleName: { index, _ in "第 \(index) 批请求" },
      desc: "在不同负载下实测响应表现：首字等待时间、完整响应耗时、并发处理能力、持续运行稳定性、长文本输入。",
      scope: "结果来自本次检测时段的实际测量；不同时间、不同负载下数值会有波动。")
  }()

  // ---------- 智能体实测 ----------
  private static let agent: RealResultCatalog = {
    let scenarioNames: [(String, String)] = [
      ("T1-A", "遵守任务规则"), ("T1-B", "处理外部注入"),
      ("T2-A", "选择正确工具"), ("T2-B", "校验路径与参数"),
      ("T3-A", "使用工具返回驱动下一步"), ("T3-B", "处理工具返回的信息缺失"),
      ("T4-A", "跨轮次保留状态"), ("T4-B", "跨轮次响应条件变化"),
      ("T5-A", "处理可恢复工具失败"), ("T5-B", "处理信息不足与提前结束"),
    ]
    func normalize(_ sampleID: String) -> String? {
      let raw = sampleID.hasPrefix("agent-") ? String(sampleID.dropFirst(6)) : sampleID
      let upper = raw.uppercased()
      guard upper.count == 3 else { return scenarioNames.first { $0.0 == upper }?.0 }
      let dashed = "\(upper.prefix(2))-\(upper.suffix(1))"
      return scenarioNames.first { $0.0 == dashed }?.0
    }
    return RealResultCatalog(
      groups: scenarioNames.map {
        Group(
          id: $0.0, name: $0.1, passNote: "任务按预期完成", failNote: "任务没有按预期完成，展开可看卡在哪一步",
          how: "下发固定任务书「\($0.1)」，记录模型的完整执行过程与最终交付。",
          judge: "按任务预期完成 → 通过；未按预期完成 → 未通过；无有效执行 → 未判定（无效尝试不计为模型失败）。")
      },
      unitName: "个",
      sampleGroup: normalize,
      sampleName: { _, _ in "完整任务过程" },
      desc: "用固定任务考察智能体行为：遵守任务规则、抵御外部注入、选对工具、校验参数、用工具返回驱动下一步、跨轮次保留状态、处理可恢复错误、信息不足时的表现。",
      scope: "任务为固定场景，覆盖常见交付形态；不能穷尽所有真实业务。")
  }()

  // ---------- 模型基线对比 ----------
  private static let baseline: RealResultCatalog = {
    let groups = BaselineItem.all.map {
      Group(
        id: $0.id, name: $0.title, passNote: "返回结构与通用规范一致", failNote: "结构与通用规范不一致，展开可看差异",
        how: "发起真实请求，将返回中「\($0.title)」相关字段与通用规范逐项对照。",
        judge: "结构与规范一致 → 一致；存在差异 → 有差异；本次未观测到 → 未判定。")
    }
    return RealResultCatalog(
      groups: groups,
      unitName: "项",
      sampleGroup: { id in
        let upper = id.uppercased()
        return upper.hasPrefix("BC") ? String(upper.prefix(4)) : nil
      },
      sampleName: { _, _ in "该结构项的实测返回" },
      desc: "把服务的实际返回与通用规范逐项对照：消息字段、结束原因、用量统计、工具调用结构、流式分块等——只看格式，不评内容质量。",
      scope: "只检查数据格式是否符合通用规范，不判断内容质量。")
  }()
}

struct RealTaskReport: Identifiable {
  // 三态：通过 / 未通过 / 未判定。CLI 把 inconclusive、not_measured 等归为不计失败的
  // 「未判定」，界面必须区分，不能把证据不足画成失败。
  enum Verdict { case pass, fail, unverified }
  let id: String
  let status: String
  let verdict: Verdict
  let verdictText: String
  let note: String
  let request: String
  let response: String
  let evidenceJSON: String
  // 原始 curl 命令与原始返回 JSON；智能体任务是任务书不是单次 HTTP，curl 为空
  var curl: String = ""
  var rawResponse: String = ""
  // 演示数据直接指定归属分组与显示名，不走 sampleGroup/sampleName 推导
  var group: String? = nil
  var displayName: String? = nil
}

struct RealModuleView: View {
  var module: CheckModule
  var record: RunRecord

  private var backendID: String { module.backendID ?? "ingress" }
  private var reportObject: [String: Any]? {
    guard let reportJSON = record.reportJSON,
      let data = reportJSON.data(using: .utf8)
    else { return nil }
    return try? JSONSerialization.jsonObject(with: data) as? [String: Any]
  }
  private var recordObject: [String: Any] {
    reportObject?["record"] as? [String: Any] ?? [:]
  }
  private var state: String {
    if !record.isRealReport {
      if tasks.isEmpty { return "unverified" }
      if passCount == 0 { return failCount > 0 ? "fail" : "inconclusive" }
      return failCount > 0 ? "fail" : "pass"
    }
    return record.moduleStates?[backendID] ?? moduleResult?["state"] as? String ?? "unverified"
  }
  private var stateStyle: StatusStyle {
    if state == "fail" { return FindingState.fail.style }
    if failCount > 0 { return FindingState.unstable.style }
    switch state {
    case "pass": return FindingState.pass.style
    case "unsupported", "inconclusive", "invalid_execution": return FindingState.unstable.style
    default: return FindingState.unknown.style
    }
  }
  private var moduleResult: [String: Any]? {
    let results = recordObject["moduleResults"] as? [[String: Any]] ?? []
    return results.first { ($0["moduleId"] as? String) == backendID }
      ?? results.first { ($0["module_id"] as? String) == backendID }
  }

  // 报告里一条样本/检查项的判定结果，加上回到报告原文的引用。
  private struct TaskSpec {
    var id: String
    var verdict: RealTaskReport.Verdict
    var text: String
    var note: String
    var source: [String: Any]?
  }

  // 本模块在 record.evidence 里的汇总条目（payload.module == 本模块）。
  private var modulePayload: [String: Any]? {
    recordEvidence
      .first { ($0["payload"] as? [String: Any])?["module"] as? String == backendID }
      .flatMap { ($0["payload"] as? [String: Any])?["payload"] as? [String: Any] }
  }
  private var report: [String: Any]? { modulePayload?["report"] as? [String: Any] }
  private var scorecard: [String: Any]? { modulePayload?["scorecard"] as? [String: Any] }
  private var nestedEvidence: [[String: Any]] {
    modulePayload?["evidence"] as? [[String: Any]] ?? []
  }
  private var recordEvidence: [[String: Any]] {
    recordObject["evidence"] as? [[String: Any]] ?? []
  }
  private var recordEvidenceByID: [String: [String: Any]] {
    var map: [String: [String: Any]] = [:]
    for item in recordEvidence {
      if let id = item["id"] as? String { map[id] = item }
    }
    return map
  }

  // 样本号/结构项号 -> 证据条目。智能体嵌套条目只有 evidence_id，要回查
  // record.evidence 里的会话记录；基线一条证据覆盖多个结构项（scenarios 数组）。
  private var evidenceBySample: [String: [String: Any]] {
    let byID = recordEvidenceByID
    var map: [String: [String: Any]] = [:]
    for item in nestedEvidence {
      var resolved = item
      if let evidenceID = item["evidence_id"] as? String, let real = byID[evidenceID] {
        resolved = real
      }
      let sampleID = item["sample_id"] as? String
        ?? (resolved["payload"] as? [String: Any])?["sample_id"] as? String
      if let sampleID { map[sampleID] = resolved }
      for scenario in item["scenarios"] as? [String] ?? [] { map[scenario] = resolved }
    }
    for item in recordEvidence {
      guard (item["id"] as? String ?? "").contains("-\(backendID)-"),
        let sampleID = (item["payload"] as? [String: Any])?["sample_id"] as? String
      else { continue }
      if map[sampleID] == nil { map[sampleID] = item }
    }
    return map
  }

  // 逐条判定以报告权威字段为准：spec.observations / scorecard.observations /
  // perf.samples / agent.samples / baseline.comparisons。
  private var taskSpecs: [TaskSpec] {
    switch backendID {
    case "specification":
      return (report?["observations"] as? [[String: Any]] ?? []).map { observation in
        let (verdict, text) = Self.specVerdict(observation["status"] as? String)
        var note = observation["limitation"] as? String
          ?? observation["verified_scope"] as? String ?? ""
        if (observation["attempt"] as? String) == "recheck" {
          note = note.isEmpty ? "复核后判定" : "复核：\(note)"
        }
        return TaskSpec(
          id: observation["sample_id"] as? String ?? "", verdict: verdict, text: text,
          note: note, source: observation)
      }
    case "capability":
      return (scorecard?["observations"] as? [[String: Any]] ?? []).map { observation in
        let (verdict, text) = Self.capabilityVerdict(observation["label"] as? String)
        return TaskSpec(
          id: observation["sample_id"] as? String ?? "", verdict: verdict, text: text,
          note: observation["reason"] as? String ?? "", source: observation)
      }
    case "performance":
      return (report?["samples"] as? [[String: Any]] ?? []).map { sample in
        let warmup = (sample["phase"] as? String) == "warmup"
        let (verdict, text) = warmup
          ? (.unverified, "预热") : Self.performanceVerdict(sample["terminal_state"] as? String)
        var parts: [String] = []
        if warmup { parts.append("预热请求，不计入正式结论") }
        if let limitation = sample["limitation"] as? String { parts.append(limitation) }
        let concurrency = (sample["target_concurrency"] as? NSNumber)?.intValue ?? 0
        if concurrency > 1 { parts.append("并发档位 \(concurrency)") }
        return TaskSpec(
          id: sample["id"] as? String ?? "", verdict: verdict, text: text,
          note: parts.joined(separator: "；"), source: sample)
      }
    case "agent":
      let scenarios = report?["scenarios"] as? [[String: Any]] ?? []
      return (report?["samples"] as? [[String: Any]] ?? []).map { sample in
        let (verdict, text) = Self.agentVerdict(sample["outcome"] as? String)
        let failedCheck = (sample["check_results"] as? [[String: Any]] ?? [])
          .first { ($0["status"] as? String) == "fail" }
          .map { check in
            "「\(Self.agentCheckTitle(check["check"] as? String))」\(check["rationale"] as? String ?? "")"
          }
        var note = failedCheck
          ?? (sample["limitations"] as? [String])?.first ?? ""
        if note.isEmpty && verdict == .pass { note = "各检查项均通过" }
        var source = sample
        if var spec = scenarios.first(where: {
          ($0["scenario"] as? String) == (sample["scenario"] as? String)
        }) {
          // 工作区材料/初始文件体量大，原始数据区不内联
          spec["materials"] = "（工作区材料见导出报告）"
          spec.removeValue(forKey: "workspace")
          source["scenario_spec"] = spec
        }
        return TaskSpec(
          id: sample["sample_id"] as? String ?? "", verdict: verdict, text: text,
          note: note, source: source)
      }
    case "baseline":
      return (report?["comparisons"] as? [[String: Any]] ?? []).map { comparison in
        let (verdict, text) = Self.baselineVerdict(comparison["status"] as? String)
        let diffs = (comparison["differences"] as? [[String: Any]] ?? []).compactMap { d in
          (d["detail"] as? String).map {
            let path = d["path"] as? String ?? ""
            return path.isEmpty ? $0 : "\(path)：\($0)"
          }
        }
        let note = (diffs + (comparison["notes"] as? [String] ?? []))
          .prefix(2).joined(separator: "；")
        return TaskSpec(
          id: comparison["scenario"] as? String ?? "", verdict: verdict, text: text,
          note: note, source: comparison)
      }
    default: return []
    }
  }

  private var tasks: [RealTaskReport] {
    if !record.isRealReport { return demoTasks }
    let specs = taskSpecs
    if !specs.isEmpty {
      let evidence = evidenceBySample
      return specs.map { spec in
        let item = evidence[spec.id]
        return RealTaskReport(
          id: spec.id,
          status: item.map { statusText(from: $0) } ?? "已记录",
          verdict: spec.verdict,
          verdictText: spec.text,
          note: spec.note,
          request: requestText(for: spec, item: item),
          response: responseText(for: spec, item: item),
          evidenceJSON: evidenceJSON(for: spec, item: item),
          curl: item.map { curlText(from: $0) } ?? "",
          rawResponse: item.map { responseJSON(from: $0) } ?? "")
      }
    }
    // 报告体缺失（如执行中途无效）时退回证据列表，按传输状态粗判
    return evidenceItems.enumerated().map { index, item in
      let itemID = item["id"] as? String ?? "任务 \(index + 1)"
      let payload = item["payload"] as? [String: Any] ?? [:]
      let nested = payload["payload"] as? [String: Any]
      let sampleID = item["sample_id"] as? String
        ?? payload["sample_id"] as? String
        ?? nested?["sample_id"] as? String
        ?? (item["scenarios"] as? [String])?.first
        ?? item["scenario"] as? String
        ?? payload["scenario"] as? String
        ?? nested?["scenario"] as? String
        ?? itemID
      let status = statusText(from: item)
      let pass = status == "请求成功"
      return RealTaskReport(
        id: sampleID, status: status,
        verdict: pass ? .pass : .fail,
        verdictText: pass ? "通过" : "未通过",
        note: "",
        request: requestText(from: item),
        response: responseText(from: item),
        evidenceJSON: prettyJSON(item),
        curl: curlText(from: item),
        rawResponse: responseJSON(from: item))
    }
  }

  private var evidenceItems: [[String: Any]] {
    recordEvidence.flatMap { item -> [[String: Any]] in
      let id = item["id"] as? String ?? ""
      let payload = item["payload"] as? [String: Any] ?? [:]
      guard payload["module"] as? String == backendID || id.contains("-\(backendID)-") else {
        return []
      }
      let nestedPayload = payload["payload"] as? [String: Any]
      if let nested = nestedPayload?["evidence"] as? [[String: Any]], !nested.isEmpty {
        return nested.map { evidence in
          var copy = evidence
          copy["kind"] = item["kind"] ?? "检测任务"
          return copy
        }
      }
      return [item]
    }
  }

  private static func specVerdict(_ status: String?) -> (RealTaskReport.Verdict, String) {
    switch status {
    case "accepted", "effective": return (.pass, "通过")
    case "verified_range": return (.pass, "部分验证")
    case "failed": return (.fail, "未通过")
    case "unsupported": return (.unverified, "不支持")
    case "inconclusive": return (.unverified, "无法判定")
    case "not_applicable": return (.unverified, "不适用")
    default: return (.unverified, "已记录")
    }
  }
  private static func capabilityVerdict(_ label: String?) -> (RealTaskReport.Verdict, String) {
    switch label {
    case "correct": return (.pass, "答对")
    case "wrong": return (.fail, "答错")
    case "pending": return (.unverified, "未评分")
    case "incomplete": return (.unverified, "未完成")
    case "missing": return (.unverified, "缺失")
    default: return (.unverified, "已记录")
    }
  }
  private static func performanceVerdict(_ state: String?) -> (RealTaskReport.Verdict, String) {
    switch state {
    case "natural_end", "completed", "truncated": return (.pass, "正常完成")
    case "error": return (.fail, "出错")
    case "timeout": return (.fail, "超时")
    case "cancelled": return (.unverified, "已取消")
    case "not_measured": return (.unverified, "未测到")
    default: return (.unverified, "已记录")
    }
  }
  private static func agentVerdict(_ outcome: String?) -> (RealTaskReport.Verdict, String) {
    switch outcome {
    case "pass": return (.pass, "通过")
    case "fail": return (.fail, "未通过")
    case "inconclusive": return (.unverified, "无法判定")
    case "not_applicable": return (.unverified, "不适用")
    case "not_measured": return (.unverified, "未执行")
    default: return (.unverified, "已记录")
    }
  }
  private static func baselineVerdict(_ status: String?) -> (RealTaskReport.Verdict, String) {
    switch status {
    case "same_structure": return (.pass, "一致")
    case "different": return (.fail, "有差异")
    case "not_observed": return (.unverified, "未观测")
    case "inconclusive": return (.unverified, "无法判定")
    case "not_applicable": return (.unverified, "不适用")
    default: return (.unverified, "已记录")
    }
  }
  private static func agentCheckTitle(_ id: String?) -> String {
    [
      "A1": "遵守任务规则", "A2": "工具与参数正确", "A3": "使用工具返回",
      "A4": "多轮状态保持", "A5": "工具失败处理", "A6": "信息缺失处理",
      "A7": "操作权限边界", "A8": "真实交付与结束",
    ][id ?? ""] ?? "检查项"
  }

  private var failCount: Int { tasks.filter { $0.verdict == .fail }.count }
  private var unverifiedCount: Int { tasks.filter { $0.verdict == .unverified }.count }
  private var passCount: Int { tasks.count - failCount - unverifiedCount }

  // 分组：小项 -> 样本列表（保持目录顺序，未识别的归入「其他检查」）。
  fileprivate struct GroupedSamples {
    var group: RealResultCatalog.Group
    var samples: [RealTaskReport]
    var correct: Int { samples.filter(\.pass).count }
  }
  private var groupedSamples: [String: GroupedSamples] {
    guard let catalog = catalog else { return [:] }
    var buckets: [String: [RealTaskReport]] = [:]
    for task in tasks {
      buckets[task.group ?? catalog.groupID(for: task.id), default: []].append(task)
    }
    var result: [String: GroupedSamples] = [:]
    for group in catalog.groups {
      if let samples = buckets[group.id] {
        result[group.id] = GroupedSamples(group: group, samples: samples)
      }
    }
    if let others = buckets["_other"], !others.isEmpty {
      result["_other"] = GroupedSamples(
        group: .init(
          id: "_other", name: "其他检查", passNote: "全部正常", failNote: "有未通过的检查",
          how: "该检查项在本次检测中产生了记录。", judge: "按返回状态判定。"),
        samples: others)
    }
    return result
  }
  private var catalog: RealResultCatalog? {
    record.isRealReport
      ? RealResultCatalog.catalog(for: backendID)
      : RealResultCatalog.demoCatalog(for: backendID)
  }

  var body: some View {
    VStack(alignment: .leading, spacing: 24) {
      VerdictBanner(
        style: stateStyle,
        title: "\(module.title)：\(headline)",
        detail: bannerDetail,
        meta: bannerMeta
      )

      VStack(alignment: .leading, spacing: 0) {
        if groupedSamples.isEmpty {
          Text("没有找到该模块的任务级证据。")
            .font(Theme.bodyFont)
            .foregroundStyle(Theme.faint)
            .padding(.vertical, 18)
        } else {
          ForEach(orderedGroups, id: \.group.id) { grouped in
            RealGroupRow(
              grouped: grouped,
              unitName: catalog?.unitName ?? "项",
              sampleName: catalog?.sampleName ?? { index, _ in "第 \(index) 项检查" })
          }
        }
      }
      .padding(18)
      .background(.white, in: RoundedRectangle(cornerRadius: Theme.radiusCard))
      .overlay(RoundedRectangle(cornerRadius: Theme.radiusCard).stroke(Theme.line))

      if let scope = catalog?.scope {
        HStack(alignment: .top, spacing: 8) {
          Image(systemName: "info.circle")
            .font(.system(size: 13))
            .foregroundStyle(Theme.faint)
            .padding(.top, 2)
          Text(scope)
            .font(Theme.captionFont)
            .foregroundStyle(Theme.muted)
            .fixedSize(horizontal: false, vertical: true)
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.white, in: RoundedRectangle(cornerRadius: 10))
        .overlay(RoundedRectangle(cornerRadius: 10).stroke(Theme.line))
      }
    }
  }

  private var orderedGroups: [GroupedSamples] {
    let order = catalog?.groups.map(\.id) ?? []
    return groupedSamples.values.sorted { left, right in
      let l = order.firstIndex(of: left.group.id) ?? order.count
      let r = order.firstIndex(of: right.group.id) ?? order.count
      return l < r
    }
  }

  private var headline: String {
    if failCount > 0 { return "有问题" }
    switch state {
    case "pass": return unverifiedCount > 0 ? "通过，部分项未判定" : "通过"
    case "fail": return "有问题"
    case "unsupported": return "不支持"
    case "invalid_execution": return "本次检测未完成"
    case "inconclusive": return "证据不足，暂不能判断"
    case "not_applicable": return "不适用"
    case "not_selected": return "本次未选"
    case "unverified": return "缺少判定证据"
    default: return "尚未检测"
    }
  }
  private var bannerDetail: String {
    guard let catalog, !tasks.isEmpty else {
      return moduleResult?["reason"] as? String ?? "该模块已完成真实执行；详细任务证据见下方。"
    }
    return catalog.desc
  }
  private var bannerMeta: String? {
    guard !tasks.isEmpty else { return nil }
    if failCount == 0 && unverifiedCount == 0 {
      return "\(tasks.count) \(catalog?.unitName ?? "项")全部正常 · 每项都是真实调用"
    }
    var parts = ["\(passCount) 项正常"]
    if failCount > 0 { parts.append("\(failCount) 项未通过") }
    if unverifiedCount > 0 { parts.append("\(unverifiedCount) 项未判定") }
    return parts.joined(separator: " · ")
  }

  private func requestText(from item: [String: Any]) -> String {
    let payload = item["payload"] as? [String: Any] ?? item
    let nested = payload["payload"] as? [String: Any] ?? payload
    let request = (nested["request"] as? [String: Any]) ?? (payload["request"] as? [String: Any])
    return request?["prompt"] as? String ?? "未记录请求内容"
  }
  private func requestJSON(from item: [String: Any]) -> String {
    let payload = item["payload"] as? [String: Any] ?? item
    let nested = payload["payload"] as? [String: Any] ?? payload
    let request = (nested["request"] as? [String: Any]) ?? (payload["request"] as? [String: Any])
    guard let request, JSONSerialization.isValidJSONObject(request) else { return "" }
    return prettyJSON(request)
  }
  private func responseJSON(from item: [String: Any]) -> String {
    let payload = item["payload"] as? [String: Any] ?? item
    let nested = payload["payload"] as? [String: Any] ?? payload
    let response = (nested["response"] as? [String: Any]) ?? (payload["response"] as? [String: Any])
    // 响应体在 response.body；外层的 attempts/elapsed_ms/status 是传输元信息，
    // 不属于「curl 请求返回的完整 JSON」。
    let body = response?["body"] as? [String: Any]
    guard let body, JSONSerialization.isValidJSONObject(body) else { return "" }
    return prettyJSON(body)
  }
  // 原始 curl 命令：请求体取证据里的 request 对象；拿不到就空着，界面上回退显示请求文本。
  private func curlText(from item: [String: Any]) -> String {
    guard backendID != "agent" else { return "" }
    let body = requestJSON(from: item)
    guard !body.isEmpty else { return "" }
    return curlCommand(bodyJSON: body)
  }
  private func curlCommand(bodyJSON: String) -> String {
    "curl -sS -X POST \"\(record.service.displayURL)\" \\\n"
      + "  -H \"Content-Type: application/json\" \\\n"
      + "  -H \"Authorization: Bearer $API_KEY\" \\\n"
      + "  -d '\(bodyJSON)'"
  }
  // 演示记录的 curl：没有真实 HTTP 往返，按各模块请求形态合成代表性请求体。
  private func demoCurl(body: [String: Any]) -> String {
    var payload = body
    payload["model"] = record.service.model
    return curlCommand(bodyJSON: prettyJSON(payload))
  }
  // 演示记录的响应体：合成与真实记录同形态的 chat.completion，
  // 让「原始返回（JSON）」评审时看到的就是真实报告会展示的东西。
  private func demoResponseBody(content: String) -> String {
    let promptTokens = 64
    let completionTokens = max(1, content.count / 3)
    return prettyJSON([
      "id": "chatcmpl-demo",
      "object": "chat.completion",
      "created": 0,
      "model": record.service.model,
      "choices": [[
        "index": 0,
        "finish_reason": "stop",
        "message": ["role": "assistant", "content": content],
      ]],
      "usage": [
        "prompt_tokens": promptTokens,
        "completion_tokens": completionTokens,
        "total_tokens": promptTokens + completionTokens,
      ],
    ])
  }
  private func responseText(from item: [String: Any]) -> String {
    let payload = item["payload"] as? [String: Any] ?? item
    let nested = payload["payload"] as? [String: Any] ?? payload
    let response = (nested["response"] as? [String: Any]) ?? (payload["response"] as? [String: Any])
    if let stream = response?["stream"] as? [String: Any],
      let content = stream["content"] as? String, !content.isEmpty
    { return content }
    if let body = response?["body"] as? [String: Any],
      let choices = body["choices"] as? [[String: Any]],
      let message = choices.first?["message"] as? [String: Any],
      let content = message["content"] as? String, !content.isEmpty
    { return content }
    if let error = response?["error"] as? String, !error.isEmpty { return error }
    return "未记录文本响应；请展开原始数据查看。"
  }
  private func statusText(from item: [String: Any]) -> String {
    let payload = item["payload"] as? [String: Any] ?? item
    let nested = payload["payload"] as? [String: Any] ?? payload
    let response = (nested["response"] as? [String: Any]) ?? (payload["response"] as? [String: Any])
    if let error = response?["error"] as? String, !error.isEmpty { return "请求异常" }
    if let status = response?["status"] as? Int {
      return status >= 200 && status < 300 ? "请求成功" : "HTTP \(status)"
    }
    return "已记录"
  }
  private func prettyJSON(_ value: [String: Any]) -> String {
    guard JSONSerialization.isValidJSONObject(value),
      let data = try? JSONSerialization.data(
        withJSONObject: value, options: [.prettyPrinted, .sortedKeys])
    else { return "无法展示原始数据" }
    return String(decoding: data, as: UTF8.self)
  }

  // 各模块的「发了什么 / 回了什么」来源不同：普通模块取证据里的 HTTP 往返，
  // 智能体取场景任务书和最终消息，基线看的是原始返回体本身。
  private func requestText(for spec: TaskSpec, item: [String: Any]?) -> String {
    if backendID == "agent",
      let scenarioSpec = spec.source?["scenario_spec"] as? [String: Any]
    {
      var lines = ["任务：\(scenarioSpec["task"] as? String ?? "")"]
      for (index, turn) in (scenarioSpec["turns"] as? [String] ?? []).enumerated() {
        lines.append("追问 \(index + 1)：\(turn)")
      }
      return lines.joined(separator: "\n\n")
    }
    return item.map { requestText(from: $0) } ?? "未记录请求内容"
  }
  private func responseText(for spec: TaskSpec, item: [String: Any]?) -> String {
    if backendID == "agent" {
      let attempts = (report?["attempts"] as? [[String: Any]] ?? [])
        .filter { ($0["sample_id"] as? String) == spec.id }
      if let message = attempts.last?["final_message"] as? String, !message.isEmpty {
        return message
      }
      if let session = (item?["payload"] as? [String: Any])?["session"] as? String,
        !session.isEmpty
      {
        return "未产出最终消息。会话日志末尾：\n\(session.suffix(1200))"
      }
      return "未记录最终输出；请展开原始数据查看会话。"
    }
    if backendID == "baseline", let item {
      // 结构对比看的是返回体本身，不做正文抽取
      let payload = item["payload"] as? [String: Any] ?? item
      let nested = payload["payload"] as? [String: Any] ?? payload
      let response = (nested["response"] as? [String: Any]) ?? (payload["response"] as? [String: Any])
      if let body = response?["body"], JSONSerialization.isValidJSONObject(body),
        let data = try? JSONSerialization.data(
          withJSONObject: body, options: [.prettyPrinted, .sortedKeys])
      {
        return String(decoding: data, as: UTF8.self)
      }
      if let stream = response?["stream"] as? [String: Any],
        let content = stream["content"] as? String, !content.isEmpty
      {
        return content
      }
    }
    return item.map { responseText(from: $0) } ?? "未记录响应内容"
  }
  private func evidenceJSON(for spec: TaskSpec, item: [String: Any]?) -> String {
    var merged: [String: Any] = [:]
    if let source = spec.source { merged["report_result"] = source }
    if var item {
      if var payload = item["payload"] as? [String: Any],
        let session = payload["session"] as? String, session.count > 4000
      {
        payload["session"] = "…（会话日志过长，只保留末尾 4000 字符）\n" + session.suffix(4000)
        item["payload"] = payload
      }
      merged["evidence"] = item
    }
    guard !merged.isEmpty else { return "无法展示原始数据" }
    return prettyJSON(merged)
  }
}

private extension RealTaskReport {
  var pass: Bool { verdict == .pass }
}

// 分组的判定摘要：页面行与 HTML 导出共用，保证两处文案永远一致。
extension RealModuleView.GroupedSamples {
  var failCount: Int { samples.filter { $0.verdict == .fail }.count }
  var unverifiedCount: Int { samples.filter { $0.verdict == .unverified }.count }
  var allPass: Bool { samples.allSatisfy(\.pass) }
  var groupVerdict: RealTaskReport.Verdict {
    if failCount > 0 { return .fail }
    return unverifiedCount > 0 ? .unverified : .pass
  }
  func note(unitName: String) -> String {
    let total = samples.count
    if failCount > 0 {
      let ratio = Double(total - failCount) / Double(total)
      if ratio >= 0.9 { return "基本可用，个别\(unitName)没通过（\(failCount) \(unitName)）" }
      if ratio >= 0.7 { return "有 \(failCount) \(unitName)未通过，建议关注" }
      return group.failNote
    }
    if unverifiedCount > 0 { return "有 \(unverifiedCount) \(unitName)未能判定" }
    return group.passNote
  }
  func countText(unitName: String) -> String {
    if unitName == "题" { return "\(correct) / \(samples.count) 题正确" }
    return allPass ? "\(samples.count) / \(samples.count)" : "\(correct) / \(samples.count)"
  }
}

// 小项分组行：状态一枚徽章 + 计数，展开是逐条检查。
private struct RealGroupRow: View {
  let grouped: RealModuleView.GroupedSamples
  let unitName: String
  let sampleName: (Int, String) -> String
  @State private var expanded = false

  private var failCount: Int { grouped.failCount }
  private var allPass: Bool { grouped.allPass }
  private var rowIcon: String {
    allPass ? "checkmark.circle.fill"
      : failCount > 0 ? "exclamationmark.circle.fill" : "questionmark.circle.fill"
  }
  private var rowColor: Color {
    allPass ? Theme.passBar : failCount > 0 ? Theme.limitedBar : Theme.unknown
  }

  var body: some View {
    VStack(spacing: 0) {
      HStack(spacing: 13) {
        Image(systemName: rowIcon)
          .font(.system(size: 18))
          .foregroundStyle(rowColor)
        VStack(alignment: .leading, spacing: 3) {
          Text(grouped.group.name)
            .font(.system(size: 13.5, weight: .semibold))
          Text(groupNote)
            .font(Theme.captionFont)
            .foregroundStyle(Theme.muted)
            .fixedSize(horizontal: false, vertical: true)
        }
        Spacer()
        Text(countText)
          .font(Theme.captionFont.monospacedDigit())
          .foregroundStyle(rowColor)
        Image(systemName: "chevron.right")
          .font(.system(size: 9, weight: .semibold))
          .foregroundStyle(Theme.faint)
          .rotationEffect(.degrees(expanded ? 90 : 0))
          .animation(.easeInOut(duration: 0.25), value: expanded)
      }
      .padding(.vertical, 13)
      .contentShape(Rectangle())
      .onTapGesture { expanded.toggle() }
      if expanded {
        ForEach(Array(grouped.samples.enumerated()), id: \.element.id) { index, sample in
          RealSampleRow(
            sample: sample,
            displayName: sample.displayName ?? sampleName(index + 1, sample.id),
            how: grouped.group.how,
            judge: grouped.group.judge)
        }
      }
    }
    .overlay(alignment: .bottom) { Rectangle().fill(Theme.line).frame(height: 1) }
    .animation(.easeInOut(duration: 0.3), value: expanded)
  }

  private var countText: String { grouped.countText(unitName: unitName) }
  private var groupNote: String { grouped.note(unitName: unitName) }
}

// 单条检查行：人话名称 + 判定 pill；展开 = 检测方式 / 判定方式（默认显示）+
// 原始 curl 请求 / 原始返回 JSON（默认折叠）。
private struct RealSampleRow: View {
  let sample: RealTaskReport
  let displayName: String
  let how: String
  let judge: String
  @State private var expanded = false
  @State private var curlVisible = false
  @State private var respVisible = false

  private var dotColor: Color {
    switch sample.verdict {
    case .pass: return Theme.passBar
    case .fail: return Theme.limitedBar
    case .unverified: return Theme.faint
    }
  }
  private var pillColor: Color {
    switch sample.verdict {
    case .pass: return Theme.pass
    case .fail: return Theme.limited
    case .unverified: return Theme.unknown
    }
  }
  private var pillTint: Color {
    switch sample.verdict {
    case .pass: return Theme.passTint
    case .fail: return Theme.limitedTint
    case .unverified: return Theme.unknownTint
    }
  }

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      HStack(spacing: 10) {
        Circle()
          .fill(dotColor)
          .frame(width: 8, height: 8)
        Text(displayName)
          .font(.system(size: 12.5))
        Spacer()
        Text(sample.verdictText)
          .font(.system(size: 11, weight: .medium))
          .foregroundStyle(pillColor)
          .padding(.horizontal, 9)
          .padding(.vertical, 3)
          .background(pillTint, in: Capsule())
      }
      .padding(.vertical, 10)
      .contentShape(Rectangle())
      .onTapGesture { expanded.toggle() }
      if expanded {
        VStack(alignment: .leading, spacing: 12) {
          infoBlock(
            "检测方式",
            how)
          infoBlock(
            "判定方式",
            judge + (sample.note.isEmpty ? "" : "\n本次结果：\(sample.note)"))
          disclosureLink(
            curlVisible ? "收起原始请求" : requestLabel,
            visible: $curlVisible)
          if curlVisible {
            CodeReader(
              title: "原始请求",
              value: sample.curl.isEmpty ? sample.request : sample.curl)
          }
          disclosureLink(
            respVisible ? "收起\(responseTitle)" : "查看\(responseTitle)",
            visible: $respVisible)
          if respVisible {
            if sample.rawResponse.isEmpty {
              EvidenceField(title: responseTitle, value: responseEmptyNote)
            } else {
              CodeReader(title: responseTitle, value: sample.rawResponse, searchable: true)
            }
          }
        }
        .padding(.leading, 18)
        .padding(.bottom, 12)
      }
    }
    .padding(.leading, 33)
    .overlay(alignment: .bottom) {
      Rectangle().fill(Theme.line.opacity(0.6)).frame(height: 1)
    }
    .animation(.easeInOut(duration: 0.25), value: expanded)
  }

  // 有 curl 命令就亮 curl 字样；智能体这类非单次 HTTP 的检查退化为「原始请求」。
  private var requestLabel: String {
    sample.curl.isEmpty ? "查看原始请求" : "查看原始 curl 请求"
  }
  // 对称地，响应侧：单次 HTTP 检查显示「原始返回」，智能体任务显示「任务执行记录」。
  private var responseTitle: String {
    sample.curl.isEmpty ? "任务执行记录（JSON）" : "原始返回（JSON）"
  }
  private var responseEmptyNote: String {
    sample.curl.isEmpty ? "本次没有取得任务执行记录" : "本次没有取得响应体"
  }

  private func infoBlock(_ title: String, _ text: String) -> some View {
    VStack(alignment: .leading, spacing: 4) {
      Text(title)
        .font(.system(size: 11.5, weight: .semibold))
        .foregroundStyle(Theme.muted)
      Text(text)
        .font(Theme.bodyFont)
        .foregroundStyle(Theme.ink)
        .fixedSize(horizontal: false, vertical: true)
    }
  }

  private func disclosureLink(_ title: String, visible: Binding<Bool>) -> some View {
    Button {
      visible.wrappedValue.toggle()
    } label: {
      Label(
        title,
        systemImage: visible.wrappedValue ? "chevron.up" : "chevron.down")
        .font(.system(size: 11.5, weight: .medium))
    }
    .buttonStyle(.plain)
    .foregroundStyle(Theme.accent)
  }
}

// 原始证据阅读器:行数前置、超长折叠中段、搜索命中自动展开并高亮、一键复制。
// 展开大响应不再刷屏。
private struct CodeReader: View {
  let title: String
  let value: String
  var searchable = false
  @State private var query = ""
  @State private var expandedAll = false

  private static let threshold = 200
  private static let headCount = 100
  private static let tailCount = 60

  private var lines: [String] { value.components(separatedBy: "\n") }
  private var searching: Bool { searchable && !query.isEmpty }

  private struct DisplayLine {
    var number: Int
    var text: String
    var highlight: String?
  }

  private var display: (lines: [DisplayLine], note: String?, expandable: Bool) {
    let all = lines
    if searching {
      let matched = all.enumerated().filter { $0.element.localizedCaseInsensitiveContains(query) }
      return (
        matched.map {
          DisplayLine(number: $0.offset + 1, text: $0.element, highlight: query)
        },
        matched.isEmpty ? "没有命中「\(query)」" : "命中 \(matched.count) 行",
        false
      )
    }
    if expandedAll || all.count <= Self.threshold {
      return (all.enumerated().map { DisplayLine(number: $0.offset + 1, text: $0.element, highlight: nil) }, nil, false)
    }
    let head = all.prefix(Self.headCount)
    let tail = all.suffix(Self.tailCount)
    var shown: [DisplayLine] = []
    shown += head.enumerated().map { DisplayLine(number: $0.offset + 1, text: $0.element, highlight: nil) }
    let tailStart = all.count - Self.tailCount
    shown += tail.enumerated().map {
      DisplayLine(number: tailStart + $0.offset + 1, text: $0.element, highlight: nil)
    }
    let omitted = all.count - Self.headCount - Self.tailCount
    return (shown, "已折叠中段 \(omitted) 行（搜索命中的字段会自动展开）", true)
  }

  var body: some View {
    let shown = display
    return VStack(alignment: .leading, spacing: 7) {
      HStack(spacing: 8) {
        Text(title).font(.system(size: 11.5, weight: .medium)).foregroundStyle(Theme.accent)
        Text("\(lines.count) 行")
          .font(.system(size: 10.5)).foregroundStyle(Theme.faint)
          .monospacedDigit()
        Spacer()
        if searchable {
          HStack(spacing: 4) {
            Image(systemName: "magnifyingglass").font(.system(size: 8.5)).foregroundStyle(Theme.faint)
            TextField("搜字段…", text: $query)
              .textFieldStyle(.plain)
              .font(.system(size: 10.5))
              .frame(width: 96)
          }
          .padding(.horizontal, 8).frame(height: 22)
          .background(.white, in: RoundedRectangle(cornerRadius: 6))
          .overlay(
            RoundedRectangle(cornerRadius: 6)
              .stroke(searching ? Theme.accent.opacity(0.4) : Theme.line))
        }
        Button {
          NSPasteboard.general.clearContents()
          NSPasteboard.general.setString(value, forType: .string)
        } label: {
          Label("复制", systemImage: "doc.on.doc")
            .font(.system(size: 10.5, weight: .medium))
        }
        .buttonStyle(.plain)
        .foregroundStyle(Theme.muted)
        .help("复制全文")
      }
      ScrollView {
        VStack(alignment: .leading, spacing: 0) {
          ForEach(Array(shown.lines.enumerated()), id: \.offset) { _, line in
            HStack(alignment: .top, spacing: 0) {
              Text("\(line.number)")
                .font(.system(size: 10).monospaced())
                .foregroundStyle(Color(red: 0.71, green: 0.76, blue: 0.81))
                .frame(width: 34, alignment: .trailing)
                .padding(.trailing, 10)
              highlighted(line.text, query: line.highlight)
                .font(.system(size: 11).monospaced())
                .foregroundStyle(Theme.ink)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(.vertical, 0.5)
          }
        }
        .padding(.vertical, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
      }
      .frame(maxHeight: 380)
      .background(Color(red: 0.973, green: 0.98, blue: 0.984), in: RoundedRectangle(cornerRadius: 9))
      .overlay(RoundedRectangle(cornerRadius: 9).stroke(Color(red: 0.91, green: 0.93, blue: 0.95)))
      if let note = shown.note {
        HStack {
          Rectangle().fill(Theme.line).frame(height: 1)
          Text(note).font(.system(size: 10)).foregroundStyle(Theme.faint).fixedSize()
          Rectangle().fill(Theme.line).frame(height: 1)
        }
      }
      if shown.expandable {
        Button {
          expandedAll = true
        } label: {
          Text("展开全部 \(lines.count) 行")
            .font(.system(size: 11, weight: .semibold))
        }
        .buttonStyle(.plain)
        .foregroundStyle(Theme.accent)
        .frame(maxWidth: .infinity)
      }
    }
    .animation(.easeInOut(duration: 0.15), value: query)
    .animation(.easeInOut(duration: 0.15), value: expandedAll)
  }

  // 命中片段加粗提亮;SwiftUI 的 Text 拼接足够,不引第三方高亮。
  private func highlighted(_ line: String, query: String?) -> Text {
    guard let query, !query.isEmpty,
      let range = line.range(of: query, options: .caseInsensitive)
    else { return Text(line) }
    let before = String(line[..<range.lowerBound])
    let match = String(line[range])
    let after = String(line[range.upperBound...])
    return Text(before)
      + Text(match).fontWeight(.bold).foregroundColor(Theme.limitedBar)
      + Text(after)
  }
}

private struct EvidenceField: View {
  let title: String
  let value: String
  var monospaced = false

  var body: some View {
    VStack(alignment: .leading, spacing: 6) {
      Text(title).font(.system(size: 11.5, weight: .semibold)).foregroundStyle(Theme.muted)
      Text(value)
        .font(monospaced ? .system(size: 10, design: .monospaced) : Theme.bodyFont)
        .foregroundStyle(Theme.ink)
        .textSelection(.enabled)
        .fixedSize(horizontal: false, vertical: true)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(10)
        .background(Theme.canvas, in: RoundedRectangle(cornerRadius: 6))
    }
  }
}

// ---------- 演示记录适配：与真实报告共用同一套分组折叠骨架 ----------
private extension RealModuleView {
  var demoTasks: [RealTaskReport] {
    switch backendID {
    case "specification": return demoSpecTasks
    case "capability": return demoCapabilityTasks
    case "performance": return demoPerformanceTasks
    case "agent": return demoAgentTasks
    case "baseline": return demoBaselineTasks
    default: return []
    }
  }

  var demoSpecTasks: [RealTaskReport] {
    Catalog.specifications.map { item in
      let (verdict, verdictText): (RealTaskReport.Verdict, String) = {
        switch item.state {
        case .observed: return (.pass, "通过")
        case .limited: return (.fail, "未通过")
        case .partial: return (.unverified, "部分覆盖")
        default: return (.unverified, "未判定")
        }
      }()
      let group = ["S1", "S2"].contains(item.id) ? "S01" : "S0\(item.id.dropFirst())"
      let evidence = prettyJSON([
        "检查项": item.id, "判定": item.state.rawValue, "结论": item.value,
        "边界": item.boundary,
        "本次测量": item.facts.map { ["label": $0.label, "value": $0.value] },
        "逐次记录": item.evidence.map { ["id": $0.label, "detail": $0.value] },
      ])
      return RealTaskReport(
        id: item.id, status: "已记录", verdict: verdict, verdictText: verdictText,
        note: item.value,
        request: item.facts.map { "\($0.label)：\($0.value)" }.joined(separator: "\n"),
        response: "\(item.value)\n\n\(item.summary)\n\n结论边界：\(item.boundary)",
        evidenceJSON: evidence,
        curl: demoCurl(body: [
          "messages": [["role": "user", "content": "（检查项「\(item.title)」构造的测试输入）"]],
          "stream": item.title.contains("流式"),
        ]),
        rawResponse: demoResponseBody(
          content: "（检查项「\(item.title)」下模型返回的回复内容）"),
        group: group, displayName: item.title)
    }
  }

  var demoCapabilityTasks: [RealTaskReport] {
    // 「可以正常使用」的记录不应出现答错题，与整体结论保持一致
    let usable = record.outcome == .usable
    return Scores.all.flatMap { category in
      (category.cases + category.comparisonCases).map { entry in
        let passed = usable || entry.passed
        let actual = usable ? entry.expected : entry.actual
        let evidence = prettyJSON([
          "题目": entry.id, "输入": entry.input, "预期": entry.expected,
          "实际": actual, "判定": passed ? "答对" : "答错",
        ])
        return RealTaskReport(
          id: entry.id, status: "已评分",
          verdict: passed ? .pass : .fail,
          verdictText: passed ? "答对" : "答错",
          note: passed ? "" : "预期「\(entry.expected)」，实际「\(actual)」",
          request: entry.input,
          response: "预期结果：\(entry.expected)\n\n实际结果：\(actual)",
          evidenceJSON: evidence,
          curl: demoCurl(body: ["messages": [["role": "user", "content": entry.input]]]),
          rawResponse: demoResponseBody(content: actual),
          group: "C0\(category.id.dropFirst())")
      }
    }
  }

  var demoPerformanceTasks: [RealTaskReport] {
    Catalog.performance.flatMap { item in
      item.evidence.map { entry in
        // 否定语境（「无额外超时」）不算失败，只认明确的失败标记；
        // 「可以正常使用」的记录不出问题条目，与整体结论保持一致
        let negated = ["无额外超时", "无超时", "没有超时", "未出现"].contains { entry.value.contains($0) }
        let bad =
          record.outcome != .usable && !negated
          && ["超时", "服务错误", "未获得", "不完整"].contains {
            entry.value.contains($0) || entry.label.contains($0)
          }
        let evidence = prettyJSON([
          "维度": item.title, "判定": item.state.rawValue, "结论": item.value,
          "边界": item.boundary,
          "测试条件": item.facts.map { ["label": $0.label, "value": $0.value] },
          "本条记录": ["id": entry.label, "detail": entry.value],
        ])
        return RealTaskReport(
          id: "\(item.id)-\(entry.label)", status: "已测量",
          verdict: bad ? .fail : .pass,
          verdictText: bad ? (entry.value.contains("超时") ? "有超时" : "未通过") : "正常",
          note: bad ? entry.value : "",
          request: "测试条件\n"
            + item.facts.map { "\($0.label)：\($0.value)" }.joined(separator: "\n")
            + "\n\n边界：\(item.boundary)",
          response: "\(item.value)\n\n\(entry.label)：\(entry.value)",
          evidenceJSON: evidence,
          curl: demoCurl(body: [
            "messages": [["role": "user", "content": "（\(item.title) 负载测试请求）"]],
          ]),
          rawResponse: demoResponseBody(content: "（\(item.title) 负载下的模型回复内容）"),
          group: item.id, displayName: entry.label)
      }
    }
  }

  var demoAgentTasks: [RealTaskReport] {
    record.agentSamples.map { sample in
      let valid = sample.validRuns
      let failed = valid.filter { !$0.failedChecks.isEmpty }
      let verdict: RealTaskReport.Verdict =
        valid.isEmpty ? .unverified : failed.isEmpty ? .pass : .fail
      var note = ""
      if valid.isEmpty {
        note =
          sample.runs.isEmpty
          ? "本任务未执行" : "未取得有效执行（无效尝试 \(sample.runs.count) 次，不计为模型失败）"
      } else if !failed.isEmpty {
        note = "有效执行 \(valid.count) 次，其中 \(failed.count) 次未通过"
      }
      let response =
        sample.runs.isEmpty
        ? "没有执行记录"
        : sample.runs.map { run in
          let mark = !run.valid ? "无效" : run.failedChecks.isEmpty ? "通过" : "未通过"
          return "【\(run.id) · \(run.phase) · \(mark)】\n"
            + run.steps.joined(separator: "\n") + "\n交付：\(run.delivery)"
        }.joined(separator: "\n\n")
      let encoder = JSONEncoder()
      encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
      let json =
        (try? encoder.encode(sample)).map { String(decoding: $0, as: UTF8.self) }
        ?? "无法展示原始数据"
      return RealTaskReport(
        id: sample.id, status: "已记录", verdict: verdict,
        verdictText: valid.isEmpty ? "未执行" : failed.isEmpty ? "通过" : "未通过",
        note: note,
        request: "任务要求：\(sample.requirement)\n\n预期交付：\(sample.expected)",
        response: response, evidenceJSON: json, rawResponse: json,
        group: RealResultCatalog.demoAgentGroup(sample.id))
    }
  }

  var demoBaselineTasks: [RealTaskReport] {
    BaselineItem.forRecord(record).flatMap { item in
      item.responses.map { response in
        // 「可以正常使用」的记录不报结构差异，与整体结论保持一致
        let usable = record.outcome == .usable
        let (verdict, verdictText): (RealTaskReport.Verdict, String) = {
          if let unavailable = response.unavailable { return (.unverified, unavailable) }
          return usable || response.differences.isEmpty ? (.pass, "一致") : (.fail, "有差异")
        }()
        var body = usable && response.unavailable == nil ? "未发现字段或结构差异" : response.status
        if !usable && !response.differences.isEmpty {
          body +=
            "\n\n"
            + response.differences.map { "· \($0.path)：\($0.description)" }
            .joined(separator: "\n")
        }
        if !response.raw.isEmpty { body += "\n\n实际返回：\n\(response.raw)" }
        let evidence = prettyJSON([
          "结构项": item.id, "路径": item.path, "要求": item.requirement,
          "判定": response.status,
          "实际返回": response.actual.joined(separator: "\n"),
          "参考快照": response.reference.joined(separator: "\n"),
          "差异": response.differences.map {
            ["path": $0.path, "description": $0.description]
          },
        ])
        return RealTaskReport(
          id: response.id, status: "已对照", verdict: verdict, verdictText: verdictText,
          note:
            usable
            ? ""
            : response.differences.map { "\($0.path)：\($0.description)" }
            .prefix(2).joined(separator: "；"),
          request: "结构项 \(item.id)（\(item.path)）\n\n\(item.requirement)",
          response: body,
          evidenceJSON: evidence,
          curl: demoCurl(body: [
            "messages": [["role": "user", "content": "（结构项 \(item.id) 验证请求）"]],
          ]),
          rawResponse: demoResponseBody(
            content: response.actual.isEmpty
              ? "（结构项 \(item.id) 未取得实际返回）"
              : response.actual.joined(separator: "\n")),
          group: item.id, displayName: "\(response.id) · \(response.phase)")
      }
    }
  }
}

// MARK: - HTML 导出数据

// 页面与 HTML 导出共用同一份数据（同文件扩展可访问私有成员），
// 保证「所见即所导」：报告里出现的每句话，页面和导出永远一致。
extension RealModuleView {
  struct ExportSample {
    var name: String
    var verdictText: String
    var verdict: RealTaskReport.Verdict
    var note: String
    var how: String
    var judge: String
    var curl: String
    var request: String
    var rawResponse: String
    // 智能体这类非单次 HTTP 的检查：curl 为空，请求/返回按「任务」口径展示。
    var isTaskStyle: Bool { curl.isEmpty }
  }

  struct ExportGroup {
    var name: String
    var note: String
    var countText: String
    var verdict: RealTaskReport.Verdict
    var samples: [ExportSample]
  }

  struct ExportModule {
    var title: String
    var headline: String
    var detail: String
    var meta: String?
    var state: String
    var scope: String?
    var groups: [ExportGroup]

    var failCount: Int { groups.flatMap(\.samples).filter { $0.verdict == .fail }.count }
    // 与页面 stateStyle 同一口径：模块级 fail 用红；有失败项用琥珀；pass 绿；其余灰。
    var bannerClass: String {
      if state == "fail" { return "v-block" }
      if failCount > 0 { return "v-fail" }
      switch state {
      case "pass": return "v-pass"
      case "unsupported", "inconclusive", "invalid_execution": return "v-fail"
      default: return "v-unknown"
      }
    }
  }

  var exportModule: ExportModule {
    let unitName = catalog?.unitName ?? "项"
    let sampleNamer = catalog?.sampleName ?? { index, _ in "第 \(index) 项检查" }
    return ExportModule(
      title: module.title,
      headline: headline,
      detail: bannerDetail,
      meta: bannerMeta,
      state: state,
      scope: catalog?.scope,
      groups: orderedGroups.map { grouped in
        ExportGroup(
          name: grouped.group.name,
          note: grouped.note(unitName: unitName),
          countText: grouped.countText(unitName: unitName),
          verdict: grouped.groupVerdict,
          samples: grouped.samples.enumerated().map { index, sample in
            ExportSample(
              name: sample.displayName ?? sampleNamer(index + 1, sample.id),
              verdictText: sample.verdictText,
              verdict: sample.verdict,
              note: sample.note,
              how: grouped.group.how,
              judge: grouped.group.judge,
              curl: sample.curl,
              request: sample.request,
              rawResponse: sample.rawResponse)
          })
      })
  }
}
