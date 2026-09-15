import AppKit
import SwiftUI

// 首次使用 / 更换服务：建立信任，收窄到三个输入框。
struct ConnectionView: View {
  @ObservedObject var store: Workbench
  @FocusState private var focused: Int?
  var body: some View {
    VStack(alignment: .leading, spacing: 30) {
      steps
      VStack(alignment: .leading, spacing: 10) {
        Text(store.service == nil ? "连接你的模型服务" : "更换模型服务")
          .font(.system(size: 27, weight: .bold))
        Text("填写接入信息后开始第一次体检。")
          .font(.system(size: 13.5)).foregroundStyle(Theme.muted)
      }
      VStack(alignment: .leading, spacing: 18) {
        field("服务地址", hint: "https://your-service.example/v1") {
          TextField("https://your-service.example/v1", text: $store.draftURL)
            .focused($focused, equals: 0)
        }
        field("模型名称", hint: "服务提供的模型 ID") {
          TextField("服务提供的模型 ID", text: $store.draftModel)
            .focused($focused, equals: 1)
        }
        field("API Key", hint: "输入服务密钥") {
          SecureField("输入服务密钥", text: $store.draftKey)
            .focused($focused, equals: 2)
        }
      }
      if let error = store.connectionError {
        Label(error, systemImage: "exclamationmark.triangle.fill")
          .font(.system(size: 12.5))
          .foregroundStyle(Theme.blocked)
          .padding(14)
          .frame(maxWidth: .infinity, alignment: .leading)
          .background(Theme.blockedTint, in: RoundedRectangle(cornerRadius: 10))
          .fixedSize(horizontal: false, vertical: true)
      }
      HStack(spacing: 14) {
        Action(
          title: store.connecting ? "正在连接…" : "连接服务", icon: "arrow.right",
          disabled: store.connecting
        ) {
          Task { await store.connect() }
        }
        if store.service != nil {
          Action(title: "取消更换", icon: "xmark", primary: false, disabled: store.connecting) {
            store.cancelConfiguration()
          }
        } else {
          TextButton(title: "使用示例配置") { store.fillExample() }
        }
        Spacer()
      }
      HStack(spacing: 6) {
        Image(systemName: "lock.fill").font(.system(size: 10))
        Text(
          store.isRealMode
            ? "真实检测：API Key 从启动环境读取，不显示、不保存、不写入报告。"
            : "API Key 仅用于本次检测，不会保存或导出。"
        )
      }
      .font(Theme.captionFont).foregroundStyle(Theme.faint)
    }
    .frame(maxWidth: 470, alignment: .leading)
    .padding(.top, 30)
    .frame(maxWidth: .infinity, alignment: .center)
    .onSubmit { Task { await store.connect() } }
  }

  private var steps: some View {
    HStack(spacing: 10) {
      step(1, "连接服务", active: true)
      Image(systemName: "chevron.right").font(.system(size: 9, weight: .semibold))
        .foregroundStyle(Theme.faint)
      step(2, "选择检测")
      Image(systemName: "chevron.right").font(.system(size: 9, weight: .semibold))
        .foregroundStyle(Theme.faint)
      step(3, "查看结论")
    }
  }

  private func step(_ number: Int, _ name: String, active: Bool = false) -> some View {
    HStack(spacing: 6) {
      Text("\(number)")
        .font(.system(size: 10.5, weight: .semibold)).monospacedDigit()
        .foregroundStyle(active ? .white : Theme.faint)
        .frame(width: 18, height: 18)
        .background(
          active ? Theme.accent : Theme.canvas,
          in: Circle())
      Text(name)
        .font(.system(size: 11.5, weight: active ? .semibold : .regular))
        .foregroundStyle(active ? Theme.ink : Theme.faint)
    }
    .padding(.horizontal, 10).padding(.vertical, 5)
    .background(
      active ? Theme.accent.opacity(0.07) : .clear,
      in: Capsule())
  }

  private func field<C: View>(_ title: String, hint: String, @ViewBuilder content: () -> C)
    -> some View
  {
    VStack(alignment: .leading, spacing: 7) {
      Text(title).font(.system(size: 12.5, weight: .medium))
      content()
        .font(.system(size: 13.5))
        .textFieldStyle(.plain)
        .padding(.horizontal, 13)
        .frame(height: 42)
        .background(.white, in: RoundedRectangle(cornerRadius: 9))
        .overlay(
          RoundedRectangle(cornerRadius: 9)
            .stroke(focused == fieldIndex(title) ? Theme.accent : Theme.line, lineWidth: 1)
        )
        .disabled(store.connecting)
    }
  }

  private func fieldIndex(_ title: String) -> Int {
    ["服务地址", "模型名称", "API Key"].firstIndex(of: title) ?? -1
  }
}

// 开始前的确认：说清这次会测什么、要多久。
struct ConfirmationView: View {
  @ObservedObject var store: Workbench
  var body: some View {
    VStack(alignment: .leading, spacing: 22) {
      HStack {
        Text("开始一次新检测").font(.system(size: 20, weight: .bold))
        Spacer()
        IconButton(symbol: "xmark", help: "取消") { store.showConfirmation = false }
          .accessibilityIdentifier("cancel-detection")
      }
      HStack(spacing: 11) {
        Image(systemName: "cpu")
          .font(.system(size: 15))
          .foregroundStyle(Theme.muted)
          .frame(width: 34, height: 34)
          .background(Theme.canvas, in: RoundedRectangle(cornerRadius: 8))
        VStack(alignment: .leading, spacing: 3) {
          Text(store.service?.model ?? "").font(.system(size: 13.5, weight: .semibold))
            .lineLimit(2)
          Text(store.service?.displayURL ?? "").font(Theme.captionFont)
            .foregroundStyle(Theme.faint).lineLimit(2)
        }
      }
      Divider()
      HStack {
        Text("检测范围").font(.system(size: 13, weight: .semibold))
        Spacer()
        Toggle(
          "全选",
          isOn: Binding(
            get: { store.selectedModules == Set(CheckModule.testModules) },
            set: { store.selectedModules = $0 ? Set(CheckModule.testModules) : [] }
          )
        ).toggleStyle(.checkbox).font(Theme.bodyFont)
      }
      VStack(spacing: 4) {
        ForEach(CheckModule.testModules) { module in
          Toggle(
            isOn: Binding(
              get: { store.selectedModules.contains(module) },
              set: {
                if $0 {
                  store.selectedModules.insert(module)
                } else {
                  store.selectedModules.remove(module)
                }
              }
            )
          ) {
            HStack(spacing: 12) {
              Image(systemName: module.symbol)
                .font(.system(size: 14))
                .foregroundStyle(Theme.accent)
                .frame(width: 30, height: 30)
                .background(Theme.accent.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
              VStack(alignment: .leading, spacing: 3) {
                Text(module.title).font(.system(size: 12.5, weight: .medium))
                Text(module.subtitle).font(Theme.captionFont).foregroundStyle(Theme.faint)
              }
              Spacer()
              Text(moduleCount(module)).font(Theme.captionFont).foregroundStyle(Theme.faint)
            }.padding(.leading, 7)
          }
          .toggleStyle(.checkbox)
          .padding(.vertical, 8)
        }
      }
      if store.selectedModules.contains(.agent) {
        Label("包含 10 个 Agent 样本，初测 30 次；失败后自动复核。", systemImage: "checklist")
          .font(Theme.captionFont).foregroundStyle(Theme.muted)
      } else {
        Label("未选智能体实测，本次不会给出「能否胜任 Agent 任务」的结论。", systemImage: "info.circle")
          .font(Theme.captionFont).foregroundStyle(Theme.limited)
      }
      HStack(alignment: .center) {
        VStack(alignment: .leading, spacing: 4) {
          Text(
            store.selectedModules.isEmpty
              ? "请选择检测范围"
              : "已选 \(store.selectedModules.count) 个模块 · 预计约 \(store.selectedModules.count * 2) 秒"
          )
          .font(.system(size: 12.5, weight: .medium))
          Text(store.isRealMode ? "真实执行会调用目标模型服务并产生对应费用。" : "开始后将调用目标模型并记录检测证据。")
            .font(Theme.captionFont).foregroundStyle(Theme.faint)
        }
        Spacer()
        Action(title: "开始检测", icon: "play.fill", disabled: store.selectedModules.isEmpty) {
          store.startRun()
        }
      }.padding(.top, 2)
    }
    .padding(28)
    .frame(width: 590)
    .background(.white)
    .foregroundStyle(Theme.ink)
  }
  private func moduleCount(_ module: CheckModule) -> String {
    switch module {
    case .info: return ""
    case .parameters: return "7 类规格"
    case .functions: return "6 类任务"
    case .performance: return "5 类负载"
    case .agent: return "8 项检查"
    case .comparison: return "14 个维度"
    }
  }
}

// 检测进行中：一眼看到进度与当前在测什么。
struct ProgressScreen: View {
  @ObservedObject var store: Workbench
  var body: some View {
    VStack(alignment: .leading, spacing: 30) {
      HStack(alignment: .top) {
        VStack(alignment: .leading, spacing: 8) {
          Text(store.service?.model ?? "")
            .font(.system(size: 23, weight: .bold))
            .fixedSize(horizontal: false, vertical: true)
          Text(store.service?.host ?? "")
            .font(Theme.bodyFont).foregroundStyle(Theme.muted)
        }
        Spacer()
        Action(title: "停止检测", icon: "stop.fill", primary: false) {
          store.showStopConfirmation = true
        }
      }
      VStack(alignment: .leading, spacing: 14) {
        HStack(alignment: .firstTextBaseline) {
          Text("正在检测：\(store.activeModule?.title ?? "整理结果")")
            .font(.system(size: 17, weight: .semibold))
          Spacer()
          HStack(alignment: .firstTextBaseline, spacing: 2) {
            Text("\(Int(store.progress * 100))")
              .font(.system(size: 44, weight: .semibold))
              .monospacedDigit()
            Text("%").font(.system(size: 15)).foregroundStyle(Theme.faint)
          }
        }
        GeometryReader { geometry in
          ZStack(alignment: .leading) {
            Capsule().fill(Theme.canvas)
            Capsule().fill(Theme.accent)
              .frame(width: max(10, geometry.size.width * store.progress))
          }
        }.frame(height: 9)
        HStack {
          VStack(alignment: .leading, spacing: 3) {
            Text(store.currentItemName.isEmpty
              ? (store.progressMessage.isEmpty
                ? (store.activeModule?.subtitle ?? "汇总各模块结果，生成使用结论")
                : store.progressMessage)
              : "正在检测：\(store.currentItemName)")
            if store.currentItemTotal > 0 {
              Text("当前项目：\(store.currentItemIndex) / \(store.currentItemTotal)")
                .foregroundStyle(Theme.muted)
            }
          }
          Spacer()
          Text("\(store.completed.count) / \(store.activeModules.count) 个模块完成")
        }
        .font(Theme.captionFont).foregroundStyle(Theme.faint)
      }
      VStack(spacing: 6) {
        ForEach(store.activeModules) { module in
          moduleRow(module)
        }
      }
      Text(store.isRealMode
        ? "检测程序实际执行；提前停止会保留已完成的模块状态。"
        : "结束后生成使用结论；提前停止会保留已完成的结果。")
        .font(Theme.captionFont).foregroundStyle(Theme.faint)
    }
  }

  private func moduleRow(_ module: CheckModule) -> some View {
    let done = store.completed.contains(module)
    let active = store.activeModule == module
    return HStack(spacing: 14) {
      Image(
        systemName: done
          ? "checkmark.circle.fill" : active ? "arrow.right.circle.fill" : "circle"
      )
      .font(.system(size: 18))
      .foregroundStyle(
        done ? Theme.passBar : active ? Theme.accent : Theme.faint)
      VStack(alignment: .leading, spacing: 4) {
        Text(module.title)
          .font(.system(size: 13.5, weight: active ? .semibold : .medium))
          .foregroundStyle(active ? Theme.ink : Theme.muted)
        Text(module.subtitle)
          .font(Theme.captionFont).foregroundStyle(Theme.faint)
      }
      Spacer()
      Text(done ? "已完成" : active ? "进行中" : "等待中")
        .font(Theme.captionFont)
        .foregroundStyle(active ? Theme.accent : Theme.faint)
    }
    .padding(.vertical, 14)
    .padding(.horizontal, 14)
    .background(
      active ? Theme.accent.opacity(0.05) : .clear,
      in: RoundedRectangle(cornerRadius: 10)
    )
    .overlay(alignment: .bottom) { Rectangle().fill(Theme.line).frame(height: 1) }
  }
}

struct SettingsView: View {
  @ObservedObject var store: Workbench
  var body: some View {
    VStack(alignment: .leading, spacing: 22) {
      HStack {
        Text("设置").font(.system(size: 20, weight: .bold))
        Spacer()
        IconButton(symbol: "xmark", help: "关闭设置") { store.showSettings = false }
      }
      SectionHeader(title: "模型服务")
      VStack(spacing: 0) {
        KVRow(label: "模型名称", value: store.service?.model ?? "尚未配置")
        KVRow(label: "接入地址", value: store.service?.displayURL ?? "尚未填写")
        KVRow(
          label: "API Key", value: "未保存",
          note: "API Key 仅用于本次检测；检测记录与导出不包含密钥。")
      }
      Action(
        title: "更换模型服务", icon: "pencil", primary: false, disabled: store.running || store.connecting
      ) {
        store.showSettings = false
        store.editService()
      }
      Divider()
      SectionHeader(title: "检测记录", caption: "\(store.records.count) 条")
      Label(store.localStatus, systemImage: "internaldrive").foregroundStyle(Theme.muted)
      Text("检测记录来自实际运行；API Key 不会写入本机记录或导出文件。")
        .font(Theme.captionFont).foregroundStyle(Theme.faint)
    }
    .padding(28).frame(width: 520).background(.white).foregroundStyle(Theme.ink)
  }
}

struct DemoControlsView: View {
  @ObservedObject var store: Workbench
  var body: some View {
    VStack(alignment: .leading, spacing: 20) {
      HStack {
        Text("检测场景").font(.system(size: 20, weight: .bold))
        Spacer()
        IconButton(symbol: "xmark", help: "返回工作台") { store.showDemoControls = false }
      }
      Text("查看检测项目、测试条件和结果范围。实际检测由 Rust CLI 执行，并保留完整证据。")
        .font(Theme.bodyFont).foregroundStyle(Theme.muted)
        .fixedSize(horizontal: false, vertical: true)
      Divider()
      VStack(alignment: .leading, spacing: 10) {
        SectionHeader(title: "下次连接")
        Picker("连接结果", selection: $store.connectionScenario) {
          Text("成功").tag("success")
          Text("密钥不匹配").tag("auth")
          Text("超时").tag("timeout")
        }.labelsHidden().pickerStyle(.segmented).disabled(store.connecting)
      }
      VStack(alignment: .leading, spacing: 12) {
        SectionHeader(title: "下次检测结果")
        Picker("使用判断", selection: $store.outcome) {
          Text("可以正常使用").tag(Outcome.usable)
          Text("可以使用，但有使用限制").tag(Outcome.limited)
          Text("目前不能正常使用").tag(Outcome.blocked)
          Text("证据不足，暂不能判断").tag(Outcome.inconclusive)
        }.labelsHidden().pickerStyle(.radioGroup).disabled(store.running)
        if store.outcome == .limited {
          Picker("错误恢复", selection: $store.agentMode) {
            Text("重复失败 · 3/5 完成").tag("standard")
            Text("单次失败 · 4/5 完成").tag("intermittent")
            Text("复核未完成").tag("pending-review")
          }.disabled(store.running)
        }
      }
      Toggle("响应模型名与配置不同", isOn: $store.responseNameDiff).toggleStyle(.checkbox).disabled(
        store.running)
      Text("场景只影响下一次检测，不改动已有记录。正式的默认检测组合、时间预算与综合计分尚未确定。")
        .font(Theme.captionFont).foregroundStyle(Theme.faint)
        .fixedSize(horizontal: false, vertical: true)
      HStack {
        Spacer()
        Action(title: "完成", icon: "checkmark") { store.showDemoControls = false }
      }
    }
    .padding(28).frame(width: 520).background(.white).foregroundStyle(Theme.ink)
  }
}
