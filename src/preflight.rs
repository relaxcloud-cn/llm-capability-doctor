use crate::{
    config::{ModelConfig, Target},
    transport::{REQUEST_BUDGET, RequestFailure, RequestOutcome, Transport, messages_body},
};
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{path::Path, time::Instant};
use tokio_util::sync::CancellationToken;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnectionReceipt {
    pub target: Target,
    pub elapsed_ms: u64,
    pub returned_model: Option<String>,
    pub evidence_id: String,
}
pub async fn check(
    config: &ModelConfig,
    root: &Path,
    cancel: CancellationToken,
) -> Result<ConnectionReceipt> {
    let folder = root.join("startup").join(uuid::Uuid::new_v4().to_string());
    let transport = Transport::new(config.clone(), &folder, cancel)?;
    let body = messages_body(
        config,
        json!([{"role":"user","content":"这是连接检查，请简短确认已收到。"}]),
        2048,
        false,
    );
    let outcome = transport
        .send(body, false, Instant::now() + REQUEST_BUDGET)
        .await?;
    if let Some(failure) = &outcome.failure {
        return Err(connection_error(&outcome, &failure.title()));
    }
    let response = outcome
        .response
        .as_ref()
        .filter(|r| r.normal_end && !r.text.trim().is_empty() && r.calls.is_empty())
        .ok_or_else(|| connection_error(&outcome, "连接检查未取得完整的正常回复"))?;
    Ok(ConnectionReceipt {
        target: config.public_target(),
        elapsed_ms: outcome.evidence.elapsed_ms,
        returned_model: response.model.clone(),
        evidence_id: outcome.evidence.id,
    })
}
fn connection_error(outcome: &RequestOutcome, fallback: &str) -> anyhow::Error {
    if matches!(outcome.failure, Some(RequestFailure::Cancelled)) {
        return anyhow!("连接检查已取消");
    }
    if let Some(attempt) = outcome.evidence.attempts.last() {
        if let Some(status) = attempt.http_status {
            return if attempt.response_body.trim().is_empty() {
                anyhow!("HTTP {status}（响应正文为空）")
            } else {
                anyhow!("HTTP {status}\n{}", attempt.response_body)
            };
        }
        if let Some(error) = &attempt.error {
            return anyhow!(error.clone());
        }
    }
    anyhow!(fallback.to_owned())
}
