use std::time::{Duration, Instant};

use chrono::Local;
use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::audit::{RequestEvidence, ResponseMetrics};
use crate::protocol::{AuthMode, Protocol};

pub struct RequestInput {
    pub request_id: String,
    pub url: Url,
    pub protocol: Protocol,
    pub auth_mode: AuthMode,
    pub body: Vec<u8>,
    pub stream: bool,
    pub api_key: String,
}

#[derive(Clone)]
pub struct HttpExecutor {
    client: reqwest::Client,
    timeout: Duration,
    insecure: bool,
}

impl HttpExecutor {
    pub fn new(timeout: Duration, insecure: bool) -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(insecure)
            .timeout(timeout)
            .build()?;
        Ok(Self {
            client,
            timeout,
            insecure,
        })
    }

    pub async fn execute(
        &self,
        input: RequestInput,
        cancellation: CancellationToken,
    ) -> RequestEvidence {
        let started_at = Local::now();
        let started = Instant::now();
        let mut request = self
            .client
            .post(input.url.clone())
            .header(ACCEPT, "application/json, text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .body(input.body.clone());
        request = match input.auth_mode {
            AuthMode::Bearer => {
                request.header("authorization", format!("Bearer {}", input.api_key))
            }
            AuthMode::ApiKey => request.header("api-key", &input.api_key),
            AuthMode::XApiKey => request
                .header("x-api-key", &input.api_key)
                .header("anthropic-version", "2023-06-01"),
            AuthMode::XGoogApiKey => request.header("x-goog-api-key", &input.api_key),
            AuthMode::None => request,
        };

        let response = tokio::select! {
            _ = cancellation.cancelled() => {
                return self.failed_evidence(input, started_at, started.elapsed(), "request cancelled".into());
            }
            response = request.send() => response,
        };

        let response = match response {
            Ok(response) => response,
            Err(error) => {
                return self.failed_evidence(
                    input,
                    started_at,
                    started.elapsed(),
                    error.to_string(),
                );
            }
        };

        let status = response.status().as_u16();
        let headers_received = started.elapsed();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect();
        let mut body = Vec::new();
        let mut first_body_byte = None;
        let mut stream = response.bytes_stream();
        let mut error = String::new();
        let mut transport_exit_code = 0;

        loop {
            let next = tokio::select! {
                _ = cancellation.cancelled() => {
                    error = "request cancelled".into();
                    transport_exit_code = 1;
                    break;
                }
                next = stream.next() => next,
            };
            match next {
                Some(Ok(chunk)) => {
                    if first_body_byte.is_none() {
                        first_body_byte = Some(started.elapsed());
                    }
                    body.extend_from_slice(&chunk);
                }
                Some(Err(stream_error)) => {
                    error = stream_error.to_string();
                    transport_exit_code = 1;
                    break;
                }
                None => break,
            }
        }

        let completed_at = Local::now();
        let total = started.elapsed();
        RequestEvidence {
            request_id: input.request_id,
            started_at,
            completed_at,
            protocol: input.protocol,
            auth_mode: input.auth_mode,
            stream: input.stream,
            url: input.url,
            timeout: self.timeout,
            insecure: self.insecure,
            body: String::from_utf8_lossy(&input.body).into_owned(),
            metrics: ResponseMetrics {
                transport_exit_code,
                http_status: Some(status),
                time_total: total,
                time_starttransfer: first_body_byte.unwrap_or(headers_received),
                size_download: body.len(),
            },
            headers,
            error,
            response_body: body,
        }
    }

    fn failed_evidence(
        &self,
        input: RequestInput,
        started_at: chrono::DateTime<Local>,
        total: Duration,
        error: String,
    ) -> RequestEvidence {
        RequestEvidence {
            request_id: input.request_id,
            started_at,
            completed_at: Local::now(),
            protocol: input.protocol,
            auth_mode: input.auth_mode,
            stream: input.stream,
            url: input.url,
            timeout: self.timeout,
            insecure: self.insecure,
            body: String::from_utf8_lossy(&input.body).into_owned(),
            metrics: ResponseMetrics {
                transport_exit_code: 1,
                http_status: None,
                time_total: total,
                time_starttransfer: Duration::ZERO,
                size_download: 0,
            },
            headers: Vec::new(),
            error,
            response_body: Vec::new(),
        }
    }
}
