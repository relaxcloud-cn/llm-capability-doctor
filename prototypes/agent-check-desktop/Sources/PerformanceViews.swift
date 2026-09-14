import SwiftUI

// 性能实测：先看三个关键数字，再按负载类型查看曲线与条件。
struct PerformanceDetails: View {
  @ObservedObject var store: Workbench
  private var selected: CatalogItem {
    Catalog.performance.first { $0.id == store.catalogSelection } ?? Catalog.performance[0]
  }
  private let names = ["响应耗时", "生成速度", "并发负载", "持续运行", "长输入与输出"]
  var body: some View {
    VStack(alignment: .leading, spacing: 24) {
      VerdictBanner(
        style: Theme.informative,
        title: "首段等待 0.8 秒，较高并发下出现超时",
        detail: "固定负载下的观测结果。并发 4 时 11/12 完成，连续运行 5 分钟内 29/30 完成。"
      )
      HStack(alignment: .top, spacing: 14) {
        HeroStat(label: "首段等待", value: "0.8", unit: "秒", meaning: "3 次请求 · 中位数")
        HeroStat(label: "完整响应", value: "6.8", unit: "秒", meaning: "输入 128 / 输出 252 token")
        HeroStat(label: "生成速度", value: "42", unit: "token/s", meaning: "不含首段等待")
      }.card(padding: 20)
      HStack(spacing: 0) {
        ForEach(Array(Catalog.performance.enumerated()), id: \.element.id) { index, item in
          Button {
            store.catalogSelection = item.id
            store.evidenceExpanded = false
          } label: {
            Text(names[index])
              .font(.system(size: 12.5, weight: selected.id == item.id ? .semibold : .regular))
              .foregroundStyle(selected.id == item.id ? Theme.accent : Theme.muted)
              .frame(maxWidth: .infinity)
              .frame(height: 38)
              .overlay(alignment: .bottom) {
                Rectangle()
                  .fill(selected.id == item.id ? Theme.accent : .clear)
                  .frame(height: 2.5)
              }
          }.buttonStyle(.plain)
        }
      }
      .overlay(alignment: .bottom) { Rectangle().fill(Theme.line).frame(height: 1).zIndex(-1) }
      HStack(alignment: .top, spacing: 26) {
        VStack(alignment: .leading, spacing: 18) {
          SectionHeader(title: selected.title)
          Text(selected.value)
            .font(.system(size: 19, weight: .semibold))
            .fixedSize(horizontal: false, vertical: true)
          chart
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        VStack(alignment: .leading, spacing: 12) {
          SectionHeader(title: "测试条件")
          VStack(spacing: 0) { ForEach(selected.facts) { KVRow(label: $0.label, value: $0.value) } }
          Text(selected.boundary)
            .font(Theme.captionFont).foregroundStyle(Theme.faint)
            .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
      }.padding(.vertical, 4)
      Fold(title: "逐次测量记录", expanded: $store.evidenceExpanded) {
        ForEach(selected.evidence) { KVRow(label: $0.label, value: $0.value) }
      }
      Text("固定演示服务与参数组。当前负载是演示条件，不是生产性能承诺。")
        .font(Theme.captionFont).foregroundStyle(Theme.faint)
    }
  }

  // 图表遵循的规则：细标记、圆角端点、数值直接标注、状态色只表达状态。
  @ViewBuilder private var chart: some View {
    switch selected.id {
    case "P1":
      VStack(alignment: .leading, spacing: 14) {
        HStack(spacing: 14) {
          legend(color: Theme.accent, label: "首段等待")
          legend(color: Theme.accent.opacity(0.35), label: "后续生成")
        }
        ForEach(Array(zip([0.8, 0.9, 0.7], [6.8, 7.1, 6.5]).enumerated()), id: \.offset) {
          index, values in
          VStack(alignment: .leading, spacing: 6) {
            HStack {
              Text("请求 \(index + 1)")
              Spacer()
              Text("\(values.0.formatted()) / \(values.1.formatted()) 秒").monospacedDigit()
            }.font(Theme.captionFont).foregroundStyle(Theme.muted)
            GeometryReader { geometry in
              HStack(spacing: 2) {
                UnevenRoundedRectangle(topLeadingRadius: 4, bottomLeadingRadius: 4)
                  .fill(Theme.accent)
                  .frame(width: max(3, geometry.size.width * values.0 / 8))
                UnevenRoundedRectangle(bottomTrailingRadius: 4, topTrailingRadius: 4)
                  .fill(Theme.accent.opacity(0.35))
                  .frame(width: max(3, geometry.size.width * (values.1 - values.0) / 8))
                Spacer(minLength: 0)
              }
            }.frame(height: 16)
          }
        }
        Text("标注为首段等待 / 完整响应；同一刻度，单位秒。")
          .font(.system(size: 10.5)).foregroundStyle(Theme.faint)
      }
    case "P2":
      VStack(alignment: .leading, spacing: 14) {
        dataBar("请求 1", value: 42, scale: 50, label: "42.0 token/s")
        dataBar("请求 2", value: 252 / 6.2, scale: 50, label: "40.6 token/s")
        dataBar("请求 3", value: 252 / 5.8, scale: 50, label: "43.4 token/s")
        Text("252 输出 token ÷ 实际生成时间；不含等待。")
          .font(.system(size: 10.5)).foregroundStyle(Theme.faint)
      }
    case "P3":
      VStack(alignment: .leading, spacing: 16) {
        dataBar(
          "并发 1", value: 3, scale: 3, label: "3 / 3 完成",
          style: FindingState.pass.style)
        dataBar(
          "并发 2", value: 6, scale: 6, label: "6 / 6 完成",
          style: FindingState.pass.style)
        dataBar(
          "并发 4", value: 11, scale: 12, label: "11 / 12 完成",
          style: FindingState.unstable.style)
        Label("并发 4 时有 1 次请求超过 15 秒预算。", systemImage: "exclamationmark.circle")
          .font(Theme.captionFont).foregroundStyle(Theme.limited)
      }
    case "P4":
      VStack(alignment: .leading, spacing: 14) {
        HStack(spacing: 2) {
          ForEach(0..<30, id: \.self) { index in
            RoundedRectangle(cornerRadius: 2)
              .fill(index == 17 ? Theme.blockedBar : Theme.passBar.opacity(0.7))
              .frame(height: 40)
              .help("请求 \(index + 1)：\(index == 17 ? "服务错误" : "完成")")
          }
        }
        HStack {
          Text("第 1 次")
          Spacer()
          Text("第 30 次")
        }.font(.system(size: 10.5)).foregroundStyle(Theme.faint)
        Label("第 18 次：服务错误；其余 29 次完成。", systemImage: "exclamationmark.circle")
          .font(Theme.captionFont).foregroundStyle(Theme.limited)
        Text("每 10 秒发起 1 次请求，并发 1；时间窗 5 分钟。")
          .font(Theme.captionFont).foregroundStyle(Theme.muted)
      }
    default:
      VStack(alignment: .leading, spacing: 16) {
        lengthPair("输入变长 · 1K → 8K", first: 6.8, second: 7.9)
        lengthPair("输出变长 · 252 → 1,024", first: 6.8, second: 25.3)
        lengthPair("历史变长 · 1K → 4K", first: 6.9, second: 7.5)
        Text("完整响应中位数，单位秒；三组分别控制变量。")
          .font(.system(size: 10.5)).foregroundStyle(Theme.faint)
      }
    }
  }

  private func legend(color: Color, label: String) -> some View {
    HStack(spacing: 6) {
      RoundedRectangle(cornerRadius: 2).fill(color).frame(width: 11, height: 11)
      Text(label).font(Theme.captionFont).foregroundStyle(Theme.muted)
    }
  }

  private func dataBar(
    _ title: String, value: Double, scale: Double, label: String, style: StatusStyle = Theme.informative
  ) -> some View {
    VStack(spacing: 7) {
      HStack {
        Text(title)
        Spacer()
        Text(label).monospacedDigit()
      }.font(Theme.captionFont).foregroundStyle(Theme.muted)
      GeometryReader { geometry in
        ZStack(alignment: .leading) {
          Capsule().fill(Theme.canvas)
          UnevenRoundedRectangle(bottomTrailingRadius: 4, topTrailingRadius: 4)
            .fill(style.bar.opacity(0.85))
            .frame(width: max(4, geometry.size.width * value / scale))
        }
      }.frame(height: 12)
    }
  }

  private func lengthPair(_ title: String, first: Double, second: Double) -> some View {
    VStack(alignment: .leading, spacing: 6) {
      HStack {
        Text(title)
        Spacer()
        Text("\(first.formatted()) → \(second.formatted()) 秒").monospacedDigit()
      }.font(Theme.captionFont).foregroundStyle(Theme.muted)
      GeometryReader { geometry in
        VStack(alignment: .leading, spacing: 2) {
          UnevenRoundedRectangle(bottomTrailingRadius: 4, topTrailingRadius: 4)
            .fill(Theme.accent.opacity(0.3))
            .frame(width: max(4, geometry.size.width * first / 26))
          UnevenRoundedRectangle(bottomTrailingRadius: 4, topTrailingRadius: 4)
            .fill(Theme.accent)
            .frame(width: max(4, geometry.size.width * second / 26))
        }
      }.frame(height: 16)
    }
  }
}
