import { describe, expect, it } from "vitest";
import {
  MemoryRecordStore,
  addAttempt,
  addEvent,
  addEvidence,
  completeRun,
  createRun,
  setModuleResult,
  setOverallConclusion,
  startRun,
  stopRun,
  uniqueIncidentIds,
} from "./records.js";

const target = {
  endpointFingerprint: "endpoint-a",
  model: "model-a",
  protocol: "chat-completions",
  authMode: "bearer",
  clientVersion: "0.1.0",
  environment: { os: "test", region: "local" },
};

function run() {
  return createRun({
    id: "run-a",
    now: "2026-09-11T00:00:00.000Z",
    target,
    conditions: {
      rules: { record: "v1" },
      testVersions: { capability: "v1" },
      settings: { temperature: 0 },
    },
    selectedModules: ["capability", "agent"],
  });
}

describe("detection record contract", () => {
  it("keeps configuration A immutable when configuration B starts", () => {
    const store = new MemoryRecordStore();
    const recordA = run();
    startRun(recordA, "2026-09-11T00:01:00.000Z");
    store.put(recordA);

    const recordB = createRun({
      id: "run-b",
      now: "2026-09-11T00:02:00.000Z",
      target: { ...target, model: "model-b" },
      selectedModules: ["capability"],
    });
    store.put(recordB);

    expect(store.get("run-a")?.target.model).toBe("model-a");
    expect(store.get("run-b")?.target.model).toBe("model-b");
    expect(store.get("run-b")?.overallConclusion).toBeUndefined();
  });

  it("shares one incident across module results without duplicating it", () => {
    const record = run();
    startRun(record);
    const evidence = addEvidence(record, {
      id: "ev-failure",
      kind: "tool-failure",
      capturedAt: "2026-09-11T00:03:00.000Z",
      payload: {
        authorization: "Bearer secret-value",
        message: "temporary failure",
      },
    });
    const event = addEvent(record, {
      id: "event-failure",
      kind: "tool_failure",
      occurredAt: "2026-09-11T00:03:01.000Z",
      summary: "Tool request failed",
      incidentId: "incident-1",
      evidenceRefs: [evidence.id],
    });
    const attempt = addAttempt(record, {
      id: "attempt-1",
      moduleId: "capability",
      kind: "initial",
      startedAt: "2026-09-11T00:03:00.000Z",
      endedAt: "2026-09-11T00:03:02.000Z",
      evidenceRefs: [evidence.id],
    });

    setModuleResult(record, {
      moduleId: "capability",
      state: "invalid_execution",
      attemptRefs: [attempt.id],
      evidenceRefs: [event.evidenceRefs[0]],
      incidentRefs: [event.incidentId!],
    });
    setModuleResult(record, {
      moduleId: "agent",
      state: "inconclusive",
      attemptRefs: [],
      evidenceRefs: [evidence.id],
      incidentRefs: [event.incidentId!],
    });

    expect(uniqueIncidentIds(record)).toEqual(["incident-1"]);
    expect(record.evidence[0]?.payload).toEqual({
      authorization: "[REDACTED]",
      message: "temporary failure",
    });
  });

  it("redacts credentials without removing measurable token fields", () => {
    const record = run();
    const evidence = addEvidence(record, {
      id: "ev-metrics",
      kind: "response",
      payload: {
        headers: { Authorization: "Bearer abc123" },
        usage: { prompt_tokens: 12, completion_tokens: 8, total_tokens: 20 },
        api_key: "sk-example-secret",
      },
    });

    expect(evidence.payload).toEqual({
      headers: { Authorization: "[REDACTED]" },
      usage: { prompt_tokens: 12, completion_tokens: 8, total_tokens: 20 },
      api_key: "[REDACTED]",
    });
    expect(evidence.redacted).toBe(true);
  });

  it("preserves selected, not-applicable, unverified and invalid execution states on stop", () => {
    const record = run();
    startRun(record);
    record.plan.find((entry) => entry.moduleId === "agent")!.state =
      "not_applicable";
    record.moduleResults.find((entry) => entry.moduleId === "agent")!.state =
      "not_applicable";
    const evidence = addEvidence(record, {
      id: "ev-partial",
      kind: "partial-result",
      payload: { value: "done" },
    });
    setModuleResult(record, {
      moduleId: "capability",
      state: "invalid_execution",
      reason: "Client stopped before a valid response",
      attemptRefs: [],
      evidenceRefs: [evidence.id],
      incidentRefs: [],
    });
    stopRun(record, "user requested stop", "2026-09-11T00:04:00.000Z");

    expect(record.lifecycle).toBe("stopped");
    expect(record.moduleResults.map((item) => item.state)).toEqual([
      "not_selected",
      "not_selected",
      "invalid_execution",
      "not_selected",
      "not_applicable",
      "not_selected",
    ]);
    expect(record.events.at(-1)?.summary).toContain("Run stopped");
    expect(record.evidence).toHaveLength(1);
  });

  it("keeps initial and recheck attempts as separate evidence", () => {
    const record = run();
    startRun(record);
    const initial = addAttempt(record, {
      id: "attempt-initial",
      moduleId: "capability",
      kind: "initial",
      startedAt: "2026-09-11T00:05:00.000Z",
    });
    const recheck = addAttempt(record, {
      id: "attempt-recheck",
      moduleId: "capability",
      kind: "recheck",
      startedAt: "2026-09-11T00:05:10.000Z",
      supersedesAttemptId: initial.id,
    });

    expect(record.attempts.map((item) => [item.id, item.kind])).toEqual([
      ["attempt-initial", "initial"],
      ["attempt-recheck", "recheck"],
    ]);
    expect(record.attempts[1]?.supersedesAttemptId).toBe(initial.id);
  });

  it("requires terminal results before completion and permits a conclusion only afterwards", () => {
    const record = run();
    startRun(record);
    expect(() => completeRun(record)).toThrow("unverified modules");

    setModuleResult(record, {
      moduleId: "capability",
      state: "pass",
      attemptRefs: [],
      evidenceRefs: [],
      incidentRefs: [],
    });
    setModuleResult(record, {
      moduleId: "agent",
      state: "inconclusive",
      attemptRefs: [],
      evidenceRefs: [],
      incidentRefs: [],
    });
    completeRun(record, "2026-09-11T00:06:00.000Z");
    setOverallConclusion(record, "limited", [], "2026-09-11T00:06:01.000Z");

    expect(record.lifecycle).toBe("completed");
    expect(record.overallConclusion).toEqual({
      state: "limited",
      evidenceRefs: [],
    });
  });
});
