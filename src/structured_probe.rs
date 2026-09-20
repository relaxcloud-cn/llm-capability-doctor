use serde_json::{Value, json};

use crate::context_probe;
use crate::transport::ChatCompletionsResponse;

/// S05 结构化输出实测的固定样本（2 个，每样本 1 次请求）：
/// json_object 模式、json_schema 模式。
pub const SAMPLE_IDS: [&str; 2] = ["json", "schema"];

/// 两模式共用的固定短输入（业务无关）。
const FIXED_PROMPT: &str = "请把下面固定资料表示为 JSON：城市为“杭州”，温度为 23，是否下雨为 false，标签为 [\"沿海\", \"春季\"]。";

#[derive(Debug, Clone)]
pub struct StructuredSample {
    pub id: &'static str,
    /// 不设格式参数的对照样本为 None。
    pub response_format: Option<Value>,
}

/// 冻结 schema 子集：四字段必填、city 非空、temperature [-80,70]、
/// tags 为字符串数组且 ≤4 项、禁止额外字段。
fn fixed_schema() -> Value {
    json!({
        "name": "city_weather",
        "strict": true,
        "schema": {
            "type": "object",
            "properties": {
                "city": {"type": "string", "minLength": 1},
                "temperature": {"type": "number", "minimum": -80, "maximum": 70},
                "raining": {"type": "boolean"},
                "tags": {"type": "array", "items": {"type": "string"}, "maxItems": 4}
            },
            "required": ["city", "temperature", "raining", "tags"],
            "additionalProperties": false
        }
    })
}

pub fn samples() -> Vec<StructuredSample> {
    vec![
        StructuredSample {
            id: "json",
            response_format: Some(json!({"type": "json_object"})),
        },
        StructuredSample {
            id: "schema",
            response_format: Some(json!({
                "type": "json_schema",
                "json_schema": fixed_schema()
            })),
        },
    ]
}

/// 构造 Chat Completions 请求体；对照样本不带 response_format。
pub fn request_payload(sample: &StructuredSample, model: &str) -> Value {
    let mut payload = json!({
        "model": model,
        "messages": [{"role": "user", "content": FIXED_PROMPT}],
        "temperature": 0,
        "max_tokens": 256,
        "stream": false,
    });
    if let Some(format) = &sample.response_format {
        payload["response_format"] = format.clone();
    }
    payload
}

#[derive(Debug, Clone, PartialEq)]
pub enum StructuredOutcome {
    /// 生成内容满足该模式要求。
    Pass { detail: String },
    /// 有效响应但内容不合规（JSON 不可解析或 schema 不符，定位字段路径）。
    Violation { detail: String },
    /// 服务明确拒绝 response_format/该结构化模式。
    Unsupported { detail: String },
    /// HTTP 2xx 但响应结构不可识别。
    Malformed { detail: String },
    /// 传输失败、内容缺失或不可归因的拒绝。
    Inconclusive { detail: String },
}

/// 单样本判定：
/// - 传输失败 → Inconclusive；
/// - 非 2xx：报文明确指向 response_format/结构化模式被拒 → Unsupported；其余 → Inconclusive；
/// - 2xx 但 choices[0].message 缺失 → Malformed；
/// - content 缺失/为空 → Inconclusive；
/// - 其余按各样本规则解析 content 原文（禁止修补后再判）。
pub fn classify(
    sample: &StructuredSample,
    response: &ChatCompletionsResponse,
) -> StructuredOutcome {
    if let Some(error) = &response.error {
        return StructuredOutcome::Inconclusive {
            detail: format!("传输失败：{error}"),
        };
    }
    let status = response.status.unwrap_or(0);
    if !(200..300).contains(&status) {
        if is_format_rejection(&response.body) {
            return StructuredOutcome::Unsupported {
                detail: format!(
                    "HTTP {status}：服务拒绝结构化模式：{}",
                    context_probe::excerpt(&response.body, 200)
                ),
            };
        }
        return StructuredOutcome::Inconclusive {
            detail: format!("HTTP {status}，非可归因结构化能力拒绝"),
        };
    }
    let Some(content) = response
        .parsed
        .as_ref()
        .and_then(|parsed| parsed.get("choices"))
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
    else {
        let detail = if response
            .parsed
            .as_ref()
            .and_then(|p| p.get("choices"))
            .is_none()
        {
            "HTTP 成功但响应缺少 choices[0].message"
        } else {
            "message.content 缺失或为空，无法判定生成内容"
        };
        return if detail.starts_with("HTTP") {
            StructuredOutcome::Malformed {
                detail: detail.into(),
            }
        } else {
            StructuredOutcome::Inconclusive {
                detail: detail.into(),
            }
        };
    };
    if content.trim().is_empty() {
        return StructuredOutcome::Inconclusive {
            detail: "message.content 为空".into(),
        };
    }
    let parsed_content = serde_json::from_str::<Value>(content);
    match sample.id {
        "json" => match parsed_content {
            Ok(_) => StructuredOutcome::Pass {
                detail: "content 原文可被标准 JSON 解析".into(),
            },
            Err(error) => StructuredOutcome::Violation {
                detail: format!("content 原文不是合法 JSON：{error}"),
            },
        },
        "schema" => match parsed_content {
            Err(error) => StructuredOutcome::Violation {
                detail: format!("content 原文不是合法 JSON：{error}"),
            },
            Ok(value) => match validate_schema(&value) {
                Ok(()) => StructuredOutcome::Pass {
                    detail: "content 满足冻结 schema 全部约束".into(),
                },
                Err(path_error) => StructuredOutcome::Violation {
                    detail: format!("JSON 合法但结构不符：{path_error}"),
                },
            },
        },
        _ => StructuredOutcome::Inconclusive {
            detail: format!("未知样本 {}", sample.id),
        },
    }
}

/// 非 2xx 报文是否明确指向结构化模式被拒。
fn is_format_rejection(body: &str) -> bool {
    let lower = body.to_lowercase();
    let mentions_format = lower.contains("response_format")
        || lower.contains("json_schema")
        || lower.contains("json_object")
        || lower.contains("structured")
        || lower.contains("结构化");
    const REJECTION: &[&str] = &[
        "not support",
        "unsupported",
        "unknown field",
        "unrecognized",
        "invalid parameter",
        "does not support",
        "not allowed",
        "不支持",
        "无法识别",
        "未识别",
    ];
    mentions_format && REJECTION.iter().any(|signal| lower.contains(signal))
}

/// 冻结 schema 子集校验；错误定位到字段路径。
fn validate_schema(value: &Value) -> Result<(), String> {
    let object = value.as_object().ok_or("根值不是 JSON 对象")?;
    const REQUIRED: [&str; 4] = ["city", "temperature", "raining", "tags"];
    for key in REQUIRED {
        if !object.contains_key(key) {
            return Err(format!("{key} 缺失"));
        }
    }
    if object["city"].as_str().is_none_or(str::is_empty) {
        return Err("city 应为非空字符串".into());
    }
    match object["temperature"].as_f64() {
        Some(temperature) if (-80.0..=70.0).contains(&temperature) => {}
        Some(_) => return Err("temperature 超出 [-80,70] 范围".into()),
        None => return Err("temperature 应为数值".into()),
    }
    if !object["raining"].is_boolean() {
        return Err("raining 应为布尔值".into());
    }
    match object["tags"].as_array() {
        None => return Err("tags 应为数组".into()),
        Some(tags) => {
            if tags.len() > 4 {
                return Err("tags 超过 4 项上限".into());
            }
            if let Some(index) = tags.iter().position(|tag| !tag.is_string()) {
                return Err(format!("tags[{index}] 应为字符串"));
            }
        }
    }
    if let Some(extra) = object.keys().find(|key| !REQUIRED.contains(&key.as_str())) {
        return Err(format!("{extra} 为 schema 外多余字段"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::ChatCompletionsResponse;

    fn sample(id: &str) -> StructuredSample {
        samples().into_iter().find(|s| s.id == id).unwrap()
    }

    fn ok_response(content: &str) -> ChatCompletionsResponse {
        let parsed =
            json!({"choices": [{"message": {"content": content}, "finish_reason": "stop"}]});
        ChatCompletionsResponse {
            status: Some(200),
            body: parsed.to_string(),
            parsed: Some(parsed),
            elapsed_ms: 1,
            error: None,
            attempts: 1,
            retry_reasons: Vec::new(),
        }
    }

    fn error_response(status: u16, body: &str) -> ChatCompletionsResponse {
        ChatCompletionsResponse {
            status: Some(status),
            body: body.into(),
            parsed: None,
            elapsed_ms: 1,
            error: None,
            attempts: 1,
            retry_reasons: Vec::new(),
        }
    }

    #[test]
    fn json_mode_parses_raw_content_only() {
        assert!(matches!(
            classify(&sample("json"), &ok_response("{\"city\":\"杭州\"}")),
            StructuredOutcome::Pass { .. }
        ));
        // 代码围栏包裹的原文不是合法 JSON，按失败处理（禁止修补后判过）。
        let fenced = "```json\n{\"city\":\"杭州\"}\n```";
        assert!(matches!(
            classify(&sample("json"), &ok_response(fenced)),
            StructuredOutcome::Violation { .. }
        ));
    }

    #[test]
    fn schema_mode_validates_all_constraints() {
        let good = r#"{"city":"杭州","temperature":23,"raining":false,"tags":["沿海","春季"]}"#;
        assert!(matches!(
            classify(&sample("schema"), &ok_response(good)),
            StructuredOutcome::Pass { .. }
        ));

        let missing = r#"{"city":"杭州","temperature":23,"raining":false}"#;
        let outcome = classify(&sample("schema"), &ok_response(missing));
        assert!(
            matches!(outcome, StructuredOutcome::Violation { ref detail } if detail.contains("tags 缺失"))
        );

        let wrong_type = r#"{"city":"杭州","temperature":"23","raining":false,"tags":[]}"#;
        let outcome = classify(&sample("schema"), &ok_response(wrong_type));
        assert!(
            matches!(outcome, StructuredOutcome::Violation { ref detail } if detail.contains("temperature"))
        );

        let extra = r#"{"city":"杭州","temperature":23,"raining":false,"tags":[],"source":"x"}"#;
        let outcome = classify(&sample("schema"), &ok_response(extra));
        assert!(
            matches!(outcome, StructuredOutcome::Violation { ref detail } if detail.contains("source"))
        );
    }

    #[test]
    fn format_rejection_is_unsupported() {
        let response = error_response(
            400,
            r#"{"error":{"message":"response_format json_schema is not supported"}}"#,
        );
        assert!(matches!(
            classify(&sample("schema"), &response),
            StructuredOutcome::Unsupported { .. }
        ));
    }

    #[test]
    fn generic_error_and_missing_content_are_inconclusive() {
        assert!(matches!(
            classify(
                &sample("schema"),
                &error_response(401, r#"{"error":"bad key"}"#)
            ),
            StructuredOutcome::Inconclusive { .. }
        ));
        let parsed = json!({"choices": [{"message": {}, "finish_reason": "stop"}]});
        let response = ChatCompletionsResponse {
            status: Some(200),
            body: parsed.to_string(),
            parsed: Some(parsed),
            elapsed_ms: 1,
            error: None,
            attempts: 1,
            retry_reasons: Vec::new(),
        };
        assert!(matches!(
            classify(&sample("json"), &response),
            StructuredOutcome::Inconclusive { .. }
        ));
    }

    #[test]
    fn payload_carries_format_for_both_samples() {
        let json_payload = request_payload(&sample("json"), "m");
        assert_eq!(json_payload["response_format"]["type"], "json_object");
        let schema = request_payload(&sample("schema"), "m");
        assert_eq!(schema["response_format"]["type"], "json_schema");
        assert_eq!(
            schema["response_format"]["json_schema"]["schema"]["required"]
                .as_array()
                .map(Vec::len),
            Some(4)
        );
    }
}
