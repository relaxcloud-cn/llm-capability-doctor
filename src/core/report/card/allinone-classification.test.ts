import { describe, expect, test } from "vitest";

import type { JsonReport } from "../json";
import {
  AGENT_RUNTIME_CHECK_GROUPS,
  AGENT_RUNTIME_HARD_CHECK_COUNT,
  AGENT_RUNTIME_SOFT_CHECK_COUNT,
} from "./agent-runtime-checks";
import { classifyAllInOneItems } from "./allinone-classification";

function reportWithPartialChecks(): JsonReport {
  return {
    version: 2,
    target: { baseUrl: "http://localhost:8080/v1", model: "test-model" },
    coverage: { byTier: [], credits: [], entries: [] },
    conformance: {
      pct: 50,
      passed: 2,
      total: 4,
      bySurface: [],
      results: [
        {
          id: "chat-basic",
          name: "基础对话生成",
          surface: "chat",
          outcome: "pass",
          failures: [],
        },
        {
          id: "chat-limits-max-tokens",
          name: "输出长度上限",
          surface: "chat",
          outcome: "fail",
          failures: [
            {
              id: "chat-max-tokens-respected",
              severity: "MUST",
              message: "输出超过限制",
            },
          ],
        },
        {
          id: "responses-basic",
          name: "Responses 基础对话生成",
          surface: "responses",
          outcome: "fail",
          failures: [],
        },
      ],
    },
    capability: {
      pct: 100,
      verdict: "strong",
      categories: [],
      weakCategories: [],
      evals: [
        {
          id: "eval-tool-select-weather",
          name: "选择天气工具",
          category: "tool-selection",
          passed: 3,
          total: 3,
        },
        {
          id: "eval-json-nested",
          name: "嵌套 JSON",
          category: "json-discipline",
          passed: 3,
          total: 3,
        },
      ],
    },
    durationMs: 1,
  };
}

describe("All-in-One report classification", () => {
  test("uses the working protocol and never promotes an alternate protocol failure", () => {
    const result = classifyAllInOneItems(reportWithPartialChecks());

    expect(result.primarySurface).toBe("chat");
    expect(result.hard.find((item) => item.id === "chat-basic")?.outcome).toBe(
      "pass",
    );
    expect(
      result.hard.find((item) => item.id === "chat-limits-max-tokens")?.outcome,
    ).toBe("fail");
    expect(
      result.soft.find((item) => item.id === "responses-basic")?.impact,
    ).toBe("record");
  });

  test("keeps required model behavior hard and JSON formatting soft", () => {
    const result = classifyAllInOneItems(reportWithPartialChecks());

    expect(
      result.hard.find((item) => item.id === "eval-tool-select-weather"),
    ).toBeDefined();
    expect(
      result.soft.find((item) => item.id === "eval-json-nested"),
    ).toBeDefined();
  });

  test("fills missing hard checks instead of reporting a partial run as complete", () => {
    const result = classifyAllInOneItems(reportWithPartialChecks());

    expect(result.complete).toBe(false);
    expect(result.hard).toHaveLength(30);
    expect(result.missingHardCount).toBe(27);
    expect(result.hardIssues).toHaveLength(28);
  });

  test("defines the complete Pi Agent checklist from issue 10", () => {
    expect(
      AGENT_RUNTIME_CHECK_GROUPS.map((group) => group.checks.length),
    ).toEqual([13, 8, 6]);
    expect(AGENT_RUNTIME_HARD_CHECK_COUNT).toBe(21);
    expect(AGENT_RUNTIME_SOFT_CHECK_COUNT).toBe(6);
  });
});
