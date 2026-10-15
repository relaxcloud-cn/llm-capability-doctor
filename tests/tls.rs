use agent_check::{
    config::{ModelConfig, Protocol},
    preflight,
    transport::{RequestFailure, Transport, messages_body},
};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{
        ServerConfig,
        pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
    },
};
use tokio_util::sync::CancellationToken;

struct Service {
    url: String,
    received: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Service {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn service() -> Service {
    // Public test-only key; certificate is deliberately untrusted and names a different host.
    let cert = CertificateDer::from_pem_slice(include_bytes!("fixtures/tls-cert.pem")).unwrap();
    let key = PrivateKeyDer::from_pem_slice(include_bytes!("fixtures/tls-key.pem")).unwrap();
    let tls = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(tls));
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("https://{}/v1", socket.local_addr().unwrap());
    let received = Arc::new(AtomicUsize::new(0));
    let count = received.clone();
    let task = tokio::spawn(async move {
        loop {
            let (stream, _) = socket.accept().await.unwrap();
            let Ok(mut stream) = acceptor.accept(stream).await else {
                continue;
            };
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let n = stream.read(&mut buffer).await.unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..n]);
                if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            count.fetch_add(1, Ordering::SeqCst);
            let body = if request.starts_with(b"POST /v1/responses ") {
                json!({"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"TLS response received"}]}]})
            } else {
                json!({"choices":[{"message":{"role":"assistant","content":"TLS response received"},"finish_reason":"stop"}]})
            }.to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            let _ = stream.shutdown().await;
        }
    });
    Service {
        url,
        received,
        task,
    }
}

#[tokio::test]
async fn insecure_is_explicit_per_client_and_applies_after_preflight() {
    let server = service().await;
    let root = tempfile::tempdir().unwrap();
    for protocol in [Protocol::ChatCompletions, Protocol::Responses] {
        let secure = ModelConfig::new(
            &server.url,
            "tls-test",
            "test-only-key".into(),
            Some(protocol),
        )
        .unwrap();
        let before = server.received.load(Ordering::SeqCst);
        let error = preflight::check(&secure, root.path(), CancellationToken::new())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("certificate"), "{error}");
        assert_eq!(server.received.load(Ordering::SeqCst), before);

        let mut insecure = secure.clone();
        insecure.insecure = true;
        let receipt = preflight::check(&insecure, root.path(), CancellationToken::new())
            .await
            .unwrap();
        assert!(receipt.target.insecure);
        let transport = Transport::new(insecure, root.path(), CancellationToken::new()).unwrap();
        let body = messages_body(
            transport.config(),
            json!([{"role":"user","content":"follow-up"}]),
            32,
            false,
        );
        let outcome = transport
            .send_once(body.clone(), Instant::now() + Duration::from_secs(5))
            .await
            .unwrap();
        assert!(outcome.failure.is_none());
        assert_eq!(outcome.response.unwrap().text, "TLS response received");
        assert!(outcome.evidence.insecure);
        assert_eq!(server.received.load(Ordering::SeqCst), before + 2);

        // Enabling bypass on one client must not disable validation for another client.
        let secure_transport =
            Transport::new(secure, root.path(), CancellationToken::new()).unwrap();
        let rejected = secure_transport
            .send_once(body, Instant::now() + Duration::from_secs(5))
            .await
            .unwrap();
        assert!(matches!(rejected.failure, Some(RequestFailure::Network)));
        assert_eq!(server.received.load(Ordering::SeqCst), before + 2);
    }
}
