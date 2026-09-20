use serde_json::Value;

use crate::transport::ChatCompletionsResponse;

/// S01 上下文容量实测的固定探测档位（tokens）。
/// 口径：四档全部发送真实长度输入，不提前停止，512K 封顶。
pub const CONTEXT_TIERS: [u64; 4] = [64_000, 128_000, 256_000, 512_000];

/// 校不准或无 usage 时的兜底字符/token 密度，仅用于构造输入。
pub const FALLBACK_CHARS_PER_TOKEN: f64 = 3.0;
const CALIBRATION_CHARS: usize = 4_000;
const MIN_CHARS_PER_TOKEN: f64 = 0.5;
const MAX_CHARS_PER_TOKEN: f64 = 16.0;
/// 构造时多写 5%，抵消密度估算误差，保证实测 token 不落在档位之下。
const OVERSHOOT: f64 = 1.05;
const SEGMENT_LINE: &str =
    "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor\n";

#[derive(Debug, Clone, PartialEq)]
pub enum ProbeOutcome {
    /// HTTP 2xx + 可解析 + 实测/估算 prompt_tokens 达到档位目标。
    Passed {
        measured_tokens: u64,
        estimated: bool,
    },
    /// 服务明确因输入过长拒绝（可归因容量拒绝）。
    Rejected { detail: String },
    /// 环境/传输/计数不足，不能归因容量。
    Inconclusive { detail: String },
    /// HTTP 2xx 但响应不可解析或结构异常。
    Malformed { detail: String },
}

/// 从规格样本 ID 解析档位目标："context-64k" -> 64000。
pub fn tier_tokens(sample_id: &str) -> Option<u64> {
    let rest = sample_id.strip_prefix("context-")?;
    let (digits, multiplier) = match rest.strip_suffix('k') {
        Some(digits) => (digits, 1_000),
        None => (rest, 1),
    };
    Some(digits.parse::<u64>().ok()?.checked_mul(multiplier)?)
}

/// 校准请求：已知字符数的短文本，用于测服务端字符/token 密度。
pub fn calibration_prompt() -> String {
    let mut prompt = String::with_capacity(CALIBRATION_CHARS + 128);
    prompt.push_str("CONTEXT_CALIBRATION_BEGIN\n");
    let mut index = 0usize;
    while prompt.len() < CALIBRATION_CHARS {
        index += 1;
        prompt.push_str(&format!(
            "CALIB {index:05} lorem ipsum dolor sit amet consectetur adipiscing elit\n"
        ));
    }
    prompt.push_str("CONTEXT_CALIBRATION_END\nReply only OK.");
    prompt
}

/// 按密度构造约 target_tokens 的真实长度输入（含唯一首尾标记，便于审计定位）。
pub fn build_probe_prompt(target_tokens: u64, chars_per_token: f64) -> String {
    let target_chars =
        (target_tokens as f64 * chars_per_token * OVERSHOOT).clamp(1.0, f64::MAX) as usize;
    let mut prompt = String::with_capacity(target_chars + 128);
    prompt.push_str("CONTEXT_PROBE_BEGIN\n");
    let mut index = 0usize;
    while prompt.len() < target_chars {
        index += 1;
        prompt.push_str(&format!("SEGMENT {index:07} {SEGMENT_LINE}"));
    }
    prompt.push_str("CONTEXT_PROBE_END\nReply only OK.");
    prompt
}

pub fn chars_per_token(prompt_chars: usize, prompt_tokens: u64) -> f64 {
    if prompt_tokens == 0 {
        return FALLBACK_CHARS_PER_TOKEN;
    }
    (prompt_chars as f64 / prompt_tokens as f64).clamp(MIN_CHARS_PER_TOKEN, MAX_CHARS_PER_TOKEN)
}

/// 优先取服务端 usage.prompt_tokens；不取其他字段冒充。
pub fn extract_prompt_tokens(parsed: Option<&Value>) -> Option<u64> {
    parsed?.get("usage")?.get("prompt_tokens")?.as_u64()
}

/// 单档判定：
/// - 传输失败 → Inconclusive；
/// - 非 2xx：命中长度关键词且非网关体积限制 → Rejected；413/5xx 及其它 → Inconclusive；
/// - 2xx 但结构不可识别 → Malformed；结构合法但 usage 低于目标 → Inconclusive（疑似截断）；
/// - 2xx + 结构合法 + prompt_tokens ≥ 目标（或估算达标）→ Passed。
pub fn classify(
    response: &ChatCompletionsResponse,
    target_tokens: u64,
    estimated_tokens: u64,
) -> ProbeOutcome {
    if let Some(error) = &response.error {
        return ProbeOutcome::Inconclusive {
            detail: format!("传输失败：{error}"),
        };
    }
    let status = response.status.unwrap_or(0);
    if !(200..300).contains(&status) {
        if status != 413 && is_context_length_rejection(&response.body) {
            return ProbeOutcome::Rejected {
                detail: format!("HTTP {status}：{}", excerpt(&response.body, 200)),
            };
        }
        let reason = if status == 413 {
            "网关请求体体积限制，属环境限制而非模型窗口"
        } else {
            "非可归因容量拒绝"
        };
        return ProbeOutcome::Inconclusive {
            detail: format!("HTTP {status}，{reason}"),
        };
    }
    if !has_chat_shape(response.parsed.as_ref()) {
        if is_context_length_rejection(&response.body) {
            return ProbeOutcome::Rejected {
                detail: format!(
                    "HTTP {status} 错误响应声明超长：{}",
                    excerpt(&response.body, 200)
                ),
            };
        }
        return ProbeOutcome::Malformed {
            detail: "HTTP 成功但响应缺少可识别的 choices/message 结构".into(),
        };
    }
    match extract_prompt_tokens(response.parsed.as_ref()) {
        Some(tokens) if tokens >= target_tokens => ProbeOutcome::Passed {
            measured_tokens: tokens,
            estimated: false,
        },
        Some(tokens) => ProbeOutcome::Inconclusive {
            detail: format!(
                "服务端 usage.prompt_tokens={tokens} 低于目标 {target_tokens}，疑似输入被截断或计数口径不一致"
            ),
        },
        None if estimated_tokens >= target_tokens => ProbeOutcome::Passed {
            measured_tokens: estimated_tokens,
            estimated: true,
        },
        None => ProbeOutcome::Inconclusive {
            detail: "响应缺少 usage.prompt_tokens，构造输入未达到可支撑目标的估算量".into(),
        },
    }
}

pub(crate) fn has_chat_shape(value: Option<&Value>) -> bool {
    value
        .and_then(|value| value.get("choices"))
        .and_then(Value::as_array)
        .is_some_and(|choices| {
            choices.first().is_some_and(|choice| {
                choice.get("message").is_some() || choice.get("text").is_some()
            })
        })
}

pub(crate) fn is_context_length_rejection(body: &str) -> bool {
    let lower = body.to_lowercase();
    const SIGNALS: &[&str] = &[
        "context length",
        "context_length",
        "maximum context",
        "max context",
        "context window",
        "too many tokens",
        "token limit",
        "tokens exceeded",
        "prompt is too long",
        "input is too long",
        "input too long",
        "exceeds the model",
        "exceed the model",
        "length exceeded",
        "length limit",
        "reduce the length",
        "上下文",
        "超长",
        "过长",
        "长度限制",
    ];
    SIGNALS.iter().any(|signal| lower.contains(signal))
}

pub(crate) fn excerpt(body: &str, max_chars: usize) -> String {
    let trimmed = body.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_owned();
    }
    format!("{}…", trimmed.chars().take(max_chars).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn response(status: Option<u16>, body: &str, parsed: Option<Value>) -> ChatCompletionsResponse {
        ChatCompletionsResponse {
            status,
            body: body.into(),
            parsed,
            elapsed_ms: 1,
            error: None,
            attempts: 1,
            retry_reasons: Vec::new(),
        }
    }

    fn ok_body(prompt_tokens: u64) -> (String, Value) {
        let parsed = json!({
            "choices": [{"message": {"role": "assistant", "content": "OK"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": prompt_tokens, "completion_tokens": 1}
        });
        (parsed.to_string(), parsed)
    }

    #[test]
    fn tier_tokens_parse_k_suffixed_and_plain_ids() {
        assert_eq!(tier_tokens("context-64k"), Some(64_000));
        assert_eq!(tier_tokens("context-512k"), Some(512_000));
        assert_eq!(tier_tokens("context-1024"), Some(1_024));
        assert_eq!(tier_tokens("context-boundary"), None);
        assert_eq!(tier_tokens("output-small"), None);
    }

    #[test]
    fn chars_per_token_is_clamped_and_falls_back_on_zero() {
        assert_eq!(chars_per_token(4_000, 0), FALLBACK_CHARS_PER_TOKEN);
        assert_eq!(chars_per_token(4_000, 1_000), 4.0);
        assert_eq!(chars_per_token(4_000, 100), MAX_CHARS_PER_TOKEN);
        assert_eq!(chars_per_token(10, 1_000), MIN_CHARS_PER_TOKEN);
    }

    #[test]
    fn probe_prompt_overshoots_target_chars() {
        let prompt = build_probe_prompt(64_000, 4.0);
        assert!(prompt.len() >= (64_000.0 * 4.0) as usize);
        assert!(prompt.starts_with("CONTEXT_PROBE_BEGIN"));
        assert!(prompt.contains("SEGMENT 0000001"));
        assert!(prompt.ends_with("Reply only OK."));
    }

    #[test]
    fn passes_when_server_usage_reaches_target() {
        let (body, parsed) = ok_body(66_000);
        let outcome = classify(&response(Some(200), &body, Some(parsed)), 64_000, 70_000);
        assert_eq!(
            outcome,
            ProbeOutcome::Passed {
                measured_tokens: 66_000,
                estimated: false
            }
        );
    }

    #[test]
    fn falls_back_to_estimate_when_usage_missing() {
        let parsed = json!({"choices": [{"message": {"content": "OK"}, "finish_reason": "stop"}]});
        let outcome = classify(
            &response(Some(200), &parsed.to_string(), Some(parsed)),
            64_000,
            67_200,
        );
        assert_eq!(
            outcome,
            ProbeOutcome::Passed {
                measured_tokens: 67_200,
                estimated: true
            }
        );
    }

    #[test]
    fn usage_below_target_is_inconclusive_not_pass() {
        let (body, parsed) = ok_body(60_000);
        let outcome = classify(&response(Some(200), &body, Some(parsed)), 64_000, 70_000);
        assert!(matches!(outcome, ProbeOutcome::Inconclusive { .. }));
    }

    #[test]
    fn explicit_length_refusal_is_rejected() {
        let outcome = classify(
            &response(
                Some(400),
                r#"{"error":{"message":"This model's maximum context length is 65536 tokens"}}"#,
                None,
            ),
            64_000,
            70_000,
        );
        assert!(matches!(outcome, ProbeOutcome::Rejected { .. }));
    }

    #[test]
    fn gateway_payload_limit_is_environment_not_rejection() {
        let outcome = classify(
            &response(Some(413), "Request Entity Too Large", None),
            512_000,
            540_000,
        );
        assert!(matches!(outcome, ProbeOutcome::Inconclusive { .. }));
    }

    #[test]
    fn non_length_4xx_is_inconclusive() {
        let outcome = classify(
            &response(
                Some(401),
                r#"{"error":{"message":"invalid api key"}}"#,
                None,
            ),
            64_000,
            70_000,
        );
        assert!(matches!(outcome, ProbeOutcome::Inconclusive { .. }));
    }

    #[test]
    fn two_hundred_error_body_with_length_claim_is_rejected() {
        let outcome = classify(
            &response(
                Some(200),
                r#"{"error":{"message":"prompt is too long"}}"#,
                Some(json!({"error": {"message": "prompt is too long"}})),
            ),
            64_000,
            70_000,
        );
        assert!(matches!(outcome, ProbeOutcome::Rejected { .. }));
    }

    #[test]
    fn unparseable_success_is_malformed() {
        let outcome = classify(
            &response(Some(200), "<html>bad gateway</html>", None),
            64_000,
            70_000,
        );
        assert!(matches!(outcome, ProbeOutcome::Malformed { .. }));
    }

    #[test]
    fn transport_error_is_inconclusive() {
        let mut resp = response(None, "", None);
        resp.error = Some("发送 HTTP 请求失败：timeout".into());
        let outcome = classify(&resp, 64_000, 70_000);
        assert!(matches!(outcome, ProbeOutcome::Inconclusive { .. }));
    }
}
