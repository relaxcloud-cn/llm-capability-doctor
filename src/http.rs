use std::time::{Duration, Instant};

use chrono::Local;
use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::audit::{RequestEvidence, ResponseMetrics};
use crate::evidence::{
    StreamEndSignal, StreamTermination, ToolContractStatus, ToolLoopOutcome, TransportOutcome,
};
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
    direct_client: reqwest::Client,
    timeout: Duration,
    insecure: bool,
}

struct TransportFacts {
    cancelled: bool,
    timed_out: bool,
    pre_response_error: bool,
    body_read_error: bool,
    http_status: Option<u16>,
}

struct TransportClassification {
    transport_outcome: TransportOutcome,
    stream_termination: StreamTermination,
}

#[derive(Default)]
struct ExecuteHooks {
    #[cfg(test)]
    first_body_chunk: Option<std::sync::Arc<tokio::sync::Notify>>,
}

impl ExecuteHooks {
    #[cfg(test)]
    fn after_body_chunk(&mut self) {
        if let Some(observer) = self.first_body_chunk.take() {
            observer.notify_one();
        }
    }

    #[cfg(not(test))]
    fn after_body_chunk(&mut self) {}
}

impl HttpExecutor {
    pub fn new(timeout: Duration, insecure: bool) -> Result<Self, reqwest::Error> {
        let builder = || {
            reqwest::Client::builder()
                .danger_accept_invalid_certs(insecure)
                .redirect(reqwest::redirect::Policy::none())
                .timeout(timeout)
        };
        let client = builder().build()?;
        let direct_client = builder().no_proxy().build()?;
        Ok(Self {
            client,
            direct_client,
            timeout,
            insecure,
        })
    }

    pub async fn execute(
        &self,
        input: RequestInput,
        cancellation: CancellationToken,
    ) -> RequestEvidence {
        self.execute_inner(input, cancellation, ExecuteHooks::default())
            .await
    }

    async fn execute_inner(
        &self,
        input: RequestInput,
        cancellation: CancellationToken,
        mut hooks: ExecuteHooks,
    ) -> RequestEvidence {
        let started_at = Local::now();
        let started = Instant::now();
        let client = if endpoint_is_loopback(&input.url) {
            &self.direct_client
        } else {
            &self.client
        };
        let mut request = client
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
            biased;
            _ = cancellation.cancelled() => {
                return self.failed_evidence(
                    input,
                    started_at,
                    started.elapsed(),
                    "request cancelled".into(),
                    TransportFacts {
                        cancelled: true,
                        timed_out: false,
                        pre_response_error: false,
                        body_read_error: false,
                        http_status: None,
                    },
                );
            }
            response = request.send() => response,
        };

        let response = match response {
            Ok(response) => response,
            Err(error) => {
                let timed_out = error.is_timeout();
                return self.failed_evidence(
                    input,
                    started_at,
                    started.elapsed(),
                    format_error_chain(&error),
                    TransportFacts {
                        cancelled: false,
                        timed_out,
                        pre_response_error: !timed_out,
                        body_read_error: false,
                        http_status: None,
                    },
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
        let mut cancelled = false;
        let mut timed_out = false;
        let mut body_read_error = false;

        loop {
            let next = tokio::select! {
                biased;
                _ = cancellation.cancelled() => {
                    error = "request cancelled".into();
                    cancelled = true;
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
                    hooks.after_body_chunk();
                }
                Some(Err(stream_error)) => {
                    timed_out = stream_error.is_timeout();
                    body_read_error = true;
                    error = format_error_chain(&stream_error);
                    break;
                }
                None => break,
            }
        }

        let completed_at = Local::now();
        let total = started.elapsed();
        let classification = classify_transport(TransportFacts {
            cancelled,
            timed_out,
            pre_response_error: false,
            body_read_error,
            http_status: Some(status),
        });
        let transport_exit_code =
            i32::from(classification.transport_outcome != TransportOutcome::CompletedEof);
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
            transport_outcome: classification.transport_outcome,
            stream_termination: classification.stream_termination,
            stream_end_signal: StreamEndSignal::None,
            model_stop_reason: None,
            stream_event_count: 0,
            tool_contract_status: ToolContractStatus::NotApplicable,
            tool_contract_errors: Vec::new(),
            tool_loop_turn: 0,
            tool_loop_outcome: ToolLoopOutcome::NotApplicable,
        }
    }

    #[cfg(test)]
    async fn execute_with_body_chunk_observer(
        &self,
        input: RequestInput,
        cancellation: CancellationToken,
        body_chunk_observed: std::sync::Arc<tokio::sync::Notify>,
    ) -> RequestEvidence {
        self.execute_inner(
            input,
            cancellation,
            ExecuteHooks {
                first_body_chunk: Some(body_chunk_observed),
            },
        )
        .await
    }

    fn failed_evidence(
        &self,
        input: RequestInput,
        started_at: chrono::DateTime<Local>,
        total: Duration,
        error: String,
        facts: TransportFacts,
    ) -> RequestEvidence {
        let classification = classify_transport(facts);
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
            transport_outcome: classification.transport_outcome,
            stream_termination: classification.stream_termination,
            stream_end_signal: StreamEndSignal::None,
            model_stop_reason: None,
            stream_event_count: 0,
            tool_contract_status: ToolContractStatus::NotApplicable,
            tool_contract_errors: Vec::new(),
            tool_loop_turn: 0,
            tool_loop_outcome: ToolLoopOutcome::NotApplicable,
        }
    }
}

fn classify_transport(facts: TransportFacts) -> TransportClassification {
    let transport_outcome = if facts.cancelled {
        TransportOutcome::ClientCancelled
    } else if facts.timed_out {
        TransportOutcome::Timeout
    } else if facts.pre_response_error {
        TransportOutcome::TransportError
    } else if facts.body_read_error {
        TransportOutcome::UpstreamDisconnect
    } else {
        TransportOutcome::CompletedEof
    };

    let stream_termination = if facts.cancelled {
        StreamTermination::ClientCancelled
    } else if facts.timed_out {
        StreamTermination::Timeout
    } else if facts.pre_response_error {
        StreamTermination::TransportError
    } else if facts
        .http_status
        .is_some_and(|status| !(200..300).contains(&status))
    {
        StreamTermination::HttpError
    } else if facts.body_read_error {
        StreamTermination::UpstreamDisconnect
    } else {
        StreamTermination::NotApplicable
    };

    TransportClassification {
        transport_outcome,
        stream_termination,
    }
}

fn endpoint_is_loopback(endpoint: &Url) -> bool {
    match endpoint.host() {
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        Some(url::Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}

fn format_error_chain(error: &dyn std::error::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::net::TcpListener;
    use tokio::sync::Notify;
    use tokio_util::sync::CancellationToken;

    use super::{HttpExecutor, RequestInput, TransportFacts, classify_transport};
    use crate::evidence::{StreamTermination, TransportOutcome};
    use crate::protocol::{AuthMode, Protocol};
    use crate::test_support::{RawTcpServer, ServerAction};

    const PARTIAL_BODY: &[u8] = b"hello";

    #[tokio::test]
    async fn completed_eof_preserves_body_without_claiming_protocol_completion() {
        let fixture = RawTcpServer::spawn(vec![vec![ServerAction::Write(response(
            "200 OK", 5, b"hello",
        ))]])
        .await;

        let evidence = execute(&fixture, Duration::from_secs(1), CancellationToken::new()).await;

        assert_eq!(evidence.response_body, b"hello");
        assert_eq!(evidence.transport_outcome, TransportOutcome::CompletedEof);
        assert_eq!(
            evidence.stream_termination,
            StreamTermination::NotApplicable
        );
        assert_eq!(evidence.metrics.http_status, Some(200));
        assert_eq!(evidence.metrics.transport_exit_code, 0);
        assert!(evidence.error.is_empty());
        let requests = fixture.recorded_requests().await;
        assert_eq!(requests.len(), 1);
        assert!(requests[0].head.starts_with("POST /v1/chat/completions "));
        assert_eq!(requests[0].body, br#"{"probe":true}"#);
    }

    #[tokio::test]
    async fn timeout_before_headers_is_classified() {
        let fixture =
            RawTcpServer::spawn(vec![vec![ServerAction::Delay(Duration::from_millis(250))]]).await;

        let evidence = execute(
            &fixture,
            Duration::from_millis(50),
            CancellationToken::new(),
        )
        .await;

        assert_eq!(evidence.transport_outcome, TransportOutcome::Timeout);
        assert_eq!(evidence.stream_termination, StreamTermination::Timeout);
        assert_eq!(evidence.metrics.http_status, None);
        assert!(evidence.response_body.is_empty());
        assert!(evidence.error.to_ascii_lowercase().contains("timed out"));
        assert_eq!(fixture.accepted_count(), 1);
    }

    #[tokio::test]
    async fn timeout_during_body_preserves_partial_bytes() {
        let fixture = RawTcpServer::spawn(vec![vec![
            ServerAction::Write(response("200 OK", 11, PARTIAL_BODY)),
            ServerAction::Delay(Duration::from_millis(250)),
        ]])
        .await;

        let evidence = execute(
            &fixture,
            Duration::from_millis(60),
            CancellationToken::new(),
        )
        .await;

        assert_eq!(evidence.response_body, PARTIAL_BODY);
        assert_eq!(evidence.transport_outcome, TransportOutcome::Timeout);
        assert_eq!(evidence.stream_termination, StreamTermination::Timeout);
        assert_eq!(evidence.metrics.http_status, Some(200));
        assert!(evidence.error.to_ascii_lowercase().contains("timed out"));
        assert!(evidence.error.contains(':'));
        assert_response_headers_retained(&evidence, "11");
    }

    #[tokio::test]
    async fn disconnect_during_body_preserves_partial_bytes() {
        let fixture = RawTcpServer::spawn(vec![vec![
            ServerAction::Write(response("200 OK", 11, PARTIAL_BODY)),
            ServerAction::Close,
        ]])
        .await;

        let evidence = execute(&fixture, Duration::from_secs(1), CancellationToken::new()).await;

        assert_eq!(evidence.response_body, PARTIAL_BODY);
        assert_eq!(
            evidence.transport_outcome,
            TransportOutcome::UpstreamDisconnect
        );
        assert_eq!(
            evidence.stream_termination,
            StreamTermination::UpstreamDisconnect
        );
        assert_eq!(evidence.metrics.http_status, Some(200));
        assert!(!evidence.error.is_empty());
        assert!(evidence.error.contains(':'));
        assert_response_headers_retained(&evidence, "11");
    }

    #[tokio::test]
    async fn cancellation_before_headers_wins() {
        let checkpoint = std::sync::Arc::new(Notify::new());
        let fixture = RawTcpServer::spawn(vec![vec![
            ServerAction::Checkpoint(checkpoint.clone()),
            ServerAction::Delay(Duration::from_millis(250)),
        ]])
        .await;
        let executor = HttpExecutor::new(Duration::from_secs(1), false).expect("HTTP executor");
        let cancellation = CancellationToken::new();
        let task = tokio::spawn({
            let cancellation = cancellation.clone();
            let input = request_input(fixture.url());
            async move { executor.execute(input, cancellation).await }
        });

        wait_for_notification(&checkpoint).await;
        cancellation.cancel();
        let evidence = task.await.expect("HTTP task");

        assert_eq!(
            evidence.transport_outcome,
            TransportOutcome::ClientCancelled
        );
        assert_eq!(
            evidence.stream_termination,
            StreamTermination::ClientCancelled
        );
        assert_eq!(evidence.metrics.http_status, None);
        assert!(evidence.response_body.is_empty());
    }

    #[tokio::test]
    async fn cancellation_during_body_preserves_partial_bytes() {
        let body_chunk_observed = std::sync::Arc::new(Notify::new());
        let fixture = RawTcpServer::spawn(vec![vec![
            ServerAction::Write(response("200 OK", 11, PARTIAL_BODY)),
            ServerAction::Delay(Duration::from_secs(2)),
        ]])
        .await;
        let executor = HttpExecutor::new(Duration::from_secs(3), false).expect("HTTP executor");
        let cancellation = CancellationToken::new();
        let task = tokio::spawn({
            let cancellation = cancellation.clone();
            let body_chunk_observed = body_chunk_observed.clone();
            let input = request_input(fixture.url());
            async move {
                executor
                    .execute_with_body_chunk_observer(input, cancellation, body_chunk_observed)
                    .await
            }
        });

        wait_for_notification(&body_chunk_observed).await;
        cancellation.cancel();
        let evidence = task.await.expect("HTTP task");

        assert_eq!(evidence.response_body, PARTIAL_BODY);
        assert_eq!(
            evidence.transport_outcome,
            TransportOutcome::ClientCancelled
        );
        assert_eq!(
            evidence.stream_termination,
            StreamTermination::ClientCancelled
        );
        assert_eq!(evidence.metrics.http_status, Some(200));
        assert_response_headers_retained(&evidence, "11");
    }

    #[tokio::test]
    async fn non_success_status_is_http_error() {
        let fixture = RawTcpServer::spawn(vec![vec![ServerAction::Write(response(
            "500 Internal Server Error",
            3,
            b"bad",
        ))]])
        .await;

        let evidence = execute(&fixture, Duration::from_secs(1), CancellationToken::new()).await;

        assert_eq!(evidence.response_body, b"bad");
        assert_eq!(evidence.transport_outcome, TransportOutcome::CompletedEof);
        assert_eq!(evidence.stream_termination, StreamTermination::HttpError);
        assert_eq!(evidence.metrics.http_status, Some(500));
    }

    #[tokio::test]
    async fn connect_failure_is_transport_error() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind port");
        let address = listener.local_addr().expect("listener address");
        drop(listener);
        let executor = HttpExecutor::new(Duration::from_secs(1), false).expect("HTTP executor");

        let evidence = executor
            .execute(
                request_input(
                    format!("http://{address}/v1/chat/completions")
                        .parse()
                        .unwrap(),
                ),
                CancellationToken::new(),
            )
            .await;

        assert_eq!(evidence.transport_outcome, TransportOutcome::TransportError);
        assert_eq!(
            evidence.stream_termination,
            StreamTermination::TransportError
        );
        assert_eq!(evidence.metrics.http_status, None);
        assert!(
            evidence.error.matches(':').count() >= 2,
            "{}",
            evidence.error
        );
    }

    #[test]
    fn cancellation_beats_a_simultaneously_ready_timeout() {
        let classification = classify_transport(TransportFacts {
            cancelled: true,
            timed_out: true,
            pre_response_error: false,
            body_read_error: false,
            http_status: None,
        });

        assert_eq!(
            classification.transport_outcome,
            TransportOutcome::ClientCancelled
        );
        assert_eq!(
            classification.stream_termination,
            StreamTermination::ClientCancelled
        );
    }

    #[tokio::test]
    async fn pre_cancelled_token_does_not_send_request() {
        let fixture = RawTcpServer::spawn(vec![vec![ServerAction::Write(response(
            "200 OK", 2, b"ok",
        ))]])
        .await;
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let evidence = execute(&fixture, Duration::from_secs(1), cancellation).await;
        tokio::task::yield_now().await;

        assert_eq!(
            evidence.transport_outcome,
            TransportOutcome::ClientCancelled
        );
        assert_eq!(fixture.accepted_count(), 0);
    }

    #[tokio::test]
    async fn body_timeout_beats_an_observed_http_500() {
        let fixture = RawTcpServer::spawn(vec![vec![
            ServerAction::Write(response("500 Internal Server Error", 11, PARTIAL_BODY)),
            ServerAction::Delay(Duration::from_millis(250)),
        ]])
        .await;

        let evidence = execute(
            &fixture,
            Duration::from_millis(60),
            CancellationToken::new(),
        )
        .await;

        assert_eq!(evidence.response_body, PARTIAL_BODY);
        assert_eq!(evidence.transport_outcome, TransportOutcome::Timeout);
        assert_eq!(evidence.stream_termination, StreamTermination::Timeout);
        assert_eq!(evidence.metrics.http_status, Some(500));
        assert_response_headers_retained(&evidence, "11");
    }

    #[tokio::test]
    async fn http_500_beats_a_later_body_disconnect() {
        let fixture = RawTcpServer::spawn(vec![vec![
            ServerAction::Write(response("500 Internal Server Error", 11, PARTIAL_BODY)),
            ServerAction::Close,
        ]])
        .await;

        let evidence = execute(&fixture, Duration::from_secs(1), CancellationToken::new()).await;

        assert_eq!(evidence.response_body, PARTIAL_BODY);
        assert_eq!(
            evidence.transport_outcome,
            TransportOutcome::UpstreamDisconnect
        );
        assert_eq!(evidence.stream_termination, StreamTermination::HttpError);
        assert_eq!(evidence.metrics.http_status, Some(500));
        assert_response_headers_retained(&evidence, "11");
    }

    async fn execute(
        fixture: &RawTcpServer,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> crate::audit::RequestEvidence {
        HttpExecutor::new(timeout, false)
            .expect("HTTP executor")
            .execute(request_input(fixture.url()), cancellation)
            .await
    }

    fn request_input(url: url::Url) -> RequestInput {
        RequestInput {
            request_id: "transport-test".to_owned(),
            url,
            protocol: Protocol::OpenAiChat,
            auth_mode: AuthMode::None,
            body: br#"{"probe":true}"#.to_vec(),
            stream: true,
            api_key: String::new(),
        }
    }

    fn response(status: &str, content_length: usize, body: &[u8]) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {content_length}\r\nConnection: close\r\n\r\n"
        )
        .into_bytes();
        response.extend_from_slice(body);
        response
    }

    async fn wait_for_notification(notification: &Notify) {
        tokio::time::timeout(Duration::from_secs(1), notification.notified())
            .await
            .expect("test notification timed out");
    }

    fn assert_response_headers_retained(
        evidence: &crate::audit::RequestEvidence,
        expected_length: &str,
    ) {
        assert!(
            evidence
                .headers
                .iter()
                .any(|(name, value)| name == "content-length" && value == expected_length),
            "{:?}",
            evidence.headers
        );
    }
}
