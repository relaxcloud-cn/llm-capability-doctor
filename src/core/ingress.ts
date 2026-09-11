import {
  type CreateRunInput,
  createRun,
  type DetectionRecord,
  type RecordStore,
  type ServiceSnapshot,
} from "./records.js";

export type DetectionStatus =
  | "not_tested"
  | "planned"
  | "running"
  | "stopping"
  | "stopped"
  | "completed";

export interface ConnectionConfig extends Omit<ServiceSnapshot, "fingerprint"> {
  endpoint: string;
}

export interface ServiceSummary {
  configuredModel: string;
  redactedEndpoint: string;
  detectionStatus: DetectionStatus;
  latestDetection: {
    state: DetectionStatus;
    recordId?: string;
    createdAt?: string;
    updatedAt?: string;
  };
}

export interface ServiceDetails {
  summary: ServiceSummary;
  configuredModel: {
    value: string;
    source: "customer_config";
  };
  serviceReturnedModel?: {
    value: string;
    source: "service_response";
    observedAt: string;
  };
  testEnvironment: {
    value: Record<string, string>;
    source: "test_environment";
  };
  protocol: string;
  authMode: string;
  clientVersion: string;
}

export interface IngressState {
  currentConfig: ConnectionConfig;
  records: DetectionRecord[];
}

export interface StartDetectionOptions {
  id?: string;
  now?: string;
  conditions?: CreateRunInput["conditions"];
  selectedModules?: CreateRunInput["selectedModules"];
}

const sensitiveQueryKey =
  /^(?:api[-_]?key|access[-_]?token|refresh[-_]?token|token|secret|password|credential|sig|signature)$/i;
const sensitiveUserInfo = /^(?:[^:]+):.+@/;
const sensitiveQuery =
  /([?&](?:api[-_]?key|access[-_]?token|refresh[-_]?token|token|secret|password|credential|sig|signature)=)[^&#]*/gi;
const bearerPattern = /\bBearer\s+[^\s&]+\b/gi;
const keyPattern = /\b(?:sk|rk)-[A-Za-z0-9_-]{8,}\b/g;

function redactEndpoint(endpoint: string): string {
  try {
    const url = new URL(endpoint);
    url.username = "";
    url.password = "";
    for (const key of [...url.searchParams.keys()]) {
      if (sensitiveQueryKey.test(key)) url.searchParams.set(key, "[REDACTED]");
    }
    return url.toString().replace(/\/$/, "");
  } catch {
    return endpoint
      .replace(sensitiveUserInfo, "[REDACTED]@")
      .replace(sensitiveQuery, "$1[REDACTED]")
      .replace(bearerPattern, "Bearer [REDACTED]")
      .replace(keyPattern, "[REDACTED]");
  }
}

function targetFromConfig(
  config: ConnectionConfig,
): Omit<ServiceSnapshot, "fingerprint"> {
  const { endpoint, ...target } = config;
  return {
    ...target,
    endpointFingerprint: redactEndpoint(target.endpointFingerprint || endpoint),
  };
}

function targetFingerprint(config: ConnectionConfig): string {
  return createRun({
    id: "fingerprint-probe",
    now: "2026-01-01T00:00:00.000Z",
    target: targetFromConfig(config),
  }).target.fingerprint;
}

function latestRecord(
  config: ConnectionConfig,
  records: DetectionRecord[],
): DetectionRecord | undefined {
  const fingerprint = targetFingerprint(config);
  return records
    .filter((record) => record.target.fingerprint === fingerprint)
    .sort((left, right) => right.createdAt.localeCompare(left.createdAt))[0];
}

function statusFor(record?: DetectionRecord): DetectionStatus {
  return record?.lifecycle ?? "not_tested";
}

export function createIngressState(
  currentConfig: ConnectionConfig,
  records: DetectionRecord[] = [],
): IngressState {
  return {
    currentConfig: structuredClone(currentConfig),
    records: records.map((record) => structuredClone(record)),
  };
}

export function buildServiceSummary(
  currentConfig: ConnectionConfig,
  records: DetectionRecord[],
): ServiceSummary {
  const record = latestRecord(currentConfig, records);
  const status = statusFor(record);
  return {
    configuredModel: currentConfig.model,
    redactedEndpoint: redactEndpoint(currentConfig.endpoint),
    detectionStatus: status,
    latestDetection: {
      state: status,
      ...(record
        ? {
            recordId: record.id,
            createdAt: record.createdAt,
            updatedAt: record.updatedAt,
          }
        : {}),
    },
  };
}

export function buildServiceDetails(
  currentConfig: ConnectionConfig,
  records: DetectionRecord[],
): ServiceDetails {
  const record = latestRecord(currentConfig, records);
  return {
    summary: buildServiceSummary(currentConfig, records),
    configuredModel: {
      value: currentConfig.model,
      source: "customer_config",
    },
    ...(record?.serviceReturnedModel
      ? {
          serviceReturnedModel: {
            value: record.serviceReturnedModel.modelId,
            source: record.serviceReturnedModel.source,
            observedAt: record.serviceReturnedModel.observedAt,
          },
        }
      : {}),
    testEnvironment: {
      value: structuredClone(currentConfig.environment),
      source: "test_environment",
    },
    protocol: currentConfig.protocol,
    authMode: currentConfig.authMode,
    clientVersion: currentConfig.clientVersion,
  };
}

export function switchConfiguration(
  state: IngressState,
  currentConfig: ConnectionConfig,
): IngressState {
  return createIngressState(currentConfig, state.records);
}

export function cancelConfigurationChange(
  state: IngressState,
  previousConfig: ConnectionConfig,
): IngressState {
  return switchConfiguration(state, previousConfig);
}

export function startDetection(
  state: IngressState,
  options: StartDetectionOptions = {},
): { state: IngressState; record: DetectionRecord } {
  const record = createRun({
    ...options,
    target: targetFromConfig(state.currentConfig),
  });
  const nextState = createIngressState(state.currentConfig, [
    ...state.records,
    record,
  ]);
  return { state: nextState, record: structuredClone(record) };
}

export function persistIngressState(
  state: IngressState,
  store: RecordStore,
): void {
  for (const record of state.records) store.put(record);
}

export function formatServiceSummary(summary: ServiceSummary): string[] {
  return [
    `模型：${summary.configuredModel}`,
    `地址：${summary.redactedEndpoint}`,
    `检测：${summary.detectionStatus}`,
    `最近一次：${summary.latestDetection.state}`,
  ];
}
