import SwiftUI

extension Workbench {
  func showKeyEvidence() {
    showModule(.agent)
    guard let record = currentRecord,
      let finding = record.agentFindings.first(where: { $0.state == .fail || $0.state == .unstable }
      )
        ?? record.agentFindings.first(where: { $0.state != .pass })
    else { return }
    agentSelection = finding.id
    if let sample = record.agentSamples.first(where: { sample in
      finding.sampleIDs.contains(sample.id)
        && sample.runs.contains { $0.failedChecks.contains(finding.id) }
    }), let index = sample.runs.firstIndex(where: { $0.failedChecks.contains(finding.id) }) {
      agentSample = sample.id
      agentRun = index
    }
    evidenceExpanded = true
  }
}

// 首页 = 工作台：只负责发起与回看。结论、指标与证据都在检测报告页。
struct HomeView: View {
  @ObservedObject var store: Workbench
  private var latest: RunRecord? { store.latestForService }
  var body: some View {
    VStack(alignment: .leading, spacing: 26) {
      hero
      if latest == nil { welcome }
      if !store.records.isEmpty { recent }
    }
  }

  // MARK: 主行动面板：深色仪器面板 = 发起体检 + 上次体检环

  private var hero: some View {
    HStack(spacing: 0) {
      VStack(alignment: .leading, spacing: 0) {
        HStack(spacing: 14) {
          HStack(spacing: 7) {
            Image(systemName: "cpu").font(.system(size: 12))
            Text("\(store.service?.model ?? "未配置服务") · \(store.service?.host ?? "")")
              .font(.system(size: 12, weight: .medium)).lineLimit(1)
          }
          .foregroundStyle(.white.opacity(0.78))
          Button {
            store.editService()
          } label: {
            Text("更换").font(.system(size: 11.5)).foregroundStyle(.white.opacity(0.42))
          }
          .buttonStyle(.plain).disabled(store.running)
        }
        Text(latest == nil ? "开始第一次体检" : "开始一次新体检")
          .font(.system(size: 30, weight: .bold)).foregroundStyle(.white)
          .padding(.top, 18)
        Text(
          latest == nil
            ? "接入智能体产品前，先回答「这个模型能不能用」\n五个模块全程只读，不改动你的服务与数据"
            : "规格 · 能力 · 性能 · 智能体 · 基线，五个模块全程只读\n完成后生成「能否接入」结论与逐项证据"
        )
        .font(.system(size: 13)).foregroundStyle(.white.opacity(0.55))
        .lineSpacing(5).padding(.top, 10)
        .fixedSize(horizontal: false, vertical: true)
        ViewThatFits(in: .horizontal) {
          HStack(alignment: .center, spacing: 22) {
            goButton
            singleChips
          }
          VStack(alignment: .leading, spacing: 14) {
            goButton
            singleChips
          }
        }
        .padding(.top, 24)
      }
      Spacer(minLength: 24)
      heroRing
    }
    .padding(.horizontal, 34).padding(.vertical, 30)
    .background(heroBackground)
    .clipShape(RoundedRectangle(cornerRadius: 16))
    .shadow(color: Color(red: 0.07, green: 0.10, blue: 0.16).opacity(0.35), radius: 18, y: 8)
  }

  private var heroBackground: some View {
    ZStack {
      LinearGradient(
        colors: [
          Color(red: 0.071, green: 0.090, blue: 0.122),
          Color(red: 0.094, green: 0.133, blue: 0.204),
          Color(red: 0.078, green: 0.102, blue: 0.149),
        ],
        startPoint: .topLeading, endPoint: .bottomTrailing)
      RadialGradient(
        colors: [Color(red: 0.231, green: 0.510, blue: 0.965).opacity(0.28), .clear],
        center: UnitPoint(x: 0.92, y: -0.1), startRadius: 0, endRadius: 420)
      RadialGradient(
        colors: [Color(red: 0.204, green: 0.827, blue: 0.6).opacity(0.10), .clear],
        center: UnitPoint(x: -0.05, y: 1.15), startRadius: 0, endRadius: 320)
      gridTexture
    }
  }

  // 细网格纹理：仪器面板质感，只在右上区域可见。
  private var gridTexture: some View {
    Canvas { context, size in
      let step: CGFloat = 34
      var path = Path()
      for x in stride(from: 0, through: size.width, by: step) {
        path.move(to: CGPoint(x: x, y: 0))
        path.addLine(to: CGPoint(x: x, y: size.height))
      }
      for y in stride(from: 0, through: size.height, by: step) {
        path.move(to: CGPoint(x: 0, y: y))
        path.addLine(to: CGPoint(x: size.width, y: y))
      }
      context.stroke(path, with: .color(.white.opacity(0.035)), lineWidth: 1)
    }
    .mask(
      RadialGradient(
        colors: [.black, .clear], center: UnitPoint(x: 0.7, y: 0),
        startRadius: 40, endRadius: 560)
    )
    .allowsHitTesting(false)
  }

  private var goButton: some View {
    Button {
      store.prepareRun()
    } label: {
      HStack(spacing: 9) {
        Image(systemName: "play.fill").font(.system(size: 11))
        Text("开始体检")
      }
      .font(.system(size: 15, weight: .semibold)).foregroundStyle(.white)
      .padding(.horizontal, 30).frame(height: 46)
      .background(
        LinearGradient(
          colors: [Color(red: 0.231, green: 0.510, blue: 0.965), Theme.accent],
          startPoint: .top, endPoint: .bottom),
        in: RoundedRectangle(cornerRadius: 12)
      )
      .overlay(RoundedRectangle(cornerRadius: 12).stroke(.white.opacity(0.2), lineWidth: 1))
      .shadow(color: Color(red: 0.231, green: 0.510, blue: 0.965).opacity(0.55), radius: 10, y: 4)
    }
    .buttonStyle(.plain)
    .disabled(store.running || store.service == nil)
  }

  private var singleChips: some View {
    HStack(spacing: 7) {
      Text("只测一项").font(.system(size: 11.5)).foregroundStyle(.white.opacity(0.4))
        .fixedSize()
      ForEach(CheckModule.testModules) { module in
        Button {
          store.prepareRun(module: module)
        } label: {
          Text(module.shortTitle)
            .font(.system(size: 12)).foregroundStyle(.white.opacity(0.82))
            .fixedSize()
            .padding(.horizontal, 11).padding(.vertical, 5)
            .background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 7))
            .overlay(RoundedRectangle(cornerRadius: 7).stroke(.white.opacity(0.13), lineWidth: 1))
        }
        .buttonStyle(.plain)
        .disabled(store.running)
        .help("只检测\(module.title)")
      }
    }
  }

  private var heroRing: some View {
    VStack(spacing: 12) {
      CoverageRing(record: latest)
      if let record = latest {
        HStack(spacing: 5) {
          Image(systemName: record.style.icon).font(.system(size: 10, weight: .semibold))
          Text(record.title)
        }
        .font(.system(size: 11.5, weight: .medium))
        .foregroundStyle(Color(red: 0.886, green: 0.910, blue: 0.941))
        .padding(.horizontal, 12).frame(height: 26)
        .background(Color(red: 0.58, green: 0.64, blue: 0.72).opacity(0.16), in: Capsule())
        .overlay(
          Capsule().stroke(Color(red: 0.58, green: 0.64, blue: 0.72).opacity(0.22), lineWidth: 1))
        Button {
          store.showCurrentReport()
        } label: {
          Text("上次体检 \(record.date.formatted(date: .numeric, time: .shortened)) · 查看报告 →")
            .font(.system(size: 11)).foregroundStyle(.white.opacity(0.45))
        }
        .buttonStyle(.plain)
      } else {
        Text("完成后环会点亮，逐段显示各模块状态")
          .font(.system(size: 11)).foregroundStyle(.white.opacity(0.45))
      }
    }
    .padding(.leading, 32)
    .overlay(alignment: .leading) {
      Rectangle().fill(.white.opacity(0.09)).frame(width: 1)
    }
  }

  // MARK: 最近检测：每行带六模块状态指纹

  private var recent: some View {
    VStack(alignment: .leading, spacing: 12) {
      HStack(alignment: .firstTextBaseline) {
        Text("最近检测").font(.system(size: 14, weight: .semibold))
        Spacer()
        Button {
          store.navigate(.history)
        } label: {
          Text("全部记录 →").font(.system(size: 11.5, weight: .medium)).foregroundStyle(Theme.accent)
        }
        .buttonStyle(.plain)
      }
      VStack(spacing: 0) {
        ForEach(store.records.prefix(5)) { record in
          recentRow(record)
        }
      }
      .background(.white, in: RoundedRectangle(cornerRadius: Theme.radiusCard))
      .overlay(RoundedRectangle(cornerRadius: Theme.radiusCard).stroke(Theme.line, lineWidth: 1))
      .clipShape(RoundedRectangle(cornerRadius: Theme.radiusCard))
    }
  }

  private func recentRow(_ record: RunRecord) -> some View {
    Button {
      store.reviewRecord(record)
    } label: {
      HStack(spacing: 14) {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
          Text(record.service.model)
            .font(.system(size: 13, weight: .semibold)).lineLimit(1)
          Text(record.service.host)
            .font(.system(size: 11)).foregroundStyle(Theme.faint).lineLimit(1)
        }
        .frame(width: 210, alignment: .leading)
        HStack(spacing: 3.5) {
          ForEach(CheckModule.testModules) { module in
            navDotView(record.navDot(module), dark: false, size: 7)
          }
        }
        .frame(width: 84, alignment: .leading)
        Pill(text: record.title, style: record.style)
        Spacer()
        Text(record.date.formatted(date: .numeric, time: .shortened))
          .font(.system(size: 11.5)).foregroundStyle(Theme.faint).monospacedDigit()
        Image(systemName: "chevron.right")
          .font(.system(size: 10)).foregroundStyle(Theme.faint)
      }
      .padding(.horizontal, 18).padding(.vertical, 13)
      .contentShape(Rectangle())
      .overlay(alignment: .bottom) { Rectangle().fill(Theme.line).frame(height: 1) }
    }
    .buttonStyle(.plain)
    .help("打开这次检测结果")
  }

  // MARK: 首次使用：同一 hero 之下接三步引导与体检清单

  private var welcome: some View {
    VStack(alignment: .leading, spacing: 26) {
      HStack(alignment: .top, spacing: 14) {
        stepCard(
          step: "第 1 步", done: store.service != nil, title: "连接服务",
          detail: store.service.map {
            "已连接 \($0.model)（\($0.host)）。API Key 不保存、不写入报告。"
          } ?? "填写服务地址、模型名称和 API Key，只测你自己的服务。")
        stepCard(
          step: "第 2 步", title: "选择检测范围",
          detail: "五个模块可以全测，也可以只测关心的一项。")
        stepCard(
          step: "第 3 步", title: "查看结论",
          detail: "先给「能否接入」的判断与限制，再展开每一项的执行依据。")
      }
      coverList
    }
  }

  private func stepCard(step: String, done: Bool = false, title: String, detail: String) -> some View {
    VStack(alignment: .leading, spacing: 9) {
      HStack {
        Text(done ? "\(step) · 已完成" : step)
          .font(.system(size: 11, weight: .bold))
          .foregroundStyle(done ? Theme.pass : Theme.faint)
        Spacer()
        if done {
          Image(systemName: "checkmark")
            .font(.system(size: 8, weight: .bold)).foregroundStyle(Theme.pass)
            .frame(width: 18, height: 18)
            .background(Theme.passTint, in: Circle())
        }
      }
      Text(title).font(.system(size: 14, weight: .semibold))
      Text(detail).font(Theme.captionFont).foregroundStyle(Theme.muted)
        .fixedSize(horizontal: false, vertical: true)
    }
    .frame(maxWidth: .infinity, alignment: .leading)
    .card(padding: 16)
  }

  private var coverList: some View {
    VStack(spacing: 0) {
      HStack(alignment: .firstTextBaseline) {
        Text("一次体检覆盖什么").font(.system(size: 14, weight: .semibold))
        Spacer()
        Text("点击任意一项可单独检测").font(.system(size: 11.5)).foregroundStyle(Theme.faint)
      }
      .padding(.horizontal, 20).padding(.top, 15).padding(.bottom, 12)
      ForEach(CheckModule.testModules) { module in
        Button {
          store.prepareRun(module: module)
        } label: {
          HStack(spacing: 13) {
            Image(systemName: module.symbol)
              .font(.system(size: 14)).foregroundStyle(Theme.accent)
              .frame(width: 30, height: 30)
              .background(Theme.accent.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
            VStack(alignment: .leading, spacing: 2) {
              Text(module.shortTitle).font(.system(size: 12.5, weight: .medium))
              Text(module.coverDesc).font(.system(size: 11.5)).foregroundStyle(Theme.faint)
            }
            Spacer()
            Text("单独检测 →").font(.system(size: 11)).foregroundStyle(Theme.faint)
          }
          .padding(.horizontal, 20).padding(.vertical, 11)
          .contentShape(Rectangle())
          .overlay(alignment: .top) { Rectangle().fill(Theme.line).frame(height: 1) }
        }
        .buttonStyle(.plain)
        .disabled(store.running)
        .help("检测\(module.title)")
      }
    }
    .background(.white, in: RoundedRectangle(cornerRadius: Theme.radiusCard))
    .overlay(RoundedRectangle(cornerRadius: Theme.radiusCard).stroke(Theme.line, lineWidth: 1))
  }
}

// 体检环：五段弧 = 五个检测模块，颜色 = 该模块当次状态。
// 无记录时呈现空环，组件状态连续。
struct CoverageRing: View {
  var record: RunRecord?
  var size: CGFloat = 132
  private var modules: [CheckModule] { CheckModule.testModules }
  var body: some View {
    ZStack {
      ZStack {
        ForEach(0..<modules.count, id: \.self) { index in
          Circle()
            .trim(
              from: CGFloat(index) / CGFloat(modules.count) + 0.012,
              to: CGFloat(index + 1) / CGFloat(modules.count) - 0.012)
            .stroke(
              (record?.navDot(modules[index]) ?? .none).darkColor,
              style: StrokeStyle(lineWidth: 8, lineCap: .round))
        }
      }
      .rotationEffect(.degrees(-90))
      VStack(spacing: 2) {
        Text(centerTitle)
          .font(.system(size: 24, weight: .bold)).foregroundStyle(.white)
          .monospacedDigit()
        Text(centerSub)
          .font(.system(size: 10.5)).foregroundStyle(.white.opacity(0.5))
      }
    }
    .frame(width: size, height: size)
    .shadow(color: Color(red: 0.231, green: 0.510, blue: 0.965).opacity(0.25), radius: 14)
  }
  private var centerTitle: String {
    guard let record else { return "待检" }
    return "\(record.completed.count)/\(record.modules.count)"
  }
  private var centerSub: String { record == nil ? "还没有体检记录" : "模块已检" }
}

struct ResultView: View {
  @ObservedObject var store: Workbench
  var body: some View {
    if let record = store.currentRecord {
      ReportOverview(store: store, record: record)
    } else {
      VStack(alignment: .leading, spacing: 16) {
        Image(systemName: "chart.bar.doc.horizontal")
          .font(.system(size: 26, weight: .light))
          .foregroundStyle(Theme.faint)
          .frame(width: 56, height: 56)
          .background(Theme.unknownTint, in: Circle())
        Text("还没有检测结果").font(.system(size: 21, weight: .semibold))
        Text("完成一次检测后，这里会先给出使用结论，再展开分类表现和依据。")
          .font(Theme.bodyFont).foregroundStyle(Theme.muted)
        Action(title: "立即测试", icon: "play.fill", disabled: store.running) { store.prepareRun() }
      }
      .padding(.vertical, 40)
    }
  }
}

// 结论总览：结论（主角）→ 本次发现的问题（跨模块汇总）→
// 能做/注意/未测/下一步（故事线）→ 检测明细（目录）。容器越少，重点越清楚。
struct ReportOverview: View {
  @ObservedObject var store: Workbench
  var record: RunRecord
  var home = false
  private var problemModules: [CheckModule] {
    CheckModule.testModules.filter {
      record.navDot($0) == .fail || record.navDot($0) == .warn
    }
  }
  var body: some View {
    VStack(alignment: .leading, spacing: 30) {
      identityStrip
      hero
      if record.hasCurrentEvidence {
        if !problemModules.isEmpty { issueCard }
        story
        contents
      }
    }
  }

  private func dotStyle(_ dot: NavDot) -> StatusStyle {
    switch dot {
    case .pass: return FindingState.pass.style
    case .fail: return FindingState.fail.style
    case .warn: return FindingState.unstable.style
    case .info: return Theme.informative
    case .none: return FindingState.unknown.style
    }
  }

  // MARK: 报告抬头：这次测的是谁、什么时候、测了多少；审计元信息在「接入信息」页

  private var identityStrip: some View {
    HStack(spacing: 12) {
      Image(systemName: "cpu")
        .font(.system(size: 13))
        .foregroundStyle(Theme.muted)
        .frame(width: 30, height: 30)
        .background(Theme.canvas, in: RoundedRectangle(cornerRadius: 8))
      VStack(alignment: .leading, spacing: 2.5) {
        Text(record.service.model)
          .font(.system(size: 13.5, weight: .semibold))
          .lineLimit(1)
          .textSelection(.enabled)
        Text(record.service.displayURL)
          .font(.system(size: 11)).foregroundStyle(Theme.faint)
          .lineLimit(1).truncationMode(.middle)
          .help(record.service.displayURL)
          .textSelection(.enabled)
      }
      Spacer(minLength: 14)
      VStack(alignment: .trailing, spacing: 4) {
        Text("\(home ? "最近一次检测" : "本次检测") · \(record.date.formatted(date: .abbreviated, time: .shortened))")
          .font(.system(size: 11)).foregroundStyle(Theme.faint)
        HStack(spacing: 8) {
          if record.stopped {
            Pill(text: "已停止", style: Outcome.inconclusive.style, icon: "stop.fill")
          }
          Text("\(record.completed.count) / \(record.modules.count) 个模块")
            .font(.system(size: 11)).foregroundStyle(Theme.muted).monospacedDigit()
        }
      }
    }
    .padding(14)
    .frame(maxWidth: .infinity, alignment: .leading)
    .background(.white, in: RoundedRectangle(cornerRadius: Theme.radiusCard))
    .overlay(RoundedRectangle(cornerRadius: Theme.radiusCard).stroke(Theme.line, lineWidth: 1))
  }

  // MARK: 主角：无边框的大结论（身份信息在报告抬头，这里只放结论）

  private var hero: some View {
    VStack(alignment: .leading, spacing: 0) {
      HStack(alignment: .top, spacing: 17) {
        Image(systemName: record.style.icon)
          .font(.system(size: 23, weight: .medium))
          .foregroundStyle(record.style.color)
          .frame(width: 50, height: 50)
          .background(record.style.tint, in: Circle())
          .overlay(Circle().stroke(record.style.color.opacity(0.14), lineWidth: 1))
        VStack(alignment: .leading, spacing: 8) {
          Text(record.admissionTitle)
            .font(.system(size: 29, weight: .bold))
            .foregroundStyle(Theme.ink)
            .fixedSize(horizontal: false, vertical: true)
          Text(record.admissionExplanation)
            .font(.system(size: 14.5)).foregroundStyle(Theme.muted)
            .fixedSize(horizontal: false, vertical: true)
        }.padding(.top, 2)
      }
      HStack(spacing: 12) {
        if record.stopped && record.service == store.service {
          Action(title: "补测未完成项", icon: "play.fill", disabled: store.running) {
            store.selectedRecordID = record.id
            store.prepareMissing()
          }
        } else {
          Action(title: "重新检测", icon: "arrow.clockwise", disabled: store.running) {
            store.prepareRun()
          }
        }
        // 首页与结果页都直接给导出入口：导出单位是「当前这一次检测」的报告
        Action(title: "导出报告", icon: "square.and.arrow.up", primary: false) {
          exportRecord(record, store: store)
        }
      }
      .padding(.top, 22)
      .padding(.leading, 67)
    }
  }

  // MARK: 故事线：能做 → 注意 → 未测

  private var story: some View {
    VStack(alignment: .leading, spacing: 0) {
      storyRow(
        icon: "checkmark", color: Theme.pass, tint: Theme.passTint, title: "能做什么",
        text: record.usableScope)
      storyRow(
        icon: "exclamationmark", color: Theme.limited, tint: Theme.limitedTint, title: "要注意",
        text: record.limitation)
      storyRow(
        icon: "questionmark", color: Theme.unknown, tint: Theme.unknownTint, title: "还没测",
        text: record.missingScope)
      storyRow(
        icon: "arrow.turn.down.right", color: Theme.accent, tint: Theme.infoTint, title: "下一步",
        text: record.admissionNextStep, last: true)
    }
  }

  private func storyRow(
    icon: String, color: Color, tint: Color, title: String, text: String, last: Bool = false
  ) -> some View {
    HStack(alignment: .top, spacing: 15) {
      VStack(spacing: 0) {
        Image(systemName: icon)
          .font(.system(size: 11, weight: .bold))
          .foregroundStyle(color)
          .frame(width: 26, height: 26)
          .background(tint, in: Circle())
        if !last {
          Rectangle().fill(Theme.line).frame(width: 1.5)
            .frame(maxHeight: .infinity)
        }
      }.fixedSize(horizontal: false, vertical: last)
      HStack(alignment: .firstTextBaseline, spacing: 10) {
        Text(title)
          .font(.system(size: 13.5, weight: .semibold))
          .foregroundStyle(color)
        Text(text)
          .font(.system(size: 13)).foregroundStyle(Theme.muted)
          .fixedSize(horizontal: false, vertical: true)
        Spacer(minLength: 0)
      }.padding(.bottom, 16).padding(.top, 4)
    }
  }

  // MARK: 本次发现的问题：跨模块汇总，每条都能点进对应模块看依据

  private var issueCard: some View {
    VStack(alignment: .leading, spacing: 6) {
      SectionHeader(title: "本次发现的问题", caption: "按模块列出需要关注的发现，点击查看逐条依据")
      VStack(spacing: 0) {
        ForEach(problemModules) { module in
          Button {
            store.selectedRecordID = record.id
            store.showModule(module)
          } label: {
            HStack(alignment: .top, spacing: 12) {
              Image(
                systemName: record.navDot(module) == .fail
                  ? "xmark.circle.fill" : "exclamationmark.circle.fill"
              )
              .font(.system(size: 15))
              .foregroundStyle(dotStyle(record.navDot(module)).color)
              .padding(.top, 1)
              VStack(alignment: .leading, spacing: 3) {
                Text(module.shortTitle).font(.system(size: 13, weight: .semibold))
                Text(record.brief(module))
                  .font(Theme.captionFont).foregroundStyle(Theme.muted)
                  .fixedSize(horizontal: false, vertical: true)
              }
              Spacer()
              Text(record.navDot(module) == .fail ? "未通过" : "有限制")
                .font(Theme.captionFont)
                .foregroundStyle(dotStyle(record.navDot(module)).color)
              Image(systemName: "chevron.right")
                .font(.system(size: 10)).foregroundStyle(Theme.faint)
            }
            .padding(.vertical, 12)
            .contentShape(Rectangle())
          }
          .buttonStyle(.plain)
          .overlay(alignment: .bottom) { Rectangle().fill(Theme.line).frame(height: 1) }
        }
      }
    }
    .padding(18)
    .frame(maxWidth: .infinity, alignment: .leading)
    .background(Theme.limitedTint.opacity(0.5), in: RoundedRectangle(cornerRadius: Theme.radiusCard))
    .overlay(
      RoundedRectangle(cornerRadius: Theme.radiusCard)
        .stroke(Theme.limited.opacity(0.18), lineWidth: 1))
  }

  // MARK: 检测明细：目录而不是卡片墙

  private var contents: some View {
    VStack(alignment: .leading, spacing: 10) {
      SectionHeader(title: "诊断依据", caption: "以下检测共同支撑本次接入建议")
      VStack(spacing: 0) {
        ForEach([CheckModule.parameters, .functions, .performance, .agent, .comparison]) { module in
          moduleRow(module)
        }
      }
    }
  }

  private func moduleRow(_ module: CheckModule) -> some View {
    let selected = record.modules.contains(module)
    let done = record.completed.contains(module) && record.hasCurrentEvidence
    return Button {
      store.selectedRecordID = record.id
      store.showModule(module)
    } label: {
      HStack(spacing: 14) {
        Image(systemName: module.symbol)
          .font(.system(size: 13))
          .foregroundStyle(Theme.accent)
          .frame(width: 32, height: 32)
          .background(Theme.accent.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
        VStack(alignment: .leading, spacing: 4) {
          HStack(spacing: 9) {
            Text(module.title).font(.system(size: 13.5, weight: .semibold))
            statusPill(module, selected: selected, done: done)
          }
          Text(record.brief(module))
            .font(Theme.captionFont).foregroundStyle(Theme.muted)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        Image(systemName: "chevron.right")
          .font(.system(size: 10.5)).foregroundStyle(Theme.faint)
      }
      .padding(.vertical, 14)
      .overlay(alignment: .bottom) { Rectangle().fill(Theme.line).frame(height: 1) }
      .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
    .help("查看\(module.title)")
  }

  @ViewBuilder private func statusPill(_ module: CheckModule, selected: Bool, done: Bool) -> some View {
    if !selected {
      Pill(text: "本次未选", style: Outcome.inconclusive.style, icon: "minus")
    } else if !done {
      Pill(text: "未完成", style: Outcome.inconclusive.style, icon: "minus")
    } else {
      Pill(text: record.navDotLabel(module), style: dotStyle(record.navDot(module)))
    }
  }
}
