use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const RECORD_SCHEMA_VERSION: &str = "detection-record/v1";
pub const MODULE_IDS: [&str; 6] = [
    "ingress",
    "specification",
    "capability",
    "performance",
    "agent",
    "baseline",
];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    Planned,
    Running,
    Stopping,
    Stopped,
    Completed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModuleSelectionState {
    Selected,
    NotSelected,
    NotApplicable,
    Unverified,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModuleResultState {
    Pass,
    Fail,
    Unsupported,
    Inconclusive,
    InvalidExecution,
    NotApplicable,
    NotSelected,
    Unverified,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttemptKind {
    Initial,
    Recheck,
    Retry,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Request,
    Response,
    ToolFailure,
    Permission,
    System,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OverallConclusion {
    Usable,
    Limited,
    Blocked,
    Inconclusive,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServiceSnapshot {
    pub endpoint_fingerprint: String,
    pub model: String,
    pub protocol: String,
    pub auth_mode: String,
    pub client_version: String,
    pub environment: BTreeMap<String, String>,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DetectionConditions {
    pub rules: BTreeMap<String, String>,
    pub test_versions: BTreeMap<String, String>,
    pub settings: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModulePlanEntry {
    pub module_id: String,
    pub state: ModuleSelectionState,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AttemptRecord {
    pub id: String,
    pub module_id: String,
    pub kind: AttemptKind,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub supersedes_attempt_id: Option<String>,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DetectionEvent {
    pub id: String,
    pub kind: EventKind,
    pub occurred_at: String,
    pub summary: String,
    pub incident_id: Option<String>,
    pub attempt_id: Option<String>,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct EventInput {
    pub id: String,
    pub kind: EventKind,
    pub occurred_at: String,
    pub summary: String,
    pub incident_id: Option<String>,
    pub attempt_id: Option<String>,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceRecord {
    pub id: String,
    pub kind: String,
    pub captured_at: String,
    pub payload: Value,
    pub digest: String,
    pub redacted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModuleResult {
    pub module_id: String,
    pub state: ModuleResultState,
    pub reason: Option<String>,
    pub attempt_refs: Vec<String>,
    pub evidence_refs: Vec<String>,
    pub incident_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServiceReturnedModel {
    pub model_id: String,
    pub source: String,
    pub observed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DetectionRecord {
    pub schema_version: String,
    pub id: String,
    pub created_at: String,
    pub updated_at: String,
    pub lifecycle: LifecycleState,
    pub target: ServiceSnapshot,
    pub conditions: DetectionConditions,
    pub plan: Vec<ModulePlanEntry>,
    pub attempts: Vec<AttemptRecord>,
    pub events: Vec<DetectionEvent>,
    pub evidence: Vec<EvidenceRecord>,
    pub module_results: Vec<ModuleResult>,
    pub service_returned_model: Option<ServiceReturnedModel>,
    pub overall_conclusion: Option<OverallConclusionRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OverallConclusionRecord {
    pub state: OverallConclusion,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CreateRunInput {
    pub id: String,
    pub now: String,
    pub target: ServiceSnapshotInput,
    pub selected_modules: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceSnapshotInput {
    pub endpoint_fingerprint: String,
    pub model: String,
    pub protocol: String,
    pub auth_mode: String,
    pub client_version: String,
    pub environment: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RedactionResult {
    pub value: Value,
    pub redacted: bool,
}

pub trait RecordStore {
    fn put(&mut self, record: DetectionRecord) -> Result<(), String>;
    fn get(&self, id: &str) -> Option<DetectionRecord>;
    fn list(&self) -> Vec<DetectionRecord>;
}

#[derive(Debug, Default)]
pub struct MemoryRecordStore {
    records: BTreeMap<String, DetectionRecord>,
}

impl RecordStore for MemoryRecordStore {
    fn put(&mut self, record: DetectionRecord) -> Result<(), String> {
        if let Some(existing) = self.records.get(&record.id)
            && existing.target.fingerprint != record.target.fingerprint
        {
            return Err(
                "A record target is immutable; create a new run for a new configuration".into(),
            );
        }
        self.records.insert(record.id.clone(), record);
        Ok(())
    }

    fn get(&self, id: &str) -> Option<DetectionRecord> {
        self.records.get(id).cloned()
    }

    fn list(&self) -> Vec<DetectionRecord> {
        self.records.values().cloned().collect()
    }
}

pub fn create_run(input: CreateRunInput) -> DetectionRecord {
    let selected: BTreeSet<String> = input
        .selected_modules
        .unwrap_or_else(|| MODULE_IDS.iter().map(|id| (*id).to_owned()).collect())
        .into_iter()
        .collect();
    assert_modules(&selected);
    let fingerprint = digest_json(&input.target);
    let target = ServiceSnapshot {
        endpoint_fingerprint: input.target.endpoint_fingerprint,
        model: input.target.model,
        protocol: input.target.protocol,
        auth_mode: input.target.auth_mode,
        client_version: input.target.client_version,
        environment: input.target.environment,
        fingerprint,
    };
    DetectionRecord {
        schema_version: RECORD_SCHEMA_VERSION.into(),
        id: input.id,
        created_at: input.now.clone(),
        updated_at: input.now,
        lifecycle: LifecycleState::Planned,
        target,
        conditions: DetectionConditions {
            rules: BTreeMap::new(),
            test_versions: BTreeMap::new(),
            settings: Map::new(),
        },
        plan: MODULE_IDS
            .iter()
            .map(|module_id| ModulePlanEntry {
                module_id: (*module_id).into(),
                state: if selected.contains(*module_id) {
                    ModuleSelectionState::Selected
                } else {
                    ModuleSelectionState::NotSelected
                },
                reason: (!selected.contains(*module_id))
                    .then(|| "Not selected for this run".into()),
            })
            .collect(),
        attempts: Vec::new(),
        events: Vec::new(),
        evidence: Vec::new(),
        module_results: MODULE_IDS
            .iter()
            .map(|module_id| ModuleResult {
                module_id: (*module_id).into(),
                state: if selected.contains(*module_id) {
                    ModuleResultState::Unverified
                } else {
                    ModuleResultState::NotSelected
                },
                reason: None,
                attempt_refs: Vec::new(),
                evidence_refs: Vec::new(),
                incident_refs: Vec::new(),
            })
            .collect(),
        service_returned_model: None,
        overall_conclusion: None,
    }
}

pub fn start_run(record: &mut DetectionRecord, at: &str) -> Result<(), String> {
    if record.lifecycle != LifecycleState::Planned {
        return Err("A run can only start from planned".into());
    }
    record.lifecycle = LifecycleState::Running;
    record.updated_at = at.into();
    Ok(())
}

pub fn record_service_returned_model(
    record: &mut DetectionRecord,
    model_id: &str,
    observed_at: &str,
) -> Result<(), String> {
    if model_id.trim().is_empty() {
        return Err("Service-returned model ID cannot be empty".into());
    }
    if matches!(
        record.lifecycle,
        LifecycleState::Stopped | LifecycleState::Completed
    ) {
        return Err("Service identity cannot be recorded after a run ends".into());
    }
    record.service_returned_model = Some(ServiceReturnedModel {
        model_id: redact_text(model_id),
        source: "service_response".into(),
        observed_at: observed_at.into(),
    });
    record.updated_at = observed_at.into();
    Ok(())
}

pub fn add_evidence(
    record: &mut DetectionRecord,
    id: &str,
    kind: &str,
    captured_at: &str,
    payload: Value,
) -> EvidenceRecord {
    if record.evidence.iter().any(|item| item.id == id) {
        panic!("Evidence already exists: {id}");
    }
    let result = redact_value(payload, None);
    let evidence = EvidenceRecord {
        id: id.into(),
        kind: kind.into(),
        captured_at: captured_at.into(),
        digest: digest_json(&result.value),
        payload: result.value,
        redacted: result.redacted,
    };
    record.evidence.push(evidence.clone());
    record.updated_at = captured_at.into();
    evidence
}

pub fn add_event(
    record: &mut DetectionRecord,
    input: EventInput,
) -> Result<DetectionEvent, String> {
    if !matches!(
        record.lifecycle,
        LifecycleState::Running | LifecycleState::Stopping
    ) {
        return Err("Events can only be recorded while a run is active".into());
    }
    assert_evidence_refs(record, &input.evidence_refs)?;
    if record.events.iter().any(|event| event.id == input.id) {
        return Err(format!("Event already exists: {}", input.id));
    }
    let event = DetectionEvent {
        id: input.id,
        kind: input.kind,
        occurred_at: input.occurred_at.clone(),
        summary: redact_text(&input.summary),
        incident_id: input.incident_id,
        attempt_id: input.attempt_id,
        evidence_refs: input.evidence_refs,
    };
    record.events.push(event.clone());
    record.updated_at = input.occurred_at;
    Ok(event)
}

pub fn add_attempt(record: &mut DetectionRecord, attempt: AttemptRecord) -> Result<(), String> {
    if !matches!(
        record.lifecycle,
        LifecycleState::Running | LifecycleState::Stopping
    ) {
        return Err("Attempts can only be recorded while a run is active".into());
    }
    assert_module(&attempt.module_id)?;
    assert_evidence_refs(record, &attempt.evidence_refs)?;
    if record.attempts.iter().any(|item| item.id == attempt.id) {
        return Err(format!("Attempt already exists: {}", attempt.id));
    }
    record.updated_at = attempt
        .ended_at
        .clone()
        .unwrap_or_else(|| attempt.started_at.clone());
    record.attempts.push(attempt);
    Ok(())
}

pub fn set_module_result(
    record: &mut DetectionRecord,
    result: ModuleResult,
    at: &str,
) -> Result<(), String> {
    if !matches!(
        record.lifecycle,
        LifecycleState::Running | LifecycleState::Stopping
    ) {
        return Err("Module results can only be recorded while a run is active".into());
    }
    assert_module(&result.module_id)?;
    assert_evidence_refs(record, &result.evidence_refs)?;
    assert_attempt_refs(record, &result.attempt_refs)?;
    assert_incident_refs(record, &result.incident_refs)?;
    let plan = record
        .plan
        .iter()
        .find(|entry| entry.module_id == result.module_id)
        .ok_or_else(|| format!("Unknown module: {}", result.module_id))?;
    if plan.state == ModuleSelectionState::NotSelected {
        return Err(format!("Module is not selected: {}", result.module_id));
    }
    let target = record
        .module_results
        .iter_mut()
        .find(|entry| entry.module_id == result.module_id)
        .ok_or_else(|| format!("Unknown module: {}", result.module_id))?;
    *target = result;
    record.updated_at = at.into();
    Ok(())
}

pub fn stop_run(record: &mut DetectionRecord, reason: &str, at: &str) -> Result<(), String> {
    if !matches!(
        record.lifecycle,
        LifecycleState::Running | LifecycleState::Stopping
    ) {
        return Err(format!("Cannot stop a {:?} run", record.lifecycle));
    }
    record.lifecycle = LifecycleState::Stopping;
    add_event(
        record,
        EventInput {
            id: format!("stop-{at}"),
            kind: EventKind::System,
            occurred_at: at.into(),
            summary: format!("Run stopped: {reason}"),
            incident_id: None,
            attempt_id: None,
            evidence_refs: Vec::new(),
        },
    )?;
    record.lifecycle = LifecycleState::Stopped;
    record.updated_at = at.into();
    Ok(())
}

pub fn complete_run(record: &mut DetectionRecord, at: &str) -> Result<(), String> {
    if record.lifecycle != LifecycleState::Running {
        return Err(format!("Cannot complete a {:?} run", record.lifecycle));
    }
    let incomplete = record
        .plan
        .iter()
        .filter(|entry| entry.state == ModuleSelectionState::Selected)
        .filter(|entry| {
            record
                .module_results
                .iter()
                .find(|result| result.module_id == entry.module_id)
                .is_none_or(|result| {
                    matches!(
                        result.state,
                        ModuleResultState::Unverified | ModuleResultState::NotSelected
                    )
                })
        })
        .map(|entry| entry.module_id.clone())
        .collect::<Vec<_>>();
    if !incomplete.is_empty() {
        return Err(format!(
            "Cannot complete with unverified modules: {incomplete:?}"
        ));
    }
    record.lifecycle = LifecycleState::Completed;
    record.updated_at = at.into();
    Ok(())
}

pub fn set_overall_conclusion(
    record: &mut DetectionRecord,
    state: OverallConclusion,
    evidence_refs: Vec<String>,
    at: &str,
) -> Result<(), String> {
    if !matches!(
        record.lifecycle,
        LifecycleState::Stopped | LifecycleState::Completed
    ) {
        return Err("Overall conclusion can only be recorded after execution".into());
    }
    assert_evidence_refs(record, &evidence_refs)?;
    record.overall_conclusion = Some(OverallConclusionRecord {
        state,
        evidence_refs,
    });
    record.updated_at = at.into();
    Ok(())
}

pub fn serialize_record(record: &DetectionRecord) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(record)
}

pub fn deserialize_record(serialized: &str) -> Result<DetectionRecord, String> {
    let record: DetectionRecord =
        serde_json::from_str(serialized).map_err(|error| error.to_string())?;
    if record.schema_version != RECORD_SCHEMA_VERSION || record.id.is_empty() {
        return Err("Invalid detection record".into());
    }
    Ok(record)
}

pub fn unique_incident_ids(record: &DetectionRecord) -> Vec<String> {
    record
        .events
        .iter()
        .filter_map(|event| event.incident_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn assert_modules(selected: &BTreeSet<String>) {
    assert!(
        selected
            .iter()
            .all(|module| MODULE_IDS.contains(&module.as_str())),
        "Unknown module in selection"
    );
}

fn assert_module(module: &str) -> Result<(), String> {
    if MODULE_IDS.contains(&module) {
        Ok(())
    } else {
        Err(format!("Unknown module: {module}"))
    }
}

fn assert_evidence_refs(record: &DetectionRecord, refs: &[String]) -> Result<(), String> {
    let known = record
        .evidence
        .iter()
        .map(|item| item.id.as_str())
        .collect::<BTreeSet<_>>();
    for reference in refs {
        if !known.contains(reference.as_str()) {
            return Err(format!("Unknown evidence reference: {reference}"));
        }
    }
    Ok(())
}

fn assert_attempt_refs(record: &DetectionRecord, refs: &[String]) -> Result<(), String> {
    let known = record
        .attempts
        .iter()
        .map(|item| item.id.as_str())
        .collect::<BTreeSet<_>>();
    for reference in refs {
        if !known.contains(reference.as_str()) {
            return Err(format!("Unknown attempt reference: {reference}"));
        }
    }
    Ok(())
}

fn assert_incident_refs(record: &DetectionRecord, refs: &[String]) -> Result<(), String> {
    let known = record
        .events
        .iter()
        .filter_map(|event| event.incident_id.as_deref())
        .collect::<BTreeSet<_>>();
    for reference in refs {
        if !known.contains(reference.as_str()) {
            return Err(format!("Unknown incident reference: {reference}"));
        }
    }
    Ok(())
}

fn digest_json<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).expect("record values must be serializable");
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn redact_text(value: &str) -> String {
    let mut result = Vec::new();
    let mut redact_next = false;
    for token in value.split_whitespace() {
        if redact_next {
            result.push("[REDACTED]");
            redact_next = false;
        } else if token.eq_ignore_ascii_case("bearer") {
            result.push(token);
            redact_next = true;
        } else if token.starts_with("sk-") || token.starts_with("rk-") {
            result.push("[REDACTED]");
        } else {
            result.push(token);
        }
    }
    result.join(" ")
}

fn redact_value(value: Value, key: Option<&str>) -> RedactionResult {
    if key.is_some_and(is_sensitive_key) {
        return RedactionResult {
            value: Value::String("[REDACTED]".into()),
            redacted: true,
        };
    }
    match value {
        Value::String(text) => {
            let redacted_text = redact_text(&text);
            RedactionResult {
                redacted: redacted_text != text,
                value: Value::String(redacted_text),
            }
        }
        Value::Array(items) => {
            let mut redacted = false;
            let values = items
                .into_iter()
                .map(|item| {
                    let result = redact_value(item, None);
                    redacted |= result.redacted;
                    result.value
                })
                .collect();
            RedactionResult {
                value: Value::Array(values),
                redacted,
            }
        }
        Value::Object(items) => {
            let mut redacted = false;
            let values = items
                .into_iter()
                .map(|(entry_key, entry_value)| {
                    let result = redact_value(entry_value, Some(&entry_key));
                    redacted |= result.redacted;
                    (entry_key, result.value)
                })
                .collect();
            RedactionResult {
                value: Value::Object(values),
                redacted,
            }
        }
        other => RedactionResult {
            value: other,
            redacted: false,
        },
    }
}

fn is_sensitive_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "authorization"
            | "api_key"
            | "api-key"
            | "access_token"
            | "access-token"
            | "refresh_token"
            | "refresh-token"
            | "token"
            | "secret"
            | "password"
            | "credential"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn input(model: &str) -> CreateRunInput {
        CreateRunInput {
            id: "run-a".into(),
            now: "2026-09-11T00:00:00Z".into(),
            target: ServiceSnapshotInput {
                endpoint_fingerprint: "endpoint-a".into(),
                model: model.into(),
                protocol: "chat-completions".into(),
                auth_mode: "bearer".into(),
                client_version: "0.1.0".into(),
                environment: BTreeMap::from([(String::from("os"), String::from("test"))]),
            },
            selected_modules: Some(vec!["capability".into(), "agent".into()]),
        }
    }

    #[test]
    fn keeps_states_and_redacts_credentials_without_losing_metrics() {
        let mut record = create_run(input("model-a"));
        start_run(&mut record, "2026-09-11T00:01:00Z").unwrap();
        let evidence = add_evidence(
            &mut record,
            "ev-1",
            "response",
            "2026-09-11T00:02:00Z",
            json!({
                "headers": { "Authorization": "Bearer abc" },
                "usage": { "prompt_tokens": 12, "completion_tokens": 8 },
                "api_key": "secret"
            }),
        );
        assert_eq!(evidence.payload["headers"]["Authorization"], "[REDACTED]");
        assert_eq!(evidence.payload["usage"]["prompt_tokens"], 12);
        assert_eq!(evidence.payload["api_key"], "[REDACTED]");
        assert!(evidence.redacted);
    }

    #[test]
    fn preserves_configured_and_service_returned_identity() {
        let mut record = create_run(input("customer-model"));
        record_service_returned_model(&mut record, "provider-model", "2026-09-11T00:03:00Z")
            .unwrap();
        assert_eq!(record.target.model, "customer-model");
        assert_eq!(
            record.service_returned_model.unwrap().model_id,
            "provider-model"
        );
    }

    #[test]
    fn round_trips_json_records() {
        let record = create_run(input("model-a"));
        let serialized = serialize_record(&record).unwrap();
        let restored = deserialize_record(&serialized).unwrap();
        assert_eq!(restored, record);
    }

    #[test]
    fn carries_shared_incidents_to_terminal_results_and_conclusion() {
        let mut record = create_run(input("model-a"));
        start_run(&mut record, "2026-09-11T00:01:00Z").unwrap();
        let evidence = add_evidence(
            &mut record,
            "ev-failure",
            "tool-failure",
            "2026-09-11T00:02:00Z",
            json!({ "message": "temporary failure" }),
        );
        let event = add_event(
            &mut record,
            EventInput {
                id: "event-failure".into(),
                kind: EventKind::ToolFailure,
                occurred_at: "2026-09-11T00:02:01Z".into(),
                summary: "Tool request failed".into(),
                incident_id: Some("incident-1".into()),
                attempt_id: None,
                evidence_refs: vec![evidence.id.clone()],
            },
        )
        .unwrap();
        add_attempt(
            &mut record,
            AttemptRecord {
                id: "attempt-1".into(),
                module_id: "capability".into(),
                kind: AttemptKind::Initial,
                started_at: "2026-09-11T00:02:00Z".into(),
                ended_at: Some("2026-09-11T00:02:02Z".into()),
                supersedes_attempt_id: None,
                evidence_refs: vec![evidence.id.clone()],
            },
        )
        .unwrap();
        set_module_result(
            &mut record,
            ModuleResult {
                module_id: "capability".into(),
                state: ModuleResultState::InvalidExecution,
                reason: Some("invalid response".into()),
                attempt_refs: vec!["attempt-1".into()],
                evidence_refs: vec![evidence.id.clone()],
                incident_refs: vec![event.incident_id.clone().unwrap()],
            },
            "2026-09-11T00:02:03Z",
        )
        .unwrap();
        set_module_result(
            &mut record,
            ModuleResult {
                module_id: "agent".into(),
                state: ModuleResultState::Inconclusive,
                reason: None,
                attempt_refs: Vec::new(),
                evidence_refs: vec![evidence.id],
                incident_refs: vec!["incident-1".into()],
            },
            "2026-09-11T00:02:04Z",
        )
        .unwrap();
        complete_run(&mut record, "2026-09-11T00:02:05Z").unwrap();
        set_overall_conclusion(
            &mut record,
            OverallConclusion::Limited,
            vec!["ev-failure".into()],
            "2026-09-11T00:02:06Z",
        )
        .unwrap();

        assert_eq!(unique_incident_ids(&record), vec!["incident-1"]);
        assert_eq!(record.lifecycle, LifecycleState::Completed);
        assert_eq!(
            record.overall_conclusion.unwrap().state,
            OverallConclusion::Limited
        );
    }
}
