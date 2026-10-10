use crate::{
    config::{ModelConfig, Target},
    transport::{REQUEST_BUDGET, RequestFailure, Transport, messages_body},
};
use anyhow::{Result, bail};
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
        bail!(
            "{}",
            match failure {
                RequestFailure::Http(401 | 403) => "模型连通性失败：API 密钥未通过验证".into(),
                RequestFailure::Http(404) => "模型连通性失败：模型或接口地址不存在".into(),
                RequestFailure::Cancelled => "连接检查已取消".into(),
                _ => format!("模型连通性失败：{}", failure.title()),
            }
        );
    }
    let response = outcome
        .response
        .filter(|r| r.normal_end && !r.text.trim().is_empty() && r.calls.is_empty())
        .ok_or_else(|| anyhow::anyhow!("模型连通性失败：未取得完整的正常回复"))?;
    Ok(ConnectionReceipt {
        target: config.public_target(),
        elapsed_ms: outcome.evidence.elapsed_ms,
        returned_model: response.model,
        evidence_id: outcome.evidence.id,
    })
}
