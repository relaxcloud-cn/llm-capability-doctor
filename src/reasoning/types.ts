export type ReasoningSource =
  | "GPQA Diamond"
  | "GPQA Diamond (modified)"
  | "SuperGPQA"
  | "AIME2025"
  | "COMPSEC";

export interface ReasoningCase {
  source: ReasoningSource;
  id: string;
  domain: string;
  title: string;
  question: string;
  /** Multiple choice when present; otherwise an integer (AIME) or line spec (COMPSEC). */
  choices?: string[];
  answer: string;
}

export type ReasoningStatus = "passed" | "failed" | "stopped" | "error";

export interface ReasoningCaseResult {
  id: string;
  source: ReasoningSource;
  domain: string;
  title: string;
  status: ReasoningStatus;
  expected: string;
  got: string;
  /** Visible answer text, thinking stripped. Kept so a run can be regraded offline. */
  text: string;
  finishReason: string | null;
  outputTokens: number | null;
  reasoningTokens: number | null;
  durationMs: number;
  error?: string;
}

export interface ReasoningSourceSummary {
  source: string;
  passed: number;
  failed: number;
  stopped: number;
  error: number;
  total: number;
}

export interface ReasoningReport {
  passed: number;
  total: number;
  /** Ran out of tokens before an answer line; reported apart from wrong. */
  stopped: number;
  error: number;
  maxTokens: number;
  temperature: number;
  bySource: ReasoningSourceSummary[];
  cases: ReasoningCaseResult[];
  /** Set when --eval-questions or --eval-cases narrowed the set, or the run aborted early. */
  scopeNote: string | null;
  /** Set when the run stopped early; the completed cases are still reported. */
  aborted: { reason: "budget" | "unreachable"; message: string } | null;
}
