use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::Value;
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::validator::CandidateEnvelope;
use crate::http::{HttpExecutor, RequestInput};
use crate::protocol::{AuthMode, Protocol, analysis_request, normalize_request_url};

pub struct AnalysisTarget {
    pub url: Url,
    pub model: String,
    pub api_key: String,
    pub timeout: Duration,
    pub insecure: bool,
    pub protocol: Protocol,
    pub auth_mode: AuthMode,
}

pub struct ModelAnalysisClient {
    target: AnalysisTarget,
    http: HttpExecutor,
    sequence: AtomicUsize,
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("self-analysis does not support an unknown protocol")]
    UnsupportedProtocol,
    #[error("analysis request origin differs from the configured model origin")]
    OriginChanged,
    #[error("analysis transport failed: {0}")]
    Transport(String),
    #[error("analysis endpoint returned HTTP {0}")]
    HttpStatus(u16),
    #[error("analysis response has no protocol-native assistant content")]
    MissingAssistantContent,
    #[error("analysis response envelope is invalid: {0}")]
    InvalidResponseEnvelope(String),
    #[error("analysis assistant content is not valid candidate JSON: {0}")]
    InvalidJson(String),
    #[error("analysis request could not be encoded: {0}")]
    Encode(#[from] serde_json::Error),
    #[error("analysis HTTP client could not be created: {0}")]
    HttpClient(#[from] reqwest::Error),
}

#[allow(async_fn_in_trait)]
pub trait AnalysisClient {
    async fn analyze(
        &self,
        prompt: &str,
        cancellation: CancellationToken,
    ) -> Result<CandidateEnvelope, ClientError>;
}

impl ModelAnalysisClient {
    pub fn new(target: AnalysisTarget) -> Result<Self, ClientError> {
        if target.protocol == Protocol::Unknown {
            return Err(ClientError::UnsupportedProtocol);
        }
        let http = HttpExecutor::new(target.timeout, target.insecure)?;
        Ok(Self {
            target,
            http,
            sequence: AtomicUsize::new(1),
        })
    }
}

impl AnalysisClient for ModelAnalysisClient {
    async fn analyze(
        &self,
        prompt: &str,
        cancellation: CancellationToken,
    ) -> Result<CandidateEnvelope, ClientError> {
        let spec = analysis_request(self.target.protocol, &self.target.model, prompt);
        let request_url =
            normalize_request_url(self.target.protocol, &self.target.url, spec.stream);
        if !same_origin(&self.target.url, &request_url) {
            return Err(ClientError::OriginChanged);
        }
        let request_id = self.sequence.fetch_add(1, Ordering::Relaxed);
        let evidence = self
            .http
            .execute(
                RequestInput {
                    request_id: format!("self-analysis-{request_id}"),
                    url: request_url,
                    protocol: self.target.protocol,
                    auth_mode: self.target.auth_mode,
                    body: serde_json::to_vec(&spec.body)?,
                    stream: false,
                    api_key: self.target.api_key.clone(),
                },
                cancellation,
            )
            .await;
        if !evidence.transport_outcome.is_success() {
            return Err(ClientError::Transport(
                evidence.transport_outcome.to_string(),
            ));
        }
        let status = evidence
            .metrics
            .http_status
            .ok_or_else(|| ClientError::Transport("missing HTTP status".into()))?;
        if !(200..300).contains(&status) {
            return Err(ClientError::HttpStatus(status));
        }
        let assistant_text = extract_assistant_text(self.target.protocol, &evidence.response_body)?;
        serde_json::from_str(&assistant_text)
            .map_err(|error| ClientError::InvalidJson(error.to_string()))
    }
}

pub fn extract_assistant_text(protocol: Protocol, body: &[u8]) -> Result<String, ClientError> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|error| ClientError::InvalidResponseEnvelope(error.to_string()))?;
    let text = match protocol {
        Protocol::OpenAiChat => value
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .map(str::to_owned),
        Protocol::OpenAiResponses => value
            .get("output_text")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| collect_responses_text(&value)),
        Protocol::AnthropicMessages => collect_typed_text(&value, "/content", "text", "text"),
        Protocol::GeminiGenerateContent => {
            collect_untyped_text(value.pointer("/candidates/0/content/parts"), "text")
        }
        Protocol::OllamaChat => value
            .pointer("/message/content")
            .and_then(Value::as_str)
            .map(str::to_owned),
        Protocol::Unknown => return Err(ClientError::UnsupportedProtocol),
    };
    text.filter(|text| !text.trim().is_empty())
        .ok_or(ClientError::MissingAssistantContent)
}

fn collect_responses_text(value: &Value) -> Option<String> {
    let output = value.get("output")?.as_array()?;
    let mut text = String::new();
    for item in output {
        if item.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let Some(content) = item.get("content").and_then(Value::as_array) else {
            continue;
        };
        for block in content {
            if block.get("type").and_then(Value::as_str) == Some("output_text")
                && let Some(value) = block.get("text").and_then(Value::as_str)
            {
                text.push_str(value);
            }
        }
    }
    (!text.is_empty()).then_some(text)
}

fn collect_typed_text(
    value: &Value,
    pointer: &str,
    expected_type: &str,
    text_field: &str,
) -> Option<String> {
    let blocks = value.pointer(pointer)?.as_array()?;
    let mut text = String::new();
    for block in blocks {
        if block.get("type").and_then(Value::as_str) == Some(expected_type)
            && let Some(value) = block.get(text_field).and_then(Value::as_str)
        {
            text.push_str(value);
        }
    }
    (!text.is_empty()).then_some(text)
}

fn collect_untyped_text(value: Option<&Value>, text_field: &str) -> Option<String> {
    let blocks = value?.as_array()?;
    let text: String = blocks
        .iter()
        .filter_map(|block| block.get(text_field).and_then(Value::as_str))
        .collect();
    (!text.is_empty()).then_some(text)
}

fn same_origin(configured: &Url, request: &Url) -> bool {
    configured.scheme() == request.scheme()
        && configured.host_str() == request.host_str()
        && configured.port_or_known_default() == request.port_or_known_default()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use httpmock::Method::POST;
    use httpmock::MockServer;
    use serde_json::json;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::protocol::{AuthMode, Protocol};

    #[test]
    fn extracts_protocol_native_analysis_text() {
        let cases = [
            (
                Protocol::OpenAiChat,
                json!({"choices":[{"message":{"content":"{\"reviews\":[]}"}}]}),
            ),
            (
                Protocol::OpenAiResponses,
                json!({"output_text":"{\"reviews\":[]}"}),
            ),
            (
                Protocol::AnthropicMessages,
                json!({"content":[{"type":"text","text":"{\"reviews\":[]}"}]}),
            ),
            (
                Protocol::GeminiGenerateContent,
                json!({"candidates":[{"content":{"parts":[{"text":"{\"reviews\":[]}"}]}}]}),
            ),
            (
                Protocol::OllamaChat,
                json!({"message":{"content":"{\"reviews\":[]}"},"done":true}),
            ),
        ];

        for (protocol, body) in cases {
            assert_eq!(
                extract_assistant_text(protocol, body.to_string().as_bytes()).unwrap(),
                "{\"reviews\":[]}"
            );
        }
    }

    #[test]
    fn responses_falls_back_to_official_output_blocks() {
        let body = json!({
            "output": [{
                "type": "message",
                "content": [
                    {"type": "output_text", "text": "{\"reviews\":"},
                    {"type": "output_text", "text": "[]}"}
                ]
            }]
        });

        assert_eq!(
            extract_assistant_text(Protocol::OpenAiResponses, body.to_string().as_bytes()).unwrap(),
            "{\"reviews\":[]}"
        );
    }

    #[tokio::test]
    async fn sends_to_configured_path_with_detected_auth() {
        let server = MockServer::start_async().await;
        let endpoint = server.url("/analyze").parse().unwrap();
        let response = json!({
            "choices": [{"message": {"content": "{\"reviews\":[]}"}}]
        });
        let request_mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/analyze")
                    .header("authorization", "Bearer secret-key");
                then.status(200).json_body(response);
            })
            .await;
        let client = ModelAnalysisClient::new(AnalysisTarget {
            url: endpoint,
            model: "model".into(),
            api_key: "secret-key".into(),
            timeout: Duration::from_secs(2),
            insecure: false,
            protocol: Protocol::OpenAiChat,
            auth_mode: AuthMode::Bearer,
        })
        .unwrap();

        let envelope = client
            .analyze("prompt", CancellationToken::new())
            .await
            .unwrap();

        assert!(envelope.reviews.is_empty());
        request_mock.assert_async().await;
    }

    #[tokio::test]
    async fn does_not_follow_redirects() {
        let server = MockServer::start_async().await;
        let endpoint = server.url("/analyze").parse().unwrap();
        let redirect = server
            .mock_async(|when, then| {
                when.method(POST).path("/analyze");
                then.status(302).header("location", "/elsewhere");
            })
            .await;
        let elsewhere = server
            .mock_async(|when, then| {
                when.method(POST).path("/elsewhere");
                then.status(200);
            })
            .await;
        let client = ModelAnalysisClient::new(AnalysisTarget {
            url: endpoint,
            model: "model".into(),
            api_key: "secret-key".into(),
            timeout: Duration::from_secs(2),
            insecure: false,
            protocol: Protocol::OpenAiChat,
            auth_mode: AuthMode::Bearer,
        })
        .unwrap();

        let error = client
            .analyze("prompt", CancellationToken::new())
            .await
            .unwrap_err();

        assert!(matches!(error, ClientError::HttpStatus(302)));
        assert_eq!(redirect.calls_async().await, 1);
        assert_eq!(elsewhere.calls_async().await, 0);
    }
}
