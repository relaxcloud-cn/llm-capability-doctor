import SwiftUI

// 全局设计系统：颜色、字号层级与通用组件。
// 状态色只用于状态（通过/受限/阻断/未知），永远伴随图标与文字，不单靠颜色区分。
enum Theme {
  // 文字
  static let ink = Color(red: 0.08, green: 0.10, blue: 0.14)
  static let muted = Color(red: 0.36, green: 0.40, blue: 0.46)
  static let faint = Color(red: 0.56, green: 0.60, blue: 0.66)

  // 界面
  static let canvas = Color(red: 0.955, green: 0.96, blue: 0.968)
  static let line = Color(red: 0.895, green: 0.905, blue: 0.922)
  static let accent = Color(red: 0.145, green: 0.388, blue: 0.922)
  static let accentDeep = Color(red: 0.11, green: 0.306, blue: 0.78)

  // 状态（文字 / 浅底 / 图表填充）
  static let pass = Color(red: 0.02, green: 0.424, blue: 0.306)
  static let passTint = Color(red: 0.914, green: 0.965, blue: 0.941)
  static let passBar = Color(red: 0.02, green: 0.588, blue: 0.412)

  static let limited = Color(red: 0.631, green: 0.384, blue: 0.027)
  static let limitedTint = Color(red: 0.984, green: 0.953, blue: 0.878)
  static let limitedBar = Color(red: 0.792, green: 0.541, blue: 0.02)

  static let blocked = Color(red: 0.725, green: 0.11, blue: 0.11)
  static let blockedTint = Color(red: 0.984, green: 0.918, blue: 0.918)
  static let blockedBar = Color(red: 0.863, green: 0.149, blue: 0.149)

  static let unknown = Color(red: 0.34, green: 0.376, blue: 0.431)
  static let unknownTint = Color(red: 0.937, green: 0.945, blue: 0.956)

  static let radiusCard: CGFloat = 12
  static let radiusControl: CGFloat = 8

  // 中性信息态：非结论性的模块摘要使用，避免滥用状态色。
  static let infoTint = Color(red: 0.937, green: 0.953, blue: 0.996)
  static let informative = StatusStyle(
    icon: "info.circle.fill", color: accent, tint: infoTint, bar: accent)

  static let bodyFont = Font.system(size: 13)
  static let captionFont = Font.system(size: 11.5)
}

// 状态在界面上的统一表达：图标、文字色、浅底、图表色。
struct StatusStyle {
  var icon: String
  var color: Color
  var tint: Color
  var bar: Color
}

extension FindingState {
  var style: StatusStyle {
    switch self {
    case .pass: return StatusStyle(icon: "checkmark.circle.fill", color: Theme.pass, tint: Theme.passTint, bar: Theme.passBar)
    case .unstable: return StatusStyle(icon: "exclamationmark.circle.fill", color: Theme.limited, tint: Theme.limitedTint, bar: Theme.limitedBar)
    case .fail: return StatusStyle(icon: "xmark.circle.fill", color: Theme.blocked, tint: Theme.blockedTint, bar: Theme.blockedBar)
    case .unknown: return StatusStyle(icon: "questionmark.circle.fill", color: Theme.unknown, tint: Theme.unknownTint, bar: Theme.unknown)
    }
  }
}

extension Outcome {
  var style: StatusStyle {
    switch self {
    case .usable: return StatusStyle(icon: "checkmark.seal.fill", color: Theme.pass, tint: Theme.passTint, bar: Theme.passBar)
    case .limited: return StatusStyle(icon: "exclamationmark.triangle.fill", color: Theme.limited, tint: Theme.limitedTint, bar: Theme.limitedBar)
    case .blocked: return StatusStyle(icon: "xmark.octagon.fill", color: Theme.blocked, tint: Theme.blockedTint, bar: Theme.blockedBar)
    case .inconclusive: return StatusStyle(icon: "questionmark.circle.fill", color: Theme.unknown, tint: Theme.unknownTint, bar: Theme.unknown)
    }
  }
}

extension RunRecord {
  // 首页与列表使用：优先展示可信的整体判断，停止 / 证据不足用未知态。
  var style: StatusStyle {
    if hasConfirmedBlocker { return Outcome.blocked.style }
    if stopped || !completed.contains(.agent) || !hasCurrentEvidence { return Outcome.inconclusive.style }
    return outcome.style
  }
}

// 模块状态点：侧栏大纲树、首页体检环、记录行指纹共用一套口径。
enum NavDot {
  case info, pass, warn, fail, none
  var color: Color {
    switch self {
    case .info: return Theme.accent
    case .pass: return Theme.passBar
    case .warn: return Theme.limitedBar
    case .fail: return Theme.blockedBar
    case .none: return .clear
    }
  }
  // 深色 hero 上的提亮版本，保证状态色在暗底上仍可辨认。
  var darkColor: Color {
    switch self {
    case .info: return Color(red: 0.231, green: 0.510, blue: 0.965)
    case .pass: return Color(red: 0.204, green: 0.827, blue: 0.600)
    case .warn: return Color(red: 0.984, green: 0.749, blue: 0.141)
    case .fail: return Color(red: 0.973, green: 0.443, blue: 0.443)
    case .none: return .white.opacity(0.14)
    }
  }
}

extension RunRecord {
  // CLI 模块终态 -> 状态点。未验证/不适用/未选择不是"有限制"，保持空心。
  static func realStateDot(_ state: String) -> NavDot {
    switch state {
    case "pass": return .pass
    case "fail": return .fail
    case "not_applicable", "not_selected", "unverified": return .none
    default: return .warn
    }
  }
  static func realStateLabel(_ state: String, info: Bool) -> String {
    switch state {
    case "pass": return info ? "已记录" : "已完成"
    case "fail": return "未通过"
    case "unsupported": return "不支持"
    case "inconclusive": return "无法判定"
    case "invalid_execution": return "执行无效"
    case "not_applicable": return "不适用"
    case "not_selected": return "本次未选"
    case "unverified": return "无证据"
    default: return "已结束"
    }
  }
  func navDot(_ module: CheckModule) -> NavDot {
    if module == .info {
      if isRealReport, let state = moduleStates?["ingress"] {
        let dot = Self.realStateDot(state)
        return dot == .pass ? .info : dot
      }
      return .info
    }
    guard modules.contains(module), completed.contains(module), hasCurrentEvidence else {
      return .none
    }
    if isRealReport, let backend = module.backendID, let state = moduleStates?[backend] {
      return Self.realStateDot(state)
    }
    // 演示记录与模块页同口径：有未通过项 -> 红点；全部未判定 -> 空心。
    // 「可以正常使用」的记录不出未通过项，与页内判定一致。
    let usable = outcome == .usable
    switch module {
    case .info: return .info
    case .agent:
      let samples = agentSamples
      if samples.allSatisfy({ $0.validRuns.isEmpty }) { return .none }
      return samples.flatMap(\.validRuns).contains { !$0.failedChecks.isEmpty } ? .fail : .pass
    case .parameters:
      return !usable && Catalog.specifications.contains { $0.state == .limited } ? .fail : .pass
    case .performance:
      return !usable && Catalog.performance.contains { $0.state == .limited } ? .fail : .pass
    case .functions:
      return !usable && Scores.all.contains { $0.correct < $0.cases.count } ? .fail : .pass
    case .comparison:
      return !usable && BaselineItem.all.contains(where: \.hasDifference) ? .fail : .pass
    }
  }
  func navDotLabel(_ module: CheckModule) -> String {
    if module == .info {
      if isRealReport, let state = moduleStates?["ingress"] {
        return Self.realStateLabel(state, info: true)
      }
      return "已记录"
    }
    if !modules.contains(module) { return "本次未选" }
    if !completed.contains(module) { return "未完成" }
    if !hasCurrentEvidence { return "旧版记录" }
    if isRealReport, let backend = module.backendID, let state = moduleStates?[backend] {
      return Self.realStateLabel(state, info: false)
    }
    switch navDot(module) {
    case .info: return "已记录"
    case .pass: return "已完成"
    case .warn: return "有限制"
    case .fail: return "有问题"
    case .none: return "无证据"
    }
  }
}

extension ObservationState {
  var style: StatusStyle {
    switch self {
    case .observed: return StatusStyle(icon: "checkmark.circle.fill", color: Theme.pass, tint: Theme.passTint, bar: Theme.passBar)
    case .partial: return StatusStyle(icon: "minus.circle.fill", color: Theme.limited, tint: Theme.limitedTint, bar: Theme.limitedBar)
    case .limited: return StatusStyle(icon: "exclamationmark.circle.fill", color: Theme.limited, tint: Theme.limitedTint, bar: Theme.limitedBar)
    case .unverified, .unknown: return StatusStyle(icon: "circle.dashed", color: Theme.unknown, tint: Theme.unknownTint, bar: Theme.unknown)
    }
  }
}

// 状态胶囊：浅底 + 图标 + 文字。
struct Pill: View {
  var text: String
  var style: StatusStyle
  var icon: String? = nil
  var body: some View {
    HStack(spacing: 4.5) {
      Image(systemName: icon ?? style.icon).font(.system(size: 10, weight: .semibold))
      Text(text).fixedSize(horizontal: false, vertical: true)
    }
    .font(.system(size: 11.5, weight: .medium))
    .foregroundStyle(style.color)
    .padding(.horizontal, 9)
    .frame(height: 24)
    .background(style.tint, in: Capsule())
  }
}

// 每页顶部的「第一眼答案」：大图标 + 大结论 + 一句话解释。
struct VerdictBanner<Actions: View>: View {
  var style: StatusStyle
  var title: String
  var detail: String
  var meta: String? = nil
  @ViewBuilder var actions: () -> Actions
  init(
    style: StatusStyle, title: String, detail: String, meta: String? = nil,
    @ViewBuilder actions: @escaping () -> Actions = { EmptyView() }
  ) {
    self.style = style
    self.title = title
    self.detail = detail
    self.meta = meta
    self.actions = actions
  }
  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      HStack(alignment: .top, spacing: 18) {
        Image(systemName: style.icon)
          .font(.system(size: 27, weight: .medium))
          .foregroundStyle(style.color)
          .frame(width: 58, height: 58)
          .background(style.tint, in: Circle())
          .overlay(Circle().stroke(style.color.opacity(0.14), lineWidth: 1))
        VStack(alignment: .leading, spacing: 7) {
          HStack(alignment: .firstTextBaseline, spacing: 12) {
            Text(title).font(.system(size: 25, weight: .bold)).foregroundStyle(Theme.ink)
              .fixedSize(horizontal: false, vertical: true)
            if let meta {
              Text(meta).font(Theme.captionFont).foregroundStyle(Theme.faint)
                .padding(.top, 5)
            }
          }
          Text(detail).font(.system(size: 13.5)).foregroundStyle(Theme.muted)
            .fixedSize(horizontal: false, vertical: true)
        }
        Spacer(minLength: 12)
      }
      HStack(spacing: 12) {
        actions()
        Spacer(minLength: 0)
      }.padding(.top, 18).padding(.leading, 76)
    }
    .padding(24)
    .frame(maxWidth: .infinity, alignment: .leading)
    .background(style.tint.opacity(0.55), in: RoundedRectangle(cornerRadius: Theme.radiusCard))
    .overlay(
      RoundedRectangle(cornerRadius: Theme.radiusCard)
        .stroke(style.color.opacity(0.16), lineWidth: 1)
    )
  }
}

struct PrimaryButtonStyle: ButtonStyle {
  @Environment(\.isEnabled) private var enabled
  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .font(.system(size: 13, weight: .medium))
      .foregroundStyle(.white)
      .padding(.horizontal, 16).frame(minHeight: 36)
      .background(
        configuration.isPressed ? Theme.accentDeep : Theme.accent,
        in: RoundedRectangle(cornerRadius: Theme.radiusControl))
      .opacity(enabled ? 1 : 0.38)
  }
}

struct SecondaryButtonStyle: ButtonStyle {
  @Environment(\.isEnabled) private var enabled
  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .font(.system(size: 13, weight: .medium))
      .foregroundStyle(Theme.ink)
      .padding(.horizontal, 14).frame(minHeight: 36)
      .background(.white, in: RoundedRectangle(cornerRadius: Theme.radiusControl))
      .overlay(
        RoundedRectangle(cornerRadius: Theme.radiusControl)
          .stroke(configuration.isPressed ? Theme.faint : Theme.line, lineWidth: 1))
      .opacity(enabled ? 1 : 0.38)
  }
}

struct Action: View {
  var title: String
  var icon = "arrow.right"
  var primary = true
  var disabled = false
  var action: () -> Void
  var body: some View {
    if primary {
      Button(action: action) { Label(title, systemImage: icon) }
        .buttonStyle(PrimaryButtonStyle())
        .disabled(disabled)
    } else {
      Button(action: action) { Label(title, systemImage: icon) }
        .buttonStyle(SecondaryButtonStyle())
        .disabled(disabled)
    }
  }
}

struct TextButton: View {
  var title: String
  var action: () -> Void
  var body: some View {
    Button(title, action: action)
      .buttonStyle(.plain)
      .font(.system(size: 12.5, weight: .medium))
      .foregroundStyle(Theme.accent)
  }
}

struct IconButton: View {
  var symbol: String
  var help: String
  var action: () -> Void
  var body: some View {
    Button(action: action) {
      Image(systemName: symbol)
        .font(.system(size: 12.5))
        .frame(width: 30, height: 30)
        .background(.white, in: RoundedRectangle(cornerRadius: 7))
        .overlay(RoundedRectangle(cornerRadius: 7).stroke(Theme.line))
    }
    .buttonStyle(.plain)
    .foregroundStyle(Theme.muted)
    .help(help)
  }
}

struct CardBackground: ViewModifier {
  var padding: CGFloat = 20
  func body(content: Content) -> some View {
    content
      .padding(padding)
      .frame(maxWidth: .infinity, alignment: .leading)
      .background(.white, in: RoundedRectangle(cornerRadius: Theme.radiusCard))
      .overlay(
        RoundedRectangle(cornerRadius: Theme.radiusCard)
          .stroke(Theme.line, lineWidth: 1))
  }
}

extension View {
  func card(padding: CGFloat = 20) -> some View { modifier(CardBackground(padding: padding)) }
}

struct SectionHeader<Trailing: View>: View {
  var title: String
  var caption = ""
  @ViewBuilder var trailing: () -> Trailing
  init(title: String, caption: String = "", @ViewBuilder trailing: @escaping () -> Trailing = { EmptyView() }) {
    self.title = title
    self.caption = caption
    self.trailing = trailing
  }
  var body: some View {
    HStack(alignment: .firstTextBaseline) {
      VStack(alignment: .leading, spacing: 3) {
        Text(title).font(.system(size: 16, weight: .semibold)).foregroundStyle(Theme.ink)
        if !caption.isEmpty {
          Text(caption).font(Theme.captionFont).foregroundStyle(Theme.faint)
            .fixedSize(horizontal: false, vertical: true)
        }
      }
      Spacer(minLength: 12)
      trailing()
    }
  }
}

// 键值行：统一的「标签 / 值 / 备注」行式信息。
struct KVRow: View {
  var label: String
  var value: String
  var note = ""
  var body: some View {
    HStack(alignment: .top, spacing: 16) {
      Text(label).foregroundStyle(Theme.muted).frame(width: 96, alignment: .leading)
      VStack(alignment: .leading, spacing: 4) {
        Text(value).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
        if !note.isEmpty {
          Text(note).font(Theme.captionFont).foregroundStyle(Theme.faint)
            .fixedSize(horizontal: false, vertical: true)
        }
      }.frame(maxWidth: .infinity, alignment: .leading)
    }
    .font(Theme.bodyFont)
    .padding(.vertical, 9)
    .overlay(alignment: .bottom) { Rectangle().fill(Theme.line.opacity(0.7)).frame(height: 1) }
  }
}

// 大数字指标：值 + 单位 + 含义。
struct HeroStat: View {
  var label: String
  var value: String
  var unit: String
  var meaning: String
  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      Text(label).font(Theme.captionFont).foregroundStyle(Theme.muted)
      HStack(alignment: .firstTextBaseline, spacing: 5) {
        Text(value).font(.system(size: 31, weight: .semibold)).monospacedDigit()
        Text(unit).font(Theme.captionFont).foregroundStyle(Theme.muted)
      }
      Text(meaning).font(Theme.captionFont).foregroundStyle(Theme.faint)
        .fixedSize(horizontal: false, vertical: true)
    }
    .frame(maxWidth: .infinity, alignment: .leading)
  }
}

// 折叠的证据区：答案在上，依据按需展开。
struct Fold<Content: View>: View {
  var title: String
  var icon = "doc.text.magnifyingglass"
  @Binding var expanded: Bool
  @ViewBuilder var content: () -> Content
  var body: some View {
    DisclosureGroup(isExpanded: $expanded) {
      VStack(alignment: .leading, spacing: 10, content: content)
        .padding(.top, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
    } label: {
      Label(title, systemImage: icon)
        .font(.system(size: 12.5, weight: .medium))
        .foregroundStyle(Theme.accent)
    }
    .padding(.vertical, 8)
  }
}

// 样例完成度条：绿 = 正确，浅灰 = 未正确。
struct SampleBar: View {
  var passed: Int
  var total: Int
  var body: some View {
    HStack(spacing: 3) {
      ForEach(0..<max(total, 1), id: \.self) { index in
        RoundedRectangle(cornerRadius: 1.5)
          .fill(index < passed ? Theme.passBar.opacity(0.85) : Theme.line)
          .frame(height: 6)
      }
    }
    .accessibilityLabel("\(passed) / \(total) 正确完成")
  }
}
