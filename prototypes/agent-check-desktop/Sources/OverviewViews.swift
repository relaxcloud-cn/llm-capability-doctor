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

// 首页：没有结果时做产品自我介绍；有结果时先给结论。
struct HomeView: View {
  @ObservedObject var store: Workbench
  var body: some View {
    VStack(alignment: .leading, spacing: 30) {
      if let record = store.latestForService {
        ReportOverview(store: store, record: record, home: true)
      } else {
        welcome
      }
    }
  }

  private var welcome: some View {
    VStack(alignment: .leading, spacing: 34) {
      VStack(alignment: .leading, spacing: 16) {
        Pill(text: "待检测", style: Outcome.inconclusive.style)
        Text("这个模型现在适合接入智能体产品吗？")
          .font(.system(size: 30, weight: .bold))
          .fixedSize(horizontal: false, vertical: true)
          .textSelection(.enabled)
        Text("AgentCheck 在模型接入前做全方位诊断，提前发现不可用问题，帮助你在接入前完成修复。")
          .font(.system(size: 14)).foregroundStyle(Theme.muted)
          .fixedSize(horizontal: false, vertical: true)
        HStack(spacing: 14) {
          Action(title: "开始接入前诊断", icon: "play.fill") { store.prepareRun() }
        }.padding(.top, 4)
      }
      HStack(alignment: .top, spacing: 16) {
        stepCard(
          "1", icon: "plug", title: "连接服务",
          detail: "填写服务地址、模型名称和 API Key，只测你自己的服务。")
        stepCard(
          "2", icon: "checklist", title: "选择检测",
          detail: "规格、能力、性能、智能体任务与基线对照，可以全测也可以单测。")
        stepCard(
          "3", icon: "doc.text.magnifyingglass", title: "查看结论",
          detail: "先看「能不能用」与限制，再按需展开每一项的执行记录。")
      }
      Divider()
      VStack(alignment: .leading, spacing: 16) {
        SectionHeader(title: "也可以只测一项")
        VStack(spacing: 4) {
          ForEach(CheckModule.testModules) { module in
            Button {
              store.prepareRun(module: module)
            } label: {
              HStack(spacing: 13) {
                Image(systemName: module.symbol)
                  .font(.system(size: 14))
                  .foregroundStyle(Theme.accent)
                  .frame(width: 32, height: 32)
                  .background(Theme.accent.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
                VStack(alignment: .leading, spacing: 2.5) {
                  Text(module.title).font(.system(size: 13, weight: .medium))
                  Text(module.subtitle).font(Theme.captionFont).foregroundStyle(Theme.faint)
                }
                Spacer()
                Image(systemName: "play").font(.system(size: 11)).foregroundStyle(Theme.faint)
              }
              .padding(.horizontal, 10).padding(.vertical, 9)
              .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("检测\(module.title)")
          }
        }
      }
      Divider()
      AccessSummary(store: store, record: nil)
    }
  }

  private func stepCard(_ number: String, icon: String, title: String, detail: String) -> some View {
    VStack(alignment: .leading, spacing: 10) {
      HStack(spacing: 9) {
        Image(systemName: icon)
          .font(.system(size: 14))
          .foregroundStyle(Theme.accent)
          .frame(width: 32, height: 32)
          .background(Theme.accent.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
        Text(number)
          .font(.system(size: 12, weight: .semibold)).monospacedDigit()
          .foregroundStyle(Theme.faint)
      }
      Text(title).font(.system(size: 14, weight: .semibold))
      Text(detail).font(Theme.captionFont).foregroundStyle(Theme.muted)
        .fixedSize(horizontal: false, vertical: true)
    }
    .frame(maxWidth: .infinity, alignment: .leading)
    .card(padding: 18)
  }
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

// 结论总览按一条故事线组织：结论（主角）→ 能做/注意/未测（故事线）→
// 需要处理的问题（唯一的强调框）→ 检测明细（目录）。容器越少，重点越清楚。
struct ReportOverview: View {
  @ObservedObject var store: Workbench
  var record: RunRecord
  var home = false
  private var hasAgent: Bool {
    record.hasCurrentEvidence && record.completed.contains(.agent)
  }
  private var issues: [AgentFinding] {
    record.agentFindings.filter { $0.state == .fail || $0.state == .unstable }
  }
  var body: some View {
    VStack(alignment: .leading, spacing: 30) {
      identityStrip
      hero
      if record.hasCurrentEvidence {
        coverage
        story
        if !issues.isEmpty { issueCard }
        contents
      }
    }
  }

  private var coverage: some View {
    VStack(alignment: .leading, spacing: 0) {
      SectionHeader(title: "智能体接入覆盖", caption: "说明本次诊断覆盖什么，以及结论能支持到哪里")
      VStack(spacing: 0) {
        coverageRow(title: "已覆盖行为", text: record.agentCoverage, icon: "checkmark.circle.fill", color: Theme.pass)
        coverageRow(title: "对应接入风险", text: record.agentRisk, icon: "exclamationmark.triangle.fill", color: Theme.limited)
        coverageRow(title: "尚未覆盖", text: record.agentUncovered, icon: "questionmark.circle.fill", color: Theme.unknown)
        coverageRow(title: "下一步", text: record.admissionNextStep, icon: "arrow.turn.down.right", color: Theme.accent, last: true)
      }
    }
    .padding(18)
    .frame(maxWidth: .infinity, alignment: .leading)
    .background(Theme.infoTint.opacity(0.55), in: RoundedRectangle(cornerRadius: Theme.radiusCard))
    .overlay(RoundedRectangle(cornerRadius: Theme.radiusCard).stroke(Theme.accent.opacity(0.14), lineWidth: 1))
  }

  private func coverageRow(
    title: String, text: String, icon: String, color: Color, last: Bool = false
  ) -> some View {
    HStack(alignment: .top, spacing: 10) {
      Image(systemName: icon)
        .font(.system(size: 12, weight: .semibold))
        .foregroundStyle(color)
        .frame(width: 18)
      VStack(alignment: .leading, spacing: 2) {
        Text(title).font(.system(size: 12.5, weight: .semibold)).foregroundStyle(Theme.ink)
        Text(text).font(Theme.captionFont).foregroundStyle(Theme.muted)
          .fixedSize(horizontal: false, vertical: true)
      }
      Spacer(minLength: 0)
    }
    .padding(.vertical, 9)
    .overlay(alignment: .bottom) {
      if !last { Rectangle().fill(Theme.line).frame(height: 1) }
    }
  }

  // MARK: 报告抬头：这次测的是谁、什么时候、测了多少；就地展开审计详情

  private var identityStrip: some View {
    VStack(alignment: .leading, spacing: 0) {
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
        Button {
          withAnimation(.easeInOut(duration: 0.18)) { store.accessExpanded.toggle() }
        } label: {
          Image(systemName: store.accessExpanded ? "chevron.up" : "chevron.down")
            .font(.system(size: 10.5, weight: .semibold))
            .foregroundStyle(Theme.muted)
            .frame(width: 26, height: 26)
            .background(Theme.canvas, in: Circle())
        }
        .buttonStyle(.plain)
        .help(store.accessExpanded ? "收起检测信息" : "展开检测信息")
      }
      if store.accessExpanded {
        Rectangle().fill(Theme.line).frame(height: 1).padding(.top, 13)
        VStack(spacing: 0) {
          KVRow(label: "接口方式", value: "对话接口（OpenAI 格式）", note: "来源：本次接入配置")
          KVRow(
            label: "响应模型名", value: record.responseModel ?? "尚无可用响应记录",
            note: "来自响应的模型名称字段；不据此确认后端模型身份。")
          if let name = record.responseModel, name != record.service.model {
            KVRow(
              label: "名称差异", value: "配置名称与响应名称不同",
              note: "保留两侧原值，不推断为假模型或不可用。")
          }
          KVRow(
            label: "运行版本", value: "AgentCheck · macOS",
            note: "本地运行信息，不是服务端部署版本。")
          KVRow(
            label: "测试条件", value: record.context?.environment ?? "尚未建立测试记录",
            note: record.context?.parameters ?? "测试开始后与记录一起保存")
        }
        .padding(.top, 4)
      }
    }
    .padding(14)
    .frame(maxWidth: .infinity, alignment: .leading)
    .background(.white, in: RoundedRectangle(cornerRadius: Theme.radiusCard))
    .overlay(RoundedRectangle(cornerRadius: Theme.radiusCard).stroke(Theme.line, lineWidth: 1))
    .id("access")
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
        if home {
          Action(title: "导出报告", icon: "square.and.arrow.up", primary: false) {
            exportRecord(record, store: store)
          }
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
        text: record.missingScope, last: true)
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

  // MARK: 需要处理的问题：全页唯一的强调框

  private var issueCard: some View {
    VStack(alignment: .leading, spacing: 6) {
      SectionHeader(title: "接入前需要处理的问题", caption: "这些问题可能导致接入智能体产品后不可用，点击查看执行依据")
      VStack(spacing: 0) {
        ForEach(issues.prefix(3)) { finding in
          Button {
            store.selectedRecordID = record.id
            store.showModule(.agent)
            store.agentSelection = finding.id
          } label: {
            HStack(alignment: .top, spacing: 12) {
              Image(systemName: finding.state == .fail ? "xmark.circle.fill" : "exclamationmark.circle.fill")
                .font(.system(size: 15))
                .foregroundStyle(finding.state.style.color)
                .padding(.top, 1)
              VStack(alignment: .leading, spacing: 3) {
                Text(finding.name).font(.system(size: 13, weight: .semibold))
                Text(finding.observation)
                  .font(Theme.captionFont).foregroundStyle(Theme.muted)
                  .fixedSize(horizontal: false, vertical: true)
              }
              Spacer()
              Text(finding.unverified ? "尚未检测" : finding.state.rawValue)
                .font(Theme.captionFont).foregroundStyle(finding.state.style.color)
              Image(systemName: "chevron.right")
                .font(.system(size: 10)).foregroundStyle(Theme.faint)
            }
            .padding(.vertical, 12)
            .contentShape(Rectangle())
          }
          .buttonStyle(.plain)
          .overlay(alignment: .bottom) { Rectangle().fill(Theme.line).frame(height: 1) }
        }
        if issues.count > 3 {
          Text("还有 \(issues.count - 3) 项，见智能体实测。")
            .font(Theme.captionFont).foregroundStyle(Theme.faint)
            .padding(.top, 10)
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
          Text(module.scope)
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
    } else if module == .agent {
      let worst =
        record.agentFindings.first { $0.state == .fail }?.state
        ?? record.agentFindings.first { $0.state == .unstable }?.state
        ?? record.agentFindings.first { $0.state == .unknown }?.state
        ?? FindingState.pass
      Pill(text: worst.rawValue, style: worst.style)
    } else {
      Pill(text: "已完成", style: FindingState.pass.style)
    }
  }
}
