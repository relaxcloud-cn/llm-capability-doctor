import { createHash } from "node:crypto";

export const recordSchemaVersion = "detection-record/v1" as const;

export const moduleIds = [
  "ingress",
  "specification",
  "capability",
  "performance",
  "agent",
  "baseline",
] as const;

export type ModuleId = (typeof moduleIds)[number];

export type LifecycleState =
  "planned" | "running" | "stopping" | "stopped" | "completed";

export type ModuleSelectionState =
  "selected" | "not_selected" | "not_applicable" | "unverified";

export type ModuleResultState =
  | "pass"
  | "fail"
  | "unsupported"
  | "inconclusive"
  | "invalid_execution"
  | "not_applicable"
  | "not_selected"
  | "unverified";

export type AttemptKind = "initial" | "recheck" | "retry";

export type EventKind =
  "request" | "response" | "tool_failure" | "permission" | "system";

export type OverallConclusion =
  "usable" | "limited" | "blocked" | "inconclusive";

export interface ServiceSnapshot {
  endpointFingerprint: string;
  model: string;
  protocol: string;
  authMode: string;
  clientVersion: string;
  environment: Record<string, string>;
  fingerprint: string;
}

export interface DetectionConditions {
  rules: Record<string, string>;
  testVersions: Record<string, string>;
  settings: Record<string, unknown>;
}

export interface ModulePlanEntry {
  moduleId: ModuleId;
  state: ModuleSelectionState;
  reason?: string;
}

export interface AttemptRecord {
  id: string;
  moduleId: ModuleId;
  kind: AttemptKind;
  startedAt: string;
  endedAt?: string;
  supersedesAttemptId?: string;
  evidenceRefs: string[];
}

export interface DetectionEvent {
  id: string;
  kind: EventKind;
  occurredAt: string;
  summary: string;
  incidentId?: string;
  attemptId?: string;
  evidenceRefs: string[];
}

export interface EvidenceRecord {
  id: string;
  kind: string;
  capturedAt: string;
  payload: unknown;
  digest: string;
  redacted: boolean;
}

export interface ModuleResult {
  moduleId: ModuleId;
  state: ModuleResultState;
  reason?: string;
  attemptRefs: string[];
  evidenceRefs: string[];
  incidentRefs: string[];
}

export interface DetectionRecord {
  schemaVersion: typeof recordSchemaVersion;
  id: string;
  createdAt: string;
  updatedAt: string;
  lifecycle: LifecycleState;
  target: ServiceSnapshot;
  conditions: DetectionConditions;
  plan: ModulePlanEntry[];
  attempts: AttemptRecord[];
  events: DetectionEvent[];
  evidence: EvidenceRecord[];
  moduleResults: ModuleResult[];
  overallConclusion?: {
    state: OverallConclusion;
    evidenceRefs: string[];
  };
}

export interface CreateRunInput {
  id?: string;
  now?: string;
  target: Omit<ServiceSnapshot, "fingerprint">;
  conditions?: Partial<DetectionConditions>;
  selectedModules?: ModuleId[];
}

export interface EvidenceInput {
  id?: string;
  kind: string;
  capturedAt?: string;
  payload: unknown;
}

export interface EventInput {
  id?: string;
  kind: EventKind;
  occurredAt?: string;
  summary: string;
  incidentId?: string;
  attemptId?: string;
  evidenceRefs?: string[];
}

export interface RecordStore {
  put(record: DetectionRecord): void;
  get(id: string): DetectionRecord | undefined;
  list(): DetectionRecord[];
}

const sensitiveKeyPattern =
  /^(authorization|api[-_]?key|access[-_]?token|refresh[-_]?token|token|secret|password|credential)$/i;
const bearerPattern = /\bBearer\s+[A-Za-z0-9._~+/=-]+\b/gi;
const keyPattern = /\b(?:sk|rk)-[A-Za-z0-9_-]{8,}\b/g;

function clone<T>(value: T): T {
  return structuredClone(value);
}

function redactText(value: string): string {
  return value
    .replace(bearerPattern, "Bearer [REDACTED]")
    .replace(keyPattern, "[REDACTED]");
}

function redactValue(
  value: unknown,
  key?: string,
): { value: unknown; redacted: boolean } {
  if (key && sensitiveKeyPattern.test(key)) {
    return { value: "[REDACTED]", redacted: true };
  }

  if (typeof value === "string") {
    const redacted = redactText(value);
    return { value: redacted, redacted: redacted !== value };
  }

  if (Array.isArray(value)) {
    let redacted = false;
    const result = value.map((item) => {
      const entry = redactValue(item);
      redacted ||= entry.redacted;
      return entry.value;
    });
    return { value: result, redacted };
  }

  if (value && typeof value === "object") {
    let redacted = false;
    const result: Record<string, unknown> = {};
    for (const [entryKey, entryValue] of Object.entries(value)) {
      const entry = redactValue(entryValue, entryKey);
      redacted ||= entry.redacted;
      result[entryKey] = entry.value;
    }
    return { value: result, redacted };
  }

  return { value, redacted: false };
}

function canonicalize(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonicalize);
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, entryValue]) => [key, canonicalize(entryValue)]),
    );
  }
  return value;
}

function digest(value: unknown): string {
  return createHash("sha256")
    .update(JSON.stringify(canonicalize(value)))
    .digest("hex");
}

function uniqueId(prefix: string, now: string): string {
  return `${prefix}_${digest({ prefix, now, entropy: Math.random() }).slice(0, 16)}`;
}

function nowIso(now?: string): string {
  return now ?? new Date().toISOString();
}

function targetFingerprint(
  target: Omit<ServiceSnapshot, "fingerprint">,
): string {
  return digest(target);
}

function emptyConditions(
  conditions?: Partial<DetectionConditions>,
): DetectionConditions {
  return {
    rules: clone(conditions?.rules ?? {}),
    testVersions: clone(conditions?.testVersions ?? {}),
    settings: clone(conditions?.settings ?? {}),
  };
}

function assertModule(moduleId: string): asserts moduleId is ModuleId {
  if (!moduleIds.includes(moduleId as ModuleId)) {
    throw new Error(`Unknown module: ${moduleId}`);
  }
}

function assertEvidenceRefs(record: DetectionRecord, refs: string[]): void {
  const known = new Set(record.evidence.map((item) => item.id));
  for (const ref of refs) {
    if (!known.has(ref)) throw new Error(`Unknown evidence reference: ${ref}`);
  }
}

function assertAttemptRefs(record: DetectionRecord, refs: string[]): void {
  const known = new Set(record.attempts.map((item) => item.id));
  for (const ref of refs) {
    if (!known.has(ref)) throw new Error(`Unknown attempt reference: ${ref}`);
  }
}

function assertIncidentRefs(record: DetectionRecord, refs: string[]): void {
  const known = new Set(
    record.events.flatMap((event) =>
      event.incidentId ? [event.incidentId] : [],
    ),
  );
  for (const ref of refs) {
    if (!known.has(ref)) throw new Error(`Unknown incident reference: ${ref}`);
  }
}

function touch(record: DetectionRecord, at: string): void {
  record.updatedAt = at;
}

export function createRun(input: CreateRunInput): DetectionRecord {
  const createdAt = nowIso(input.now);
  const target = {
    ...clone(input.target),
    fingerprint: targetFingerprint(input.target),
  };
  const selected = new Set(input.selectedModules ?? moduleIds);
  for (const moduleId of selected) assertModule(moduleId);

  const plan = moduleIds.map<ModulePlanEntry>((moduleId) =>
    selected.has(moduleId)
      ? { moduleId, state: "selected" }
      : {
          moduleId,
          state: "not_selected",
          reason: "Not selected for this run",
        },
  );

  return {
    schemaVersion: recordSchemaVersion,
    id: input.id ?? uniqueId("run", createdAt),
    createdAt,
    updatedAt: createdAt,
    lifecycle: "planned",
    target,
    conditions: emptyConditions(input.conditions),
    plan,
    attempts: [],
    events: [],
    evidence: [],
    moduleResults: moduleIds.map<ModuleResult>((moduleId) => ({
      moduleId,
      state: selected.has(moduleId) ? "unverified" : "not_selected",
      attemptRefs: [],
      evidenceRefs: [],
      incidentRefs: [],
    })),
  };
}

export function selectModule(
  record: DetectionRecord,
  moduleId: ModuleId,
  state: ModuleSelectionState,
  reason?: string,
  at?: string,
): void {
  if (record.lifecycle !== "planned") {
    throw new Error("Module selection is immutable after a run starts");
  }
  const planEntry = record.plan.find((entry) => entry.moduleId === moduleId);
  const result = record.moduleResults.find(
    (entry) => entry.moduleId === moduleId,
  );
  if (!planEntry || !result) throw new Error(`Unknown module: ${moduleId}`);
  planEntry.state = state;
  planEntry.reason = reason;
  result.state = state === "selected" ? "unverified" : state;
  result.reason = reason;
  touch(record, nowIso(at));
}

export function startRun(record: DetectionRecord, at?: string): void {
  if (record.lifecycle !== "planned") {
    throw new Error(`Cannot start a ${record.lifecycle} run`);
  }
  record.lifecycle = "running";
  touch(record, nowIso(at));
}

export function addEvidence(
  record: DetectionRecord,
  input: EvidenceInput,
): EvidenceRecord {
  const capturedAt = nowIso(input.capturedAt);
  const redacted = redactValue(input.payload);
  const evidence: EvidenceRecord = {
    id: input.id ?? uniqueId("evidence", capturedAt),
    kind: input.kind,
    capturedAt,
    payload: redacted.value,
    digest: digest(redacted.value),
    redacted: redacted.redacted,
  };
  if (record.evidence.some((item) => item.id === evidence.id)) {
    throw new Error(`Evidence already exists: ${evidence.id}`);
  }
  record.evidence.push(evidence);
  touch(record, capturedAt);
  return evidence;
}

export function addEvent(
  record: DetectionRecord,
  input: EventInput,
): DetectionEvent {
  if (record.lifecycle !== "running" && record.lifecycle !== "stopping") {
    throw new Error("Events can only be recorded while a run is active");
  }
  const occurredAt = nowIso(input.occurredAt);
  const evidenceRefs = input.evidenceRefs ?? [];
  assertEvidenceRefs(record, evidenceRefs);
  const event: DetectionEvent = {
    id: input.id ?? uniqueId("event", occurredAt),
    kind: input.kind,
    occurredAt,
    summary: redactText(input.summary),
    ...(input.incidentId ? { incidentId: input.incidentId } : {}),
    ...(input.attemptId ? { attemptId: input.attemptId } : {}),
    evidenceRefs: [...evidenceRefs],
  };
  if (record.events.some((item) => item.id === event.id)) {
    throw new Error(`Event already exists: ${event.id}`);
  }
  record.events.push(event);
  touch(record, occurredAt);
  return event;
}

export function addAttempt(
  record: DetectionRecord,
  input: Omit<AttemptRecord, "id" | "evidenceRefs"> & {
    id?: string;
    evidenceRefs?: string[];
  },
): AttemptRecord {
  if (record.lifecycle !== "running" && record.lifecycle !== "stopping") {
    throw new Error("Attempts can only be recorded while a run is active");
  }
  assertEvidenceRefs(record, input.evidenceRefs ?? []);
  const attempt: AttemptRecord = {
    ...input,
    id: input.id ?? uniqueId("attempt", input.startedAt),
    evidenceRefs: [...(input.evidenceRefs ?? [])],
  };
  assertModule(attempt.moduleId);
  if (record.attempts.some((item) => item.id === attempt.id)) {
    throw new Error(`Attempt already exists: ${attempt.id}`);
  }
  record.attempts.push(attempt);
  touch(record, attempt.endedAt ?? attempt.startedAt);
  return attempt;
}

export function setModuleResult(
  record: DetectionRecord,
  resultInput: ModuleResult,
  at?: string,
): void {
  if (record.lifecycle !== "running" && record.lifecycle !== "stopping") {
    throw new Error(
      "Module results can only be recorded while a run is active",
    );
  }
  assertEvidenceRefs(record, resultInput.evidenceRefs);
  assertAttemptRefs(record, resultInput.attemptRefs);
  assertIncidentRefs(record, resultInput.incidentRefs);
  const planEntry = record.plan.find(
    (entry) => entry.moduleId === resultInput.moduleId,
  );
  const result = record.moduleResults.find(
    (entry) => entry.moduleId === resultInput.moduleId,
  );
  if (!planEntry || !result)
    throw new Error(`Unknown module: ${resultInput.moduleId}`);
  if (planEntry.state === "not_selected") {
    throw new Error(`Module is not selected: ${resultInput.moduleId}`);
  }
  Object.assign(result, clone(resultInput));
  touch(record, nowIso(at));
}

export function setOverallConclusion(
  record: DetectionRecord,
  conclusion: OverallConclusion,
  evidenceRefs: string[],
  at?: string,
): void {
  if (record.lifecycle !== "completed" && record.lifecycle !== "stopped") {
    throw new Error("Overall conclusion can only be recorded after execution");
  }
  assertEvidenceRefs(record, evidenceRefs);
  record.overallConclusion = {
    state: conclusion,
    evidenceRefs: [...evidenceRefs],
  };
  touch(record, nowIso(at));
}

export function stopRun(
  record: DetectionRecord,
  reason: string,
  at?: string,
): void {
  if (record.lifecycle !== "running" && record.lifecycle !== "stopping") {
    throw new Error(`Cannot stop a ${record.lifecycle} run`);
  }
  const stoppedAt = nowIso(at);
  record.lifecycle = "stopping";
  addEvent(record, {
    kind: "system",
    occurredAt: stoppedAt,
    summary: `Run stopped: ${reason}`,
  });
  record.lifecycle = "stopped";
  touch(record, stoppedAt);
}

export function completeRun(record: DetectionRecord, at?: string): void {
  if (record.lifecycle !== "running") {
    throw new Error(`Cannot complete a ${record.lifecycle} run`);
  }
  const selected = record.plan
    .filter((entry) => entry.state === "selected")
    .map((entry) => entry.moduleId);
  const terminal = new Set<ModuleResultState>([
    "pass",
    "fail",
    "unsupported",
    "inconclusive",
    "invalid_execution",
    "not_applicable",
  ]);
  const incomplete = selected.filter((moduleId) => {
    const result = record.moduleResults.find(
      (entry) => entry.moduleId === moduleId,
    );
    return !result || !terminal.has(result.state);
  });
  if (incomplete.length > 0) {
    throw new Error(
      `Cannot complete with unverified modules: ${incomplete.join(", ")}`,
    );
  }
  record.lifecycle = "completed";
  touch(record, nowIso(at));
}

export function uniqueIncidentIds(record: DetectionRecord): string[] {
  return [
    ...new Set(
      record.events.flatMap((event) =>
        event.incidentId ? [event.incidentId] : [],
      ),
    ),
  ];
}

export function serializeRecord(record: DetectionRecord): string {
  return JSON.stringify(record, null, 2);
}

export function deserializeRecord(serialized: string): DetectionRecord {
  const parsed = JSON.parse(serialized) as DetectionRecord;
  if (parsed.schemaVersion !== recordSchemaVersion) {
    throw new Error(`Unsupported record schema: ${parsed.schemaVersion}`);
  }
  if (
    !parsed.id ||
    !parsed.target?.fingerprint ||
    !Array.isArray(parsed.plan)
  ) {
    throw new Error("Invalid detection record");
  }
  return parsed;
}

export class MemoryRecordStore implements RecordStore {
  private readonly records = new Map<string, DetectionRecord>();

  put(record: DetectionRecord): void {
    const existing = this.records.get(record.id);
    if (existing && existing.target.fingerprint !== record.target.fingerprint) {
      throw new Error(
        "A record target is immutable; create a new run for a new configuration",
      );
    }
    this.records.set(record.id, deserializeRecord(serializeRecord(record)));
  }

  get(id: string): DetectionRecord | undefined {
    const record = this.records.get(id);
    return record ? deserializeRecord(serializeRecord(record)) : undefined;
  }

  list(): DetectionRecord[] {
    return [...this.records.values()].map((record) =>
      deserializeRecord(serializeRecord(record)),
    );
  }
}
