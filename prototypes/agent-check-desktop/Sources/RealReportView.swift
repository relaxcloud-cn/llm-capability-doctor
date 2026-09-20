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
  }

  var groups: [Group]
  var unitName: String
  var cardTitle: String
  var cardCaption: String
  var sampleGroup: (String) -> String?
  var sampleName: (Int, String) -> String
  var bannerPass: (Int) -> String
  var bannerFail: (Int, Int) -> String
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
        failNote: "服务拒绝了部分长度的请求；超长业务内容需要先拆分或截断"),
      Group(
        id: "S04", name: "工具调用",
        passNote: "都能按格式返回工具调用",
        failNote: "有请求没有按格式返回工具调用；业务依赖工具调用的话需要先处理"),
      Group(
        id: "S05", name: "结构化输出",
        passNote: "JSON、按字段模板两种都能按格式输出",
        failNote: "部分格式要求没有满足；程序直接解析返回值的场景需要适配"),
      Group(
        id: "S06", name: "消息与多轮输入",
        passNote: "单轮、多轮、系统提示都能正确处理",
        failNote: "部分消息形态处理不正确；多轮对话业务建议验证"),
      Group(
        id: "S07", name: "流式输出",
        passNote: "流式返回能正常开始和正常结束",
        failNote: "流式返回未能正常开始或结束；流式场景需要先排查"),
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
      cardTitle: "五项检查的结果",
      cardCaption: "点开每一项可看当时的请求和回答",
      sampleGroup: { membership[$0] },
      sampleName: plain(names, fallback: { "第 \($0) 项检查" }),
      bannerPass: { _ in
        "我们用固定检查项问了这套服务：能接受多大的请求、工具调用、结构化输出、多轮对话和流式返回。全部得到了符合格式的回答。"
      },
      bannerFail: { pass, fail in
        "大部分检查正常，但有 \(fail) 项未通过（\(pass) 项正常）。建议展开对应分组看具体是哪一步；影响取决于你的业务是否用到该项。"
      },
      scope: "以上结论来自固定检查项，覆盖日常使用的请求形态；它说明「这些常规用法没问题」，不代表该模型的能力上限。")
  }()

  // ---------- 模型能力跑分 ----------
  private static let capability: RealResultCatalog = {
    let groups = [
      Group(id: "C01", name: "文本理解与指令执行", passNote: "这类题做得稳", failNote: "错得较多，重要指令建议复核"),
      Group(id: "C02", name: "信息提取与结构化填写", passNote: "这类题做得稳", failNote: "错得较多，提取结果建议复核"),
      Group(id: "C03", name: "工具选择与参数填写", passNote: "这类题做得稳", failNote: "近半数选择或参数出错，重点复核"),
      Group(id: "C04", name: "多轮对话与条件承接", passNote: "这类题做得稳", failNote: "错得较多，条件变化场景建议复核"),
      Group(id: "C05", name: "长材料理解与信息利用", passNote: "这类题做得稳", failNote: "错得较多，长材料结论建议复核"),
      Group(id: "C06", name: "逻辑推理与计算", passNote: "这类题做得稳", failNote: "错得最多；重要计算务必人工复核"),
    ]
    return RealResultCatalog(
      groups: groups,
      unitName: "题",
      cardTitle: "六类题目的表现",
      cardCaption: "点开可看答错题目的原文和模型的回答",
      sampleGroup: { id in id.split(separator: "-").first.map(String.init) },
      sampleName: { index, _ in "第 \(index) 题" },
      bannerPass: { total in "六类固定题目共 \(total) 道，全部回答正确。" },
      bannerFail: { correct, total in
        "六类固定题目共 \(total) 道，答对 \(correct) 道。哪类稳、哪类容易错，看下面的分组；错得多的类别建议留人工复核。"
      },
      scope: "题目为固定题库，衡量「这类任务它做得稳不稳」，不是通用智力评分；同题不同次作答可能有小幅波动。")
  }()

  // ---------- 模型性能实测 ----------
  private static let performance: RealResultCatalog = {
    let groups = [
      Group(id: "P01", name: "首字响应时间", passNote: "从发出请求到第一个字返回的等待正常", failNote: "部分请求的首字等待异常"),
      Group(id: "P02", name: "完整响应时间", passNote: "完整生成一段回答的耗时正常", failNote: "部分请求耗时异常"),
      Group(id: "P03", name: "并发处理能力", passNote: "不同并发档位下请求都能完成", failNote: "部分并发档位出现失败"),
      Group(id: "P04", name: "持续运行稳定性", passNote: "持续运行期间没有出现失败或明显变慢", failNote: "持续运行期间出现失败或明显变慢"),
      Group(id: "P05", name: "长文本负载", passNote: "不同长度的输入材料下表现正常", failNote: "长输入场景下表现异常"),
    ]
    return RealResultCatalog(
      groups: groups,
      unitName: "批",
      cardTitle: "五类负载的结果",
      cardCaption: "点开可看每批请求的实际情况",
      sampleGroup: { id in id.split(separator: "-").first.map(String.init) },
      sampleName: { index, _ in "第 \(index) 批请求" },
      bannerPass: { total in "在不同负载下实际发起了 \(total) 批请求，等待、并发与持续运行表现均在正常范围。" },
      bannerFail: { pass, fail in
        "共 \(pass + fail) 批请求里有 \(fail) 批出现错误或超时；建议展开对应分组看是哪类负载出了问题。"
      },
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
        Group(id: $0.0, name: $0.1, passNote: "任务按预期完成", failNote: "任务没有按预期完成，展开可看卡在哪一步")
      },
      unitName: "个",
      cardTitle: "十个任务的结果",
      cardCaption: "点开可看任务的完整过程",
      sampleGroup: normalize,
      sampleName: { _, _ in "完整任务过程" },
      bannerPass: { _ in "10 个固定智能体任务（规则遵循、工具运用、错误恢复等）全部按预期完成。" },
      bannerFail: { pass, fail in
        "\(pass + fail) 个任务里有 \(fail) 个没有按预期完成；展开对应任务可看具体在哪一步出了问题。"
      },
      scope: "任务为固定场景，覆盖常见交付形态；不能穷尽所有真实业务。")
  }()

  // ---------- 模型基线对比 ----------
  private static let baseline: RealResultCatalog = {
    let groups = BaselineItem.all.map {
      Group(id: $0.id, name: $0.title, passNote: "返回结构与通用规范一致", failNote: "结构与通用规范不一致，展开可看差异")
    }
    return RealResultCatalog(
      groups: groups,
      unitName: "项",
      cardTitle: "十四个结构项的结果",
      cardCaption: "点开可看实际返回的结构",
      sampleGroup: { id in
        let upper = id.uppercased()
        return upper.hasPrefix("BC") ? String(upper.prefix(4)) : nil
      },
      sampleName: { _, _ in "该结构项的实测返回" },
      bannerPass: { _ in "14 个结构对照项的返回格式均与通用规范一致。" },
      bannerFail: { pass, fail in
        "\(pass + fail) 个结构项里有 \(fail) 项与通用规范不一致；程序按通用规范解析返回值时需要适配。"
      },
      scope: "只检查数据格式是否符合通用规范，不判断内容质量。")
  }()
}

private struct RealTaskReport: Identifiable {
  let id: String
  let status: String
  let verdictPass: Bool?
  let request: String
  let response: String
  let evidenceJSON: String
}

struct RealModuleView: View {
  @ObservedObject var store: Workbench
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
    record.moduleStates?[backendID] ?? moduleResult?["state"] as? String ?? "unverified"
  }
  private var stateStyle: StatusStyle {
    if failCount > 0 { return FindingState.unstable.style }
    switch state {
    case "pass": return FindingState.pass.style
    case "fail": return FindingState.fail.style
    case "unsupported", "inconclusive", "invalid_execution": return FindingState.unstable.style
    default: return FindingState.unknown.style
    }
  }
  private var moduleResult: [String: Any]? {
    let results = recordObject["moduleResults"] as? [[String: Any]] ?? []
    return results.first { ($0["moduleId"] as? String) == backendID }
      ?? results.first { ($0["module_id"] as? String) == backendID }
  }

  private var tasks: [RealTaskReport] {
    let verdicts = verdictsByTask
    return evidenceItems.enumerated().map { index, item in
      let itemID = item["id"] as? String ?? "任务 \(index + 1)"
      let payload = item["payload"] as? [String: Any] ?? [:]
      let nested = payload["payload"] as? [String: Any]
      let sampleID = item["sample_id"] as? String
        ?? payload["sample_id"] as? String
        ?? nested?["sample_id"] as? String
        ?? item["scenario"] as? String
        ?? payload["scenario"] as? String
        ?? nested?["scenario"] as? String
        ?? itemID
      let verdict = verdicts[sampleID]
      return RealTaskReport(
        id: sampleID,
        status: statusText(from: item),
        verdictPass: verdict.map(Self.isPass),
        request: requestText(from: item),
        response: responseText(from: item),
        evidenceJSON: prettyJSON(item))
    }
  }
  private var evidenceItems: [[String: Any]] {
    let all = recordObject["evidence"] as? [[String: Any]] ?? []
    return all.flatMap { item -> [[String: Any]] in
      let id = item["id"] as? String ?? ""
      let payload = item["payload"] as? [String: Any] ?? [:]
      guard payload["module"] as? String == backendID || id.contains("\(backendID)") else {
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
  private var verdictsByTask: [String: String] {
    let all = recordObject["evidence"] as? [[String: Any]] ?? []
    for item in all {
      let payload = item["payload"] as? [String: Any] ?? [:]
      guard payload["module"] as? String == backendID,
        let modulePayload = payload["payload"] as? [String: Any]
      else { continue }
      if let scorecard = modulePayload["scorecard"] as? [String: Any],
        let observations = scorecard["observations"] as? [[String: Any]]
      {
        return Dictionary(uniqueKeysWithValues: observations.compactMap { observation in
          guard let id = observation["sample_id"] as? String,
            let result = observation["label"] as? String
          else { return nil }
          return (id, result)
        })
      }
      if let report = modulePayload["report"] as? [String: Any],
        let rows = report["rows"] as? [[String: Any]]
      {
        return Dictionary(uniqueKeysWithValues: rows.compactMap { row in
          guard let id = row["sample_id"] as? String,
            let result = row["result"] as? String
          else { return nil }
          return (id, result)
        })
      }
    }
    return [:]
  }

  private static func isPass(_ verdict: String) -> Bool {
    ["accepted", "correct", "pass"].contains(verdict.lowercased())
  }
  private var failCount: Int {
    groupedSamples.values
      .reduce(0) { count, group in count + group.samples.filter { !$0.pass }.count }
  }
  private var passCount: Int { tasks.count - failCount }

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
      buckets[catalog.groupID(for: task.id), default: []].append(task)
    }
    var result: [String: GroupedSamples] = [:]
    for group in catalog.groups {
      if let samples = buckets[group.id] {
        result[group.id] = GroupedSamples(group: group, samples: samples)
      }
    }
    if let others = buckets["_other"], !others.isEmpty {
      result["_other"] = GroupedSamples(
        group: .init(id: "_other", name: "其他检查", passNote: "全部正常", failNote: "有未通过的检查"),
        samples: others)
    }
    return result
  }
  private var catalog: RealResultCatalog? { RealResultCatalog.catalog(for: backendID) }

  var body: some View {
    VStack(alignment: .leading, spacing: 24) {
      VerdictBanner(
        style: stateStyle,
        title: "\(module.title)：\(headline)",
        detail: bannerDetail,
        meta: bannerMeta
      )

      VStack(alignment: .leading, spacing: 0) {
        SectionHeader(
          title: catalog?.cardTitle ?? "检查结果",
          caption: catalog?.cardCaption ?? "点开可查看每条检查"
        )
        .padding(.bottom, 6)
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
    case "pass": return "通过"
    case "fail": return "有问题"
    case "unsupported": return "不支持"
    case "invalid_execution": return "本次检测未完成"
    case "inconclusive": return "证据不足，暂不能判断"
    default: return "尚未检测"
    }
  }
  private var bannerDetail: String {
    guard let catalog, !tasks.isEmpty else {
      return moduleResult?["reason"] as? String ?? "该模块已完成真实执行；详细任务证据见下方。"
    }
    if failCount > 0 { return catalog.bannerFail(passCount, failCount) }
    return catalog.bannerPass(tasks.count)
  }
  private var bannerMeta: String? {
    guard !tasks.isEmpty else { return nil }
    if failCount > 0 { return "\(passCount) 项正常 · \(failCount) 项未通过" }
    return "\(tasks.count) \(catalog?.unitName ?? "项")全部正常 · 每项都是真实调用"
  }

  private func requestText(from item: [String: Any]) -> String {
    let payload = item["payload"] as? [String: Any] ?? item
    let nested = payload["payload"] as? [String: Any] ?? payload
    let request = (nested["request"] as? [String: Any]) ?? (payload["request"] as? [String: Any])
    return request?["prompt"] as? String ?? "未记录请求内容"
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
}

private extension RealTaskReport {
  var pass: Bool { verdictPass ?? (status == "请求成功") }
  var verdictNote: String {
    if status == "请求异常" || status.hasPrefix("HTTP") { return "请求失败，服务没有返回结果" }
    if let verdictPass {
      return verdictPass ? "服务返回正常，回答符合要求" : "返回内容不符合要求"
    }
    return "服务返回正常"
  }
}

// 小项分组行：状态一枚徽章 + 计数，展开是逐条检查。
private struct RealGroupRow: View {
  let grouped: RealModuleView.GroupedSamples
  let unitName: String
  let sampleName: (Int, String) -> String
  @State private var expanded = false

  private var allPass: Bool { grouped.samples.allSatisfy(\.pass) }

  var body: some View {
    VStack(spacing: 0) {
      HStack(spacing: 13) {
        Image(systemName: allPass ? "checkmark.circle.fill" : "exclamationmark.circle.fill")
          .font(.system(size: 18))
          .foregroundStyle(allPass ? Theme.passBar : Theme.limitedBar)
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
          .foregroundStyle(allPass ? Theme.pass : Theme.limited)
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
            displayName: sampleName(index + 1, sample.id),
            isQuestion: unitName == "题")
        }
      }
    }
    .overlay(alignment: .bottom) { Rectangle().fill(Theme.line).frame(height: 1) }
    .animation(.easeInOut(duration: 0.3), value: expanded)
  }

  private var countText: String {
    let correct = grouped.correct, total = grouped.samples.count
    if unitName == "题" { return "\(correct) / \(total) 题正确" }
    return allPass ? "\(total) / \(total)" : "\(correct) / \(total)"
  }
  private var groupNote: String {
    let correct = grouped.correct, total = grouped.samples.count
    if total > 0, correct < total {
      let ratio = Double(correct) / Double(total)
      if ratio >= 0.9 { return "基本可用，个别\(unitName)没通过（\(total - correct) \(unitName)）" }
      if ratio >= 0.7 { return "有 \(total - correct) \(unitName)未通过，建议关注" }
      return grouped.group.failNote
    }
    return grouped.group.passNote
  }
}

// 单条检查行：人话名称 + 结果说明；展开是「发了什么 / 回了什么 / 原始数据」。
private struct RealSampleRow: View {
  let sample: RealTaskReport
  let displayName: String
  let isQuestion: Bool
  @State private var expanded = false
  @State private var rawVisible = false

  private var pass: Bool { sample.pass }

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      HStack(spacing: 10) {
        Circle()
          .fill(pass ? Theme.passBar : Theme.limitedBar)
          .frame(width: 8, height: 8)
        Text(displayName)
          .font(.system(size: 12.5))
        Spacer()
        Text(pass ? (isQuestion ? "答对" : "通过") : (isQuestion ? "答错" : "未通过"))
          .font(.system(size: 11, weight: .medium))
          .foregroundStyle(pass ? Theme.pass : Theme.limited)
          .padding(.horizontal, 9)
          .padding(.vertical, 3)
          .background(
            pass ? Theme.passTint : Theme.limitedTint,
            in: Capsule())
      }
      .padding(.vertical, 10)
      .contentShape(Rectangle())
      .onTapGesture { expanded.toggle() }
      if expanded {
        VStack(alignment: .leading, spacing: 10) {
          Text(sample.verdictNote)
            .font(Theme.captionFont)
            .foregroundStyle(Theme.muted)
          EvidenceField(title: "检测程序发了什么", value: sample.request)
          EvidenceField(title: "模型回了什么", value: sample.response)
          Button {
            rawVisible.toggle()
          } label: {
            Label(
              rawVisible ? "收起原始数据（JSON）" : "查看原始数据（JSON）",
              systemImage: rawVisible ? "chevron.up" : "chevron.down")
              .font(.system(size: 11.5, weight: .medium))
          }
          .buttonStyle(.plain)
          .foregroundStyle(Theme.accent)
          if rawVisible {
            EvidenceField(title: "原始数据", value: sample.evidenceJSON, monospaced: true)
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
