import type { JsonReport } from "../json";

export type ReportItemRole = "hard" | "soft";
export type SoftImpact = "strong" | "small" | "record";

export interface ClassifiedReportItem {
  id: string;
  key: string;
  name: string;
  source: "conformance" | "capability";
  sourceLabel: string;
  outcome: string;
  role: ReportItemRole;
  reason: string;
  impact?: SoftImpact;
  sample?: string;
  detail?: string;
}

export interface AllInOneClassification {
  available: boolean;
  complete: boolean;
  primarySurface: string | null;
  hard: ClassifiedReportItem[];
  soft: ClassifiedReportItem[];
  hardPassed: number;
  hardIssues: ClassifiedReportItem[];
  missingHardCount: number;
}

const SURFACE_NAMES: Record<string, string> = {
  chat: "Chat Completions",
  responses: "Responses",
  messages: "Messages",
};

const HARD_CONFORMANCE_REASONS: Record<string, string> = {
  basic: "基础生成失败时，后续 Agent 流程无法开始。",
  streaming: "流式格式错误会让运行时无法读取或结束本轮输出。",
  "finish-length": "运行时依赖结束标记判断回答是否被截断。",
  "limits-max-tokens": "输出上限失效会导致任务超出时间和 Token 预算。",
  "limits-stop": "停止条件失效时，运行时不能按要求结束生成。",
  unicode: "中文指令、日志和证据必须完整传输。",
  "tool-serialization": "工具名称和参数必须按协议交给运行时。",
  "tool-stream-reassembly": "流式工具参数无法拼接时，工具不能执行。",
  "tool-result-turn": "工具结果必须能够回传给模型并继续后续步骤。",
  "tool-choice-none": "运行时禁止工具调用时，模型不能绕过这个限制。",
  "parallel-tools-off": "运行时要求串行调用时，模型不能擅自发出多个调用。",
  "tool-arg-types": "参数类型损坏会直接导致工具调用失败。",
  "reasoning-scratchpad": "内部思考过程不能混入面向用户的回答和报告。",
  concurrency: "多个任务同时运行时，请求和回答不能相互串线。",
};

const HARD_CONFORMANCE_NAMES: Record<string, string> = {
  basic: "基础对话生成",
  streaming: "流式输出与结束",
  "finish-length": "输出达到上限时的结束标记",
  "limits-max-tokens": "输出长度上限",
  "limits-stop": "停止条件",
  unicode: "中文和 Unicode 内容传输",
  "tool-serialization": "工具调用格式",
  "tool-stream-reassembly": "流式工具参数拼接",
  "tool-result-turn": "工具结果回传",
  "tool-choice-none": "禁止工具调用",
  "parallel-tools-off": "单工具调用限制",
  "tool-arg-types": "工具参数类型",
  "reasoning-scratchpad": "内部思考不进入回答",
  concurrency: "并发任务相互隔离",
};

const HARD_CAPABILITY_NAMES: Record<string, string> = {
  "eval-tool-select-weather": "从候选工具中选择天气工具",
  "eval-tool-select-time": "区分时间工具和天气工具",
  "eval-tool-restraint-chitchat": "闲聊时不调用工具",
  "eval-tool-restraint-knowledge": "已知问题不调用无关工具",
  "eval-tool-args-typed": "填写正确类型的工具参数",
  "eval-tool-args-enum": "遵守工具参数允许范围",
  "eval-multiturn-recall": "记住前面对话中的事实",
  "eval-multiturn-tool-result": "使用工具结果而不是编造答案",
  "eval-multiturn-system": "多轮对话后仍遵守系统要求",
  "eval-instructions-exact": "准确输出指定内容",
  "eval-instructions-negative": "遵守禁止性要求",
  "eval-instructions-length": "遵守回答长度要求",
  "eval-long-context-needle": "从长文本中找到关键信息",
  "eval-reasoning-arithmetic": "完成基础计算",
  "eval-reasoning-transitive": "完成关系推理",
  "eval-reasoning-counting": "完成基础统计",
};

const HARD_CAPABILITY_REASONS: Record<string, string> = {
  "eval-tool-select-weather":
    "模型必须从多个候选工具中选择完成任务所需的工具。",
  "eval-tool-select-time": "模型必须区分用途相近的不同工具。",
  "eval-tool-restraint-chitchat": "没有必要时不能擅自调用工具。",
  "eval-tool-restraint-knowledge": "能够直接回答时不能调用无关工具。",
  "eval-tool-args-typed": "模型填写的工具参数必须使用正确类型。",
  "eval-tool-args-enum": "模型填写的工具参数必须符合允许范围。",
  "eval-multiturn-recall": "多步任务中必须保留前序信息。",
  "eval-multiturn-tool-result": "模型必须使用真实工具结果，不能自行编造。",
  "eval-multiturn-system": "多轮任务中必须持续遵守系统要求。",
  "eval-instructions-exact": "模型必须能够准确执行明确的输出要求。",
  "eval-instructions-negative": "模型必须遵守明确禁止执行的要求。",
  "eval-instructions-length": "模型必须遵守明确的回答长度要求。",
  "eval-long-context-needle": "模型必须能从长日志和长证据中找到关键信息。",
  "eval-reasoning-arithmetic": "基础计算错误会直接影响数量和风险判断。",
  "eval-reasoning-transitive": "模型必须能完成基本的关系推理。",
  "eval-reasoning-counting": "模型必须能完成基本的信息统计。",
};

const INFO_ONLY_CHAT_TESTS = new Set([
  "usage",
  "stream-usage",
  "logprobs",
  "prompt-caching",
  "prompt-cache-prefix",
  "rate-limit-headers",
  "n-choices",
  "max-tokens-alias",
  "assistant-prefill",
  "template-kwargs",
]);

const STRONG_CAPABILITY_GROUPS = new Set([
  "tool-selection",
  "tool-restraint",
  "tool-args",
  "multiturn",
  "instructions",
  "json-discipline",
  "long-context",
  "reasoning",
]);

const STRONG_SOFT_ID_PARTS = [
  "finish",
  "max-tokens",
  "stop",
  "tool-choice",
  "parallel-tools-off",
  "json",
  "structured",
  "vision",
  "reasoning-scratchpad",
  "unicode",
  "errors",
];

function resultSuffix(id: string, surface: string): string {
  return id.startsWith(`${surface}-`) ? id.slice(surface.length + 1) : id;
}

function selectPrimarySurface(report: JsonReport): string | null {
  const results = report.conformance?.results ?? [];
  const candidates = ["chat", "responses", "messages"]
    .map((surface, order) => {
      const surfaceResults = results.filter(
        (result) => result.surface === surface,
      );
      const basic = surfaceResults.find(
        (result) => result.id === `${surface}-basic`,
      );
      const hardPasses = surfaceResults.filter((result) => {
        const suffix = resultSuffix(result.id, surface);
        return suffix in HARD_CONFORMANCE_REASONS && result.outcome === "pass";
      }).length;
      return {
        surface,
        order,
        usable: basic?.outcome === "pass",
        hardPasses,
        tests: surfaceResults.length,
      };
    })
    .filter((candidate) => candidate.tests > 0)
    .sort(
      (left, right) =>
        Number(right.usable) - Number(left.usable) ||
        right.hardPasses - left.hardPasses ||
        left.order - right.order,
    );
  return candidates[0]?.surface ?? null;
}

function softReason(item: {
  id: string;
  source: "conformance" | "capability";
  group?: string;
}): string {
  if (item.source === "capability") {
    const reasons: Record<string, string> = {
      "tool-selection":
        "影响模型选择正确工具的成功率，需要在真实任务中继续确认。",
      "tool-restraint": "影响是否会产生不必要的工具调用。",
      "tool-args": "影响模型填写工具参数的准确率。",
      multiturn: "影响模型记忆前序信息和使用工具结果的能力。",
      instructions: "影响复杂指令的完成质量。",
      "json-discipline": "影响报告或中间结果的结构稳定性。",
      "long-context": "影响长日志和多份证据中的信息召回质量。",
      reasoning: "影响研判和多步推理质量。",
      knowledge: "影响无需工具时的知识回答质量。",
    };
    return reasons[item.group ?? ""] ?? "影响任务效果，但不直接阻断接口调用。";
  }
  if (
    item.id.includes("finish") ||
    item.id.includes("max-tokens") ||
    item.id.includes("stop")
  ) {
    return "影响输出截断和停止控制。";
  }
  if (item.id.includes("tool-choice") || item.id.includes("parallel-tool")) {
    return "影响工具调用的限制和并行控制。";
  }
  if (item.id.includes("json") || item.id.includes("structured")) {
    return "影响结构化结果的稳定性，可根据实际使用方式处理。";
  }
  if (item.id.includes("vision")) return "只有任务包含图片证据时才会产生影响。";
  if (
    item.id.includes("seed") ||
    item.id.includes("top-p") ||
    item.id.includes("sampling")
  ) {
    return "影响结果复现和采样调节。";
  }
  if (item.id.includes("reasoning")) return "影响推理内容的呈现或续接。";
  if (item.id.includes("concurrency")) return "影响并发运行时的稳定性。";
  return "会影响兼容性或任务效果，但不直接阻断当前主要流程。";
}

function recordReason(
  surface: string,
  id: string,
  primarySurface: string,
): string {
  if (surface !== primarySurface) {
    return `当前主要使用 ${SURFACE_NAMES[primarySurface] ?? primarySurface}；${SURFACE_NAMES[surface] ?? surface} 的结果只用于了解备用路径。`;
  }
  if (id.includes("usage")) return "用于 Token 统计和成本核算。";
  if (id.includes("cache")) return "用于评估成本和性能优化。";
  if (id.includes("rate-limit")) return "用于容量和运维评估。";
  if (id.includes("logprobs")) return "用于概率分析和调试。";
  return "用于补充能力范围，不决定必要任务能否完成。";
}

function softImpact(
  item: { id: string; source: "conformance" | "capability"; group?: string },
  primarySurface: string,
): SoftImpact {
  if (item.source === "conformance") {
    const suffix = resultSuffix(item.id, primarySurface);
    if (
      !item.id.startsWith(`${primarySurface}-`) ||
      INFO_ONLY_CHAT_TESTS.has(suffix)
    ) {
      return "record";
    }
  }
  if (
    item.source === "capability" &&
    STRONG_CAPABILITY_GROUPS.has(item.group ?? "")
  ) {
    return "strong";
  }
  return STRONG_SOFT_ID_PARTS.some((part) => item.id.includes(part))
    ? "strong"
    : "small";
}

export function classifyAllInOneItems(
  report: JsonReport,
): AllInOneClassification {
  const primarySurface = selectPrimarySurface(report);
  if (!primarySurface) {
    return {
      available: false,
      complete: false,
      primarySurface: null,
      hard: [],
      soft: [],
      hardPassed: 0,
      hardIssues: [],
      missingHardCount: 0,
    };
  }

  const hard: ClassifiedReportItem[] = [];
  const soft: ClassifiedReportItem[] = [];
  const conformanceResults = report.conformance?.results ?? [];

  for (const result of conformanceResults) {
    const suffix = resultSuffix(result.id, primarySurface);
    const hardReason =
      result.surface === primarySurface
        ? HARD_CONFORMANCE_REASONS[suffix]
        : undefined;
    if (hardReason) {
      hard.push({
        id: result.id,
        key: `conformance:${result.id}`,
        name: result.name ?? result.id,
        source: "conformance",
        sourceLabel: `协议检测 · ${SURFACE_NAMES[result.surface] ?? result.surface}`,
        outcome: result.outcome,
        role: "hard",
        reason: hardReason,
        detail: result.failures
          ?.map((failure) => failure.message)
          .filter(Boolean)
          .join("；"),
      });
      continue;
    }
    const impact = softImpact(
      { id: result.id, source: "conformance" },
      primarySurface,
    );
    soft.push({
      id: result.id,
      key: `conformance:${result.id}`,
      name: result.name ?? result.id,
      source: "conformance",
      sourceLabel: `协议检测 · ${SURFACE_NAMES[result.surface] ?? result.surface}`,
      outcome: result.outcome,
      role: "soft",
      impact,
      reason:
        impact === "record"
          ? recordReason(result.surface, result.id, primarySurface)
          : softReason({ id: result.id, source: "conformance" }),
      detail:
        result.reason ??
        result.failures
          ?.map((failure) => failure.message)
          .filter(Boolean)
          .join("；"),
    });
  }

  for (const evaluation of report.capability?.evals ?? []) {
    const hardReason = HARD_CAPABILITY_REASONS[evaluation.id];
    const outcome = evaluation.passed >= evaluation.total ? "pass" : "fail";
    const item: ClassifiedReportItem = {
      id: evaluation.id,
      key: `capability:${evaluation.id}`,
      name: evaluation.name ?? evaluation.id,
      source: "capability",
      sourceLabel: "模型行为任务",
      outcome,
      role: hardReason ? "hard" : "soft",
      reason:
        hardReason ??
        softReason({
          id: evaluation.id,
          source: "capability",
          group: evaluation.category,
        }),
      sample: `${evaluation.passed}/${evaluation.total}`,
      detail: evaluation.failures?.join("；"),
      ...(hardReason
        ? {}
        : {
            impact: softImpact(
              {
                id: evaluation.id,
                source: "capability",
                group: evaluation.category,
              },
              primarySurface,
            ),
          }),
    };
    (hardReason ? hard : soft).push(item);
  }

  let missingHardCount = 0;
  const presentHardIds = new Set(hard.map((item) => item.id));
  for (const [suffix, reason] of Object.entries(HARD_CONFORMANCE_REASONS)) {
    if (primarySurface === "responses" && suffix === "limits-stop") continue;
    const id = `${primarySurface}-${suffix}`;
    if (presentHardIds.has(id)) continue;
    missingHardCount += 1;
    hard.push({
      id,
      key: `conformance:${id}`,
      name: HARD_CONFORMANCE_NAMES[suffix] ?? id,
      source: "conformance",
      sourceLabel: `协议检测 · ${SURFACE_NAMES[primarySurface] ?? primarySurface}`,
      outcome: "not-run",
      role: "hard",
      reason,
      detail: "本轮没有该项结果，不能视为通过。",
    });
  }
  for (const [id, reason] of Object.entries(HARD_CAPABILITY_REASONS)) {
    if (presentHardIds.has(id)) continue;
    missingHardCount += 1;
    hard.push({
      id,
      key: `capability:${id}`,
      name: HARD_CAPABILITY_NAMES[id] ?? id,
      source: "capability",
      sourceLabel: "模型行为任务",
      outcome: "not-run",
      role: "hard",
      reason,
      detail: "本轮没有该项结果，不能视为通过。",
    });
  }

  const hardIssues = hard.filter((item) => item.outcome !== "pass");
  return {
    available: true,
    complete: missingHardCount === 0,
    primarySurface,
    hard,
    soft,
    hardPassed: hard.length - hardIssues.length,
    hardIssues,
    missingHardCount,
  };
}
