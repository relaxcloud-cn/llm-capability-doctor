use crate::gui::{DesktopEnvironment, DesktopPlatform, detect_desktop};
use crate::transport::{ChatCompletionsRequest, ChatCompletionsTransport};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::time::Duration;

const PREFLIGHT_PROMPT: &str = "请只回复：启动检查通过。";
const PREFLIGHT_MAX_TOKENS: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectivityState {
    Passed,
    Failed,
    NotRun,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelConnectivityResult {
    pub state: ConnectivityState,
    pub elapsed_ms: u128,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct StartupPreflight {
    pub connectivity: ModelConnectivityResult,
    pub desktop: DesktopEnvironment,
}

pub fn run_startup_preflight(
    endpoint: Option<&str>,
    model: Option<&str>,
    api_key: Option<&str>,
    cached_token: Option<&str>,
    timeout: Duration,
    platform: DesktopPlatform,
    environment: &BTreeMap<String, String>,
) -> StartupPreflight {
    StartupPreflight {
        connectivity: check_model_connectivity(endpoint, model, api_key, cached_token, timeout),
        desktop: detect_desktop(platform, environment),
    }
}

pub fn check_model_connectivity(
    endpoint: Option<&str>,
    model: Option<&str>,
    api_key: Option<&str>,
    cached_token: Option<&str>,
    timeout: Duration,
) -> ModelConnectivityResult {
    let Some(endpoint) = non_empty(endpoint) else {
        return not_run("配置不完整：缺少服务 URL");
    };
    let Some(model) = non_empty(model) else {
        return not_run("配置不完整：缺少模型名称");
    };
    let Some(api_key) = non_empty(api_key) else {
        return not_run("配置不完整：缺少 API 密钥");
    };

    if cached_token == Some(preflight_token(endpoint, model, api_key).as_str()) {
        return ModelConnectivityResult {
            state: ConnectivityState::Passed,
            elapsed_ms: 0,
            reason: "已复用父进程预检查结果（服务配置未变化）".into(),
        };
    }

    let transport = match ChatCompletionsTransport::new(endpoint, model, api_key, timeout) {
        Ok(transport) => transport,
        Err(error) => {
            return failed(
                0,
                format!("无法创建请求客户端：{}", sanitize_reason(&error)),
            );
        }
    };
    let response = transport.send(ChatCompletionsRequest {
        module_id: "startup-preflight".into(),
        messages: None,
        tools: None,
        prompt: PREFLIGHT_PROMPT.into(),
        max_tokens: PREFLIGHT_MAX_TOKENS,
        stream: false,
    });
    let elapsed_ms = response.elapsed_ms;

    if let Some(error) = response.error {
        return failed(elapsed_ms, classify_transport_error(&error));
    }

    let status = response.status.unwrap_or_default();
    if !(200..300).contains(&status) {
        return failed(elapsed_ms, classify_http_status(status));
    }
    if !is_chat_completion_shape(response.parsed.as_ref()) {
        return failed(
            elapsed_ms,
            "HTTP 请求成功，但响应不符合 Chat Completions 协议".into(),
        );
    }

    ModelConnectivityResult {
        state: ConnectivityState::Passed,
        elapsed_ms,
        reason: format!("HTTP {status}，响应符合 Chat Completions 协议"),
    }
}

pub fn preflight_token(endpoint: &str, model: &str, api_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(endpoint.as_bytes());
    hasher.update([0]);
    hasher.update(model.as_bytes());
    hasher.update([0]);
    hasher.update(api_key.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

fn not_run(reason: &str) -> ModelConnectivityResult {
    ModelConnectivityResult {
        state: ConnectivityState::NotRun,
        elapsed_ms: 0,
        reason: reason.into(),
    }
}

fn failed(elapsed_ms: u128, reason: String) -> ModelConnectivityResult {
    ModelConnectivityResult {
        state: ConnectivityState::Failed,
        elapsed_ms,
        reason,
    }
}

fn is_chat_completion_shape(value: Option<&serde_json::Value>) -> bool {
    value
        .and_then(|value| value.get("choices"))
        .and_then(serde_json::Value::as_array)
        .is_some_and(|choices| {
            choices.first().is_some_and(|choice| {
                choice.get("message").is_some() || choice.get("text").is_some()
            })
        })
}

fn classify_http_status(status: u16) -> String {
    let reason = match status {
        401 | 403 => "鉴权或权限失败，请检查 API 密钥和模型权限",
        404 => "服务地址或模型不存在，请检查 URL 和模型名称",
        408 => "请求超时，请检查网络和服务端响应时间",
        429 => "服务限流，请稍后重试或检查配额",
        500..=599 => "服务端错误，请检查服务状态",
        _ => "服务返回非成功状态，请检查 URL、模型和服务协议",
    };
    format!("HTTP {status}：{reason}")
}

fn classify_transport_error(error: &str) -> String {
    let normalized = error.to_ascii_lowercase();
    let reason = if normalized.contains("timed out") || normalized.contains("timeout") {
        "请求超时，请检查网络和服务端响应时间"
    } else if normalized.contains("dns") || normalized.contains("resolve") {
        "域名解析失败，请检查服务 URL 和网络"
    } else if normalized.contains("tls") || normalized.contains("certificate") {
        "TLS 连接失败，请检查证书和服务地址"
    } else {
        "网络连接失败，请检查服务 URL 和网络"
    };
    format!("{reason}（已脱敏）")
}

fn sanitize_reason(reason: &str) -> String {
    if reason.contains("http") || reason.contains("://") {
        "请求客户端配置无效（已脱敏）".into()
    } else {
        reason.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn serve_once(status: &str, body: &str) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_owned();
        let body = body.to_owned();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let size = stream.read(&mut request).unwrap_or(0);
            let content = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(content.as_bytes()).unwrap();
            String::from_utf8_lossy(&request[..size]).into_owned()
        });
        (format!("http://{address}/v1/chat/completions"), handle)
    }

    #[test]
    fn missing_configuration_is_not_run() {
        let result = check_model_connectivity(
            None,
            Some("model"),
            Some("key"),
            None,
            Duration::from_secs(1),
        );
        assert_eq!(result.state, ConnectivityState::NotRun);
        assert!(result.reason.contains("服务 URL"));
    }

    #[test]
    fn successful_probe_checks_real_response_shape() {
        let (endpoint, server) =
            serve_once("200 OK", r#"{"choices":[{"message":{"content":"ok"}}]}"#);
        let result = check_model_connectivity(
            Some(&endpoint),
            Some("model-a"),
            Some("secret-value"),
            None,
            Duration::from_secs(5),
        );
        let request = server.join().unwrap();
        assert_eq!(result.state, ConnectivityState::Passed);
        assert!(request.contains("\"model\":\"model-a\""));
        assert!(request.contains("\"max_tokens\":8"));
    }

    #[test]
    fn auth_failure_is_reported_without_leaking_key() {
        let (endpoint, server) = serve_once("401 Unauthorized", r#"{"error":{"message":"no"}}"#);
        let result = check_model_connectivity(
            Some(&endpoint),
            Some("model-a"),
            Some("secret-value"),
            None,
            Duration::from_secs(5),
        );
        server.join().unwrap();
        assert_eq!(result.state, ConnectivityState::Failed);
        assert!(result.reason.contains("鉴权"));
        assert!(!result.reason.contains("secret-value"));
    }

    #[test]
    fn startup_preflight_keeps_desktop_result_when_model_is_not_run() {
        let mut environment = BTreeMap::new();
        environment.insert("DISPLAY".into(), ":0".into());
        let result = run_startup_preflight(
            None,
            Some("model"),
            Some("key"),
            None,
            Duration::from_secs(1),
            DesktopPlatform::Linux,
            &environment,
        );
        assert_eq!(result.connectivity.state, ConnectivityState::NotRun);
        assert!(result.desktop.supported);
    }

    #[test]
    fn matching_token_reuses_parent_probe_without_request() {
        let token = preflight_token("https://example.test", "model", "key");
        let result = check_model_connectivity(
            Some("https://example.test"),
            Some("model"),
            Some("key"),
            Some(&token),
            Duration::from_secs(1),
        );
        assert_eq!(result.state, ConnectivityState::Passed);
        assert_eq!(result.elapsed_ms, 0);
    }
}
