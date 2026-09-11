import { describe, expect, it } from "vitest";
import {
  buildServiceDetails,
  buildServiceSummary,
  cancelConfigurationChange,
  createIngressState,
  formatServiceSummary,
  startDetection,
  switchConfiguration,
} from "./ingress.js";
import { recordServiceReturnedModel, startRun } from "./records.js";

const configA = {
  endpoint: "https://api.example.com/v1/chat?api_key=secret&region=cn",
  endpointFingerprint: "endpoint-a",
  model: "customer-model-a",
  protocol: "chat-completions",
  authMode: "bearer",
  clientVersion: "0.1.0",
  environment: { os: "test", region: "local" },
};

const configB = {
  ...configA,
  model: "customer-model-b",
  endpointFingerprint: "endpoint-b",
};

describe("service ingress view", () => {
  it("shows a new configuration as not tested without invented identity fields", () => {
    const summary = buildServiceSummary(configA, []);
    const details = buildServiceDetails(configA, []);

    expect(summary).toEqual({
      configuredModel: "customer-model-a",
      redactedEndpoint:
        "https://api.example.com/v1/chat?api_key=%5BREDACTED%5D&region=cn",
      detectionStatus: "not_tested",
      latestDetection: { state: "not_tested" },
    });
    expect(details.serviceReturnedModel).toBeUndefined();
    expect(details.configuredModel.source).toBe("customer_config");
    expect(details.testEnvironment.source).toBe("test_environment");
  });

  it("keeps configured and service-returned model identities separate", () => {
    let state = createIngressState(configA);
    const started = startDetection(state, {
      id: "run-a",
      now: "2026-09-11T01:00:00.000Z",
      selectedModules: ["capability"],
    });
    state = started.state;
    const record = state.records[0]!;
    expect(record.target.endpointFingerprint).not.toContain("secret");
    recordServiceReturnedModel(record, {
      modelId: "provider-model-a",
      observedAt: "2026-09-11T01:01:00.000Z",
    });

    const details = buildServiceDetails(configA, [record]);
    expect(details.configuredModel).toEqual({
      value: "customer-model-a",
      source: "customer_config",
    });
    expect(details.serviceReturnedModel).toEqual({
      value: "provider-model-a",
      source: "service_response",
      observedAt: "2026-09-11T01:01:00.000Z",
    });
  });

  it("keeps A history owned by A when switching to B and supports cancellation", () => {
    let state = createIngressState(configA);
    state = startDetection(state, {
      id: "run-a",
      now: "2026-09-11T02:00:00.000Z",
      selectedModules: ["capability"],
    }).state;
    const recordA = state.records[0]!;
    startRun(recordA, "2026-09-11T02:01:00.000Z");

    const switched = switchConfiguration(state, configB);
    expect(
      buildServiceSummary(switched.currentConfig, switched.records),
    ).toMatchObject({
      configuredModel: "customer-model-b",
      detectionStatus: "not_tested",
      latestDetection: { state: "not_tested" },
    });
    expect(switched.records).toHaveLength(1);
    expect(switched.records[0]?.target.model).toBe("customer-model-a");

    const restored = cancelConfigurationChange(switched, configA);
    expect(restored.currentConfig.model).toBe("customer-model-a");
    expect(
      buildServiceSummary(restored.currentConfig, restored.records)
        .detectionStatus,
    ).toBe("running");
  });

  it("does not expose credentials and keeps long model names on one logical line", () => {
    const longConfig = {
      ...configA,
      model:
        "provider-model-with-a-deliberately-long-name-for-narrow-terminals",
      endpoint:
        "https://user:password@example.com/v1/chat?access_token=secret&x=1",
    };
    const summary = buildServiceSummary(longConfig, []);
    const lines = formatServiceSummary(summary);

    expect(summary.redactedEndpoint).toBe(
      "https://example.com/v1/chat?access_token=%5BREDACTED%5D&x=1",
    );
    expect(summary.redactedEndpoint).not.toContain("password");
    expect(lines).toHaveLength(4);
    expect(lines[0]).toContain(longConfig.model);
    expect(lines.every((line) => !line.includes("secret"))).toBe(true);
  });

  it("uses the same serialized summary for CLI and GUI consumers", () => {
    const state = createIngressState(configA);
    const summary = buildServiceSummary(state.currentConfig, state.records);
    const serialized = JSON.stringify(summary);

    expect(JSON.parse(serialized)).toEqual(summary);
    expect(formatServiceSummary(summary)).toEqual([
      "模型：customer-model-a",
      "地址：https://api.example.com/v1/chat?api_key=%5BREDACTED%5D&region=cn",
      "检测：not_tested",
      "最近一次：not_tested",
    ]);
  });
});
