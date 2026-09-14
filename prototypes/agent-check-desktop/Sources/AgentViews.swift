import SwiftUI

// 智能体实测：先给整体判断，再逐项检查，每项都能落到执行记录。
struct AgentDetails: View {
  @ObservedObject var store: Workbench
  var record: RunRecord
  private var visible: [AgentFinding] {
    record.agentFindings.filter {
      store.agentFilter == "all"
        || (store.agentFilter == "problems" ? $0.state != .pass && !$0.unverified : $0.unverified)
    }
  }
  private var selected: AgentFinding? {
    visible.first { $0.id == store.agentSelection } ?? visible.first
  }
  var body: some View {
    VStack(alignment: .leading, spacing: 24) {
      VerdictBanner(style: record.style, title: record.title, detail: record.explanation)
      HStack(spacing: 16) {
        HStack(spacing: 8) {
          stat(
            record.agentFindings.filter { $0.state == .pass }.count, label: "项通过",
                style: FindingState.pass.style)
          stat(
            record.agentFindings.filter { $0.state == .fail || $0.state == .unstable }.count,
            label: "项需注意", style: FindingState.unstable.style)
          stat(
            record.agentFindings.filter { $0.state == .unknown }.count, label: "项未确认",
            style: FindingState.unknown.style)
        }
        Spacer()
        Text(record.coverageSummary)
          .font(Theme.captionFont).foregroundStyle(Theme.faint)
          .multilineTextAlignment(.trailing)
      }
      HStack {
        Picker("任务范围", selection: $store.agentTab) {
          Text("通用 Agent 任务").tag("general")
          Text("业务流程验证").tag("business")
        }.pickerStyle(.segmented).frame(width: 300)
        Spacer()
      }
      if store.agentTab == "business" {
        VStack(alignment: .leading, spacing: 14) {
          VerdictBanner(
            style: Theme.informative,
            title: "业务流程尚未检测",
            detail: "本次只检测了受控通用任务；你的真实业务任务需要单独验证。")
          VStack(spacing: 0) {
            KVRow(label: "尚未检测", value: "按业务技能完成操作")
            KVRow(label: "尚未检测", value: "调用工具调查并核对信息")
            KVRow(label: "尚未检测", value: "形成带证据的交付结果")
          }
        }
      } else {
        HStack(alignment: .top, spacing: 20) {
          VStack(spacing: 5) {
            Picker("检查项筛选", selection: $store.agentFilter) {
              Text("全部").tag("all")
              Text("问题").tag("problems")
              Text("尚未检测").tag("unverified")
            }.labelsHidden().pickerStyle(.segmented).padding(.bottom, 6)
            ForEach(visible) { finding in
              Button {
                store.agentSelection = finding.id
                store.agentSample = ""
                store.agentRun = 0
                store.evidenceExpanded = false
              } label: {
                HStack(spacing: 9) {
                  Image(systemName: finding.state.style.icon)
                    .font(.system(size: 12))
                    .foregroundStyle(finding.state.style.color)
                    .frame(width: 14)
                  Text(finding.name)
                    .font(.system(size: 11.5))
                    .lineLimit(2)
                    .multilineTextAlignment(.leading)
                    .foregroundStyle(Theme.ink)
                  Spacer(minLength: 2)
                  Text(finding.unverified ? "尚未检测" : finding.state.rawValue)
                    .font(.system(size: 10.5, weight: .medium))
                    .foregroundStyle(finding.state.style.color)
                }
                .padding(.horizontal, 10).frame(minHeight: 42)
                .background(
                  selected?.id == finding.id ? Theme.accent.opacity(0.06) : .white,
                  in: RoundedRectangle(cornerRadius: 9))
                .overlay(
                  RoundedRectangle(cornerRadius: 9)
                    .stroke(
                      selected?.id == finding.id ? Theme.accent.opacity(0.45) : Theme.line,
                      lineWidth: 1))
              }.buttonStyle(.plain)
            }
          }.frame(width: 262)
          if let finding = selected {
            AgentFindingView(store: store, record: record, finding: finding).id(finding.id)
          } else {
            VStack(alignment: .leading, spacing: 14) {
              Text("没有符合条件的检查项").font(.system(size: 16, weight: .semibold))
              Text("可以切换到全部检查项。").foregroundStyle(Theme.muted)
              Action(title: "查看全部", icon: "line.3.horizontal.decrease", primary: false) {
                store.agentFilter = "all"
              }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.top, 12)
          }
        }
      }
      Divider()
      Text("受控只读任务 · 检测样本集 3 · 不包含生产业务验收")
        .font(Theme.captionFont).foregroundStyle(Theme.faint)
    }
  }

  private func stat(_ value: Int, label: String, style: StatusStyle) -> some View {
    HStack(spacing: 7) {
      Text("\(value)").font(.system(size: 19, weight: .bold)).monospacedDigit()
        .foregroundStyle(style.color)
      Text(label).font(Theme.captionFont).foregroundStyle(Theme.muted)
    }
  }
}

// 单项检查：观察、影响、适用样本与执行记录。
struct AgentFindingView: View {
  @ObservedObject var store: Workbench
  var record: RunRecord
  var finding: AgentFinding
  private var samples: [AgentSample] {
    record.agentSamples.filter { finding.sampleIDs.contains($0.id) }
  }
  private var sample: AgentSample? { samples.first { $0.id == store.agentSample } ?? samples.first }
  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      SectionHeader(title: finding.name, caption: "检查项 A\(finding.id + 1)")
      HStack(alignment: .top, spacing: 10) {
        Pill(
          text: finding.unverified ? "尚未检测" : finding.state.rawValue,
          style: finding.state.style)
        Text(finding.observation)
          .font(Theme.bodyFont)
          .fixedSize(horizontal: false, vertical: true)
      }
      Text(finding.effect)
        .font(.system(size: 13)).foregroundStyle(Theme.muted)
        .fixedSize(horizontal: false, vertical: true)
      if let sample {
        Picker(
          "适用样本",
          selection: Binding(
            get: { sample.id },
            set: {
              store.agentSample = $0
              store.agentRun = 0
            })
        ) {
          ForEach(samples) { Text("\($0.id) · \($0.title)").tag($0.id) }
        }.frame(maxWidth: .infinity, alignment: .leading)
        VStack(alignment: .leading, spacing: 7) {
          Text("任务要求")
            .font(.system(size: 11.5, weight: .semibold)).foregroundStyle(Theme.faint)
          Text(sample.requirement)
            .font(.system(size: 13.5))
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(13)
            .background(Theme.canvas, in: RoundedRectangle(cornerRadius: 9))
        }.padding(.top, 4)
        VStack(spacing: 0) {
          KVRow(label: "完成情况", value: counts(sample))
          KVRow(label: "预期交付", value: sample.expected)
        }
        if !store.evidenceExpanded,
          let failedIndex = sample.runs.firstIndex(where: { $0.failedChecks.contains(finding.id) })
        {
          TextButton(title: "查看失败的那次执行") {
            store.agentRun = failedIndex
            store.evidenceExpanded = true
          }
        }
        Fold(title: "执行过程与交付", expanded: $store.evidenceExpanded) {
          if sample.runs.isEmpty {
            Text("此条件未执行，不计入已验证样本。").foregroundStyle(Theme.muted)
          } else {
            let index = min(max(0, store.agentRun), sample.runs.count - 1)
            Picker("执行记录", selection: Binding(get: { index }, set: { store.agentRun = $0 })) {
              ForEach(Array(sample.runs.enumerated()), id: \.offset) { index, run in
                Text(
                  "\(run.id) · \(run.phase) · \(!run.valid ? "无效" : run.failedChecks.contains(finding.id) ? "不通过" : "通过")"
                ).tag(index)
              }
            }
            let run = sample.runs[index]
            Text("记录 \(record.readableID)/\(run.id)")
              .font(.system(size: 10, design: .monospaced))
              .foregroundStyle(Theme.faint)
            VStack(alignment: .leading, spacing: 8) {
              ForEach(Array(run.steps.enumerated()), id: \.offset) { index, step in
                HStack(alignment: .top, spacing: 10) {
                  Text("\(index + 1)")
                    .font(.system(size: 10.5, weight: .semibold)).monospacedDigit()
                    .foregroundStyle(Theme.muted)
                    .frame(width: 21, height: 21)
                    .background(Theme.canvas, in: Circle())
                  Text(step)
                    .font(.system(size: 12.5))
                    .fixedSize(horizontal: false, vertical: true)
                }.padding(.vertical, 3)
              }
            }
            ResultBlock(
              title: "实际交付",
              text: run.delivery,
              style: run.failedChecks.contains(finding.id)
                ? FindingState.fail.style : FindingState.pass.style)
            Text("同一次执行同时支撑 " + sample.checks.map { "A\($0 + 1)" }.joined(separator: "、"))
              .font(.system(size: 10.5)).foregroundStyle(Theme.faint)
          }
        }
      }
    }
    .frame(maxWidth: .infinity, alignment: .leading)
  }
  private func passed(_ runs: [EvidenceRun]) -> Int {
    runs.filter { !$0.failedChecks.contains(finding.id) }.count
  }
  private func counts(_ sample: AgentSample) -> String {
    guard !sample.validRuns.isEmpty else {
      return "尚未取得有效执行；无效尝试 \(sample.runs.filter { !$0.valid }.count) 次，不计为模型失败。"
    }
    let initial = sample.validRuns.filter { $0.phase == "初测" }
    let reviews = sample.validRuns.filter { $0.phase == "失败复核" }
    let review =
      reviews.isEmpty
      ? (passed(initial) < initial.count ? "复核尚未完成" : "未触发失败复核")
      : "复核 \(passed(reviews))/\(reviews.count)"
    return
      "初测 \(passed(initial))/\(initial.count) · \(review) · 合计 \(passed(sample.validRuns))/\(sample.validRuns.count) 次通过"
  }
}
