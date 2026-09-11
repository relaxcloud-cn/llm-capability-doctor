use crate::records::{
    CreateRunInput, DetectionRecord, LifecycleState, ServiceSnapshotInput, create_run,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectionConfig {
    pub endpoint: String,
    pub endpoint_fingerprint: String,
    pub model: String,
    pub protocol: String,
    pub auth_mode: String,
    pub client_version: String,
    pub environment: BTreeMap<String, String>,
}

impl ConnectionConfig {
    pub fn new(endpoint: String, model: String) -> Self {
        Self {
            endpoint_fingerprint: redact_endpoint(&endpoint),
            endpoint,
            model,
            protocol: "chat-completions".into(),
            auth_mode: "bearer".into(),
            client_version: env!("CARGO_PKG_VERSION").into(),
            environment: BTreeMap::new(),
        }
    }

    fn target(&self) -> ServiceSnapshotInput {
        ServiceSnapshotInput {
            endpoint_fingerprint: redact_endpoint(if self.endpoint_fingerprint.is_empty() {
                &self.endpoint
            } else {
                &self.endpoint_fingerprint
            }),
            model: self.model.clone(),
            protocol: self.protocol.clone(),
            auth_mode: self.auth_mode.clone(),
            client_version: self.client_version.clone(),
            environment: self.environment.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DetectionStatus {
    NotTested,
    Planned,
    Running,
    Stopping,
    Stopped,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServiceSummary {
    pub configured_model: String,
    pub redacted_endpoint: String,
    pub detection_status: DetectionStatus,
    pub latest_detection: LatestDetection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LatestDetection {
    pub state: DetectionStatus,
    pub record_id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServiceDetails {
    pub summary: ServiceSummary,
    pub configured_model: IdentityField,
    pub service_returned_model: Option<ServiceReturnedField>,
    pub test_environment: EnvironmentField,
    pub protocol: String,
    pub auth_mode: String,
    pub client_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IdentityField {
    pub value: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceReturnedField {
    pub value: String,
    pub source: String,
    pub observed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentField {
    pub value: BTreeMap<String, String>,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IngressState {
    pub current_config: ConnectionConfig,
    pub records: Vec<DetectionRecord>,
}

pub fn create_ingress_state(
    config: ConnectionConfig,
    records: Vec<DetectionRecord>,
) -> IngressState {
    IngressState {
        current_config: config,
        records,
    }
}

pub fn build_service_summary(
    config: &ConnectionConfig,
    records: &[DetectionRecord],
) -> ServiceSummary {
    let record = latest_record(config, records);
    let status = record.map_or(DetectionStatus::NotTested, |record| {
        status_for(record.lifecycle)
    });
    ServiceSummary {
        configured_model: config.model.clone(),
        redacted_endpoint: redact_endpoint(&config.endpoint),
        detection_status: status.clone(),
        latest_detection: LatestDetection {
            state: status,
            record_id: record.map(|item| item.id.clone()),
            created_at: record.map(|item| item.created_at.clone()),
            updated_at: record.map(|item| item.updated_at.clone()),
        },
    }
}

pub fn build_service_details(
    config: &ConnectionConfig,
    records: &[DetectionRecord],
) -> ServiceDetails {
    let record = latest_record(config, records);
    ServiceDetails {
        summary: build_service_summary(config, records),
        configured_model: IdentityField {
            value: config.model.clone(),
            source: "customer_config".into(),
        },
        service_returned_model: record
            .and_then(|item| item.service_returned_model.as_ref())
            .map(|model| ServiceReturnedField {
                value: model.model_id.clone(),
                source: model.source.clone(),
                observed_at: model.observed_at.clone(),
            }),
        test_environment: EnvironmentField {
            value: config.environment.clone(),
            source: "test_environment".into(),
        },
        protocol: config.protocol.clone(),
        auth_mode: config.auth_mode.clone(),
        client_version: config.client_version.clone(),
    }
}

pub fn switch_configuration(state: &IngressState, config: ConnectionConfig) -> IngressState {
    IngressState {
        current_config: config,
        records: state.records.clone(),
    }
}

pub fn start_detection(
    state: &IngressState,
    id: String,
    now: String,
    selected_modules: Option<Vec<String>>,
) -> (IngressState, DetectionRecord) {
    let record = create_run(CreateRunInput {
        id,
        now,
        target: state.current_config.target(),
        selected_modules,
    });
    let mut records = state.records.clone();
    records.push(record.clone());
    (
        IngressState {
            current_config: state.current_config.clone(),
            records,
        },
        record,
    )
}

pub fn format_service_summary(summary: &ServiceSummary) -> [String; 4] {
    [
        format!("模型：{}", summary.configured_model),
        format!("地址：{}", summary.redacted_endpoint),
        format!("检测：{}", status_label(&summary.detection_status)),
        format!(
            "最近一次：{}",
            status_label(&summary.latest_detection.state)
        ),
    ]
}

fn status_label(status: &DetectionStatus) -> &'static str {
    match status {
        DetectionStatus::NotTested => "not_tested",
        DetectionStatus::Planned => "planned",
        DetectionStatus::Running => "running",
        DetectionStatus::Stopping => "stopping",
        DetectionStatus::Stopped => "stopped",
        DetectionStatus::Completed => "completed",
    }
}

pub fn redact_endpoint(endpoint: &str) -> String {
    let Ok(mut url) = Url::parse(endpoint) else {
        return redact_fallback(endpoint);
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    let query = url
        .query_pairs()
        .map(|(key, value)| {
            if is_sensitive_query_key(&key) {
                (key.into_owned(), "[REDACTED]".into())
            } else {
                (key.into_owned(), value.into_owned())
            }
        })
        .collect::<Vec<_>>();
    let query_string = query
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    url.set_query(if query.is_empty() {
        None
    } else {
        Some(&query_string)
    });
    url.to_string().trim_end_matches('/').into()
}

fn latest_record<'a>(
    config: &ConnectionConfig,
    records: &'a [DetectionRecord],
) -> Option<&'a DetectionRecord> {
    let target = create_run(CreateRunInput {
        id: "fingerprint-probe".into(),
        now: "2026-01-01T00:00:00Z".into(),
        target: config.target(),
        selected_modules: Some(vec![]),
    })
    .target
    .fingerprint;
    records
        .iter()
        .filter(|record| record.target.fingerprint == target)
        .max_by(|left, right| left.created_at.cmp(&right.created_at))
}

fn status_for(lifecycle: LifecycleState) -> DetectionStatus {
    match lifecycle {
        LifecycleState::Planned => DetectionStatus::Planned,
        LifecycleState::Running => DetectionStatus::Running,
        LifecycleState::Stopping => DetectionStatus::Stopping,
        LifecycleState::Stopped => DetectionStatus::Stopped,
        LifecycleState::Completed => DetectionStatus::Completed,
    }
}

fn is_sensitive_query_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "api_key"
            | "api-key"
            | "access_token"
            | "access-token"
            | "refresh_token"
            | "refresh-token"
            | "token"
            | "secret"
            | "password"
            | "credential"
            | "sig"
            | "signature"
    )
}

fn redact_fallback(endpoint: &str) -> String {
    let mut result = endpoint.to_owned();
    for key in [
        "api_key",
        "api-key",
        "access_token",
        "access-token",
        "refresh_token",
        "token",
        "secret",
        "password",
        "credential",
        "sig",
        "signature",
    ] {
        let prefix = format!("{key}=");
        while let Some(start) = result.to_ascii_lowercase().find(&prefix) {
            let value_start = start + prefix.len();
            let value_end = result[value_start..]
                .find(['&', '#'])
                .map_or(result.len(), |offset| value_start + offset);
            result.replace_range(value_start..value_end, "[REDACTED]");
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::{record_service_returned_model, start_run};

    #[test]
    fn new_config_is_not_tested_and_endpoint_is_redacted() {
        let config = ConnectionConfig::new(
            "https://user:password@example.com/v1/chat?api_key=secret&region=cn".into(),
            "model-a".into(),
        );
        let summary = build_service_summary(&config, &[]);
        assert_eq!(summary.detection_status, DetectionStatus::NotTested);
        assert_eq!(
            summary.redacted_endpoint,
            "https://example.com/v1/chat?api_key=[REDACTED]&region=cn"
        );
    }

    #[test]
    fn switches_configuration_without_reassigning_history() {
        let config_a = ConnectionConfig::new("https://a.example.com".into(), "model-a".into());
        let config_b = ConnectionConfig::new("https://b.example.com".into(), "model-b".into());
        let state = create_ingress_state(config_a.clone(), vec![]);
        let (state, mut record) = start_detection(
            &state,
            "run-a".into(),
            "2026-09-11T00:00:00Z".into(),
            Some(vec!["capability".into()]),
        );
        start_run(&mut record, "2026-09-11T00:01:00Z").unwrap();
        let state = IngressState {
            current_config: state.current_config,
            records: vec![record],
        };
        let switched = switch_configuration(&state, config_b);
        assert_eq!(
            build_service_summary(&switched.current_config, &switched.records).detection_status,
            DetectionStatus::NotTested
        );
        assert_eq!(switched.records[0].target.model, "model-a");
        assert_eq!(
            build_service_summary(&config_a, &switched.records).detection_status,
            DetectionStatus::Running
        );
    }

    #[test]
    fn keeps_identity_sources_separate() {
        let config = ConnectionConfig::new("https://a.example.com".into(), "customer-model".into());
        let state = create_ingress_state(config.clone(), vec![]);
        let (_, mut record) =
            start_detection(&state, "run-a".into(), "2026-09-11T00:00:00Z".into(), None);
        record_service_returned_model(&mut record, "provider-model", "2026-09-11T00:01:00Z")
            .unwrap();
        let details = build_service_details(&config, &[record]);
        assert_eq!(details.configured_model.source, "customer_config");
        assert_eq!(
            details.service_returned_model.unwrap().source,
            "service_response"
        );
    }
}
