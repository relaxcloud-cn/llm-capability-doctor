use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::{fmt, path::Path};
use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Protocol {
    #[value(name = "chat-completions", alias = "chat")]
    ChatCompletions,
    Responses,
}
impl Protocol {
    pub fn label(self) -> &'static str {
        match self {
            Self::ChatCompletions => "Chat Completions",
            Self::Responses => "Responses",
        }
    }
    pub fn suffix(self) -> &'static str {
        match self {
            Self::ChatCompletions => "/chat/completions",
            Self::Responses => "/responses",
        }
    }
}

/// Credentials deliberately cannot be serialized or printed through Debug.
#[derive(Clone)]
pub struct ModelConfig {
    pub model: String,
    pub endpoint: Url,
    pub protocol: Protocol,
    pub insecure: bool,
    api_key: String,
}
impl fmt::Debug for ModelConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModelConfig")
            .field("model", &self.model)
            .field("endpoint", &self.public_target().endpoint)
            .field("protocol", &self.protocol)
            .field("insecure", &self.insecure)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Target {
    pub model: String,
    pub endpoint: String,
    pub protocol: Protocol,
    #[serde(default)]
    pub insecure: bool,
}
#[derive(Deserialize)]
struct ConfigFile {
    model: String,
    #[serde(alias = "url", alias = "endpoint")]
    base_url: String,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    api_key_env: Option<String>,
    #[serde(default)]
    protocol: Option<Protocol>,
}
impl ModelConfig {
    pub fn new(
        address: &str,
        model: &str,
        api_key: String,
        selected: Option<Protocol>,
    ) -> Result<Self> {
        let model = model.trim();
        if model.is_empty() || model.len() > 256 {
            bail!("请填写有效的模型名称");
        }
        if api_key.trim().is_empty() {
            bail!("请提供 API Key");
        }
        let mut endpoint = Url::parse(address.trim())
            .map_err(|_| anyhow::anyhow!("服务地址需要是完整 HTTP 或 HTTPS 地址"))?;
        if !matches!(endpoint.scheme(), "http" | "https") || endpoint.host_str().is_none() {
            bail!("服务地址需要是完整 HTTP 或 HTTPS 地址");
        }
        if !endpoint.username().is_empty() || endpoint.password().is_some() {
            bail!("服务地址不能包含账号或密码，请通过 API Key 配置鉴权");
        }
        endpoint.set_fragment(None);
        let path = endpoint.path().trim_end_matches('/').to_owned();
        let detected = if path.ends_with(Protocol::ChatCompletions.suffix()) {
            Some(Protocol::ChatCompletions)
        } else if path.ends_with(Protocol::Responses.suffix()) {
            Some(Protocol::Responses)
        } else {
            None
        };
        if let (Some(detected), Some(selected)) = (detected, selected) {
            if detected != selected {
                bail!("完整接口地址与所选协议不匹配，请核对地址和协议");
            }
        }
        let protocol = selected.or(detected).unwrap_or(Protocol::ChatCompletions);
        if detected.is_none() {
            endpoint.set_path(&format!(
                "{}{}",
                if path.is_empty() { "/v1" } else { &path },
                protocol.suffix()
            ));
        }
        Ok(Self {
            model: model.to_owned(),
            endpoint,
            protocol,
            insecure: false,
            api_key,
        })
    }
    pub fn load(path: &Path) -> Result<Self> {
        Self::load_with_protocol(path, None)
    }
    pub fn load_with_protocol(path: &Path, override_protocol: Option<Protocol>) -> Result<Self> {
        let raw: ConfigFile =
            serde_json::from_slice(&std::fs::read(path).context("无法读取模型配置")?)
                .map_err(|_| anyhow::anyhow!("模型配置不是有效 JSON，或字段类型不符合要求"))?;
        let key = match (raw.api_key, raw.api_key_env) {
            (Some(key), _) => key,
            (None, Some(name)) => std::env::var(name).context("配置指定的密钥环境变量未设置")?,
            _ => std::env::var("MODEL_API_KEY")
                .context("请在配置或 MODEL_API_KEY 环境变量中提供密钥")?,
        };
        Self::new(
            &raw.base_url,
            &raw.model,
            key,
            override_protocol.or(raw.protocol),
        )
    }
    pub fn api_key(&self) -> &str {
        &self.api_key
    }
    pub fn public_target(&self) -> Target {
        let mut endpoint = self.endpoint.clone();
        // Query parameters may carry credentials. No query values enter presentation or evidence.
        if endpoint.query().is_some() {
            endpoint.set_query(Some("REDACTED"));
        }
        Target {
            model: self.model.clone(),
            endpoint: endpoint.to_string(),
            protocol: self.protocol,
            insecure: self.insecure,
        }
    }
    pub fn redact(&self, value: &str) -> String {
        let mut secrets = vec![self.api_key.clone()];
        if let Some(query) = self.endpoint.query() {
            secrets.push(query.to_owned());
            secrets.extend(
                self.endpoint
                    .query_pairs()
                    .filter(|(key, _)| {
                        matches!(
                            key.to_ascii_lowercase().as_str(),
                            "key"
                                | "api_key"
                                | "apikey"
                                | "api-key"
                                | "token"
                                | "access_token"
                                | "auth"
                                | "authorization"
                                | "password"
                                | "secret"
                                | "signature"
                                | "sig"
                        )
                    })
                    .map(|(_, value)| value.into_owned()),
            );
        }
        let mut patterns = Vec::new();
        for secret in secrets.into_iter().filter(|value| !value.is_empty()) {
            let escaped = serde_json::to_string(&secret).unwrap();
            patterns.push(escaped[1..escaped.len() - 1].to_owned());
            patterns
                .push(url::form_urlencoded::byte_serialize(secret.as_bytes()).collect::<String>());
            patterns.push(secret);
        }
        patterns.sort_unstable_by_key(|value| std::cmp::Reverse(value.len()));
        patterns.dedup();
        let mut redacted = value.to_owned();
        for pattern in patterns {
            redacted = redacted.replace(&pattern, "[REDACTED]");
        }
        redacted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_base_full_and_explicit_protocol_consistently() {
        for (url, selected, protocol, endpoint) in [
            (
                "https://example.org/llm_v2/v1/",
                None,
                Protocol::ChatCompletions,
                "https://example.org/llm_v2/v1/chat/completions",
            ),
            (
                "https://example.org/v1/responses",
                None,
                Protocol::Responses,
                "https://example.org/v1/responses",
            ),
            (
                "https://example.org/v1",
                Some(Protocol::Responses),
                Protocol::Responses,
                "https://example.org/v1/responses",
            ),
            (
                "http://127.0.0.1:8080",
                None,
                Protocol::ChatCompletions,
                "http://127.0.0.1:8080/v1/chat/completions",
            ),
        ] {
            let c = ModelConfig::new(url, "model", "secret".into(), selected).unwrap();
            assert_eq!(c.protocol, protocol);
            assert_eq!(c.endpoint.as_str(), endpoint);
        }
        assert!(
            ModelConfig::new(
                "https://example.org/v1/responses",
                "model",
                "secret".into(),
                Some(Protocol::ChatCompletions)
            )
            .is_err()
        );
    }
    #[test]
    fn secrets_are_removed_from_debug_target_and_error_evidence() {
        let c = ModelConfig::new(
            "https://example.org/v1?token=private",
            "model",
            "sensitive-key".into(),
            None,
        )
        .unwrap();
        let target = serde_json::to_string(&c.public_target()).unwrap();
        let debug = format!("{c:?}");
        assert!(!target.contains("private"));
        assert!(!debug.contains("private"));
        assert!(!debug.contains("sensitive-key"));
        assert_eq!(
            c.redact("request sensitive-key token=private"),
            "request [REDACTED] [REDACTED]"
        );
    }
}
