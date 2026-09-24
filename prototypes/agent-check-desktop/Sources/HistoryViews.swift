import SwiftUI

// 检测记录：按时间列出，可打开、可比对（条件一致时）。
struct HistoryView: View {
  @ObservedObject var store: Workbench
  var body: some View {
    VStack(alignment: .leading, spacing: 24) {
      // 页面大标题已并入顶部工具栏（面包屑），这里直接从筛选开始
      HStack {
        // 用「模型」说人话：客户按模型找记录，「当前」指侧栏正在配置的这个。
        Picker("记录范围", selection: $store.historyFilter) {
          Text("全部记录").tag("all")
          Text("当前模型").tag("current")
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
        // 记录卡片列表：每张卡自描述（结论 → 模型 → 关键数字 → 限制摘要 → 操作），
        // 不再依赖表头对齐。
        VStack(spacing: 10) {
          ForEach(store.visibleRecords) { record in
            HistoryRow(record: record, store: store)
          }
        }
        // 底部操作条：选择进度 + 对比入口收进同一容器，不再散落
        HStack(spacing: 14) {
          VStack(alignment: .leading, spacing: 4) {
            Text("已选 \(store.historySelection.count) / 2 条记录")
              .font(.system(size: 12.5, weight: .medium))
            Text(store.comparisonBlocker ?? "选择任意两条记录（不同模型也可以）按模块并排对照。")
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
        }
        .padding(.horizontal, 16).padding(.vertical, 12)
        .background(Theme.canvas.opacity(0.6), in: RoundedRectangle(cornerRadius: 10))
      }
    }
    .sheet(isPresented: $store.showHistoryComparison) { HistoryComparison(store: store) }
  }
}

// 单条历史记录卡片：结论先行；第二行给关键数字、限制摘要和明确的操作入口。
// 与 prototypes/history-redesign.html 设计稿同构。
private struct HistoryRow: View {
  let record: RunRecord
  @ObservedObject var store: Workbench
  @State private var hovered = false
  // 模块失败统计在出现时算一次并缓存：检测进行中工作台每秒多次刷新，
  // 之前每次刷新都对每条记录全量重算六模块证据管道，主线程直接卡死。
  @State private var moduleFailures: [(title: String, fails: Int)]?

  private var selected: Bool { store.historySelection.contains(record.id) }

  private var failedCount: Int { (moduleFailures ?? []).reduce(0) { $0 + $1.fails } }

  // 一句话点名问题在哪，替代原来那句人人适用的「限制摘要」：
  // 通过的记录没有可说的，就什么都不写。
  private var failureSummary: String? {
    guard let failures = moduleFailures, !failures.isEmpty else { return nil }
    let head = failures.prefix(2)
      .map { "\($0.title) \($0.fails) 项" }
      .joined(separator: "、")
    let rest = failures.count > 2 ? " 等 \(failures.count) 个模块" : ""
    return "主要问题：\(head)\(rest)"
  }

  var body: some View {
    HStack(alignment: .top, spacing: 14) {
      Toggle(
        "选择记录 \(record.readableID)",
        isOn: Binding(
          get: { selected },
          set: { _ in store.toggleHistory(record.id) }
        )
      )
      .labelsHidden().toggleStyle(.checkbox)
      .disabled(store.historySelection.count == 2 && !selected)
      .padding(.top, 7)
      VStack(alignment: .leading, spacing: 10) {
        // 第一行：模型与地址 + 结论胶囊 + 时间（点这行进只读回看）
        Button {
          store.reviewRecord(record)
        } label: {
          HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
              Text(record.service.model)
                .font(.system(size: 15, weight: .semibold)).foregroundStyle(Theme.ink)
                .lineLimit(1)
              Text(record.service.host)
                .font(Theme.captionFont).foregroundStyle(Theme.faint).lineLimit(1)
            }
            verdictCapsule
            Spacer(minLength: 12)
            Text(record.date.formatted(date: .numeric, time: .shortened))
              .font(Theme.captionFont).foregroundStyle(Theme.faint)
          }
          .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("打开这次检测结果")
        // 第二行：关键数字 + 问题点名（只给有价值的：哪几个模块没通过）+ 操作
        HStack(spacing: 10) {
          statChips
          if let summary = failureSummary {
            Text(summary)
              .font(Theme.captionFont).foregroundStyle(Theme.muted).lineLimit(1)
              .truncationMode(.tail)
          }
          Spacer(minLength: 8)
          HStack(spacing: 8) {
            rowButton("打开结果", icon: nil, prominent: false) { store.reviewRecord(record) }
            rowButton("导出报告", icon: "square.and.arrow.up", prominent: true) {
              exportRecord(record, store: store)
            }
          }
        }
      }
    }
    .padding(.horizontal, 16).padding(.vertical, 15)
    .background(cardBackground, in: RoundedRectangle(cornerRadius: Theme.radiusCard))
    .overlay(
      RoundedRectangle(cornerRadius: Theme.radiusCard)
        .stroke(cardBorder, lineWidth: 1)
    )
    .onHover { hovered = $0 }
    .task(id: record.id) {
      moduleFailures = Self.computeModuleFailures(record)
    }
  }

  private static func computeModuleFailures(
    _ record: RunRecord
  ) -> [(title: String, fails: Int)] {
    guard record.hasCurrentEvidence else { return [] }
    return CheckModule.testModules
      .filter { record.modules.contains($0) }
      .map { (title: $0.title, fails: RealModuleView(module: $0, record: record).exportModule.failCount) }
      .filter { $0.fails > 0 }
      .sorted { $0.fails > $1.fails }
  }

  private var cardBackground: Color {
    if selected { return Theme.infoTint }
    return hovered ? Color(red: 0.97, green: 0.975, blue: 0.98) : .white
  }
  private var cardBorder: Color {
    if selected { return Theme.accent.opacity(0.35) }
    return hovered ? Theme.faint.opacity(0.4) : Theme.line
  }

  private var verdictCapsule: some View {
    let (color, tint, icon): (Color, Color, String) = {
      switch record.outcome {
      case .usable: return (Theme.pass, Theme.passTint, "checkmark")
      case .limited: return (Theme.limited, Theme.limitedTint, "exclamationmark")
      case .blocked: return (Theme.blocked, Theme.blockedTint, "xmark")
      case .inconclusive: return (Theme.unknown, Theme.unknownTint, "questionmark")
      }
    }()
    return HStack(spacing: 7) {
      Image(systemName: icon)
        .font(.system(size: 10, weight: .bold)).foregroundStyle(.white)
        .frame(width: 20, height: 20).background(color, in: Circle())
      Text(record.title)
        .font(.system(size: 12.5, weight: .semibold)).foregroundStyle(color)
    }
    .padding(.leading, 4).padding(.trailing, 12).padding(.vertical, 3)
    .background(tint, in: Capsule())
  }

  @ViewBuilder private var statChips: some View {
    HStack(spacing: 6) {
      if !record.hasCurrentEvidence {
        statChip("旧版记录", color: Theme.muted, tint: Theme.canvas, bordered: true)
      } else if moduleFailures == nil {
        statChip("统计中", color: Theme.muted, tint: Theme.canvas, bordered: true)
      } else {
        if failedCount > 0 {
          statChip("\(failedCount) 项未通过", color: Theme.blocked, tint: Theme.blockedTint)
        } else {
          statChip("全部通过", color: Theme.pass, tint: Theme.passTint)
        }
        statChip(
          "\(record.completed.count)/\(record.modules.count) 个模块\(record.stopped ? " · 未完成" : "")",
          color: Theme.muted, tint: Theme.canvas, bordered: true)
      }
    }
  }

  private func statChip(_ text: String, color: Color, tint: Color, bordered: Bool = false) -> some View {
    Text(text)
      .font(.system(size: 11.5)).foregroundStyle(color)
      .padding(.horizontal, 9).padding(.vertical, 2)
      .background(tint, in: RoundedRectangle(cornerRadius: 6))
      .overlay(
        RoundedRectangle(cornerRadius: 6)
          .stroke(bordered ? Theme.line : .clear, lineWidth: 0.8))
  }

  private func rowButton(
    _ title: String, icon: String?, prominent: Bool, action: @escaping () -> Void
  ) -> some View {
    Button(action: action) {
      HStack(spacing: 5) {
        if let icon {
          Image(systemName: icon).font(.system(size: 11, weight: .medium))
        }
        Text(title).font(.system(size: 12.5, weight: .medium))
      }
      .padding(.horizontal, 13).frame(height: 29)
      .background(
        prominent ? Theme.infoTint : .white,
        in: RoundedRectangle(cornerRadius: Theme.radiusControl))
      .overlay(
        RoundedRectangle(cornerRadius: Theme.radiusControl)
          .stroke(prominent ? Theme.accent.opacity(0.35) : Theme.line))
      .foregroundStyle(prominent ? Theme.accent : Theme.muted)
    }
    .buttonStyle(.plain)
    .help(prominent ? "导出这次检测的报告" : "打开这次检测结果")
  }
}

// 两次检测的对照：同条件下逐模块比。
// 模型对比（选型参考）：任选两条记录按模块并排对照。
// 条件一致时是严格 A/B；不一致（典型是跨模型选型）呈现逐模块判定与更优方，
// 底部注明条件差异，不冒充同条件结论。
struct HistoryComparison: View {
  @ObservedObject var store: Workbench
  // 每侧每模块的摘要缓存：对比页一次渲染会读 20+ 次数据管道，
  // 检测进行中每秒重渲染多次，不缓存同样会把主线程卡死。
  @State private var sideCache: [String: SideModule] = [:]

  private var pair: [RunRecord] { store.comparedRecords }
  private var sameModel: Bool { pair.count == 2 && pair[0].service.model == pair[1].service.model }

  // 单侧单模块的表现摘要：与结果页同一数据管道。
  private struct SideModule {
    var headline: String
    var failCount: Int
    var dot: NavDot
    var tested: Bool
    // 排序用：通过且零失败最好，其次有失败，未测/无证据最弱
    var rank: Int {
      if !tested { return 0 }
      if failCount == 0 && dot == .pass { return 3 }
      return 2
    }
  }

  private static func computeSide(_ record: RunRecord, module: CheckModule) -> SideModule {
    guard record.modules.contains(module), record.hasCurrentEvidence else {
      return SideModule(headline: record.modules.contains(module) ? "缺少判定证据" : "本次未选",
        failCount: 0, dot: .none, tested: false)
    }
    let export = RealModuleView(module: module, record: record).exportModule
    return SideModule(headline: export.headline, failCount: export.failCount,
      dot: record.navDot(module), tested: true)
  }

  private func side(_ record: RunRecord, module: CheckModule) -> SideModule {
    let key = "\(record.id)-\(module.id)"
    return sideCache[key] ?? SideModule(
      headline: "统计中…", failCount: 0, dot: .none, tested: false)
  }

  var body: some View {
    VStack(alignment: .leading, spacing: 18) {
      HStack {
        Text(sameModel ? "两次检测对比" : "模型对比 · 选型参考")
          .font(.system(size: 20, weight: .bold))
        Spacer()
        IconButton(symbol: "xmark", help: "关闭对比") { store.showHistoryComparison = false }
      }
      if store.comparisonSameConditions {
        Pill(text: "样本、规则与运行条件一致，可作同条件 A/B", style: FindingState.pass.style, icon: "checkmark.seal")
      } else {
        Text("两次检测的条件（样本、规则或平台）不同：按模块并排对照，供选型参考，不构成同条件结论。")
          .font(Theme.captionFont).foregroundStyle(Theme.muted)
          .fixedSize(horizontal: false, vertical: true)
      }
      ScrollView {
        VStack(alignment: .leading, spacing: 0) {
          HStack(alignment: .top, spacing: 14) {
            ForEach(Array(pair.enumerated()), id: \.element.id) { _, record in
              comparisonCard(record)
            }
          }.padding(.bottom, 18)
          ForEach(commonModules, id: \.self) { module in
            moduleRow(module)
          }
          readingBar
        }
      }.frame(maxHeight: 560)
    }
    .padding(28).frame(width: 820).background(.white).foregroundStyle(Theme.ink)
    .task {
      for record in pair {
        for module in CheckModule.testModules where record.modules.contains(module) {
          let key = "\(record.id)-\(module.id)"
          if sideCache[key] == nil {
            sideCache[key] = Self.computeSide(record, module: module)
          }
        }
      }
    }
  }

  private var commonModules: [CheckModule] {
    CheckModule.testModules.filter { module in pair.contains { $0.modules.contains(module) } }
  }

  private func comparisonCard(_ record: RunRecord) -> some View {
    let failed = commonModules.reduce(0) { $0 + side(record, module: $1).failCount }
    return VStack(alignment: .leading, spacing: 8) {
      Text(record.service.model)
        .font(.system(size: 15, weight: .bold)).lineLimit(2)
      Text("\(record.service.host) · \(record.date.formatted(date: .numeric, time: .shortened))")
        .font(Theme.captionFont).foregroundStyle(Theme.faint).lineLimit(1)
      Pill(text: record.title, style: record.style)
      HStack(spacing: 6) {
        if failed > 0 {
          Text("\(failed) 项未通过")
            .font(.system(size: 10.5, weight: .semibold)).foregroundStyle(Theme.blocked)
            .padding(.horizontal, 8).padding(.vertical, 2)
            .background(Theme.blockedTint, in: RoundedRectangle(cornerRadius: 6))
        } else {
          Text("全部通过")
            .font(.system(size: 10.5, weight: .semibold)).foregroundStyle(Theme.pass)
            .padding(.horizontal, 8).padding(.vertical, 2)
            .background(Theme.passTint, in: RoundedRectangle(cornerRadius: 6))
        }
        Text("\(record.completed.count)/\(record.modules.count) 个模块")
          .font(.system(size: 10.5)).foregroundStyle(Theme.muted)
          .padding(.horizontal, 8).padding(.vertical, 2)
          .background(Theme.canvas, in: RoundedRectangle(cornerRadius: 6))
      }
      Action(title: "查看这次依据", icon: "doc.text.magnifyingglass", primary: false) {
        store.showHistoryComparison = false
        store.reviewRecord(record)
      }
    }
    .frame(maxWidth: .infinity, alignment: .leading)
    .padding(14)
    .background(Theme.canvas.opacity(0.5), in: RoundedRectangle(cornerRadius: 12))
    .overlay(RoundedRectangle(cornerRadius: 12).stroke(Theme.line))
  }

  private func moduleRow(_ module: CheckModule) -> some View {
    let left = side(pair[0], module: module)
    let right = side(pair[1], module: module)
    let leftWins = left.rank > right.rank
    let rightWins = right.rank > left.rank
    return HStack(spacing: 0) {
      Text(module.title)
        .font(.system(size: 12.5, weight: .semibold)).foregroundStyle(Theme.muted)
        .frame(width: 128, alignment: .leading)
      moduleCell(left, wins: leftWins)
      moduleCell(right, wins: rightWins)
    }
    .padding(.vertical, 9)
    .overlay(alignment: .top) { Rectangle().fill(Theme.line).frame(height: 1) }
  }

  private func moduleCell(_ side: SideModule, wins: Bool) -> some View {
    HStack(spacing: 8) {
      navDotView(side.dot, dark: false, size: 8)
      Text(side.headline)
        .font(.system(size: 12.5, weight: .medium))
      if side.failCount > 0 {
        Text("\(side.failCount) 项未通过")
          .font(.system(size: 10.5)).foregroundStyle(Theme.blocked)
      }
    }
    .frame(maxWidth: .infinity, alignment: .leading)
    .padding(.horizontal, 10).padding(.vertical, 5)
    .background(
      wins ? Theme.passTint : Theme.canvas.opacity(0.4),
      in: RoundedRectangle(cornerRadius: 8))
  }

  private var readingBar: some View {
    let leftWins = commonModules.filter { side(pair[0], module: $0).rank > side(pair[1], module: $0).rank }.count
    let rightWins = commonModules.filter { side(pair[1], module: $0).rank > side(pair[0], module: $0).rank }.count
    let better = leftWins >= rightWins ? pair[0] : pair[1]
    let wins = max(leftWins, rightWins)
    let agent = CheckModule.testModules.first { $0 == .agent }
    var agentNote = ""
    if let agent, pair.allSatisfy({ $0.modules.contains(agent) }) {
      let l = side(pair[0], module: agent), r = side(pair[1], module: agent)
      if l.rank != r.rank {
        agentNote = "，智能体任务 \((l.rank > r.rank ? pair[0] : pair[1]).service.model) 更稳"
      }
    }
    return HStack(alignment: .top, spacing: 8) {
      Image(systemName: "text.book.closed")
        .font(.system(size: 12)).foregroundStyle(Theme.accent)
      Text("读法：\(better.service.model) 在 \(wins) 个模块表现更优\(agentNote)；另一侧在其余模块各有取舍，按业务侧重选择。绿底 = 该模块更优的一方。")
        .font(Theme.captionFont).foregroundStyle(Theme.muted)
        .fixedSize(horizontal: false, vertical: true)
    }
    .padding(12)
    .background(Theme.infoTint, in: RoundedRectangle(cornerRadius: 10))
    .overlay(RoundedRectangle(cornerRadius: 10).stroke(Theme.accent.opacity(0.2)))
    .padding(.top, 14)
  }
}

// 历史记录只读查看器：回看过去某一次检测的完整报告。
// 它不改写工作台——「检测报告」与各模块页永远属于当前这一次检测，
// 历史回看是一次明确进入、明确退出的动作，不是悄悄切换上下文。
struct HistoryRecordViewer: View {
  @ObservedObject var store: Workbench
  let record: RunRecord
  @State private var currentModule: CheckModule?

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      HStack(spacing: 14) {
        VStack(alignment: .leading, spacing: 3) {
          Text(
            "\(record.service.model) · \(record.date.formatted(date: .numeric, time: .shortened))"
          )
          .font(.system(size: 15, weight: .semibold))
          Text("历史记录回看 · 只读，不影响当前工作台")
            .font(Theme.captionFont).foregroundStyle(Theme.faint)
        }
        Spacer()
        Action(title: "导出报告", icon: "square.and.arrow.up", primary: false) {
          exportRecord(record, store: store)
        }
        IconButton(symbol: "xmark", help: "关闭回看") { store.viewedRecord = nil }
      }
      .padding(.horizontal, 28).padding(.vertical, 16)
      .overlay(alignment: .bottom) { Rectangle().fill(Theme.line).frame(height: 1) }
      // 吸附式模块目录：点击跳转，滚动联动高亮，状态色点顺带当摘要
      ScrollViewReader { proxy in
        VStack(alignment: .leading, spacing: 0) {
          HStack(spacing: 8) {
            ForEach(visibleModules) { module in
              Button {
                currentModule = module
                withAnimation { proxy.scrollTo(module.id, anchor: .top) }
              } label: {
                HStack(spacing: 6) {
                  navDotView(record.navDot(module), dark: false, size: 7)
                  Text(module.shortTitle)
                    .font(.system(size: 11.5, weight: currentModule == module ? .semibold : .regular))
                    .foregroundStyle(currentModule == module ? Theme.accent : Theme.muted)
                }
                .padding(.horizontal, 12).padding(.vertical, 5)
                .background(
                  currentModule == module ? Theme.infoTint : .white,
                  in: Capsule())
                .overlay(
                  Capsule().stroke(currentModule == module ? Theme.accent.opacity(0.3) : Theme.line))
              }
              .buttonStyle(.plain)
              .help("跳到\(module.title)")
            }
            Spacer()
          }
          .padding(.horizontal, 28).padding(.vertical, 10)
          .overlay(alignment: .bottom) { Rectangle().fill(Theme.canvas).frame(height: 1) }
          ScrollView {
            VStack(alignment: .leading, spacing: 26) {
              VerdictBanner(
                style: record.style, title: record.admissionTitle,
                detail: record.admissionExplanation)
              ForEach(visibleModules) { module in
                RealModuleView(module: module, record: record)
                  .id(module.id)
                  .background(
                    ViewerSectionSpy(module: module) { currentModule = $0 })
              }
            }
            .padding(28).frame(maxWidth: 980, alignment: .leading)
          }
        }
      }
    }
    .frame(minWidth: 920, minHeight: 640)
    .background(.white)
    .foregroundStyle(Theme.ink)
    .onAppear { currentModule = visibleModules.first }
  }

  private var visibleModules: [CheckModule] {
    CheckModule.testModules.filter { record.modules.contains($0) }
  }
}

// 滚动联动：模块区块进入视口顶部时回调，目录同步高亮。
private struct ViewerSectionSpy: View {
  let module: CheckModule
  let onVisible: (CheckModule) -> Void
  var body: some View {
    GeometryReader { proxy in
      Color.clear
        .onChange(of: proxy.frame(in: .global).minY) { _, y in
          if y < 140 && y > -400 { onVisible(module) }
        }
    }
    .allowsHitTesting(false)
  }
}
