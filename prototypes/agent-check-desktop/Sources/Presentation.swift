import Foundation

struct EvidenceFact: Codable, Identifiable {
  var id: String { label }
  var label: String
  var value: String
  init(_ label: String, _ value: String) {
    self.label = label
    self.value = value
  }
}
enum ObservationState: String, Codable {
  case observed = "已有结果"
  case partial = "部分已验证"
  case limited = "有使用限制"
  case unverified = "尚未检测"
  case unknown = "证据不足，暂不能判断"
}
struct CatalogItem: Codable, Identifiable {
  var id: String
  var title: String
  var value: String
  var state: ObservationState
  var summary: String
  var boundary: String
  var facts: [EvidenceFact]
  var evidence: [EvidenceFact]
}

enum Catalog {
  static let specifications: [CatalogItem] = [
    .init(
      id: "S1", title: "上下文容量", value: "已接收 8,192 token", state: .observed,
      summary: "在本次输出预算下，8,192 token 输入被接收。", boundary: "没有找到容量上限；材料能否被正确理解，需要看模型能力跑分。",
      facts: [
        .init("输入档位", "2,048 / 4,096 / 8,192 token"), .init("输出预算", "每次最多 1,024 token"),
        .init("结果", "三个输入档位均收到完整回复；更长输入尚未检测"),
      ],
      evidence: [
        .init("S1-01", "输入 2,048 token → 请求接收，回复结束"), .init("S1-02", "输入 4,096 token → 请求接收，回复结束"),
        .init("S1-03", "输入 8,192 token → 请求接收，回复结束"),
      ]),
    .init(
      id: "S2", title: "输出长度", value: "长度限制样例生效", state: .observed,
      summary: "三个指定长度限制均触发了对应的输出结束。", boundary: "只验证这些限制档位，不宣称已经探到最大输出长度。",
      facts: [
        .init("输入条件", "固定 128 token；要求持续输出编号"), .init("限制档位", "128 / 512 / 1,024 token"),
        .init("观察方式", "分别核对实际长度和结束原因，不把自然结束当成触顶"),
      ],
      evidence: [
        .init("S2-01", "限制 128；输出 128；结束原因：达到长度限制"),
        .init("S2-02", "限制 512；输出 512；结束原因：达到长度限制"),
        .init("S2-03", "限制 1,024；输出 1,024；结束原因：达到长度限制"),
      ]),
    .init(
      id: "S3", title: "常用参数支持", value: "已接受，生效待验证", state: .unknown,
      summary: "带 temperature 的请求被接受，但尚无足够对照证明其影响。", boundary: "接收成功不等于参数生效，也不代表其他参数均受支持。",
      facts: [
        .init("本次参数", "temperature: 0 / 1"), .init("已观察", "两个请求均返回文本"),
        .init("证据缺口", "缺少控制其他条件的重复对照；暂不判定生效或不支持"),
      ],
      evidence: [.init("S3-01", "temperature: 0 → 收到文本"), .init("S3-02", "temperature: 1 → 收到文本")]),
    .init(
      id: "S4", title: "工具调用支持", value: "最小闭环已完成", state: .observed,
      summary: "发起函数调用、回传工具结果、继续回复的闭环已完成。", boundary: "不据此判断复杂工具选择质量；流式工具调用、并行调用尚未检测。",
      facts: [
        .init("工具", "查询状态；只读；对象 示例对象 17"), .init("已验证方式", "自动选择、指定单个函数"),
        .init("最终结果", "引用工具返回的已就绪状态后结束"),
      ],
      evidence: [
        .init("S4-01", "查询状态（对象：示例对象 17）→ 已就绪 → 回复引用已就绪状态"),
        .init("S4-02", "指定查询状态 → 工具结果回传 → 完整续答"),
      ]),
    .init(
      id: "S5", title: "结构化输出支持", value: "结构化数据（JSON）与结构约束分开验证", state: .observed,
      summary: "结构化数据（JSON）样例可解析，指定结构样例包含约定字段与类型。", boundary: "只覆盖本次结构；合法结构化数据不自动等于满足任意结构约束。",
      facts: [
        .init("结构化数据输出", "可解析为对象"), .init("指定结构", "名称：文本；数量：整数；必需字段齐全"),
        .init("未覆盖", "更复杂的嵌套、组合约束和全部关键字"),
      ],
      evidence: [
        .init("S5-01", "结构化数据：{\"name\":\"alpha\",\"count\":2}"),
        .init("S5-02", "指定结构：字段存在、类型正确，无额外字段"),
      ]),
    .init(
      id: "S6", title: "消息与多轮输入支持", value: "历史回传后可续答", state: .observed,
      summary: "接收已选消息角色，并在回传历史后完成最小续答。", boundary: "历史由请求方回传，不表示服务自动保存对话；工具结果回传归工具调用支持。",
      facts: [
        .init("本次消息", "system / user / assistant"), .init("历史条件", "后续请求显式携带之前的消息"),
        .init("未覆盖", "平台未选用的消息组合与更长历史"),
      ],
      evidence: [.init("S6-01", "用户指定 alpha；下一请求带历史询问对象 → alpha")]),
    .init(
      id: "S7", title: "流式输出支持", value: "普通回复完成；工具组合未测", state: .partial,
      summary: "普通文本增量可以汇集并正常结束；流式工具调用还没有执行证据。", boundary: "未覆盖部分不判失败。生成速度和断流表现另见模型性能实测。",
      facts: [
        .init("普通回复", "三个样例均收到增量、可汇集、完整结束"), .init("工具调用组合", "尚未检测"),
        .init("结构对照", "模型基线对比另有独立分块样例，不借作本项证据"),
      ],
      evidence: [.init("S7-01 / 02 / 03", "普通流式回复已汇集，结束标记已记录；未发起流式工具任务")]),
  ]

  static let performance: [CatalogItem] = [
    .init(
      id: "P1", title: "响应等待", value: "首个有效输出 0.8 秒", state: .observed,
      summary: "本组请求开始返回的等待中位数为 0.8 秒，完整耗时中位数为 6.8 秒。", boundary: "本次未设业务等待合格线；不能仅因偏慢判整套服务不可用。",
      facts: [
        .init("输入 / 输出", "128 / 252 token"), .init("负载 / 样本", "并发 1；三个有效请求；相同参数与环境"),
        .init("计量口径", "演示口径：请求发出到首段非空文本；完整耗时到正常结束"),
      ],
      evidence: [
        .init("P1-01", "首段 0.8 秒；完整 6.8 秒；正常结束"), .init("P1-02", "首段 0.9 秒；完整 7.1 秒；正常结束"),
        .init("P1-03", "首段 0.7 秒；完整 6.5 秒；正常结束"),
      ]),
    .init(
      id: "P2", title: "生成速度与流畅度", value: "42 token/秒", state: .observed,
      summary: "开始输出后的生成速度中位数为 42 token/秒。", boundary: "不含首段等待，不使用多请求总吞吐代替单请求生成体验。",
      facts: [
        .init("共用证据", "P1 三个请求；不重复计为额外请求"), .init("生成阶段", "6.0 / 6.2 / 5.8 秒；每次输出 252 token"),
        .init("输出间隔", "最长 0.18 / 0.22 / 0.16 秒；未设停顿合格线"),
      ],
      evidence: [
        .init("P1-01", "252 ÷ 6.0 = 42.0 token/秒"), .init("P1-02", "252 ÷ 6.2 ≈ 40.6 token/秒"),
        .init("P1-03", "252 ÷ 5.8 ≈ 43.4 token/秒"),
      ]),
    .init(
      id: "P3", title: "并发承载", value: "并发 4 时出现 1 次超时", state: .limited,
      summary: "并发 1、2 的样例完整返回；并发 4 的十二次请求中有一次超时。", boundary: "这是本次负载下的观察，不是最大承载量，也不能换算为支持多少用户。",
      facts: [
        .init("固定内容", "输入 128 token；输出目标 252 token"), .init("请求预算", "每次等待上限 15 秒；演示负载，非正式默认"),
        .init("环境", "同一接入点、参数组与隔离测试环境"),
      ],
      evidence: [
        .init("并发 1", "3/3 完整；首段中位数 0.8 秒；整组 20.4 秒，输出 756 token"),
        .init("并发 2", "6/6 完整；首段中位数 1.1 秒；整组 22 秒，输出 1,512 token"),
        .init("并发 4", "11/12 完整；1 次超时；整组 30 秒，完整输出 2,772 token；超时不计快速完成"),
      ]),
    .init(
      id: "P4", title: "连续运行稳定性", value: "5 分钟内 29/30 完整返回", state: .limited,
      summary: "固定负载连续运行五分钟，有一次服务错误；其余请求完整返回。", boundary: "五分钟短测不是长期可靠性证明；完整返回比例不是任务正确率。",
      facts: [
        .init("时长 / 负载", "5 分钟；每 10 秒发起一个请求；并发上限 1"),
        .init("输入 / 输出", "128 / 252 token；每次请求预算 8 秒"), .init("延迟变化", "首分钟首段中位数 0.8 秒；末分钟 1.1 秒"),
      ],
      evidence: [
        .init("第 18 次", "服务错误，未获得完整输出；保留错误记录"), .init("其他 29 次", "正常结束；无额外超时或断流；不能由此推断内容都正确"),
      ]),
    .init(
      id: "P5", title: "长输入与长输出性能", value: "长度增加后等待变长", state: .observed,
      summary: "分别观察输入、输出和历史变长后的耗时；均在已验证规格范围内。", boundary: "耗时增加不自动等于失败；不在这里再次探测容量上限。",
      facts: [
        .init("测试方式", "每档三个请求；并发 1；除长度外固定参数"), .init("规格依据", "输入不超过 8,192 token；输出不超过 1,024 token"),
        .init("统计范围", "以下为各档中位数；三类变化分开测试"),
      ],
      evidence: [
        .init("输入变长", "1K → 8K，输出固定 252；首段 0.8 → 1.7 秒；完整 6.8 → 7.9 秒"),
        .init("输出变长", "252 → 1,024，输入固定 128；首段 0.8 → 0.9 秒；完整 6.8 → 25.3 秒"),
        .init("历史变长", "1K → 4K，输出固定 252；首段 0.9 → 1.3 秒；完整 6.9 → 7.5 秒"),
        .init("流畅度与异常", "各档生成约 41–42 token/秒；最长间隔 0.26 秒；全部正常结束"),
      ]),
  ]
}

struct ScoreCase: Codable, Identifiable {
  var id: String
  var input: String
  var expected: String
  var actual: String
  var passed: Bool
}
struct ScoreCategory: Codable, Identifiable {
  var id: String
  var title: String
  var summary: String
  var condition: String
  var cases: [ScoreCase]
  var comparisonCases: [ScoreCase] = []
  var correct: Int { cases.filter(\.passed).count }
  var result: String { "\(correct) / \(cases.count)" }
}
enum Scores {
  static func cases(_ prefix: String, _ rows: [(String, String, String)]) -> [ScoreCase] {
    rows.enumerated().map { index, row in
      .init(
        id: "\(prefix)-\(index + 1)", input: row.0, expected: row.1, actual: row.2,
        passed: row.1 == row.2)
    }
  }
  static let all: [ScoreCategory] = [
    .init(
      id: "C1", title: "文本理解与指令执行", summary: "事实理解正确；多项输出要求有一次遗漏",
      condition: "五个短文本样例；同时核对内容与格式；不评文风",
      cases: cases(
        "C1",
        [
          ("甲组完成 2 项，乙组完成 3 项。只输出总数。", "5", "5"), ("仅列出红色物品：红笔、蓝杯、红盒。", "红笔、红盒", "红笔、红盒"),
          ("A 已完成，B 未开始。仅输出未开始项。", "B", "B"), ("甲于周二提交、周三通过。只输出提交日。", "周二", "周二"),
          ("服务已恢复。用结构化数据的状态字段返回中文状态。", "{\"status\":\"已恢复\"}", "已恢复"),
        ])),
    .init(
      id: "C2", title: "信息提取与结构化填写", summary: "显式信息提取完整，缺失字段未编造", condition: "五条记录；核对字段、对象及缺失信息",
      cases: cases(
        "C2",
        [
          ("姓名小林，部门研发。提取部门。", "研发", "研发"), ("A 属于甲组，B 属于乙组。B 的组别？", "乙组", "乙组"),
          ("名称 alpha，日期未提供。提取日期。", "未知", "未知"), ("工单 12 状态关闭。提取状态。", "关闭", "关闭"),
          ("负责人：小周；电话未填写。提取电话。", "未知", "未知"),
        ])),
    .init(
      id: "C3", title: "工具选择与参数填写", summary: "常规查询选对；缺参数与无需调用场景有误",
      condition: "五个独立标准任务；工具目录固定；不是连续 Agent 测试",
      cases: cases(
        "C3",
        [
          ("查 A 状态；使用查询工具。", "查询（A）", "查询（A）"),
          ("查 B 详情；使用详情查询和列表查询。", "查询详情（B）", "查询详情（B）"),
          ("只用已给出的状态已就绪作答。", "不调用工具", "查询（A）"),
          ("查询对象未指定；不允许猜测。", "澄清对象", "查询默认对象"), ("要求修改记录，目录仅有只读工具。", "无合适工具", "无合适工具"),
        ])),
    .init(
      id: "C4", title: "多轮对话与条件承接", summary: "对象保持较好；修改时间后有一次沿用旧值",
      condition: "五组双轮对话；检查更新条件及仍有效的约束",
      cases: cases(
        "C4",
        [
          ("先选 A；追问当前对象。", "A", "A"), ("原日期周一；改为周二；问最终日期。", "周二", "周一"),
          ("A 仅查询；日期改周三；问权限。", "仅查询", "仅查询"), ("甲数量 2、乙数量 4；追问乙。", "4", "4"),
          ("原对象 A；明确改为 B；问对象。", "B", "B"),
        ])),
    .init(
      id: "C5", title: "长材料理解与信息利用", summary: "4K 档出现遗漏与干扰项混淆；1K 档五个样例均正确",
      condition: "五个同类检索任务，1K / 4K 两档；均处于已接收范围。按长度分别核对，不混合计分。",
      cases: cases(
        "C5-4K",
        [
          ("4K 材料中最终版负责人为甲，旧版为乙。", "甲", "甲"), ("4K 材料：关联第 1、4 段，A 属甲组，甲组在东区。", "东区", "未知"),
          ("4K 材料：查最新数量，旧记录 7，新记录 9。", "9", "7"), ("4K 材料：筛选已关闭项 A、C，排除未关闭 B。", "A、C", "A、C"),
          ("4K 材料：查未提供的到期日。", "未知", "未知"),
        ]),
      comparisonCases: cases(
        "C5-1K",
        [
          ("1K 材料中最终版负责人为甲，旧版为乙。", "甲", "甲"), ("1K 材料：A 属甲组，甲组在东区。查询所在区域。", "东区", "东区"),
          ("1K 材料：查最新数量，旧记录 7，新记录 9。", "9", "9"), ("1K 材料：筛选已关闭项 A、C，排除未关闭 B。", "A、C", "A、C"),
          ("1K 材料：查未提供的到期日。", "未知", "未知"),
        ])),
    .init(
      id: "C6", title: "逻辑推理与计算", summary: "简单计算完成；跨日时间有一次错误", condition: "五个贴近使用场景的确定性问题，不采用竞赛门槛",
      cases: cases(
        "C6",
        [
          ("3 个批次，每批 4 条，共多少条？", "12", "12"), ("周二晚 23 点，3 小时后？", "周三 02:00", "周二 02:00"),
          ("A 比 B 多，B 比 C 多。谁最少？", "C", "C"), ("预算 100，花费 25 与 30，余额？", "45", "45"),
          ("仅当 A、B 均真允许；A 真、B 假。", "不允许", "不允许"),
        ])),
  ]
}

extension CheckModule {
  var pageTitle: String { title }
  var actionTitle: String { "重新检查" }
}
extension RunRecord {
  var readableID: String { String(id.uuidString.prefix(8)).lowercased() }
  var isRealReport: Bool { reportJSON != nil }
  var agentCoverage: String {
    if isRealReport { return "真实 CLI 已完成的 Agent 模块与其证据入口" }
    return "查询、连续追问、多步操作、工具调用、错误恢复、权限边界与结果交付"
  }
  var agentRisk: String {
    if isRealReport { return "具体限制以本次真实报告中的模块状态、事件和证据为准" }
    if hasConfirmedBlocker { return "未经批准的写入请求，可能造成越权操作或生产数据风险" }
    if outcome == .limited { return "工具出错后的自动恢复不稳定，可能导致任务中断或结果不完整" }
    return "当前未发现已确认的核心 Agent 行为阻断"
  }
  var agentUncovered: String {
    if isRealReport { return "未选模块、未完成项目和报告中明确列出的限制" }
    return "具体业务流程、生产写入、长期无人值守、复杂流式工具组合"
  }
  var admissionNextStep: String {
    if admissionTitle == "建议接入" { return "可进入智能体产品接入评审，并补做目标业务场景验证" }
    if admissionTitle == "修复后再接入" { return "先处理接入风险，再重新运行全方位诊断" }
    if admissionTitle == "暂不建议接入" { return "先解决阻断问题；修复前不要接入生产智能体" }
    return "补齐未完成诊断后，再形成接入决定"
  }
  var admissionTitle: String {
    if hasConfirmedBlocker { return "暂不建议接入" }
    if stopped || !completed.contains(.agent) || !hasCurrentEvidence { return "暂不能决定是否接入" }
    switch outcome {
    case .usable: return "建议接入"
    case .limited: return "修复后再接入"
    case .blocked: return "暂不建议接入"
    case .inconclusive: return "暂不能决定是否接入"
    }
  }
  var admissionExplanation: String {
    if hasConfirmedBlocker { return "发现会导致智能体产品不可用的阻断问题；请修复后重新诊断。" }
    if stopped || !completed.contains(.agent) || !hasCurrentEvidence {
      return "全方位诊断尚未完成，当前结果不能作为模型接入依据。"
    }
    switch outcome {
    case .usable: return "核心智能体任务已通过本次诊断，可进入接入评审；上线前仍需补充业务场景验证。"
    case .limited: return "核心任务可以运行，但存在接入风险；请先处理限制项，再重新诊断。"
    case .blocked: return "发现关键接入风险，当前不建议将该模型接入智能体产品。"
    case .inconclusive: return "没有取得足够证据，不能据此判断模型是否适合接入。"
    }
  }
  var agentSamples: [AgentSample] {
    !isRealReport && hasCurrentEvidence && completed.contains(.agent)
      ? AgentEvidence.samples(outcome, mode: agentMode ?? "standard") : []
  }
  var agentFindings: [AgentFinding] { AgentEvidence.findings(samples: agentSamples) }
  var explanation: String {
    if isRealReport {
      return "本页展示检测程序产生的状态摘要；完整请求、事件、模块状态和限制保存在导出报告中。"
    }
    if !hasCurrentEvidence { return "旧版记录保留当时摘要：\(outcome.title)。不套用本版分类和证据。" }
    if hasConfirmedBlocker { return "只读任务中发生未经批准的写入请求；尚无已验证的可靠规避方式。" }
    if stopped { return "检查提前停止，必要证据不足。已取得的结果仍保留，未完成不判失败。" }
    if !completed.contains(.agent) { return "本次未完成智能体实测，不能从规格、分类成绩或性能推断任务可用性。" }
    if agentMode == "pending-review" { return "已观察到一次恢复失败，规定复核尚未完成；当前不能确定使用结论。" }
    switch outcome {
    case .usable: return "本次受控只读任务均按要求完成，异常处理与权限边界符合要求；完整业务流程尚未检测。"
    case .limited:
      return agentMode == "intermittent"
        ? "本次受控只读任务中，工具临时出错后有 1 次未能恢复，后续 2 次复核成功；仍保留这一限制。"
        : "本次受控只读任务中，查询、多步操作和连续追问正常完成。工具临时出错后，5 次中有 2 次未能恢复。"
    case .blocked: return "本次必要工作方式存在已确认阻断。"
    case .inconclusive: return "工作区准备失败，未取得有效任务证据；不能归因为模型能力失败。"
    }
  }
  var usableScope: String {
    if hasConfirmedBlocker { return "只读与审批边界未满足，不能确认本次工作方式可用" }
    guard hasCurrentEvidence, completed.contains(.agent), !stopped, outcome != .inconclusive else {
      return "证据不足，尚不能确认可用范围"
    }
    return outcome == .usable ? "本次只读任务、异常处理与结果交付" : "正常查询、多步操作、连续追问、真实结果交付"
  }
  var limitation: String {
    if !hasCurrentEvidence { return "旧版证据不包含本版分类" }
    if hasConfirmedBlocker { return "审批拒绝后仍请求写入" }
    if stopped { return "必要检查未完成" }
    if !completed.contains(.agent) { return "未完成智能体实测" }
    if agentMode == "pending-review" { return "失败复核未完成" }
    if outcome == .limited {
      return agentMode == "intermittent" ? "恢复样本初测有一次失败，复核均成功" : "可恢复错误下 3/5 完成，不能依赖自动恢复"
    }
    return outcome == .usable ? "仅限已测条件，不保证全部业务" : "测试环境未就绪"
  }
  var missingScope: String {
    let missing = CheckModule.testModules.filter { !completed.contains($0) }.map(\.title)
    return (missing + ["业务场景、生产写入、长期无人值守", completed.contains(.parameters) ? "流式工具组合、规格上限" : ""])
      .filter { !$0.isEmpty }.joined(separator: "；")
  }
  func brief(_ module: CheckModule) -> String {
    if module == .info { return "当前服务与本次记录" }
    guard modules.contains(module) else { return "本次未选择" }
    guard completed.contains(module) else { return "未完成，保留已有事实" }
    guard hasCurrentEvidence else { return "旧版记录，不套用新版结果" }
    if isRealReport {
      let backendID = module.backendID ?? "ingress"
      return "真实执行状态：\(moduleStates?[backendID] ?? "已完成")；详细证据见导出报告"
    }
    switch module {
    case .info: return "当前服务与本次记录"
    case .parameters: return "输入已测至 8K，输出至 1,024 token；未找到上限，参数生效待验证"
    case .functions: return "信息提取 5/5；工具选择 3/5；4K 长材料 3/5，其余三类各 4/5"
    case .performance: return "首段等待 0.8 秒；并发 4 时 11/12 完成，连续 5 分钟内 29/30 完成"
    case .agent: return limitation
    case .comparison: return "14 个结构维度，5 项包含差异；部分错误分支尚未检测"
    }
  }
}
struct ReportExport: Encodable {
  let simulated: Bool
  let record: RunRecord
  init(record: RunRecord) {
    self.simulated = !record.isRealReport
    self.record = record
  }
  enum CodingKeys: String, CodingKey {
    case simulated, record, summary, specifications, scores, performance, agentSamples,
      agentFindings, baseline, limitations
  }
  func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: CodingKeys.self)
    try c.encode(simulated, forKey: .simulated)
    var safeRecord = record
    safeRecord.service.url = record.service.displayURL
    try c.encode(safeRecord, forKey: .record)
    try c.encode(record.explanation, forKey: .summary)
    guard record.hasCurrentEvidence else { return }
    if record.completed.contains(.parameters) {
      try c.encode(Catalog.specifications, forKey: .specifications)
    }
    if record.completed.contains(.functions) { try c.encode(Scores.all, forKey: .scores) }
    if record.completed.contains(.performance) {
      try c.encode(Catalog.performance, forKey: .performance)
    }
    try c.encode(record.agentSamples, forKey: .agentSamples)
    try c.encode(record.agentFindings, forKey: .agentFindings)
    if record.completed.contains(.comparison) {
      try c.encode(BaselineItem.forRecord(record), forKey: .baseline)
    }
    try c.encode(
      simulated
        ? "所有数据为演示；题库、规格子项、计量与负载不是正式默认。基线样例归属独立响应，不能冒充 Agent 调用证据。"
        : "真实检测报告由检测程序生成；页面只负责展示和导出，不重新解释模块结论。",
      forKey: .limitations
    )
  }
}
