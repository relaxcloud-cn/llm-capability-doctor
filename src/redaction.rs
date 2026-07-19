use std::collections::HashSet;

use url::Url;

const SECRET_QUERY_KEYS: &[&str] = &[
    "api_key",
    "key",
    "token",
    "access_token",
    "client_secret",
    "password",
];

const SECRET_HEADER_NAMES: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "api-key",
    "x-api-key",
    "x-goog-api-key",
    "cookie",
    "set-cookie",
];

pub struct Redactor {
    secrets: Vec<String>,
}

impl Redactor {
    pub fn new(api_key: &str, url: &Url) -> Self {
        let mut secrets = HashSet::new();
        if !api_key.is_empty() {
            secrets.insert(api_key.to_owned());
        }

        for (key, value) in url.query_pairs() {
            if is_secret_query_key(&key) && !value.is_empty() {
                secrets.insert(value.into_owned());
            }
        }
        if let Some(query) = url.query() {
            for parameter in query.split('&') {
                let (key, value) = parameter.split_once('=').unwrap_or((parameter, ""));
                if is_secret_query_key(key) && !value.is_empty() {
                    secrets.insert(value.to_owned());
                }
            }
        }

        let mut secrets: Vec<String> = secrets.into_iter().collect();
        secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        Self { secrets }
    }

    pub fn redact_url(&self, url: &Url) -> String {
        self.redact_text(url.as_str())
    }

    pub fn redact_headers(&self, headers: &[(String, String)]) -> Vec<(String, String)> {
        headers
            .iter()
            .map(|(name, value)| {
                let safe_value = if is_secret_header(name) {
                    "[REDACTED]".to_owned()
                } else {
                    self.redact_text(value)
                };
                (name.clone(), safe_value)
            })
            .collect()
    }

    pub fn redact_text(&self, value: &str) -> String {
        let mut redacted = value.to_owned();
        for secret in &self.secrets {
            if !secret.is_empty() && secret != "[REDACTED]" {
                redacted = redacted.replace(secret, "[REDACTED]");
            }
        }
        redacted
    }
}

pub fn mask_api_key(value: &str) -> String {
    let characters: Vec<char> = value.chars().collect();
    if characters.len() <= 8 {
        return "[MASKED]".to_owned();
    }
    let first: String = characters[..4].iter().collect();
    let last: String = characters[characters.len() - 4..].iter().collect();
    format!("{first}********{last}")
}

fn is_secret_query_key(value: &str) -> bool {
    SECRET_QUERY_KEYS
        .iter()
        .any(|candidate| value.eq_ignore_ascii_case(candidate))
}

fn is_secret_header(value: &str) -> bool {
    SECRET_HEADER_NAMES
        .iter()
        .any(|candidate| value.eq_ignore_ascii_case(candidate))
}
