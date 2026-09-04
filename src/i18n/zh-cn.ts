import type { EvalCategory } from "../core/outcome";

export const TIER_LABELS_ZH: Record<string, string> = {
  core: "核心能力",
  extended: "扩展能力",
  frontier: "前沿能力",
};

export const SURFACE_LABELS_ZH: Record<string, string> = {
  models: "模型列表",
  chat: "Chat Completions",
  responses: "Responses",
  messages: "Messages",
  "count-tokens": "Token 计数",
  embeddings: "向量嵌入",
  completions: "旧版 Completions",
  images: "图片生成",
  "images-edit": "图片编辑",
  "audio-speech": "语音合成",
  "audio-transcriptions": "语音转写",
};

export const CATEGORY_LABELS_ZH: Record<EvalCategory, string> = {
  "tool-selection": "工具选择",
  "tool-restraint": "工具使用克制性",
  "tool-args": "工具参数准确性",
  multiturn: "多轮对话状态",
  instructions: "指令遵循",
  "json-discipline": "JSON 输出规范",
  "long-context": "长上下文召回",
  reasoning: "基础推理",
  knowledge: "基础知识",
};

export const OUTCOME_LABELS_ZH: Record<string, string> = {
  pass: "通过",
  fail: "失败",
  unsupported: "不支持",
  inconclusive: "无法确定",
  skipped: "未执行",
  unreachable: "无法连接",
  supported: "已支持",
  missing: "缺失",
  "not-probed": "未检测",
  partial: "部分完成",
};

export const VERDICT_LABELS_ZH: Record<string, string> = {
  "below-floor": "未达到最低要求",
  capable: "具备基本能力",
  strong: "能力较强",
};

export const PHASE_LABELS_ZH: Record<string, string> = {
  measured: "已检测",
  partial: "部分完成",
  "not-run": "未执行",
  unavailable: "不可用",
  interrupted: "已中断",
  failed: "失败",
};

export const AGENTIC_FAILURE_LABELS: Record<string, string> = {
  "no-tool-call": "未使用工具就直接回答",
  "wrong-answer": "使用了工具，但最终答案或文件状态错误",
  "step-limit": "超过步骤上限",
  "engine-error": "Agent 循环中发生接口错误",
};

const CONFORMANCE_SUFFIX_LABELS: Record<string, string> = {
  basic: "基础对话生成",
  streaming: "流式输出与 SSE 格式",
  parity: "流式与非流式结果一致性",
  usage: "Token 用量统计",
  "stream-usage": "流式 Token 用量统计",
  "finish-length": "输出达到上限时的结束原因",
  "limits-max-tokens": "max_tokens 参数生效",
  "limits-stop": "stop 参数生效",
  unicode: "Unicode 内容完整传输",
  errors: "错误状态码与错误结构",
  "tool-serialization": "工具调用序列化",
  "tool-stream-reassembly": "流式工具参数拼接",
  "tool-result-turn": "工具结果回传",
  "parallel-tools": "并行工具调用",
  "tool-choice-none": "tool_choice=none 禁止工具调用",
  "parallel-tools-off": "关闭并行工具调用",
  "tool-arg-types": "工具参数类型传输",
  "json-mode": "JSON 模式",
  "structured-outputs": "严格结构化输出",
  vision: "图片输入",
  logprobs: "Token 概率信息",
  seed: "seed 确定性",
  "top-p": "top_p 参数生效",
  reasoning: "推理内容与回答分离",
  "reasoning-scratchpad": "回答不是思考草稿",
  "reasoning-roundtrip": "多轮推理内容回传",
  "prompt-caching": "提示词缓存统计",
  "prompt-cache-prefix": "递增长对话复用提示词缓存",
  "rate-limit-headers": "限流响应头",
  concurrency: "并发请求相互隔离",
  "concurrency-cache": "并发相同请求结果一致",
};

const SURFACE_TEST_NAMES_ZH: Record<string, string> = {
  "models-list": "模型列表",
  "embeddings-basic": "基础向量嵌入",
  "embeddings-dimensions": "向量维度参数生效",
  "completions-basic": "旧版 Completions 基础调用",
  "images-generate": "图片生成",
  "images-edit": "图片编辑",
  "audio-speech": "语音合成",
  "responses-event-order": "Responses 流式事件顺序",
  "responses-previous-response-id":
    "Responses 通过 previous_response_id 续接对话",
  "responses-background": "Responses 后台模式",
  "responses-mcp-tools": "Responses 服务端 MCP 工具",
  "count-tokens": "Token 计数",
  "messages-event-order": "Messages 流式事件顺序",
  "messages-max-tokens-required": "Messages 要求 max_tokens",
  "messages-stop-sequence-echo": "Messages 返回 stop_sequence",
  "messages-assistant-prefill": "Messages assistant 消息预填充",
  "messages-thinking-budget": "Messages thinking 内容与预算",
  "messages-system-blocks": "Messages 系统消息块",
  "messages-content-blocks": "Messages 用户内容块",
  "messages-top-k": "Messages top_k 参数生效",
  "messages-cache-control": "Messages 缓存控制",
  "messages-error-envelope": "Messages 错误结构",
  "chat-n-choices": "Chat Completions 多候选结果",
  "chat-max-tokens-alias": "Chat Completions 旧版 max_tokens 参数",
  "chat-assistant-prefill": "Chat Completions assistant 消息预填充",
  "chat-stop-string": "Chat Completions 字符串形式 stop",
  "chat-template-kwargs": "Chat 模板扩展参数",
  "chat-sampling-extensions": "Chat 采样扩展参数",
};

const EVAL_NAMES_ZH: Record<string, string> = {
  "eval-tool-select-weather": "从多个候选工具中选择天气工具",
  "eval-tool-select-time": "区分时间工具和天气工具",
  "eval-tool-restraint-chitchat": "闲聊时不调用工具",
  "eval-tool-restraint-knowledge": "已知问题不调用无关工具",
  "eval-tool-args-typed": "按要求填写正确类型的工具参数",
  "eval-tool-args-enum": "遵守工具参数的枚举限制",
  "eval-multiturn-recall": "记住前面对话中的事实",
  "eval-multiturn-tool-result": "使用工具结果而不是编造答案",
  "eval-multiturn-system": "多轮对话后仍遵守系统指令",
  "eval-instructions-exact": "准确输出指定字符串",
  "eval-instructions-negative": "遵守禁止性要求",
  "eval-instructions-length": "遵守单词数量限制",
  "eval-json-no-fences": "按要求输出不带 Markdown 的原始 JSON",
  "eval-json-nested": "按要求生成嵌套 JSON",
  "eval-long-context-needle": "从长文本中找出隐藏信息",
  "eval-reasoning-arithmetic": "完成两位数乘法",
  "eval-reasoning-transitive": "完成传递关系推理",
  "eval-reasoning-counting": "准确统计字母数量",
  "eval-knowledge-capital": "回答非显而易见的首都问题",
  "eval-knowledge-symbol": "回答化学元素符号",
  "eval-knowledge-author": "回答知名小说作者",
};

const AGENTIC_TASK_NAMES_ZH: Record<string, string> = {
  "agentic-read": "读取正确配置，而不是凭经验猜测",
  "agentic-edit": "找到真正配置位置并只修改该位置",
  "agentic-indirect": "按照配置中的线索找到目标文件",
};

const FIDELITY_LABELS_ZH: Record<string, string> = {
  Correctness: "正确性",
  Confidence: "置信度",
  Determinism: "确定性",
  "Logprob consistency": "Token 概率一致性",
};

export function translateConformanceName(id: string, fallback: string): string {
  if (SURFACE_TEST_NAMES_ZH[id]) return SURFACE_TEST_NAMES_ZH[id]!;
  const suffix = id.replace(/^(?:chat|responses|messages)-/, "");
  return CONFORMANCE_SUFFIX_LABELS[suffix] ?? fallback;
}

export function translateEvalName(id: string, fallback: string): string {
  return EVAL_NAMES_ZH[id] ?? fallback;
}

export function translateAgenticTaskName(id: string, fallback: string): string {
  return AGENTIC_TASK_NAMES_ZH[id] ?? fallback;
}

export function translateFidelityLabel(label: string): string {
  return FIDELITY_LABELS_ZH[label] ?? label;
}

export function translateCategory(category: string): string {
  return CATEGORY_LABELS_ZH[category as EvalCategory] ?? category;
}

export function translateSurface(surface: string): string {
  return SURFACE_LABELS_ZH[surface] ?? surface;
}

const CAPABILITY_LABELS_ZH: Record<string, string> = {
  "/v1/models": "模型列表接口",
  "chat/completions": "Chat Completions 接口",
  responses: "Responses 接口",
  messages: "Messages 接口",
  "messages/count_tokens": "Messages Token 计数接口",
  embeddings: "向量嵌入接口",
  "completions (legacy)": "旧版 Completions 接口",
  "images/generations": "图片生成接口",
  "images/edits": "图片编辑接口",
  "audio/speech": "语音合成接口",
  "audio/transcriptions": "语音转写接口",
  "streaming + SSE framing": "流式输出与 SSE 格式",
  "tool calling": "工具调用",
  "JSON mode": "JSON 模式",
  "usage tokens": "Token 用量统计",
  "finish reasons": "结束原因",
  "error codes": "错误码",
  "stop + max_tokens": "stop 与 max_tokens",
  "structured outputs": "结构化输出",
  "parallel tool calls": "并行工具调用",
  vision: "图片理解",
  logprobs: "Token 概率信息",
  "reasoning items": "推理内容",
  "reasoning history round-trip": "多轮推理内容回传",
  "streamed usage": "流式 Token 用量",
  "seed determinism": "seed 确定性",
  "top_p sampling": "top_p 采样",
  "max_tokens legacy alias": "旧版 max_tokens 参数",
  "MCP tools": "MCP 工具",
  "n>1 choices": "多候选结果",
  "rate limiting": "限流",
  "prompt caching": "提示词缓存",
  "previous_response_id chaining": "previous_response_id 对话续接",
  "background responses": "后台响应",
};

export function translateCapabilityLabel(label: string | undefined): string {
  return label ? (CAPABILITY_LABELS_ZH[label] ?? label) : "";
}

export function translateTier(tier: string): string {
  return TIER_LABELS_ZH[tier] ?? tier;
}

export function translateOutcome(outcome: string): string {
  return OUTCOME_LABELS_ZH[outcome] ?? outcome;
}

export function translateVerdict(verdict: string): string {
  return VERDICT_LABELS_ZH[verdict] ?? verdict;
}

export function translatePhase(phase: string): string {
  return PHASE_LABELS_ZH[phase] ?? phase;
}

/** Translate stable browser-report copy without touching ids, classes, or protocol values. */
export function translateHtml(html: string): string {
  const replacements: Array<[string, string]> = [
    ['<html lang="en"', '<html lang="zh-CN"'],
    ["report card", "检测报告"],
    ["compare models", "比较模型"],
    ["Compare runs", "比较检测结果"],
    ["Select runs in each column", "在每一列选择一次检测结果"],
    ["in library", "个结果在报告库中"],
    ["What changed", "变化情况"],
    ["Pick two to four runs to compare.", "选择 2 到 4 个检测结果进行比较。"],
    [
      "Choose runs once at the top — columns stay aligned as you scroll.",
      "在顶部选择检测结果，向下滚动时各列会保持对齐。",
    ],
    [
      "Scores stay independent: Coverage, Conformance, and Capability are never averaged.",
      "各项结果保持独立：覆盖情况、协议正确性和模型能力不会合并平均。",
    ],
    ["Model pickers", "模型选择"],
    [
      "Best/worst marked green/red per row only when at least two columns have a run",
      "只有至少两列有检测结果时，才会在每行标记最好和最差结果",
    ],
    ["Never a blended overall score", "不会合并成一个总分"],
    ["Overview", "概览"],
    ["Surface coverage", "接口与能力覆盖"],
    ["Engine conformance", "接口协议正确性"],
    ["Model capability", "模型能力"],
    ["Agentic", "Agent 任务"],
    ["Engine fidelity", "引擎保真度"],
    ["Performance", "性能"],
    ["Reasoning", "专项推理"],
    ["Primary scores", "主要结果"],
    ["Secondary signals", "辅助结果"],
    ["Three independent scores — never averaged", "三个独立结果，不合并平均"],
    [
      "Only what this run measured — the scores stay independent",
      "只展示本轮实际检测的内容，各项结果保持独立",
    ],
    ["Core complete", "核心能力完整"],
    ["core complete", "核心能力完整"],
    ["Core gap", "核心能力缺失"],
    ["No surface breakdown.", "没有接口明细。"],
    ["No regressions recorded.", "没有记录到退化。"],
    ["No entry detail in this save.", "当前报告没有更详细的能力数据。"],
    ["Failures", "失败"],
    ["All checks", "全部检测"],
    ["not treated as pass", "不视为通过"],
    ["No per-eval detail in this save for", "当前报告没有"],
    ["Same-model comparisons only", "仅适用于同一模型的比较"],
    ["Informational — never scored", "仅供参考，不参与评分"],
    ["same-machine comparisons only", "只适合在同一台机器上比较"],
    ["Model library", "模型报告库"],
    ["Library", "报告库"],
    ["Quick compare", "快速比较"],
    ["model library", "模型报告库"],
    ["compare models", "比较模型"],
    ["Compare runs", "比较检测结果"],
    ["Select run", "选择检测结果"],
    ["open report", "打开报告"],
    ["Choose a run to load scores", "选择检测结果以加载数据"],
    ["Run ", "检测结果 "],
    ["Pick two to four runs to compare.", "选择 2 到 4 个检测结果进行比较。"],
    [
      "Each column starts empty — choose a run from the dropdown above that column.",
      "每一列开始为空，请从上方下拉菜单选择检测结果。",
    ],
    [
      "The same model on two servers is two different runs; pick each by host.",
      "同一模型在两台服务器上属于两个不同的检测结果，请按主机分别选择。",
    ],
    [
      "No measured score deltas between these runs.",
      "这些检测结果之间没有可比较的分数变化。",
    ],
    ["Comparing ", "正在比较 "],
    ["Mixed models and engines", "模型和引擎均不同"],
    ["Same model, different engines", "同一模型，不同引擎"],
    ["Same engine, different models", "同一引擎，不同模型"],
    [
      "Same model and engine — treat as before/after or depth change",
      "同一模型和引擎，可作为前后版本或检测深度变化比较",
    ],
    ["Coverage core", "核心能力覆盖"],
    ["Coverage extended", "扩展能力覆盖"],
    ["Coverage frontier", "前沿能力覆盖"],
    ["MUST violations (fewest)", "必须项违规（越少越好）"],
    ["MUST violations", "必须项违规"],
    ["Capability stayed ", "模型能力保持为 "],
    ["Agentic ", "Agent 任务 "],
    [
      "No measured score deltas between these two runs.",
      "这两个检测结果之间没有可比较的分数变化。",
    ],
    ["Primary scores", "主要结果"],
    ["Coverage detail", "覆盖情况明细"],
    ["Capability categories", "模型能力类别"],
    ["Side-by-side floor check", "并列最低能力检查"],
    ["Conformance by surface", "按接口查看协议正确性"],
    ["Charts", "图表"],
    [
      "Scores and context scaling — hardware-dependent timings only compare across runs on the same machine",
      "分数和上下文变化；与硬件有关的时间数据只适合在同一台机器上的检测结果之间比较",
    ],
    ["Coverage (Core)", "覆盖情况（核心能力）"],
    ["Coverage · core", "覆盖情况 · 核心能力"],
    ["Coverage · extended", "覆盖情况 · 扩展能力"],
    ["Coverage · frontier", "覆盖情况 · 前沿能力"],
    ["Missing · ", "缺失项 · "],
    ["Surfaces", "接口"],
    ["Conformance", "协议正确性"],
    ["Capability", "模型能力"],
    ["Fidelity", "引擎保真度"],
    ["Model", "模型"],
    ["Decode", "生成速度"],
    ["Prefill", "输入处理速度"],
    ["TTFT", "首 Token 延迟"],
    ["Last run", "最近检测"],
    ["Actions", "操作"],
    ["How to use this library", "如何使用报告库"],
    [
      "Rank models by any column. Open a full report, or pick two models to compare.",
      "可以按任意列对模型排序，打开完整报告，或选择模型进行比较。",
    ],
    ["Search models", "搜索模型"],
    ["Search models…", "搜索模型…"],
    ["Clear search", "清除搜索"],
    ["Sort by", "排序方式"],
    ["Most recent", "最近检测"],
    ["Last run", "最近检测"],
    ["Actions", "操作"],
    ["Compare selection", "比较选择"],
    ["Select run", "选择检测结果"],
    ["open report →", "打开报告 →"],
    ["Choose a run to load scores", "选择检测结果以加载数据"],
    [
      "No models match your search. Clear the filter to see the full library.",
      "没有匹配搜索条件的模型。清除筛选可以查看完整报告库。",
    ],
    ["Most recent", "最近检测"],
    ["High → low", "从高到低"],
    ["Low → high", "从低到高"],
    ["Newer first", "最新在前"],
    ["just now", "刚刚"],
    ["day ago", "天前"],
    ["hours ago", "小时前"],
    ["minutes ago", "分钟前"],
    ["Theme", "主题"],
    ["Light", "浅色"],
    ["Dark", "深色"],
    ["Cyber", "赛博"],
    ["llmprobe comparison", "llmprobe 比较报告"],
    ["comparison of", "比较"],
    ["runs side by side", "个检测结果并列比较"],
    ["Dashboard perspective", "报告视角"],
    ["Engine reference", "引擎对比参考"],
    [
      "choose a run to inspect regressions and improvements",
      "选择一个检测结果查看退化和改进",
    ],
    ["Reference run", "参考检测结果"],
    [
      "Select a reference to compare supported surfaces and MUST outcomes.",
      "选择参考检测结果，比较已支持接口和必须项结果。",
    ],
    ["Core gaps", "核心能力缺失"],
    ["Scorecard", "结果对比表"],
    [
      "Coverage, conformance and capability are hardware-independent. Timings require identical hardware; fidelity requires the same model identifier.",
      "覆盖情况、协议正确性和模型能力与硬件无关。时间数据需要相同硬件；保真度需要相同模型标识。",
    ],
    ["Performance vs context", "性能与上下文关系"],
    ["one line per run", "每个检测结果一条曲线"],
    [
      "No run carries benchmark data — re-run with",
      "没有检测结果包含性能数据，请使用",
    ],
    ["to compare curves.", "重新检测以比较曲线。"],
    ["All runs measured on", "所有检测结果均在"],
    [
      "These runs are from different machines — coverage, conformance and capability still compare, but every timing below does not.",
      "这些检测结果来自不同机器；覆盖情况、协议正确性和模型能力仍可比较，但下方时间数据不可比较。",
    ],
    ["Decode vs context", "生成速度与上下文关系"],
    ["First token vs context", "首 Token 延迟与上下文关系"],
    ["Context scaling", "上下文扩展"],
    ["Prefix cache", "前缀缓存"],
    ["Concurrency", "并发"],
    ["Speculative decode", "推测解码"],
    ["Sustained load", "持续运行"],
    [
      "informational — not scored; hardware-dependent, same-machine comparisons only",
      "仅供参考，不计分；与硬件有关，只适合在同一台机器上比较",
    ],
    ["Decode throughput", "生成速度"],
    ["Time to first token", "首 Token 延迟"],
    ["Prefill throughput", "输入处理速度"],
    ["Prompt", "输入长度"],
    ["First token", "首 Token"],
    ["Speculation", "推测解码"],
    ["Measurement", "测量项目"],
    ["How it was taken", "测量方式"],
    [
      "steady-state generation while writing code",
      "模拟代码生成时的持续生成速度",
    ],
    [
      "latency before the first generated token",
      "从请求开始到首个生成 Token 的时间",
    ],
    ["prompt ingestion rate", "输入内容处理速度"],
    ["out of tokens", "Token 用尽"],
    ["correct", "正确"],
    ["accuracy", "正确率"],
    ["Errors", "错误"],
    ["Source", "题目来源"],
    ["Domain", "领域"],
    ["Question", "题目"],
    ["Result", "结果"],
    ["Got", "实际答案"],
    ["Expected", "正确答案"],
    ["Output tokens", "输出 Token"],
  ];
  return replacements.reduce(
    (value, [from, to]) => value.split(from).join(to),
    html,
  );
}

/** Translate CLI progress text while keeping ANSI color codes and technical values intact. */
export function translateConsoleLine(line: string): string {
  const replacements: Array<[RegExp, string]> = [
    [/\bprobing\b/gi, "正在检测"],
    [/\bmodel:\s*/gi, "模型："],
    [/\bdepth:\s*/gi, "检测深度："],
    [/reasoning model\s*—\s*/gi, "推理模型 — "],
    [
      /fidelity \(cloze battery \+ greedy self-consistency\)\.\.\./gi,
      "引擎保真度（基础题与确定性检查）...",
    ],
    [
      /benchmarking \(warmup \+ median of/gi,
      "正在进行性能检测（预热 + 中位数，共",
    ],
    [/discarded/gi, "已丢弃"],
    [
      /benchmark discarded — the target stopped answering\./gi,
      "性能检测已丢弃：目标接口停止响应。",
    ],
    [/benchmark failed:/gi, "性能检测失败："],
    [/benchmark used/gi, "性能检测消耗"],
    [/tokens\s*\(/gi, "Token（"],
    [/over (\d+) requests/gi, "共 $1 次请求"],
    [/reasoning eval \(up to/gi, "专项推理测试（每题最多"],
    [/keeping the (\d+) answered questions/gi, "保留已回答的 $1 道题"],
    [/eval used/gi, "专项推理测试消耗"],
    [/fidelity failed:/gi, "引擎保真度检测失败："],
    [/cloze battery (\d+)\/(\d+)/gi, "基础题测试 $1/$2"],
    [/greedy:\s*/gi, "确定性检查："],
    [/context calibrating/gi, "正在校准上下文长度"],
    [/context (~[\w.]+) ceiling/gi, "上下文 $1 上限内容"],
    [/context (~[\w.]+)/gi, "上下文 $1"],
    [/prefix cache cold/gi, "前缀缓存冷启动"],
    [/prefix cache warm/gi, "前缀缓存热启动"],
    [/batching 1 stream/gi, "单路并发测试"],
    [/batching (\d+) streams/gi, "$1 路并发测试"],
    [/decode length probe/gi, "生成长度检查"],
    [/sustained load/gi, "持续运行"],
    [/tok\/s decode/gi, "Token/秒生成"],
    [/tok\/s prefill/gi, "Token/秒输入处理"],
    [/s first token/gi, "秒首 Token"],
    [/tok\/step/gi, "Token/步"],
    [/\bn\/a\b/gi, "无数据"],
    [/target became unreachable after/gi, "目标接口在以下检测后无法连接："],
    [/scores below are partial\./gi, "以下结果只是部分结果。"],
    [/checks? not run/gi, "项未执行"],
    [/non-interactive — using first model:/gi, "非交互模式 — 使用第一个模型："],
    [/pass --model to pick another/gi, "请使用 --model 选择其他模型"],
    [
      /could not determine a model — pass one with --model <id>\./gi,
      "无法确定模型，请使用 --model <id> 指定。",
    ],
    [/failed to list \/v1\/models/gi, "读取 \/v1\/models 失败"],
    [/detected, not scored/gi, "已检测到但不计分"],
    [/opened →/gi, "已打开 →"],
    [/comparison of/gi, "比较"],
    [/runs →/gi, "个检测结果 →"],
    [/library →/gi, "报告库 →"],
    [/library\b/gi, "报告库"],
    [/ingested →/gi, "已导入 →"],
    [/models →/gi, "模型 →"],
    [/index →/gi, "索引 →"],
    [/compare →/gi, "比较页 →"],
    [/cards →/gi, "报告卡片 →"],
    [/report card/gi, "检测报告"],
  ];
  return replacements.reduce(
    (value, [pattern, replacement]) => value.replace(pattern, replacement),
    line,
  );
}

/** Translate stable protocol vocabulary while leaving technical evidence intact. */
export function translateDetail(
  detail: string | undefined,
): string | undefined {
  if (!detail) return detail;
  let value = detail;
  const replacements: Array<[RegExp, string]> = [
    [/not measured/gi, "未检测"],
    [/not probed(?: at this depth)?/gi, "本轮未检测"],
    [/not supported/gi, "不支持"],
    [/unsupported/gi, "不支持"],
    [/inconclusive/gi, "无法确定"],
    [/engine never exercised/gi, "未实际执行该接口"],
    [/model did not exercise the path/gi, "模型没有触发该路径"],
    [/model never emitted a tool call/gi, "模型没有触发工具调用"],
    [/no implemented surface ran/gi, "没有已实现的接口被执行"],
    [/no evals ran/gi, "未执行模型能力测试"],
    [/sample failure/gi, "样本失败"],
    [/feature unsupported/gi, "能力不支持"],
    [/not exercised/gi, "未实际执行"],
    [/skipped/gi, "未执行"],
    [/present/gi, "已提供"],
    [/failed/gi, "失败"],
    [/pass(?:ed)?/gi, "通过"],
    [/wrong-answer/gi, "答案或状态错误"],
    [/no-tool-call/gi, "未调用工具"],
    [/step-limit/gi, "超过步骤上限"],
    [/engine-error/gi, "接口错误"],
  ];
  for (const [pattern, replacement] of replacements)
    value = value.replace(pattern, replacement);
  return value;
}
