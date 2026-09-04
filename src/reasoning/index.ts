import { runConcurrent } from "../core/assert";
import { BudgetExceededError, TargetUnreachableError } from "../core/client";
import type { RunContext } from "../core/context";
import {
  answerMatches,
  extractAnswerDetailed,
  isCompsec,
  isMultipleChoice,
  visibleText,
} from "./grade";
import { REASONING_CASES } from "./cases";
import type {
  ReasoningCase,
  ReasoningCaseResult,
  ReasoningReport,
  ReasoningSourceSummary,
} from "./types";

export { REASONING_CASES } from "./cases";
export * from "./types";

export const SYSTEM_PROMPT =
  "You are solving a hard benchmark question. Reason carefully. " +
  "The final answer must follow the requested format exactly.";

const TAIL =
  "At the end, write exactly one final line in this format and do not write anything after it:\n";

export function buildPrompt(tc: ReasoningCase): string {
  if (isMultipleChoice(tc)) {
    const choices = tc
      .choices!.map((c, i) => `${String.fromCharCode(65 + i)}. ${c}`)
      .join("\n");
    return `${tc.question}\n\nChoices:\n${choices}\n\nSolve the question. ${TAIL}Answer: <letter>`;
  }
  if (isCompsec(tc)) {
    return `${tc.question}\n\n${TAIL}Answer: <line number or comma-separated line numbers>`;
  }
  return `${tc.question}\n\nSolve the problem. ${TAIL}Answer: <integer>`;
}

const SOURCE_ALIASES: Record<string, string[]> = {
  gpqa: ["GPQA Diamond", "GPQA Diamond (modified)"],
  gpqadiamond: ["GPQA Diamond", "GPQA Diamond (modified)"],
  aime2025: ["AIME2025"],
  supergpqa: ["SuperGPQA"],
  aime: ["AIME2025"],
  compsec: ["COMPSEC"],
};

/** "1,5,9" (1-based), case ids, or sources (gpqa, supergpqa, aime, compsec), in the order given. */
export function selectCases(
  all: ReasoningCase[],
  opts: { limit?: number; sequence?: string },
): ReasoningCase[] {
  let picked = all;
  if (opts.sequence) {
    picked = opts.sequence.split(",").flatMap((raw) => {
      const s = raw.trim();
      const sources = SOURCE_ALIASES[s.toLowerCase().replace(/[^a-z]/g, "")];
      if (sources) return all.filter((c) => sources.includes(c.source));
      const n = Number(s);
      const tc =
        Number.isInteger(n) && n >= 1 && n <= all.length
          ? all[n - 1]
          : all.find((c) => c.id === s);
      if (!tc) {
        throw new Error(
          `--eval-cases: unknown case '${s}' (1..${all.length}, a case id, or ${Object.keys(SOURCE_ALIASES).join("/")})`,
        );
      }
      return [tc];
    });
  }
  if (opts.limit !== undefined && opts.limit > 0)
    picked = picked.slice(0, opts.limit);
  return picked;
}

export interface ReasoningOptions {
  maxTokens: number;
  temperature: number;
  topP?: number;
  limit?: number;
  sequence?: string;
  /** Questions in flight at once. 1 (default) is fully sequential. */
  concurrency?: number;
  onCase?: (result: ReasoningCaseResult, index: number, total: number) => void;
}

/**
 * conformance runner's rule (two is a blip on a busy box, three is a corpse),
 * applied where it costs most: one dropped socket 40 minutes into a 92-case
 * eval must not throw away every question already answered.
 */
const UNREACHABLE_ATTEMPTS = 3;

export async function runReasoning(
  ctx: RunContext,
  opts: ReasoningOptions,
): Promise<ReasoningReport> {
  const surface = ctx.evalSurface;
  if (!surface) throw new Error("no chat-shaped surface available for --eval");

  const cases = selectCases(REASONING_CASES, opts);
  const concurrency = Math.max(1, opts.concurrency ?? 1);
  let aborted: ReasoningReport["aborted"] = null;

  // One question, socket-drop retries included. Returns null only when the
  // run must stop: the budget is gone, or the target is a corpse.
  const attemptCase = async (
    index: number,
  ): Promise<ReasoningCaseResult | null> => {
    const tc = cases[index];
    const started = Date.now();
    const base = {
      id: tc.id,
      source: tc.source,
      domain: tc.domain,
      title: tc.title,
      expected: tc.answer,
    };
    let unreachableMessage: string | null = null;

    for (let attempt = 1; attempt <= UNREACHABLE_ATTEMPTS; attempt += 1) {
      try {
        const res = await ctx.send(
          surface,
          {
            system: SYSTEM_PROMPT,
            turns: [{ type: "user", text: buildPrompt(tc) }],
            maxTokens: opts.maxTokens,
            temperature: opts.temperature,
            ...(opts.topP !== undefined ? { topP: opts.topP } : {}),
            allowReasoning: false,
          },
          // A 16k-token think runs for minutes; the token cap is the bound
          // here, and a clock would grade our patience rather than the model.
          { timeoutMs: null },
        );
        const text = res.reply.text ?? "";
        const { got, anchored } = extractAnswerDetailed(tc, text);
        const truncated = res.reply.finishReason === "length";
        // A truncated reply never finished; a match there only counts when it
        // came from an explicit answer line, not a lucky trailing token.
        const passed = answerMatches(tc, got) && (!truncated || anchored);
        return {
          ...base,
          status: passed ? "passed" : truncated ? "stopped" : "failed",
          got,
          text: visibleText(text).trim(),
          finishReason: res.reply.finishReason,
          outputTokens: res.reply.usage.outputTokens,
          reasoningTokens: res.reply.usage.reasoningTokens,
          durationMs: Date.now() - started,
        };
      } catch (err) {
        if (err instanceof BudgetExceededError) {
          // Keep what was answered; hours of eval work should survive the
          // abort. No retry: the budget is gone, not the target.
          aborted = { reason: "budget", message: err.message };
          return null;
        }
        if (err instanceof TargetUnreachableError) {
          unreachableMessage = err.message;
          if (attempt < UNREACHABLE_ATTEMPTS) {
            // A corpse refuses instantly, so this backoff never delays the
            // honest verdict — it only gives a dropped keep-alive socket or
            // a Wi-Fi blip a second to heal. (Executor form: the package
            // runs on Node 20, which predates Promise.withResolvers.)
            await new Promise<void>((resolve) =>
              setTimeout(resolve, attempt * 1_000),
            );
            continue;
          }
          break;
        }
        return {
          ...base,
          status: "error",
          got: "?",
          text: "",
          finishReason: null,
          outputTokens: null,
          reasoningTokens: null,
          durationMs: Date.now() - started,
          error: err instanceof Error ? err.message : String(err),
        };
      }
    }

    aborted = {
      reason: "unreachable",
      message: unreachableMessage ?? "target unreachable",
    };
    return null;
  };

  // In-flight questions reserve their token ceiling so a parallel burst
  // cannot each pass a budget check the others' unrecorded spending is
  // about to fail; sequential runs hold nothing and behave exactly as
  // before. The first abort stops new dispatch, but questions already in
  // flight finish and stay in the report.
  const holdBudget = concurrency > 1;
  const settled = await runConcurrent(
    cases.length,
    concurrency,
    async (index) => {
      if (holdBudget) ctx.client.reserveOutput(opts.maxTokens);
      try {
        const result = await attemptCase(index);
        if (result) opts.onCase?.(result, index, cases.length);
        return result;
      } finally {
        if (holdBudget) ctx.client.releaseOutput(opts.maxTokens);
      }
    },
    { shouldStop: () => aborted !== null },
  );

  const results = settled.filter((r): r is ReasoningCaseResult => r !== null);
  // `aborted` is only ever set inside the attempt closures, which control-flow
  // analysis cannot see; widen it before reading.
  const abortedFinal = aborted as ReasoningReport["aborted"];

  const bySource: ReasoningSourceSummary[] = [];
  for (const r of results) {
    let s = bySource.find((x) => x.source === r.source);
    if (!s) {
      s = {
        source: r.source,
        passed: 0,
        failed: 0,
        stopped: 0,
        error: 0,
        total: 0,
      };
      bySource.push(s);
    }
    s[r.status] += 1;
    s.total += 1;
  }
  const count = (st: ReasoningCaseResult["status"]) =>
    results.filter((r) => r.status === st).length;

  const scopeNote = abortedFinal
    ? `stopped after ${results.length} of ${cases.length} questions (${
        abortedFinal.reason === "budget"
          ? "token budget exhausted"
          : "target unreachable"
      }) — not comparable to a full run`
    : cases.length !== REASONING_CASES.length
      ? `${cases.length} of ${REASONING_CASES.length} questions — not comparable to a full run`
      : null;

  return {
    passed: count("passed"),
    total: results.length,
    stopped: count("stopped"),
    error: count("error"),
    maxTokens: opts.maxTokens,
    temperature: opts.temperature,
    bySource,
    cases: results,
    scopeNote,
    aborted: abortedFinal,
  };
}
