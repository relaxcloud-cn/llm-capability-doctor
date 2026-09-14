import AppKit
import SwiftUI

struct ModuleView: View {
  @ObservedObject var store: Workbench
  var module: CheckModule
  var body: some View {
    VStack(alignment: .leading, spacing: 26) {
      if module == .info {
        AccessSummary(store: store, record: store.currentRecord)
      } else if let record = store.currentRecord, record.completed.contains(module),
        record.hasCurrentEvidence
      {
        Group {
          if record.isRealReport {
            RealModuleView(store: store, module: module, record: record)
          } else {
            switch module {
            case .info: EmptyView()
            case .parameters:
              CatalogDetails(store: store, items: Catalog.specifications, performance: false)
            case .functions: ScoreDetails(store: store)
            case .performance:
              PerformanceDetails(store: store)
            case .agent: AgentDetails(store: store, record: record)
            case .comparison: BaselineDetails(store: store, record: record)
            }
          }
        }.id(record.id)
        Divider()
        HStack(spacing: 14) {
          Action(title: "单独复测", icon: "arrow.clockwise", primary: false, disabled: store.running) {
            store.prepareRun(module: module)
          }
          Text(
            record.service == store.service
              ? "新建一条检测记录，保留本次结果。" : "将使用当前服务：\(store.service?.model ?? "")"
          )
          .font(Theme.captionFont).foregroundStyle(Theme.faint).lineLimit(2)
          Spacer()
        }
      } else {
        EmptyModule(store: store, module: module, record: store.currentRecord)
      }
    }
  }
}

// 规格实测：左侧七类规格，右侧「测到了哪里 + 意味着什么 + 依据」。
struct CatalogDetails: View {
  @ObservedObject var store: Workbench
  var items: [CatalogItem]
  var performance: Bool
  private var selected: CatalogItem { items.first { $0.id == store.catalogSelection } ?? items[0] }
  var body: some View {
    VStack(alignment: .leading, spacing: 24) {
      VerdictBanner(
        style: performance ? Theme.informative : Theme.informative,
        title: performance ? "首段等待 0.8 秒，较高并发下出现超时" : "输入测到 8K，输出测到 1,024 token",
        detail: performance
          ? "首段等待中位数 0.8 秒；并发 4 时 11/12 完成，连续运行 5 分钟内 29/30 完成。不同负载分开记录。"
          : "没有探测到容量上限；更长输入尚未检测。参数被接受，但生效与否还没有对照证据。"
      )
      HStack(alignment: .top, spacing: 20) {
        VStack(spacing: 6) {
          ForEach(items) { item in
            Button {
              store.catalogSelection = item.id
              store.evidenceExpanded = false
            } label: {
              HStack(spacing: 9) {
                Image(systemName: item.state.style.icon)
                  .font(.system(size: 12))
                  .foregroundStyle(item.state.style.color)
                  .frame(width: 15)
                VStack(alignment: .leading, spacing: 4) {
                  Text(item.title)
                    .font(.system(size: 12.5, weight: .semibold))
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
                  Text(item.value)
                    .font(Theme.captionFont).foregroundStyle(Theme.faint)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 2)
              }
              .padding(11)
              .frame(maxWidth: .infinity, minHeight: 64, alignment: .leading)
              .background(
                selected.id == item.id ? Theme.accent.opacity(0.06) : .white,
                in: RoundedRectangle(cornerRadius: 10))
              .overlay(
                RoundedRectangle(cornerRadius: 10)
                  .stroke(
                    selected.id == item.id ? Theme.accent.opacity(0.45) : Theme.line,
                    lineWidth: 1))
            }
            .buttonStyle(.plain)
          }
        }.frame(width: 240)
        VStack(alignment: .leading, spacing: 16) {
          SectionHeader(title: selected.title)
          Text(selected.value)
            .font(.system(size: 20, weight: .semibold))
            .fixedSize(horizontal: false, vertical: true)
          Text(selected.summary)
            .font(Theme.bodyFont).foregroundStyle(Theme.muted)
            .fixedSize(horizontal: false, vertical: true)
          HStack(alignment: .top, spacing: 10) {
            Image(systemName: "info.circle")
              .font(.system(size: 12))
              .foregroundStyle(Theme.limited)
              .padding(.top, 1)
            VStack(alignment: .leading, spacing: 3) {
              Text("这个结论的边界")
                .font(.system(size: 11.5, weight: .semibold)).foregroundStyle(Theme.limited)
              Text(selected.boundary)
                .font(Theme.captionFont).foregroundStyle(Theme.muted)
                .fixedSize(horizontal: false, vertical: true)
            }
          }
          .padding(13)
          .frame(maxWidth: .infinity, alignment: .leading)
          .background(Theme.limitedTint.opacity(0.55), in: RoundedRectangle(cornerRadius: 9))
          SectionHeader(title: "本次测量")
          VStack(spacing: 0) { ForEach(selected.facts) { KVRow(label: $0.label, value: $0.value) } }
          Fold(title: "逐次记录", expanded: $store.evidenceExpanded) {
            ForEach(selected.evidence) { KVRow(label: $0.label, value: $0.value) }
          }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
      }
    }
  }
}

// 能力跑分：分类成绩 + 单个样例的输入 / 预期 / 实际对照。
struct ScoreDetails: View {
  @ObservedObject var store: Workbench
  @State private var caseID = ""
  @State private var length = "4K"
  private var category: ScoreCategory {
    Scores.all.first { $0.id == store.catalogSelection } ?? Scores.all[0]
  }
  private var cases: [ScoreCase] {
    length == "1K" && !category.comparisonCases.isEmpty ? category.comparisonCases : category.cases
  }
  private var sample: ScoreCase {
    cases.first { $0.id == caseID } ?? cases.first { !$0.passed } ?? cases[0]
  }
  var body: some View {
    VStack(alignment: .leading, spacing: 22) {
      VerdictBanner(
        style: Theme.informative,
        title: "信息提取完整，工具选择与长材料是短板",
        detail: "六类任务分别记录完成情况，每类 5 个样例。成绩不替代 Agent 任务验证。"
      )
      HStack(alignment: .top, spacing: 20) {
        VStack(spacing: 6) {
          ForEach(Scores.all) { item in
            Button {
              store.catalogSelection = item.id
              caseID = ""
            } label: {
              VStack(alignment: .leading, spacing: 8) {
                HStack {
                  Text(item.title)
                    .font(.system(size: 11.5, weight: .medium))
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
                  Spacer(minLength: 3)
                  Text(item.result)
                    .font(.system(size: 12.5, weight: .semibold)).monospacedDigit()
                    .foregroundStyle(
                      item.correct == item.cases.count ? Theme.pass : Theme.ink)
                }
                SampleBar(passed: item.correct, total: item.cases.count)
              }
              .padding(11)
              .frame(maxWidth: .infinity, minHeight: 62, alignment: .leading)
              .background(
                category.id == item.id ? Theme.accent.opacity(0.06) : .white,
                in: RoundedRectangle(cornerRadius: 10))
              .overlay(
                RoundedRectangle(cornerRadius: 10)
                  .stroke(
                    category.id == item.id ? Theme.accent.opacity(0.45) : Theme.line,
                    lineWidth: 1))
            }
            .buttonStyle(.plain)
          }
          Text("绿色 = 正确完成 / 已测样例")
            .font(Theme.captionFont).foregroundStyle(Theme.faint).padding(.top, 6)
        }.frame(width: 258)
        VStack(alignment: .leading, spacing: 14) {
          SectionHeader(title: category.title)
          HStack(alignment: .firstTextBaseline, spacing: 9) {
            Text("\(cases.filter(\.passed).count) / \(cases.count)")
              .font(.system(size: 29, weight: .bold)).monospacedDigit()
            Text("个样例正确完成")
              .font(Theme.bodyFont).foregroundStyle(Theme.muted)
          }
          Text(category.summary)
            .font(.system(size: 14, weight: .medium))
            .fixedSize(horizontal: false, vertical: true)
          Text(category.condition)
            .font(Theme.captionFont).foregroundStyle(Theme.faint)
            .fixedSize(horizontal: false, vertical: true)
          if !category.comparisonCases.isEmpty {
            Picker("材料长度", selection: $length) {
              Text("1K · 5/5").tag("1K")
              Text("4K · 3/5").tag("4K")
            }.pickerStyle(.segmented).frame(width: 280)
          }
          Divider()
          HStack(spacing: 7) {
            Text("样例").font(Theme.captionFont).foregroundStyle(Theme.faint)
            ForEach(Array(cases.enumerated()), id: \.element.id) { index, entry in
              Button {
                caseID = entry.id
              } label: {
                HStack(spacing: 4) {
                  Text("\(index + 1)").monospacedDigit()
                  Image(systemName: entry.passed ? "checkmark" : "xmark")
                    .font(.system(size: 8.5, weight: .bold))
                }
                .font(.system(size: 11.5))
                .foregroundStyle(entry.passed ? Theme.pass : Theme.blocked)
                .frame(width: 44, height: 30)
                .background(
                  entry.passed ? Theme.passTint : Theme.blockedTint,
                  in: RoundedRectangle(cornerRadius: 7))
                .overlay(
                  RoundedRectangle(cornerRadius: 7)
                    .stroke(sample.id == entry.id ? Theme.ink.opacity(0.55) : .clear, lineWidth: 1.5))
              }
              .buttonStyle(.plain)
              .help("样例 \(index + 1)：\(entry.passed ? "正确完成" : "未正确完成")")
            }
          }
          VStack(alignment: .leading, spacing: 7) {
            Text("任务输入")
              .font(.system(size: 11.5, weight: .semibold)).foregroundStyle(Theme.faint)
            Text(sample.input)
              .font(.system(size: 13.5)).textSelection(.enabled)
              .fixedSize(horizontal: false, vertical: true)
              .frame(maxWidth: .infinity, alignment: .leading)
              .padding(13)
              .background(Theme.canvas, in: RoundedRectangle(cornerRadius: 9))
          }.padding(.top, 4)
          ResultBlock(
            title: "预期结果", text: sample.expected, style: FindingState.unknown.style)
          ResultBlock(
            title: "实际结果 · \(sample.passed ? "正确完成" : "未正确完成")",
            text: sample.actual,
            style: sample.passed ? FindingState.pass.style : FindingState.fail.style)
          Text("依据：逐项核对固定答案与任务要求 · 检测样本集 3。")
            .font(Theme.captionFont).foregroundStyle(Theme.faint)
            .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
      }
    }
  }
}

// 结果对照块：预期 / 实际 / 交付共用的着色版式。
struct ResultBlock: View {
  var title: String
  var text: String
  var style: StatusStyle
  var body: some View {
    VStack(alignment: .leading, spacing: 7) {
      Text(title)
        .font(.system(size: 11.5, weight: .semibold)).foregroundStyle(style.color)
      Text(text)
        .font(.system(size: 13.5)).textSelection(.enabled)
        .fixedSize(horizontal: false, vertical: true)
    }
    .padding(13)
    .frame(maxWidth: .infinity, alignment: .leading)
    .background(style.tint.opacity(0.5), in: RoundedRectangle(cornerRadius: 9))
    .overlay(alignment: .leading) {
      UnevenRoundedRectangle(topLeadingRadius: 9, bottomLeadingRadius: 9)
        .fill(style.color).frame(width: 3)
    }
  }
}
