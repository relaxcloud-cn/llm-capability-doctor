use serde_json::{Value, json};

use crate::context_probe;
use crate::transport::ChatCompletionsResponse;

/// S06 消息与多轮输入实测的固定样本（4 个，每样本 1 次请求）。
/// 所有历史都构造在同一个请求的 messages 数组里，不跨请求。
pub const SAMPLE_IDS: [&str; 4] = ["M01", "M02", "M03", "M04"];

/// M02 历史中埋入的唯一暗号。
const MARKER_M02: &str = "蓝鲸-7392";
/// M03 两条 user 历史中的两个标记：暗号（目标）与任务编号（干扰）。
const MARKER_M03_TARGET: &str = "青鸟-482";
const MARKER_M03_DECOY: &str = "T-315";

#[derive(Debug, Clone)]
pub struct MessagesSample {
    pub id: &'static str,
    pub messages: Value,
}

pub fn samples() -> Vec<MessagesSample> {
    vec![
        MessagesSample {
            id: "M01",
            messages: json!([
                {"role": "system", "content": "你是简洁的中文助手。"},
                {"role": "user", "content": "请只回复：OK"}
            ]),
        },
        MessagesSample {
            id: "M02",
            messages: json!([
                {"role": "system", "content": "你是简洁的中文助手。"},
                {"role": "user", "content": format!("请记住这个暗号：{MARKER_M02}。只回复：好的")},
                {"role": "assistant", "content": "好的"},
                {"role": "user", "content": "我之前给你的暗号是什么？只回答暗号本身。"}
            ]),
        },
        MessagesSample {
            id: "M03",
            messages: json!([
                {"role": "user", "content": format!("暗号一号是 {MARKER_M03_TARGET}，请只回复：已记录")},
                {"role": "user", "content": format!("补充：任务编号 {MARKER_M03_DECOY}，请只回复：已记录")},
                {"role": "assistant", "content": "已记录"},
                {"role": "user", "content": "暗号一号是什么？只回答暗号本身。"}
            ]),
        },
        MessagesSample {
            id: "M04",
            messages: json!([
                {"role": "user", "content": "查一下北京天气。"},
                {"role": "assistant", "content": null, "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {"name": "lookup_weather", "arguments": "{\"city\":\"北京\"}"}
                }]},
                {"role": "tool", "tool_call_id": "call_1", "content": "{\"temperature\":23,\"condition\":\"晴\"}"},
                {"role": "user", "content": "根据上面的工具结果，用一句话告诉我北京天气。"}
            ]),
        },
    ]
}

/// 构造携带自定义消息序列的 Chat Completions 请求体。
pub fn request_payload(sample: &MessagesSample, model: &str) -> Value {
    json!({
        "model": model,
        "messages": sample.messages.clone(),
        "temperature": 0,
        "max_tokens": 256,
        "stream": false,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub enum MessagesOutcome {
    /// 角色被接受且满足该样本的判定条件。
    Pass { detail: String },
    /// 有效响应但角色被拒、内容为空或标记错引/丢失。
    Violation { detail: String },
    /// 该消息形态在当前协议不适用（如 tool 角色被拒）。
    NotApplicable { detail: String },
    /// HTTP 2xx 但响应结构不可识别。
    Malformed { detail: String },
    /// 传输失败或不可归因的拒绝。
    Inconclusive { detail: String },
}

/// 单样本判定：
/// - 传输失败 → Inconclusive；
/// - 非 2xx：M04 的 tool 角色被拒 → NotApplicable；M01–M03 角色被拒 → Violation；
///   其余 → Inconclusive；
/// - 2xx 但 choices[0].message 缺失 → Malformed；
/// - content 为空 → Violation（角色被接受但生成内容为空）；
/// - 其余按各样本标记规则判定。
pub fn classify(sample: &MessagesSample, response: &ChatCompletionsResponse) -> MessagesOutcome {
    if let Some(error) = &response.error {
        return MessagesOutcome::Inconclusive {
            detail: format!("传输失败：{error}"),
        };
    }
    let status = response.status.unwrap_or(0);
    if !(200..300).contains(&status) {
        if is_role_rejection(&response.body) {
            return if sample.id == "M04" {
                MessagesOutcome::NotApplicable {
                    detail: format!(
                        "HTTP {status}：tool 角色/工具消息序列被拒：{}",
                        context_probe::excerpt(&response.body, 200)
                    ),
                }
            } else {
                MessagesOutcome::Violation {
                    detail: format!(
                        "HTTP {status}：消息角色被拒：{}",
                        context_probe::excerpt(&response.body, 200)
                    ),
                }
            };
        }
        return MessagesOutcome::Inconclusive {
            detail: format!("HTTP {status}，非可归因角色/消息拒绝"),
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
        return MessagesOutcome::Malformed {
            detail: "HTTP 成功但响应缺少 choices[0].message.content".into(),
        };
    };
    if content.trim().is_empty() {
        return MessagesOutcome::Violation {
            detail: "响应 content 为空".into(),
        };
    }
    let result = match sample.id {
        "M01" => Ok("system+user 角色被接受，返回非空 assistant 内容".to_owned()),
        "M02" => {
            if content.contains(MARKER_M02) {
                Ok(format!("正确回引历史唯一标记 {MARKER_M02}"))
            } else {
                Err(format!("未回引历史标记 {MARKER_M02}"))
            }
        }
        "M03" => {
            let has_target = content.contains(MARKER_M03_TARGET);
            let has_decoy = content.contains(MARKER_M03_DECOY);
            if has_target && !has_decoy {
                Ok(format!(
                    "正确区分标记：回答 {MARKER_M03_TARGET} 且未混入 {MARKER_M03_DECOY}"
                ))
            } else if has_target && has_decoy {
                Err(format!("目标标记正确但混入干扰标记 {MARKER_M03_DECOY}"))
            } else {
                Err(format!("未回引目标标记 {MARKER_M03_TARGET}"))
            }
        }
        "M04" => Ok("含 tool 角色的消息序列被接受并正常续答".to_owned()),
        _ => Err(format!("未知样本 {}", sample.id)),
    };
    match result {
        Ok(detail) => MessagesOutcome::Pass { detail },
        Err(detail) => MessagesOutcome::Violation { detail },
    }
}

/// 非 2xx 报文是否明确指向消息角色/结构被拒。
fn is_role_rejection(body: &str) -> bool {
    let lower = body.to_lowercase();
    let mentions_role = lower.contains("role")
        || lower.contains("tool_call")
        || lower.contains("system message")
        || lower.contains("角色")
        || lower.contains("消息");
    const REJECTION: &[&str] = &[
        "invalid",
        "not support",
        "unsupported",
        "unrecognized",
        "unknown",
        "not allowed",
        "不支持",
        "无法识别",
        "非法",
    ];
    mentions_role && REJECTION.iter().any(|signal| lower.contains(signal))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::ChatCompletionsResponse;

    fn sample(id: &str) -> MessagesSample {
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
    fn m01_passes_on_nonempty_content() {
        assert!(matches!(
            classify(&sample("M01"), &ok_response("OK")),
            MessagesOutcome::Pass { .. }
        ));
        assert!(matches!(
            classify(&sample("M01"), &ok_response("   ")),
            MessagesOutcome::Violation { .. }
        ));
    }

    #[test]
    fn m02_requires_marker_recall() {
        assert!(matches!(
            classify(&sample("M02"), &ok_response("蓝鲸-7392")),
            MessagesOutcome::Pass { .. }
        ));
        assert!(matches!(
            classify(&sample("M02"), &ok_response("我不记得了")),
            MessagesOutcome::Violation { .. }
        ));
    }

    #[test]
    fn m03_distinguishes_target_from_decoy() {
        assert!(matches!(
            classify(&sample("M03"), &ok_response("青鸟-482")),
            MessagesOutcome::Pass { .. }
        ));
        let mixed = classify(&sample("M03"), &ok_response("青鸟-482，编号 T-315"));
        assert!(matches!(mixed, MessagesOutcome::Violation { .. }));
        let wrong = classify(&sample("M03"), &ok_response("T-315"));
        assert!(matches!(wrong, MessagesOutcome::Violation { .. }));
    }

    #[test]
    fn m04_tool_role_rejection_is_not_applicable() {
        let rejected = error_response(
            400,
            r#"{"error":{"message":"invalid role 'tool' in messages"}}"#,
        );
        assert!(matches!(
            classify(&sample("M04"), &rejected),
            MessagesOutcome::NotApplicable { .. }
        ));
        // 同样的角色拒绝落在 M01 上则是失败。
        let rejected = error_response(
            400,
            r#"{"error":{"message":"invalid role 'system' in messages"}}"#,
        );
        assert!(matches!(
            classify(&sample("M01"), &rejected),
            MessagesOutcome::Violation { .. }
        ));
    }

    #[test]
    fn auth_error_is_inconclusive() {
        let response = error_response(401, r#"{"error":{"message":"invalid api key"}}"#);
        assert!(matches!(
            classify(&sample("M02"), &response),
            MessagesOutcome::Inconclusive { .. }
        ));
    }

    #[test]
    fn payloads_carry_role_sequences() {
        let m02 = request_payload(&sample("M02"), "m");
        let roles: Vec<&str> = m02["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m["role"].as_str())
            .collect();
        assert_eq!(roles, ["system", "user", "assistant", "user"]);
        assert!(
            m02["messages"][1]["content"]
                .as_str()
                .unwrap()
                .contains(MARKER_M02)
        );

        let m04 = request_payload(&sample("M04"), "m");
        assert_eq!(m04["messages"][2]["role"], "tool");
        assert_eq!(m04["messages"][2]["tool_call_id"], "call_1");
    }
}
