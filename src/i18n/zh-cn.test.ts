import { describe, expect, test } from "vitest";

import {
  AGENTIC_FAILURE_LABELS,
  CATEGORY_LABELS_ZH,
  OUTCOME_LABELS_ZH,
  SURFACE_LABELS_ZH,
  TIER_LABELS_ZH,
  translateConformanceName,
  translateEvalName,
} from "./zh-cn";

describe("Simplified Chinese presentation labels", () => {
  test("translates the report vocabulary", () => {
    expect(TIER_LABELS_ZH.core).toBe("核心能力");
    expect(SURFACE_LABELS_ZH.chat).toBe("Chat Completions");
    expect(CATEGORY_LABELS_ZH["tool-selection"]).toBe("工具选择");
    expect(OUTCOME_LABELS_ZH.inconclusive).toBe("无法确定");
    expect(AGENTIC_FAILURE_LABELS["step-limit"]).toBe("超过步骤上限");
  });

  test("translates detection and eval names without changing their IDs", () => {
    expect(translateConformanceName("chat-basic", "basic completion")).toBe(
      "基础对话生成",
    );
    expect(
      translateEvalName(
        "eval-tool-select-weather",
        "picks the weather tool from a menu of decoys",
      ),
    ).toBe("从多个候选工具中选择天气工具");
  });
});
