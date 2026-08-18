use std::time::{Duration, Instant};

use chrono::Local;
use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::audit::{RequestEvidence, ResponseMetrics};
use crate::protocol::stream::{StreamControl, StreamInspector, StreamState};
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
                    format_error_chain(&error),
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
        let mut reached_eof = false;
        let mut inspector = (input.stream && StreamInspector::supports(input.protocol))
            .then(|| StreamInspector::new(input.protocol));

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
                    if inspector
                        .as_mut()
                        .is_some_and(|inspector| inspector.push(&chunk) == StreamControl::Stop)
                    {
                        let inspector = inspector
                            .as_ref()
                            .expect("an inspector stopped the response stream");
                        if inspector.state() == StreamState::Error {
                            error = inspector
                                .error_message()
                                .unwrap_or("stream ended with a protocol error")
                                .into();
                            transport_exit_code = 1;
                        }
                        break;
                    }
                }
                Some(Err(stream_error)) => {
                    error = stream_error.to_string();
                    transport_exit_code = 1;
                    break;
                }
                None => {
                    reached_eof = true;
                    break;
                }
            }
        }

        if reached_eof
            && transport_exit_code == 0
            && let Some(inspector) = inspector.as_mut()
        {
            let _ = inspector.finish();
            match inspector.state() {
                StreamState::Success => {}
                StreamState::Error => {
                    error = inspector
                        .error_message()
                        .unwrap_or("stream ended with a protocol error")
                        .into();
                    transport_exit_code = 1;
                }
                StreamState::Pending => {
                    error = "stream ended without a terminal event".into();
                    transport_exit_code = 1;
                }
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

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;
    use tokio::task::JoinHandle;
    use tokio_util::sync::CancellationToken;
    use url::Url;

    use super::{HttpExecutor, RequestInput};
    use crate::protocol::stream::MAX_SSE_RECORD_BYTES;
    use crate::protocol::{AuthMode, Protocol};

    struct SseFixture {
        url: Url,
        release: Option<oneshot::Sender<()>>,
        task: JoinHandle<()>,
    }

    impl SseFixture {
        async fn start(body_chunk: &[u8], hold_open: bool) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let body_chunk = body_chunk.to_vec();
            let (release, wait_for_release) = oneshot::channel();
            let task = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                read_request(&mut socket).await;
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n",
                    )
                    .await
                    .unwrap();
                socket
                    .write_all(format!("{:x}\r\n", body_chunk.len()).as_bytes())
                    .await
                    .unwrap();
                socket.write_all(&body_chunk).await.unwrap();
                socket.write_all(b"\r\n").await.unwrap();
                socket.flush().await.unwrap();

                if hold_open {
                    let _ = wait_for_release.await;
                }
                let _ = socket.write_all(b"0\r\n\r\n").await;
            });
            Self {
                url: Url::parse(&format!("http://{address}/v1/stream")).unwrap(),
                release: hold_open.then_some(release),
                task,
            }
        }

        async fn close(mut self) {
            if let Some(release) = self.release.take() {
                let _ = release.send(());
            }
            self.task.await.unwrap();
        }
    }

    async fn read_request(socket: &mut tokio::net::TcpStream) {
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let count = socket.read(&mut buffer).await.unwrap();
            if count == 0 {
                return;
            }
            request.extend_from_slice(&buffer[..count]);
            let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
            else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if request.len() >= header_end + 4 + content_length {
                return;
            }
        }
    }

    fn request(url: Url, protocol: Protocol, stream: bool) -> RequestInput {
        RequestInput {
            request_id: "http-stream-test".into(),
            url,
            protocol,
            auth_mode: AuthMode::None,
            body: b"{}".to_vec(),
            stream,
            api_key: String::new(),
        }
    }

    #[tokio::test]
    async fn chat_done_is_recorded_before_returning_from_a_held_open_response() {
        let terminal = b"data: [DONE]\n\n";
        let fixture = SseFixture::start(terminal, true).await;
        let executor = HttpExecutor::new(Duration::from_secs(10), false).unwrap();

        let evidence = tokio::time::timeout(
            Duration::from_secs(2),
            executor.execute(
                request(fixture.url.clone(), Protocol::OpenAiChat, true),
                CancellationToken::new(),
            ),
        )
        .await
        .expect("[DONE] should stop without waiting for the connection to close");

        assert_eq!(evidence.metrics.transport_exit_code, 0);
        assert_eq!(evidence.response_body, terminal);
        assert_eq!(evidence.metrics.size_download, terminal.len());
        fixture.close().await;
    }

    #[tokio::test]
    async fn explicit_stream_error_returns_failure() {
        let fixture = SseFixture::start(b"event: error\ndata: {}\n\n", false).await;
        let executor = HttpExecutor::new(Duration::from_secs(2), false).unwrap();

        let evidence = executor
            .execute(
                request(fixture.url.clone(), Protocol::OpenAiResponses, true),
                CancellationToken::new(),
            )
            .await;

        assert_eq!(evidence.metrics.transport_exit_code, 1);
        assert!(evidence.error.contains("error event"), "{}", evidence.error);
        fixture.close().await;
    }

    #[tokio::test]
    async fn oversized_stream_record_is_fully_recorded_before_immediate_failure() {
        let mut oversized = b"data: ".to_vec();
        oversized.resize(MAX_SSE_RECORD_BYTES + 1, b'x');
        let fixture = SseFixture::start(&oversized, true).await;
        let executor = HttpExecutor::new(Duration::from_secs(10), false).unwrap();

        let evidence = tokio::time::timeout(
            Duration::from_secs(2),
            executor.execute(
                request(fixture.url.clone(), Protocol::OpenAiResponses, true),
                CancellationToken::new(),
            ),
        )
        .await
        .expect("an oversized SSE record should stop before server EOF");

        assert_eq!(evidence.metrics.transport_exit_code, 1);
        assert_eq!(evidence.response_body, oversized);
        assert!(evidence.error.contains("1 MiB"), "{}", evidence.error);
        fixture.close().await;
    }

    #[tokio::test]
    async fn responses_completed_then_eof_succeeds() {
        let fixture =
            SseFixture::start(b"data: {\"type\":\"response.completed\"}\n\n", false).await;
        let executor = HttpExecutor::new(Duration::from_secs(2), false).unwrap();

        let evidence = executor
            .execute(
                request(fixture.url.clone(), Protocol::OpenAiResponses, true),
                CancellationToken::new(),
            )
            .await;

        assert_eq!(evidence.metrics.transport_exit_code, 0);
        assert!(evidence.error.is_empty());
        fixture.close().await;
    }

    #[tokio::test]
    async fn responses_completed_on_a_held_open_response_times_out() {
        let fixture = SseFixture::start(b"data: {\"type\":\"response.completed\"}\n\n", true).await;
        let executor = HttpExecutor::new(Duration::from_millis(100), false).unwrap();

        let evidence = executor
            .execute(
                request(fixture.url.clone(), Protocol::OpenAiResponses, true),
                CancellationToken::new(),
            )
            .await;

        assert_eq!(evidence.metrics.transport_exit_code, 1);
        assert!(!evidence.error.is_empty());
        fixture.close().await;
    }

    #[tokio::test]
    async fn stream_eof_without_a_native_terminal_returns_failure() {
        let fixture = SseFixture::start(
            b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n\n",
            false,
        )
        .await;
        let executor = HttpExecutor::new(Duration::from_secs(2), false).unwrap();

        let evidence = executor
            .execute(
                request(fixture.url.clone(), Protocol::OpenAiResponses, true),
                CancellationToken::new(),
            )
            .await;

        assert_eq!(evidence.metrics.transport_exit_code, 1);
        assert!(
            evidence.error.contains("without a terminal"),
            "{}",
            evidence.error
        );
        fixture.close().await;
    }

    #[tokio::test]
    async fn non_stream_ollama_and_unknown_requests_keep_eof_behavior() {
        for (protocol, stream) in [
            (Protocol::OpenAiResponses, false),
            (Protocol::OllamaChat, true),
            (Protocol::Unknown, true),
        ] {
            let fixture = SseFixture::start(b"not an SSE terminal", false).await;
            let executor = HttpExecutor::new(Duration::from_secs(2), false).unwrap();

            let evidence = executor
                .execute(
                    request(fixture.url.clone(), protocol, stream),
                    CancellationToken::new(),
                )
                .await;

            assert_eq!(evidence.metrics.transport_exit_code, 0, "{protocol}");
            assert!(evidence.error.is_empty(), "{protocol}: {}", evidence.error);
            fixture.close().await;
        }
    }
}
