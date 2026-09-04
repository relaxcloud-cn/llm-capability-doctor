import { STEP_SPECULATION_FLOOR } from "../../bench/stats";
import type { ReasoningReport } from "../../reasoning/types";
import type {
  AgenticScore,
  BenchReport,
  BenchStat,
  CapabilityScore,
  ConformanceScore,
  ContextSpeculative,
  CoverageScore,
  EvalCategory,
  FidelityScore,
  RunReport,
} from "../outcome";
import { type Palette, paletteFor } from "./colors";
import {
  CATEGORY_LABELS_ZH,
  translateDetail,
  translateFidelityLabel,
  translateSurface,
  translateTier,
  translateVerdict,
} from "../../i18n/zh-cn";

const WIDTH = 74;

export const CATEGORY_LABELS: Record<EvalCategory, string> = {
  ...CATEGORY_LABELS_ZH,
};

// eslint-disable-next-line no-control-regex
const ANSI = /\x1b\[[0-9;]*m/g;

/** Length as the terminal sees it — colour codes take no columns. */
function visualLen(s: string): number {
  return s.replace(ANSI, "").length;
}

/** Left text, right text, flushed to the card's edges. */
function spread(left: string, right: string): string {
  const gap = Math.max(1, WIDTH - visualLen(left) - visualLen(right));
  return `${left}${" ".repeat(gap)}${right}`;
}

function fmtPct(n: number): string {
  return `${n}%`;
}

function bar(pct: number, c: Palette): string {
  const filled = Math.max(0, Math.min(10, Math.round(pct / 10)));
  const body = "█".repeat(filled) + "░".repeat(10 - filled);
  if (pct >= 90) return c.green(body);
  if (pct >= 50) return c.yellow(body);
  return c.red(body);
}

function fmtDuration(ms: number): string {
  const total = Math.round(ms / 1000);
  const mins = Math.floor(total / 60);
  const secs = total % 60;
  return mins > 0 ? `${mins}m ${secs}s` : `${secs}s`;
}

function fmtCount(n: number): string {
  return n.toLocaleString("en-US");
}

function plural(n: number, one: string, many = `${one}s`): string {
  return n === 1 ? one : many;
}

// ── sections ────────────────────────────────────────────────────────────────

function renderCoverage(coverage: CoverageScore, c: Palette): string[] {
  const lines = [c.bold("接口与能力覆盖情况")];

  for (const tier of coverage.byTier) {
    const label = translateTier(tier.tier).padEnd(10);
    const ratio = `${tier.supported}/${tier.total}`.padEnd(7);
    const pct = fmtPct(tier.pct).padStart(6);
    lines.push(`  ${label}${ratio}${pct}  ${bar(tier.pct, c)}`);

    // Name names. The missing list is the entire point of a normative suite —
    // "✗ responses" is the pressure on the engine to go implement it.
    if (tier.missing.length > 0) {
      const missing = tier.missing.map((m) => `✗ ${m}`).join("   ");
      lines.push(`            ${c.red(missing)}`);
    }

    // Never let "we skipped the check" read as "the engine lacks it".
    if (tier.unprobed.length > 0) {
      lines.push(
        `            ${c.gray(`? ${tier.unprobed.join(", ")} — 本轮未检测`)}`,
      );
    }
  }

  // Detected, shown, worth zero — we reward standards, not native APIs.
  for (const credit of coverage.credits) {
    lines.push(
      `  ${"已发现".padEnd(10)}${c.gray(`${credit.label} — 已检测到但不计分`)}`,
    );
  }

  return lines;
}

function renderConformance(conf: ConformanceScore, c: Palette): string[] {
  const lines = [
    spread(c.bold("接口协议正确性"), c.bold(fmtPct(conf.pct))),
    `  ${c.gray("只统计已实现接口中的 MUST（必须满足）断言")}`,
  ];

  if (conf.total === 0) {
    lines.push(`  ${c.gray("没有执行任何检测：未发现可执行的接口")}`);
    return lines;
  }

  const width = Math.max(
    ...conf.bySurface.map((s) => translateSurface(s.surface).length),
  );
  for (const surface of conf.bySurface) {
    const label = translateSurface(surface.surface).padEnd(width + 2);
    const ratio = `${surface.passed}/${surface.total}`.padEnd(10);
    const pct = fmtPct(surface.pct).padStart(6);
    const colour =
      surface.pct === 100 ? c.green : surface.pct >= 90 ? c.yellow : c.red;
    lines.push(`  ${label}${ratio}${colour(pct)}`);
  }

  // Inconclusive is the honest third state: the engine was never exercised
  // because the model wouldn't cooperate. Loud, and out of the denominator.
  if (conf.inconclusive.length > 0) {
    const n = conf.inconclusive.length;
    lines.push(
      `  ${c.yellow(`⚠ ${n} 项无法确定`)} ${c.gray("— 未能实际执行该接口")}`,
    );
    for (const result of conf.inconclusive) {
      lines.push(
        `      ${c.gray(`${result.name} — ${translateDetail(result.reason) ?? "?"}`)}`,
      );
    }
  }

  if (conf.warnings.length > 0) {
    const n = conf.warnings.length;
    lines.push(`  ${c.yellow(`⚠ ${n} 项 SHOULD 警告`)}`);
    for (const w of conf.warnings) {
      const detail = w.message ? ` — ${translateDetail(w.message)}` : "";
      lines.push(`      ${c.gray(`${w.label}${detail}`)}`);
    }
  }

  if (conf.nits.length > 0) {
    const n = conf.nits.length;
    lines.push(`  ${c.gray(`· ${n} 项 MAY 提示`)}`);
  }

  return lines;
}

function renderCapability(cap: CapabilityScore, c: Palette): string[] {
  const verdict =
    cap.verdict === "below-floor"
      ? c.red("未达到最低要求 ✗")
      : c.green(`${translateVerdict(cap.verdict)} ✓`);
  const headline = cap.total === 0 ? c.gray("未执行能力测试") : fmtPct(cap.pct);

  const lines = [
    spread(
      c.bold("模型能力"),
      cap.total === 0 ? headline : `${c.bold(headline)}   ${verdict}`,
    ),
  ];

  if (cap.total === 0) return lines;

  const labels = cap.categories.map((cat) => CATEGORY_LABELS[cat.category]);
  const width = Math.max(...labels.map((l) => l.length));

  for (const cat of cap.categories) {
    const label = CATEGORY_LABELS[cat.category].padEnd(width + 2);
    const ratio = `${cat.passed}/${cat.total}`.padEnd(8);
    const pct = fmtPct(cat.pct).padStart(5);
    const weak = cap.weakCategories.includes(cat.category);
    const shown = weak ? c.red(pct) : pct;
    lines.push(`  ${label}${ratio}${shown}  ${bar(cat.pct, c)}`);
  }

  if (cap.weakCategories.length > 0) {
    const names = cap.weakCategories
      .map((cat) => CATEGORY_LABELS[cat])
      .join(", ");
    lines.push(`  ${c.gray(`低于最低要求：${names}`)}`);
  }

  // Never let a category we could not measure read as a category the model
  // passed. Silence here would certify a model on the half of the card it
  // happened to be able to attempt.
  if (cap.unmeasured.length > 0) {
    const names = cap.unmeasured.map((cat) => CATEGORY_LABELS[cat]).join(", ");
    lines.push(
      `  ${c.yellow(`⚠ 未检测：${names}`)} ${c.gray("— 当前模型未能完成这些测试请求")}`,
    );
  }

  return lines;
}

function renderAgentic(agentic: AgenticScore, c: Palette): string[] {
  const headline = `${agentic.passed}/${agentic.total} 项`;
  const lines = [
    spread(c.bold("Agent 任务"), c.bold(headline)),
    `  ${c.gray("在模拟工作区中执行多步工具调用；要求高于基础能力测试，结果不会与模型能力合并")}`,
  ];

  const width = Math.max(...agentic.tasks.map((t) => t.name.length));
  for (const task of agentic.tasks) {
    const icon = task.passed ? c.green("✓") : c.red("✗");
    const name = task.name.padEnd(width + 2);
    const steps = c.gray(`${task.steps} 步`);
    lines.push(`  ${icon} ${name}${steps}`);
    if (!task.passed && task.detail) {
      lines.push(`      ${c.red("→")} ${c.gray(task.detail)}`);
    }
  }

  return lines;
}

function renderFidelity(fid: FidelityScore, c: Palette): string[] {
  const lines = [
    spread(c.bold("引擎保真度"), c.bold(fmtPct(fid.pct))),
    `  ${c.gray("仅适用于同一模型的比较；模型保持不变，因此该数值主要反映引擎差异")}`,
  ];

  const width = Math.max(
    ...fid.slices.map((s) => translateFidelityLabel(s.label).length),
  );
  for (const s of fid.slices) {
    const label = translateFidelityLabel(s.label).padEnd(width + 2);
    if (!s.measured) {
      lines.push(
        `  ${label}${c.gray("—      not measured")}  ${c.gray(s.detail)}`,
      );
      continue;
    }
    const scorePct = Math.round(s.score * 10000) / 100;
    const pct = fmtPct(scorePct).padStart(7);
    lines.push(`  ${label}${pct}  ${bar(scorePct, c)}  ${c.gray(s.detail)}`);
  }

  // The greedy self-divergence fact: a temperature-0 rerun that failed to
  // reproduce is a pure engine bug, and where it split is the useful part.
  if (fid.firstDivergence) {
    const d = fid.firstDivergence;
    lines.push(
      `  ${c.yellow(`⚠ 温度为 0 的重复运行在第 ${d.charIndex} 个字符处出现差异`)} ${c.gray(`（${d.itemId}，第 ${d.run}/${d.runs} 次）— 引擎存在非确定性`)}`,
    );
  }

  // Never let "no logprobs" read as a zero — name what dropped out instead.
  if (fid.unmeasured.length > 0) {
    lines.push(
      `  ${c.gray(`· ${fid.unmeasured.join(", ")} 未检测 — 引擎未提供 logprobs`)}`,
    );
  }

  if (fid.reasoningCaveat) {
    lines.push(
      `  ${c.gray("（推理模型 — 置信度取自思考后的概率分布，因此该分数只应作为最低参考）")}`,
    );
  }

  return lines;
}

function fmtStat(stat: BenchStat | null, unit: string): string {
  if (!stat) return "无数据";
  const range = stat.min === stat.max ? "" : ` (${stat.min}–${stat.max})`;
  return `${stat.median} ${unit}${range}`;
}

function fmtTokensK(n: number): string {
  return n >= 1000 ? `${Math.round(n / 100) / 10}k` : String(n);
}

/** ms under a second, seconds above — 38283ms reads better as "38.3s". */
function fmtLatency(ms: number): string {
  return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms} ms`;
}

/**
 * The speculation column of a context rung. Three readings, any of which can be
 * absent: tokens per decode step (silent when the engine buffers its stream),
 * the echo-vs-novel ratio (silent when the model did not echo), and the
 * counting ratio (--full only). When none survived, the rung says why instead
 * of showing a blank.
 *
 * Everything is shown against this rung's own novel rate, because a raw tok/s
 * at 32k says nothing on its own — the comparison is the measurement.
 */
function rungSpeculation(spec: ContextSpeculative | null, c: Palette): string {
  if (!spec) return "";

  const bits: string[] = [];
  if (spec.tokensPerStep !== null) {
    const text = `每步 ${spec.tokensPerStep} Token`;
    bits.push(
      spec.tokensPerStep >= STEP_SPECULATION_FLOOR
        ? c.green(text)
        : c.gray(text),
    );
  }
  // How much faster maximally predictable output runs at this same prompt
  // size — the headroom the draft path still has once the task is realistic.
  if (spec.ratio !== null) {
    const text = `${spec.ratio}× 上限`;
    bits.push(spec.verdict === "effective" ? c.green(text) : c.gray(text));
  }
  if (spec.predictableTokensPerStep !== null) {
    bits.push(c.gray(`${spec.predictableTokensPerStep} Token/步（上限内容）`));
  }

  if (bits.length === 0) {
    return c.gray(`  ${spec.note ?? "没有推测解码信号"}`);
  }
  const why = spec.note ? c.gray(` (${spec.note})`) : "";
  return `  ${bits.join(c.gray(" · "))}${why}`;
}

function renderReasoning(r: ReasoningReport, c: Palette): string[] {
  const pct = r.total > 0 ? Math.round((100 * r.passed) / r.total) : 0;
  const lines = [
    c.bold("专项推理"),
    `  ${c.gray(`仅供参考，不计分；每题最多 ${r.maxTokens} 个 Token，温度为 ${r.temperature}`)}`,
  ];
  if (r.scopeNote) lines.push(`  ${c.yellow(`⚠ ${r.scopeNote}`)}`);
  lines.push(
    `  ${"正确率".padEnd(22)}${c.bold(`${r.passed}/${r.total}`)} ${c.gray(`(${pct}%)`)}` +
      (r.stopped ? c.gray(`  · ${r.stopped} 题 Token 用尽`) : "") +
      (r.error ? c.red(`  · ${r.error} 题出错`) : ""),
  );
  for (const s of r.bySource) {
    lines.push(
      `  ${s.source.padEnd(22)}${`${s.passed}/${s.total}`.padEnd(8)}` +
        c.gray(
          [
            s.stopped ? `${s.stopped} 题中止` : "",
            s.error ? `${s.error} 题出错` : "",
          ]
            .filter(Boolean)
            .join(" · "),
        ),
    );
  }
  return lines;
}

function renderBench(bench: BenchReport, c: Palette): string[] {
  const machine = [
    bench.machine.cpu,
    `${bench.machine.memGB} GB`,
    `${bench.machine.platform} ${bench.machine.arch}`,
  ]
    .filter(Boolean)
    .join(" · ");

  // Sits above the figures it qualifies: if the box slowed while they were
  // being taken, that is the first thing to know about all of them.
  const drift = bench.loadDrift;
  const driftLines: string[] = [];
  if (drift && drift.driftPct !== null) {
    const sign = drift.driftPct > 0 ? "+" : "";
    const summary = `${drift.firstTokPerSec} → ${drift.lastTokPerSec} tok/s over ${fmtDuration(drift.elapsedMs)} (${sign}${drift.driftPct}%)`;
    if (drift.verdict === "steady") {
      driftLines.push(`  ${c.gray(`sustained load: steady — ${summary}`)}`);
    } else {
      const why =
        drift.verdict === "degraded"
          ? "机器在测量期间变慢，可能存在降频或其他负载"
          : "机器在运行中变快，说明预热不足，以下数据可能偏低";
      driftLines.push(`  ${c.yellow(`⚠ sustained load: ${summary}`)}`);
      driftLines.push(
        `  ${c.gray(`  ${why}; treat the figures below as a range`)}`,
      );
    }
  }

  const caveatLines: string[] = [];
  if (bench.streamCaveat) {
    caveatLines.push(`  ${c.yellow(`⚠ ${bench.streamCaveat}`)}`);
  }
  if (bench.decodeLengthNote) {
    caveatLines.push(`  ${c.gray(bench.decodeLengthNote)}`);
  }
  if (bench.samplingNote) {
    caveatLines.push(`  ${c.yellow(`⚠ ${bench.samplingNote}`)}`);
  }
  if (bench.runsNote) {
    caveatLines.push(`  ${c.yellow(`⚠ ${bench.runsNote}`)}`);
  }

  const lines = [
    c.bold("性能"),
    `  ${c.gray("仅供参考，不计分；与硬件有关，只适合在同一台机器上比较")}`,
    `  ${c.gray(`运行环境：${machine}`)}`,
    ...driftLines,
    ...caveatLines,
    `  ${"生成速度".padEnd(22)}${fmtStat(bench.decodeTokPerSec, "Token/秒")}`,
    `  ${"首 Token 延迟".padEnd(22)}${fmtStat(bench.ttftMs, "毫秒")}`,
  ];

  const prefill = fmtStat(bench.prefillTokPerSec, "Token/秒");
  const promptNote = bench.prefillPromptTokens
    ? c.gray(`  （${bench.prefillPromptTokens} Token 输入）`)
    : "";
  lines.push(`  ${"输入处理速度".padEnd(22)}${prefill}${promptNote}`);

  if (bench.contextScaling && bench.contextScaling.length > 0) {
    lines.push(
      `  ${c.gray("上下文扩展 生成速度 · 首 Token 延迟 · 推测解码效果随输入长度的变化")}`,
    );
    for (const point of bench.contextScaling) {
      const size = point.inputTokens ?? point.targetTokens;
      const sizeLabel = `~${fmtTokensK(size)}`.padStart(8);
      if (point.note) {
        lines.push(`  ${c.gray(sizeLabel)}   ${c.yellow(`✗ ${point.note}`)}`);
        continue;
      }
      const decode = (
        point.decodeTokPerSec !== null
          ? `${point.decodeTokPerSec} Token/秒`
          : "无数据"
      ).padEnd(13);
      const ttft = (
        point.ttftMs !== null ? fmtLatency(point.ttftMs) : "无数据"
      ).padEnd(10);
      lines.push(
        `  ${c.gray(`${sizeLabel}   ${decode}${ttft}`)}${rungSpeculation(point.speculative, c)}`,
      );
    }
  }

  const cache = bench.prefixCache;
  if (cache) {
    const detail =
      cache.verdict === "unknown"
        ? c.gray("无法测量")
        : `${cache.speedup}× ${c.gray(`（${fmtLatency(cache.coldTtftMs!)} 冷启动 → ${fmtLatency(cache.warmTtftMs!)} 热启动）`)}`;
    const label =
      cache.verdict === "active"
        ? c.green("已启用")
        : cache.verdict === "none"
          ? c.yellow("未发现")
          : c.gray("未知");
    lines.push(`  ${"前缀缓存".padEnd(22)}${label}  ${detail}`);
    // usage claiming a hit while the clock says otherwise is the whole point.
    if (cache.cachedTokens !== null) {
      lines.push(
        `  ${c.gray(`  报告显示 ${cache.cachedTokens}/${cache.promptTokens ?? "?"} 个输入 Token 命中缓存`)}`,
      );
    }
  }

  const batch = bench.batching;
  if (batch) {
    const label =
      batch.verdict === "batched"
        ? c.green("批处理")
        : batch.verdict === "partial"
          ? c.yellow("部分支持")
          : batch.verdict === "serialized"
            ? c.yellow("串行 — 请求在队列中等待")
            : c.gray("未知");
    lines.push(
      `  ${`并发（${batch.streams} 路）`.padEnd(22)}${label}${
        batch.efficiency !== null
          ? c.gray(`  ${batch.efficiency} 并发效率`)
          : ""
      }`,
    );
    if (batch.aggregateTokPerSec !== null) {
      const worst =
        batch.worstTtftMs !== null
          ? ` · 最慢首 Token ${fmtLatency(batch.worstTtftMs)}`
          : "";
      lines.push(
        `  ${c.gray(`  并发 ${batch.aggregateTokPerSec} Token/秒，单请求 ${batch.singleTokPerSec} Token/秒${worst}`)}`,
      );
    }
  }

  const spec = bench.speculative;
  if (spec) {
    const label =
      spec.verdict === "effective"
        ? c.green(`${spec.ratio}× — 有效（MTP/草稿路径已启用）`)
        : spec.verdict === "marginal"
          ? c.yellow(`${spec.ratio}× — 效果有限`)
          : c.gray(`${spec.ratio}× — 未发现`);
    lines.push(`  ${"推测解码".padEnd(22)}${label}`);
    lines.push(
      `  ${c.gray(`  可预测内容 ${spec.predictableTokPerSec} Token/秒 · 原创内容 ${spec.novelTokPerSec} Token/秒`)}`,
    );
    // Read straight off frame arrival gaps — no comparison involved, so it
    // stands even when the ratio is muddied.
    const steps =
      spec.tokensPerStep !== null
        ? `每次生成 ${spec.tokensPerStep} 个 Token`
        : `每次生成的 Token 数：${spec.tokensPerStepNote ?? "不可用"}`;
    lines.push(`  ${c.gray(`  ${steps}`)}`);
    if (spec.reasoningCaveat) {
      lines.push(
        `  ${c.gray("  （推理模型 — 思考阶段属于原创内容，因此该结果可能低估真实收益）")}`,
      );
    }
  }

  return lines;
}

// ── entry point ─────────────────────────────────────────────────────────────

export function renderReport(
  report: RunReport,
  options: { color?: boolean; benchOnly?: boolean } = {},
): string {
  const c = paletteFor(options.color ?? true);
  const rule = "━".repeat(WIDTH);

  const target = [
    report.target.baseUrl,
    report.target.engine,
    report.target.model,
  ]
    .filter(Boolean)
    .join(" · ");

  // --bench-only never ran the scored phases, so their cards would be three
  // empty sections claiming nothing was found. Absent beats zeroed.
  const scored = options.benchOnly
    ? []
    : [
        ...renderCoverage(report.coverage, c),
        "",
        ...renderConformance(report.conformance, c),
        "",
        ...renderCapability(report.capability, c),
        "",
      ];

  const lines: string[] = [
    rule,
    ` ${c.bold("llmprobe")} ${c.gray("·")} ${target}`,
    rule,
    "",
  ];

  // Above the cards, not below them: a reader who stops at the first number has
  // to have already been told the run did not finish.
  if (report.incomplete) {
    lines.push(
      ` ${c.red("✗ INCOMPLETE RUN")} ${c.gray("— the cards below are partial")}`,
      ` ${c.gray(report.incomplete)}`,
      "",
    );
  }

  lines.push(...scored);

  if (report.agentic) {
    lines.push(...renderAgentic(report.agentic, c), "");
  }

  if (report.fidelity) {
    lines.push(...renderFidelity(report.fidelity, c), "");
  }

  if (report.bench) {
    lines.push(...renderBench(report.bench, c), "");
  }

  if (report.reasoning) {
    lines.push(...renderReasoning(report.reasoning, c), "");
  }

  const footer: string[] = [];
  if (report.usage) {
    const { inputTokens, outputTokens } = report.usage;
    // Split, because the two are not interchangeable: on a paid endpoint they
    // are priced differently, and a run dominated by input is a context ladder
    // while one dominated by output is evals.
    footer.push(
      `${fmtCount(inputTokens + outputTokens)} tokens ` +
        `(${fmtCount(inputTokens)} in · ${fmtCount(outputTokens)} out)`,
    );
  }
  footer.push(fmtDuration(report.durationMs));
  lines.push(`  ${c.gray(footer.join(" · "))}`);

  return lines.join("\n");
}
