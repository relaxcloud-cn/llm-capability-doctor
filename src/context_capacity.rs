//! 自适应上下文容量检测。
//!
//! 旧的 014-018 用「4 字符 ≈ 1 token」估算请求大小，重复 ASCII 填充块在真实
//! 分词器上的密度可达 1.5-2.5 倍，导致按字符构造的「128K」请求实际发出
//! 192K-320K token，把真实上下文充足的模型误判为不通过。
//!
//! 新逻辑只信服务端统计：先发一个校准请求读取响应 `usage` 里的 prompt token
//! 数，得到该模型真实的字符/token 密度，再按实测密度构造目标 token 数的探针，
//! 用对半夹逼定位真实上限。结论只写实测 token 数，字符数不进结论。

use std::time::Duration;

use crate::protocol::{Protocol, RequestSpec, basic_request};

/// 智能体部署要求的最低上下文（token）。
pub const CONTEXT_REQUIREMENT_TOKENS: u64 = 128_000;
/// 主探针目标：要求的 97%，给恰好 128K 上限的模型留出回复余量。
pub const CONTEXT_MAIN_TARGET_TOKENS: u64 = 124_000;
/// 向上探测的上限（token）。主探针通过后按 2 倍向上试，到此为止。
pub const CONTEXT_UPWARD_CAP_TOKENS: u64 = 496_000;
/// 校准请求的填充字符数。同质填充下小请求测得的密度可直接外推。
pub const CALIBRATION_FILLER_CHARS: usize = 40_000;
/// 容量探针（不含校准）最多发送次数。
pub const MAX_CAPACITY_PROBES: usize = 5;
/// 向下夹逼的最小区间（token），小于它就停止细分。
pub const MIN_INTERVAL_TOKENS: u64 = 8_000;
/// 密度未知时的兜底假设（英文散文的常见值）。
pub const FALLBACK_CHARS_PER_TOKEN: f64 = 4.0;
/// 容量探针允许的输出 token 上限，避免「输入+默认输出」撞上限。
const PROBE_MAX_OUTPUT_TOKENS: u64 = 16;
/// 校准请求 ID。
pub const CALIBRATION_REQUEST_ID: &str = "test-018-calibration";
/// 容量探针请求 ID 前缀。
pub const PROBE_REQUEST_PREFIX: &str = "test-018-probe-";

const FILLER_BLOCK: &str = "FILLER_BLOCK_0123456789 ";

/// 单个探针的分类结果。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeOutcome {
    /// 服务端接受（HTTP 2xx、传输成功、回复非空）。
    Accepted { measured_tokens: Option<u64> },
    /// 服务端以超长为由拒绝（HTTP 4xx 且错误内容指向上下文超限），
    /// 携带报错里声明的模型上下文上限（如有）。
    RejectedTooLong { declared_limit: Option<u64> },
    /// 超时（含重试后仍超时）。
    TimedOut,
    /// 其他失败（5xx、断连等），不能归因为上下文容量。
    RejectedOther,
}

/// 上下文实测结论。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextConclusion {
    pub status: ContextStatus,
    /// 已验证可接受的最大 token 数（服务端统计优先，否则为构造值）。
    pub floor_tokens: Option<u64>,
    /// floor 是否来自服务端 usage 统计。
    pub floor_is_measured: bool,
    /// 已确认被拒的最小目标 token 数。
    pub ceiling_tokens: Option<u64>,
    /// ceiling 是否来自服务端报错里声明的上限值（而非我们的探针档位）。
    pub ceiling_is_declared: bool,
    /// 无法判断时的原因（其他状态为空）。
    pub reason: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextStatus {
    /// 实测满足 128K 要求。
    Satisfied,
    /// 实测上限不足 128K 要求。
    NotSatisfied,
    /// 超时、疑似截断或其他错误，无法判断。
    Indeterminate,
}

impl ContextConclusion {
    pub fn satisfied_with(
        floor: u64,
        measured: bool,
        ceiling: Option<u64>,
        ceiling_is_declared: bool,
    ) -> Self {
        Self {
            status: ContextStatus::Satisfied,
            floor_tokens: Some(floor),
            floor_is_measured: measured,
            ceiling_tokens: ceiling,
            ceiling_is_declared,
            reason: String::new(),
        }
    }

    pub fn not_satisfied(floor: Option<u64>, ceiling: u64, ceiling_is_declared: bool) -> Self {
        Self {
            status: ContextStatus::NotSatisfied,
            floor_tokens: floor,
            floor_is_measured: false,
            ceiling_tokens: Some(ceiling),
            ceiling_is_declared,
            reason: String::new(),
        }
    }

    pub fn indeterminate(reason: impl Into<String>) -> Self {
        Self {
            status: ContextStatus::Indeterminate,
            floor_tokens: None,
            floor_is_measured: false,
            ceiling_tokens: None,
            ceiling_is_declared: false,
            reason: reason.into(),
        }
    }
}

/// 校准请求的完整 prompt。
pub fn calibration_prompt() -> String {
    format!(
        "MODEL_DOCTOR_CONTEXT_CALIBRATION. This probe measures token density only. \
         Response accuracy is not evaluated. {}The request is complete. Return any \
         short non-empty response.",
        filler(CALIBRATION_FILLER_CHARS)
    )
}

/// 按实测密度构造目标 token 数的探针 prompt。
pub fn capacity_prompt(target_tokens: u64, chars_per_token: f64) -> String {
    const INSTRUCTION: &str = "MODEL_DOCTOR_CONTEXT_CAPACITY. This probe measures context \
                                acceptance only. Response accuracy and exact wording are not \
                                evaluated. ";
    const SUFFIX: &str = " The full request is complete. Return any short non-empty response.";
    let total_chars = (target_tokens as f64 * chars_per_token) as usize;
    let filler_budget = total_chars.saturating_sub(INSTRUCTION.len() + SUFFIX.len());
    format!("{INSTRUCTION}{}{SUFFIX}", filler(filler_budget))
}

fn filler(target_chars: usize) -> String {
    let mut filler = String::with_capacity(target_chars + FILLER_BLOCK.len());
    while filler.len() < target_chars {
        filler.push_str(FILLER_BLOCK);
    }
    filler
}

/// 上下文探针请求：非流式，并显式限制输出 token。
pub fn probe_request(protocol: Protocol, model: &str, prompt: &str) -> RequestSpec {
    let mut request = basic_request(protocol, model, prompt, false);
    let object = request
        .body
        .as_object_mut()
        .expect("basic request bodies are JSON objects");
    match protocol {
        Protocol::OpenAiChat | Protocol::OllamaChat => {
            object.insert("max_tokens".into(), PROBE_MAX_OUTPUT_TOKENS.into());
        }
        Protocol::OpenAiResponses => {
            object.insert("max_output_tokens".into(), PROBE_MAX_OUTPUT_TOKENS.into());
        }
        Protocol::AnthropicMessages => {
            object.insert("max_tokens".into(), PROBE_MAX_OUTPUT_TOKENS.into());
        }
        Protocol::GeminiGenerateContent => {
            let generation = object
                .get_mut("generationConfig")
                .and_then(serde_json::Value::as_object_mut)
                .expect("Gemini requests always contain generationConfig");
            generation.insert("maxOutputTokens".into(), PROBE_MAX_OUTPUT_TOKENS.into());
        }
        Protocol::Unknown => {}
    }
    request
}

/// 容量探针的超时：至少留 60 秒，再按目标 token 数线性加宽。
pub fn probe_timeout(base: Duration, target_tokens: u64) -> Duration {
    let scaled = Duration::from_secs(60 + target_tokens / 1000);
    base.max(scaled)
}

/// 容量探针请求 ID（含尝试序号）。
pub fn probe_request_id(target_tokens: u64, attempt: u32) -> String {
    format!("{PROBE_REQUEST_PREFIX}{target_tokens}-a{attempt}")
}

/// 从请求 ID 解析目标 token 数；校准请求或无关请求返回 None。
pub fn parse_probe_target(request_id: &str) -> Option<u64> {
    let rest = request_id.strip_prefix(PROBE_REQUEST_PREFIX)?;
    let target = rest.split("-a").next()?;
    target.parse().ok()
}

pub fn is_calibration_request(request_id: &str) -> bool {
    request_id == CALIBRATION_REQUEST_ID
}

/// 夹逼搜索的下一个目标；返回 None 表示搜索结束。
pub fn next_target(low: Option<u64>, high: Option<u64>, probes_used: usize) -> Option<u64> {
    if probes_used == 0 {
        return Some(CONTEXT_MAIN_TARGET_TOKENS);
    }
    if probes_used >= MAX_CAPACITY_PROBES {
        return None;
    }
    match (low, high) {
        // 已有接受点和被拒上限：未达 128K 要求时二分逼近真实上限；
        // 要求已满足时区间只用于表述，不再追加长请求。
        (Some(low), Some(high)) => {
            if low >= CONTEXT_MAIN_TARGET_TOKENS || high.saturating_sub(low) <= MIN_INTERVAL_TOKENS
            {
                None
            } else {
                Some((low + high) / 2)
            }
        }
        // 只有被拒上限、还没有任何接受点：向下减半。
        (None, Some(high)) => {
            let next = high / 2;
            (next >= 2_000).then_some(next)
        }
        // 未被拒：向上翻倍找上限，到顶即止。
        (Some(low), None) => {
            if low >= CONTEXT_UPWARD_CAP_TOKENS {
                None
            } else {
                Some((low * 2).min(CONTEXT_UPWARD_CAP_TOKENS))
            }
        }
        (None, None) => Some(CONTEXT_MAIN_TARGET_TOKENS),
    }
}

/// 从证据日志里的 protocol 字段名还原协议。
pub fn protocol_from_wire_name(name: &str) -> Protocol {
    match name {
        "openai_chat" => Protocol::OpenAiChat,
        "openai_responses" => Protocol::OpenAiResponses,
        "anthropic_messages" => Protocol::AnthropicMessages,
        "gemini_generate_content" => Protocol::GeminiGenerateContent,
        "ollama_chat" => Protocol::OllamaChat,
        _ => Protocol::Unknown,
    }
}

/// 从响应体提取服务端统计的 prompt token 数（各协议原生字段）。
pub fn extract_prompt_tokens(protocol: Protocol, body: &[u8]) -> Option<u64> {
    let parsed: serde_json::Value = serde_json::from_slice(body).ok()?;
    let value = match protocol {
        Protocol::OpenAiChat | Protocol::OllamaChat => &parsed["usage"]["prompt_tokens"],
        Protocol::OpenAiResponses => &parsed["usage"]["input_tokens"],
        Protocol::AnthropicMessages => &parsed["usage"]["input_tokens"],
        Protocol::GeminiGenerateContent => &parsed["usageMetadata"]["promptTokenCount"],
        Protocol::Unknown => return None,
    };
    value.as_u64().filter(|tokens| *tokens > 0)
}

/// 判断一个 4xx 响应是否明确以「上下文超长」为由拒绝。
pub fn is_context_length_rejection(http_status: Option<u16>, body: &[u8]) -> bool {
    let is_client_error = http_status.is_some_and(|status| (400..500).contains(&status));
    if !is_client_error {
        return false;
    }
    let text = String::from_utf8_lossy(body).to_lowercase();
    [
        "context_length_exceeded",
        "maximum context length",
        "context length",
        "context window",
        "max_model_len",
        "prompt is too long",
        "input is too long",
        "too many tokens",
        "token limit",
        "prompt token",
        "input token count",
        "maximum number of tokens",
        "reduce the length",
    ]
    .iter()
    .any(|pattern| text.contains(pattern))
}

/// 从超长拒绝报错里提取服务端声明的上下文上限（token 数）。
///
/// 各家都会在报错里带上自己的上限值，例如：
/// - OpenAI / vLLM / SGLang：`maximum context length is 262144 tokens`、
///   `exceeds 'max_model_len' (262144)`
/// - Anthropic：`prompt is too long: 200001 tokens > 200000 maximum`
/// - Gemini：`exceeds the maximum number of tokens allowed (128000)`
/// - Ollama：`prompt must have at most 131072 tokens`
///
/// 只锚定上限侧的关键词，避免把「you requested 320000」这类请求侧数字当成上限。
/// 多处命中时取最小值（保守）。
pub fn extract_declared_context_limit(body: &[u8]) -> Option<u64> {
    let text = String::from_utf8_lossy(body).to_lowercase();
    let mut candidates: Vec<u64> = Vec::new();

    // "maximum context length is 262144"
    if let Some(value) = number_after(&text, "maximum context length is ") {
        candidates.push(value);
    }
    // "context window ... limit of 262144"? 罕见，跳过。
    // "exceeds 'max_model_len' (262144)" / "max_model_len limit 262144"
    if let Some(index) = text.find("max_model_len") {
        if let Some(value) = first_number_in(&text[index + "max_model_len".len()..]) {
            candidates.push(value);
        }
    }
    // Anthropic: "... tokens > 200000 maximum"
    if let Some(index) = text.find(" maximum") {
        if let Some(value) = number_before(&text[..index]) {
            candidates.push(value);
        }
    }
    // Gemini: "maximum number of tokens allowed (128000)"
    if let Some(value) = number_after(&text, "maximum number of tokens allowed ") {
        candidates.push(value);
    }
    // Ollama: "at most 131072 tokens"
    if let Some(value) = number_after(&text, "at most ") {
        candidates.push(value);
    }
    // Bedrock/其他： "exceed max tokens 200000"
    if let Some(value) = number_after(&text, "max tokens ") {
        candidates.push(value);
    }

    // 合理上限范围：8K 以上、16M 以下，过滤误抓的请求侧/无关数字。
    candidates
        .into_iter()
        .filter(|value| (8_000..=16_000_000).contains(value))
        .min()
}

/// 返回锚点之后第一段数字（跳过非数字字符，但不超过锚点后 32 个字符，
/// 避免跨句子误抓）。
fn number_after(text: &str, anchor: &str) -> Option<u64> {
    let index = text.find(anchor)?;
    let rest = &text[index + anchor.len()..];
    let window = &rest[..rest.len().min(32)];
    first_number_in(window)
}

/// 返回锚点之前紧邻的一段数字（跳过空白）。
fn number_before(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && bytes[start - 1].is_ascii_digit() {
        start -= 1;
    }
    if start == end {
        return None;
    }
    text[start..end].parse().ok()
}

fn first_number_in(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor].is_ascii_digit() {
            let start = cursor;
            while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                cursor += 1;
            }
            let value: u64 = text[start..cursor].parse().ok()?;
            // 带逗分的大数（如 1,048,576）继续拼接。
            if cursor + 1 < bytes.len()
                && bytes[cursor] == b','
                && bytes[cursor + 1].is_ascii_digit()
            {
                let mut merged = value.to_string();
                let mut scan = cursor;
                while scan + 1 < bytes.len()
                    && bytes[scan] == b','
                    && bytes[scan + 1].is_ascii_digit()
                {
                    let digits_start = scan + 1;
                    let mut digits_end = digits_start;
                    while digits_end < bytes.len() && bytes[digits_end].is_ascii_digit() {
                        digits_end += 1;
                    }
                    merged.push_str(&text[digits_start..digits_end]);
                    scan = digits_end;
                }
                return merged.parse().ok();
            }
            return Some(value);
        }
        cursor += 1;
    }
    None
}

/// 用与采集端相同的规则对单条证据分类。
pub fn classify_probe(
    protocol: Protocol,
    http_status: Option<u16>,
    transport_outcome: &str,
    response_body: &[u8],
) -> ProbeOutcome {
    let transport_success = matches!(transport_outcome, "completed_eof" | "protocol_terminated");
    let http_success = http_status.is_some_and(|status| (200..300).contains(&status));
    let body_non_empty = std::str::from_utf8(response_body)
        .map(|text| !text.trim().is_empty())
        .unwrap_or(!response_body.is_empty());
    if response_is_length_limited(protocol, response_body) {
        return ProbeOutcome::RejectedOther;
    }
    if http_success && transport_success && body_non_empty {
        return ProbeOutcome::Accepted {
            measured_tokens: extract_prompt_tokens(protocol, response_body),
        };
    }
    if transport_outcome == "timeout" {
        return ProbeOutcome::TimedOut;
    }
    if is_context_length_rejection(http_status, response_body) {
        return ProbeOutcome::RejectedTooLong {
            declared_limit: extract_declared_context_limit(response_body),
        };
    }
    ProbeOutcome::RejectedOther
}

fn response_is_length_limited(protocol: Protocol, response_body: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(response_body) else {
        return false;
    };
    match protocol {
        Protocol::OpenAiChat => {
            value
                .pointer("/choices/0/finish_reason")
                .and_then(serde_json::Value::as_str)
                == Some("length")
        }
        Protocol::OpenAiResponses => {
            value.get("status").and_then(serde_json::Value::as_str) == Some("incomplete")
                || value
                    .pointer("/incomplete_details/reason")
                    .and_then(serde_json::Value::as_str)
                    == Some("max_output_tokens")
        }
        Protocol::AnthropicMessages => {
            value.get("stop_reason").and_then(serde_json::Value::as_str) == Some("max_tokens")
        }
        Protocol::GeminiGenerateContent => {
            value
                .pointer("/candidates/0/finishReason")
                .and_then(serde_json::Value::as_str)
                == Some("MAX_TOKENS")
        }
        Protocol::OllamaChat => {
            value.get("done_reason").and_then(serde_json::Value::as_str) == Some("length")
        }
        Protocol::Unknown => false,
    }
}

/// 把 token 数格式化为报告用语（124012 → "124K"，15500 → "15.5K"）。
pub fn format_tokens_k(tokens: u64) -> String {
    let thousands = tokens as f64 / 1000.0;
    if thousands >= 100.0 || (thousands - thousands.round()).abs() < f64::EPSILON {
        format!("{}K", thousands.round() as u64)
    } else {
        format!("{thousands:.1}K")
    }
}

/// 生成「经检测，上下文在XXK-XXK左右，满足/不满足…」格式的说明列。
pub fn conclusion_sentence(conclusion: &ContextConclusion) -> String {
    match conclusion.status {
        ContextStatus::Satisfied => {
            let floor = format_tokens_k(conclusion.floor_tokens.unwrap_or_default());
            let estimate_note = if conclusion.floor_is_measured {
                ""
            } else {
                "（服务端未返回 usage，按构造值估计）"
            };
            match conclusion.ceiling_tokens {
                Some(ceiling) => format!(
                    "经检测，上下文在{floor}-{}左右，满足智能体部署所需要的128K的要求{estimate_note}。",
                    format_tokens_k(ceiling)
                ),
                None => format!(
                    "经检测，上下文不低于{floor}{estimate_note}，满足智能体部署所需要的128K的要求。"
                ),
            }
        }
        ContextStatus::NotSatisfied => {
            let ceiling = format_tokens_k(conclusion.ceiling_tokens.unwrap_or_default());
            match conclusion.floor_tokens {
                Some(floor) => format!(
                    "经检测，上下文在{}-{ceiling}左右，不满足智能体部署所需要的128K的要求。",
                    format_tokens_k(floor)
                ),
                None if conclusion.ceiling_is_declared => format!(
                    "经检测，上下文上限为服务端声明的{ceiling}，不满足智能体部署所需要的128K的要求。"
                ),
                None => {
                    format!("经检测，上下文上限低于{ceiling}，不满足智能体部署所需要的128K的要求。")
                }
            }
        }
        ContextStatus::Indeterminate => {
            format!("无法判断：{}。本轮不计通过或不通过。", conclusion.reason)
        }
    }
}

/// 由全部探针结果推导最终结论。这是采集端和分析端共用的唯一判定入口。
pub fn conclude(outcomes: &[(u64, ProbeOutcome)]) -> ContextConclusion {
    let mut floor: Option<(u64, bool)> = None;
    let mut target_ceiling: Option<u64> = None;
    let mut declared_min: Option<u64> = None;
    let mut truncation: Option<(u64, u64)> = None;
    let mut timed_out_at: Option<u64> = None;
    let mut other_error_at: Option<u64> = None;

    for &(target, outcome) in outcomes {
        match outcome {
            ProbeOutcome::Accepted { measured_tokens } => {
                let effective = measured_tokens.unwrap_or(target);
                if floor.is_none_or(|(current, _)| effective > current) {
                    floor = Some((effective, measured_tokens.is_some()));
                }
                if let Some(measured) = measured_tokens
                    && measured + MIN_INTERVAL_TOKENS < target
                {
                    truncation = Some((target, measured));
                }
            }
            ProbeOutcome::RejectedTooLong { declared_limit } => {
                target_ceiling =
                    Some(target_ceiling.map_or(target, |current: u64| current.min(target)));
                if let Some(declared) = declared_limit {
                    declared_min =
                        Some(declared_min.map_or(declared, |current| current.min(declared)));
                }
            }
            ProbeOutcome::TimedOut => timed_out_at = Some(target),
            ProbeOutcome::RejectedOther => other_error_at = Some(target),
        }
    }

    // 上界取两者中更紧的：探针档位（实测被拒）与服务端声明值。
    let ceiling = match (target_ceiling, declared_min) {
        (Some(target), Some(declared)) => Some((target.min(declared), declared <= target)),
        (None, Some(declared)) => Some((declared, true)),
        (Some(target), None) => Some((target, false)),
        (None, None) => None,
    };

    if let Some((target, measured)) = truncation {
        return ContextConclusion::indeterminate(format!(
            "目标{}的请求被接受，但服务端只统计到{}输入token，疑似网关截断",
            format_tokens_k(target),
            format_tokens_k(measured)
        ));
    }
    if floor.is_some_and(|(tokens, _)| tokens >= CONTEXT_MAIN_TARGET_TOKENS) {
        let (tokens, measured) = floor.expect("checked above");
        let (ceiling_tokens, ceiling_is_declared) =
            ceiling.map_or((None, false), |(value, declared)| (Some(value), declared));
        return ContextConclusion::satisfied_with(
            tokens,
            measured,
            ceiling_tokens,
            ceiling_is_declared,
        );
    }
    // 未达标：有被拒上限就能给出实测区间。
    if let Some((ceiling, ceiling_is_declared)) = ceiling {
        let floor_tokens = floor.map(|(tokens, _)| tokens);
        return ContextConclusion::not_satisfied(floor_tokens, ceiling, ceiling_is_declared);
    }
    // 没有被拒上限：按失败方式解释为什么无法判断。
    if let Some(target) = timed_out_at {
        return ContextConclusion::indeterminate(format!(
            "{}的请求连续超时",
            format_tokens_k(target)
        ));
    }
    if let Some(target) = other_error_at {
        return ContextConclusion::indeterminate(format!(
            "{}的请求返回了与上下文长度无关的错误",
            format_tokens_k(target)
        ));
    }
    match floor {
        Some((tokens, _)) => ContextConclusion::indeterminate(format!(
            "仅验证到{}，未完成 128K 档位判断",
            format_tokens_k(tokens)
        )),
        None => ContextConclusion::indeterminate("未获得任何容量探针结果"),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn calibration_and_capacity_prompts_carry_distinct_markers() {
        let calibration = calibration_prompt();
        assert!(calibration.contains("MODEL_DOCTOR_CONTEXT_CALIBRATION"));
        assert!(calibration.len() >= CALIBRATION_FILLER_CHARS);

        let capacity = capacity_prompt(10_000, 4.0);
        assert!(capacity.contains("MODEL_DOCTOR_CONTEXT_CAPACITY"));
        assert!(capacity.contains("Return any short non-empty response"));
        assert!(capacity.len() >= 40_000, "{}", capacity.len());
    }

    #[test]
    fn capacity_prompt_scales_with_measured_density() {
        let dense = capacity_prompt(100_000, 2.0).len();
        let sparse = capacity_prompt(100_000, 4.0).len();
        assert!(
            (sparse as f64 / dense as f64 - 2.0).abs() < 0.1,
            "dense={dense} sparse={sparse}"
        );
    }

    #[test]
    fn probe_request_caps_output_tokens_for_each_protocol() {
        for protocol in [
            Protocol::OpenAiChat,
            Protocol::OpenAiResponses,
            Protocol::AnthropicMessages,
            Protocol::GeminiGenerateContent,
            Protocol::OllamaChat,
        ] {
            let body = probe_request(protocol, "fixture-model", "prompt").body;
            let encoded = serde_json::to_string(&body).unwrap();
            assert!(
                encoded.contains("16"),
                "{protocol} probe did not cap output tokens: {encoded}"
            );
            assert!(!encoded.contains("\"stream\":true"));
        }
    }

    #[test]
    fn probe_timeout_scales_and_never_shrinks_the_base() {
        let base = Duration::from_secs(300);
        assert_eq!(probe_timeout(base, 124_000), Duration::from_secs(300));
        assert_eq!(
            probe_timeout(Duration::from_secs(10), 124_000),
            Duration::from_secs(184)
        );
        assert_eq!(
            probe_timeout(Duration::from_secs(10), 0),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn probe_request_ids_round_trip_their_targets() {
        assert_eq!(
            parse_probe_target("test-018-probe-124000-a1"),
            Some(124_000)
        );
        assert_eq!(parse_probe_target("test-018-probe-62000-a2"), Some(62_000));
        assert_eq!(parse_probe_target(CALIBRATION_REQUEST_ID), None);
        assert_eq!(parse_probe_target("test-019"), None);
        assert_eq!(probe_request_id(124_000, 2), "test-018-probe-124000-a2");
    }

    #[test]
    fn search_starts_at_the_bar_then_brackets_or_doubles() {
        assert_eq!(next_target(None, None, 0), Some(CONTEXT_MAIN_TARGET_TOKENS));
        // 未被拒：从接受点向上翻倍。
        assert_eq!(next_target(Some(124_000), None, 1), Some(248_000));
        assert_eq!(
            next_target(Some(248_000), None, 2),
            Some(CONTEXT_UPWARD_CAP_TOKENS)
        );
        assert_eq!(next_target(Some(496_000), None, 3), None);
        // 被拒：二分夹逼。
        assert_eq!(next_target(Some(5_000), Some(124_000), 1), Some(64_500));
        assert_eq!(next_target(Some(64_500), Some(124_000), 2), Some(94_250));
        // 区间足够窄就停。
        assert_eq!(next_target(Some(116_000), Some(124_000), 3), None);
        // 要求已满足时，向上探针被拒即停，不再细分高位区间。
        assert_eq!(next_target(Some(248_000), Some(496_000), 3), None);
        // 无接受点时向下减半。
        assert_eq!(next_target(None, Some(124_000), 1), Some(62_000));
        // 探针预算用尽即停。
        assert_eq!(next_target(Some(116_000), Some(124_000), 5), None);
    }

    #[test]
    fn prompt_tokens_are_extracted_from_each_protocol_usage_shape() {
        let chat = br#"{"usage":{"prompt_tokens":160069,"total_tokens":160080}}"#;
        assert_eq!(
            extract_prompt_tokens(Protocol::OpenAiChat, chat),
            Some(160_069)
        );
        let responses = br#"{"usage":{"input_tokens":42}}"#;
        assert_eq!(
            extract_prompt_tokens(Protocol::OpenAiResponses, responses),
            Some(42)
        );
        let anthropic = br#"{"usage":{"input_tokens":42}}"#;
        assert_eq!(
            extract_prompt_tokens(Protocol::AnthropicMessages, anthropic),
            Some(42)
        );
        let gemini = br#"{"usageMetadata":{"promptTokenCount":42}}"#;
        assert_eq!(
            extract_prompt_tokens(Protocol::GeminiGenerateContent, gemini),
            Some(42)
        );
        assert_eq!(
            extract_prompt_tokens(Protocol::OpenAiChat, br#"{"usage":null}"#),
            None
        );
        assert_eq!(
            extract_prompt_tokens(Protocol::OpenAiChat, b"not json"),
            None
        );
    }

    #[test]
    fn context_length_rejections_are_recognized_across_vendors() {
        let vllm = br#"{"error":{"message":"This model's maximum context length is 262144 tokens. However, you requested 320000 tokens","code":400}}"#;
        assert!(is_context_length_rejection(Some(400), vllm));

        let openai = br#"{"error":{"code":"context_length_exceeded","message":"This model's maximum context length is 128000 tokens."}}"#;
        assert!(is_context_length_rejection(Some(400), openai));

        let anthropic = br#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 200001 tokens > 200000 maximum"}}"#;
        assert!(is_context_length_rejection(Some(400), anthropic));

        // 无关错误不算超长。
        let bad_model = br#"{"error":{"message":"model not found","code":404}}"#;
        assert!(!is_context_length_rejection(Some(404), bad_model));
        // 5xx 不算超长拒绝。
        let crashed = br#"{"error":{"message":"maximum context length exceeded"}}"#;
        assert!(!is_context_length_rejection(Some(500), crashed));
    }

    #[test]
    fn probes_are_classified_from_wire_observations() {
        let accepted = classify_probe(
            Protocol::OpenAiChat,
            Some(200),
            "completed_eof",
            br#"{"usage":{"prompt_tokens":124012}}"#,
        );
        assert_eq!(
            accepted,
            ProbeOutcome::Accepted {
                measured_tokens: Some(124_012)
            }
        );

        let timeout = classify_probe(Protocol::OpenAiChat, None, "timeout", b"");
        assert_eq!(timeout, ProbeOutcome::TimedOut);

        let too_long = classify_probe(
            Protocol::OpenAiChat,
            Some(400),
            "completed_eof",
            br#"{"error":{"message":"prompt is too long"}}"#,
        );
        assert_eq!(
            too_long,
            ProbeOutcome::RejectedTooLong {
                declared_limit: None
            }
        );

        let crashed = classify_probe(
            Protocol::OpenAiChat,
            Some(500),
            "completed_eof",
            br#"{"error":"internal"}"#,
        );
        assert_eq!(crashed, ProbeOutcome::RejectedOther);
    }

    #[test]
    fn length_limited_response_is_not_treated_as_accepted_probe() {
        let response = br#"{
            "choices":[{
                "message":{"role":"assistant","content":"partial"},
                "finish_reason":"length"
            }],
            "usage":{"prompt_tokens":131072}
        }"#;

        assert_eq!(
            classify_probe(Protocol::OpenAiChat, Some(200), "completed_eof", response,),
            ProbeOutcome::RejectedOther
        );
    }

    #[test]
    fn token_counts_format_as_report_kilounits() {
        assert_eq!(format_tokens_k(124_000), "124K");
        assert_eq!(format_tokens_k(96_000), "96K");
        assert_eq!(format_tokens_k(15_500), "15.5K");
        assert_eq!(format_tokens_k(1_000), "1K");
    }

    #[test]
    fn conclude_reports_satisfied_range_from_measured_floor() {
        let conclusion = conclude(&[
            (
                124_000,
                ProbeOutcome::Accepted {
                    measured_tokens: Some(124_012),
                },
            ),
            (
                248_000,
                ProbeOutcome::Accepted {
                    measured_tokens: None,
                },
            ),
            (
                496_000,
                ProbeOutcome::RejectedTooLong {
                    declared_limit: None,
                },
            ),
        ]);

        assert_eq!(conclusion.status, ContextStatus::Satisfied);
        assert_eq!(conclusion.floor_tokens, Some(248_000));
        assert_eq!(conclusion.ceiling_tokens, Some(496_000));
        let sentence = conclusion_sentence(&conclusion);
        assert_eq!(
            sentence,
            "经检测，上下文在248K-496K左右，满足智能体部署所需要的128K的要求（服务端未返回 usage，按构造值估计）。"
        );
    }

    #[test]
    fn conclude_reports_satisfied_floor_without_ceiling() {
        let conclusion = conclude(&[(
            496_000,
            ProbeOutcome::Accepted {
                measured_tokens: Some(500_000),
            },
        )]);

        assert_eq!(conclusion.status, ContextStatus::Satisfied);
        assert!(conclusion.floor_is_measured);
        assert!(conclusion.ceiling_tokens.is_none());
        assert_eq!(
            conclusion_sentence(&conclusion),
            "经检测，上下文不低于500K，满足智能体部署所需要的128K的要求。"
        );
    }

    #[test]
    fn conclude_marks_estimated_floor_when_usage_is_missing() {
        let conclusion = conclude(&[(
            124_000,
            ProbeOutcome::Accepted {
                measured_tokens: None,
            },
        )]);

        assert_eq!(conclusion.status, ContextStatus::Satisfied);
        assert!(!conclusion.floor_is_measured);
        assert!(conclusion_sentence(&conclusion).contains("按构造值估计"));
    }

    #[test]
    fn conclude_reports_bracket_below_the_requirement() {
        let conclusion = conclude(&[
            (
                124_000,
                ProbeOutcome::RejectedTooLong {
                    declared_limit: None,
                },
            ),
            (
                62_000,
                ProbeOutcome::Accepted {
                    measured_tokens: Some(62_100),
                },
            ),
            (
                93_000,
                ProbeOutcome::RejectedTooLong {
                    declared_limit: None,
                },
            ),
        ]);

        assert_eq!(conclusion.status, ContextStatus::NotSatisfied);
        assert_eq!(conclusion.floor_tokens, Some(62_100));
        assert_eq!(conclusion.ceiling_tokens, Some(93_000));
        assert_eq!(
            conclusion_sentence(&conclusion),
            "经检测，上下文在62.1K-93K左右，不满足智能体部署所需要的128K的要求。"
        );
    }

    #[test]
    fn conclude_flags_suspected_truncation() {
        let conclusion = conclude(&[(
            124_000,
            ProbeOutcome::Accepted {
                measured_tokens: Some(4_000),
            },
        )]);

        assert_eq!(conclusion.status, ContextStatus::Indeterminate);
        assert!(conclusion_sentence(&conclusion).contains("疑似网关截断"));
    }

    #[test]
    fn conclude_treats_timeouts_as_indeterminate() {
        let conclusion = conclude(&[(124_000, ProbeOutcome::TimedOut)]);

        assert_eq!(conclusion.status, ContextStatus::Indeterminate);
        assert!(conclusion_sentence(&conclusion).contains("连续超时"));
    }

    #[test]
    fn conclude_treats_unrelated_errors_as_indeterminate() {
        let conclusion = conclude(&[(124_000, ProbeOutcome::RejectedOther)]);

        assert_eq!(conclusion.status, ContextStatus::Indeterminate);
        assert!(conclusion_sentence(&conclusion).contains("无关的错误"));
    }

    #[test]
    fn declared_limits_are_extracted_from_each_vendor_error_shape() {
        let vllm = br#"{"error":{"message":"This model's maximum context length is 262144 tokens. However, you requested 320000 tokens (including 16 for the generated output), which exceeds the context length of 262144","code":400}}"#;
        assert_eq!(extract_declared_context_limit(vllm), Some(262_144));

        let anthropic = br#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 200001 tokens > 200000 maximum"}}"#;
        assert_eq!(extract_declared_context_limit(anthropic), Some(200_000));

        let gemini =
            b"The input token count (128001) exceeds the maximum number of tokens allowed (128000)";
        assert_eq!(extract_declared_context_limit(gemini), Some(128_000));

        let ollama = b"prompt must have at most 131072 tokens but found 131073";
        assert_eq!(extract_declared_context_limit(ollama), Some(131_072));

        let sglang = br#"{"object":"error","message":"The total number of tokens (320016) exceeds max_model_len limit 262144"}"#;
        assert_eq!(extract_declared_context_limit(sglang), Some(262_144));

        // 无关报错提取不出上限。
        assert_eq!(
            extract_declared_context_limit(br#"{"error":{"message":"model not found"}}"#),
            None
        );
    }

    #[test]
    fn classify_probe_carries_the_declared_limit() {
        let outcome = classify_probe(
            Protocol::OpenAiChat,
            Some(400),
            "completed_eof",
            br#"{"error":{"message":"This model's maximum context length is 96000 tokens. However, you requested 124016 tokens"}}"#,
        );

        assert_eq!(
            outcome,
            ProbeOutcome::RejectedTooLong {
                declared_limit: Some(96_000)
            }
        );
    }

    #[test]
    fn conclude_uses_the_declared_limit_as_the_tighter_ceiling() {
        let conclusion = conclude(&[
            (
                124_000,
                ProbeOutcome::Accepted {
                    measured_tokens: Some(124_012),
                },
            ),
            (
                248_000,
                ProbeOutcome::Accepted {
                    measured_tokens: Some(248_030),
                },
            ),
            (
                496_000,
                ProbeOutcome::RejectedTooLong {
                    declared_limit: Some(262_144),
                },
            ),
        ]);

        assert_eq!(conclusion.status, ContextStatus::Satisfied);
        assert_eq!(conclusion.floor_tokens, Some(248_030));
        assert_eq!(conclusion.ceiling_tokens, Some(262_144));
        assert!(conclusion.ceiling_is_declared);
        assert_eq!(
            conclusion_sentence(&conclusion),
            "经检测，上下文在248K-262K左右，满足智能体部署所需要的128K的要求。"
        );
    }

    #[test]
    fn conclude_reports_declared_limit_when_only_rejections_exist() {
        let conclusion = conclude(&[(
            124_000,
            ProbeOutcome::RejectedTooLong {
                declared_limit: Some(96_000),
            },
        )]);

        assert_eq!(conclusion.status, ContextStatus::NotSatisfied);
        assert_eq!(conclusion.ceiling_tokens, Some(96_000));
        assert!(conclusion.ceiling_is_declared);
        assert_eq!(
            conclusion_sentence(&conclusion),
            "经检测，上下文上限为服务端声明的96K，不满足智能体部署所需要的128K的要求。"
        );
    }

    #[test]
    fn conclude_handles_rejection_without_any_accepted_probe() {
        let conclusion = conclude(&[
            (
                124_000,
                ProbeOutcome::RejectedTooLong {
                    declared_limit: None,
                },
            ),
            (
                62_000,
                ProbeOutcome::RejectedTooLong {
                    declared_limit: None,
                },
            ),
        ]);

        assert_eq!(conclusion.status, ContextStatus::NotSatisfied);
        assert_eq!(conclusion.ceiling_tokens, Some(62_000));
        assert!(conclusion_sentence(&conclusion).contains("低于62K"));
    }
}
