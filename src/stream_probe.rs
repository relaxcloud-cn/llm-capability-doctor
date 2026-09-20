use serde_json::{Value, json};

use crate::context_probe;
use crate::tool_probe;
use crate::transport::{StreamEvent, StreamResponse};

/// S07 流式输出实测的固定样本（2 个，每样本 1 次真实 stream:true 请求）。
pub const SAMPLE_IDS: [&str; 2] = ["stream-text", "stream-tool"];

const TEXT_BEGIN: &str = "STREAM_BEGIN";
const TEXT_END: &str = "STREAM_END";

#[derive(Debug, Clone)]
pub struct StreamSample {
    pub id: &'static str,
    /// None 表示纯文本流；Some 表示携带工具的流式调用场景。
    pub with_tools: bool,
}

pub fn samples() -> Vec<StreamSample> {
    vec![
        StreamSample {
            id: "stream-text",
            with_tools: false,
        },
        StreamSample {
            id: "stream-tool",
            with_tools: true,
        },
    ]
}

/// 构造流式 Chat Completions 请求体。
pub fn request_payload(sample: &StreamSample, model: &str) -> Value {
    if sample.with_tools {
        json!({
            "model": model,
            "messages": [{"role": "user", "content":
                "请调用天气工具查询北京明天天气（1天、摄氏度、不含警报、覆盖区域海淀、语言中文）。不要直接回答。"}],
            "tools": tool_probe::fixed_tools(),
            "tool_choice": "required",
            "temperature": 0,
            "max_tokens": 512,
            "stream": true,
        })
    } else {
        json!({
            "model": model,
            "messages": [{"role": "user", "content": format!(
                "请回复一句简短的话，必须以 {TEXT_BEGIN} 开头、以 {TEXT_END} 结尾。"
            )}],
            "temperature": 0,
            "max_tokens": 128,
            "stream": true,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamOutcome {
    /// 语义增量、完整组装、协议终止均满足。
    Pass { detail: String },
    /// 有效响应但内容/调用完整性明确违反。
    Violation { detail: String },
    /// 服务明确拒绝流式模式。
    Unsupported { detail: String },
    /// 客户端无法解析 SSE 事件流。
    Malformed { detail: String },
    /// 传输失败或证据不足。
    Inconclusive { detail: String },
}

/// 由增量事件归组拼接出的一个工具调用。
#[derive(Debug, Clone, PartialEq)]
pub struct AssembledCall {
    pub id: Option<String>,
    pub name: String,
    pub arguments: String,
}

/// 把各事件的 delta.tool_calls 片段按 index 归组、按到达顺序拼接。
/// 单个片段可以是非法 JSON（STT02 场景），只有拼接结果参与解析。
pub fn assemble_tool_calls(events: &[StreamEvent]) -> Vec<AssembledCall> {
    let mut order: Vec<u64> = Vec::new();
    let mut calls: std::collections::BTreeMap<u64, AssembledCall> =
        std::collections::BTreeMap::new();
    for event in events {
        let Some(deltas) = event.tool_calls_delta.as_ref().and_then(Value::as_array) else {
            continue;
        };
        for delta in deltas {
            let index = delta.get("index").and_then(Value::as_u64).unwrap_or(0);
            let call = calls.entry(index).or_insert_with(|| {
                order.push(index);
                AssembledCall {
                    id: None,
                    name: String::new(),
                    arguments: String::new(),
                }
            });
            if let Some(id) = delta.get("id").and_then(Value::as_str) {
                if call.id.is_none() {
                    call.id = Some(id.to_owned());
                }
            }
            if let Some(function) = delta.get("function") {
                if let Some(name) = function.get("name").and_then(Value::as_str) {
                    call.name.push_str(name);
                }
                if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                    call.arguments.push_str(arguments);
                }
            }
        }
    }
    order
        .into_iter()
        .filter_map(|index| calls.remove(&index))
        .collect()
}

/// 单样本判定：
/// - 传输失败 → Inconclusive；
/// - 非 2xx：报文明确拒绝流式 → Unsupported；其余 → Inconclusive；
/// - SSE 事件存在解析失败 → Malformed；
/// - 未正常终止（缺 finish_reason/[DONE]）→ Violation（不判正常结束）；
/// - 其余按各样本规则验证组装结果。
pub fn classify(sample: &StreamSample, stream: &StreamResponse) -> StreamOutcome {
    if let Some(error) = &stream.response.error {
        return StreamOutcome::Inconclusive {
            detail: format!("传输失败：{error}"),
        };
    }
    let status = stream.response.status.unwrap_or(0);
    if !(200..300).contains(&status) {
        if is_stream_rejection(&stream.response.body) {
            return StreamOutcome::Unsupported {
                detail: format!(
                    "HTTP {status}：服务拒绝流式模式：{}",
                    context_probe::excerpt(&stream.response.body, 200)
                ),
            };
        }
        return StreamOutcome::Inconclusive {
            detail: format!("HTTP {status}，非可归因流式拒绝"),
        };
    }
    if !stream.parse_errors.is_empty() {
        return StreamOutcome::Malformed {
            detail: format!("SSE 事件解析失败：{}", stream.parse_errors.join("；")),
        };
    }
    if stream.events.is_empty() {
        return StreamOutcome::Inconclusive {
            detail: "未收到任何流式事件".into(),
        };
    }
    if !stream.terminated {
        return StreamOutcome::Violation {
            detail: "缺少终止事件（finish_reason/[DONE]），不判正常结束".into(),
        };
    }
    let result = match sample.id {
        "stream-text" => validate_text(stream),
        "stream-tool" => validate_tool_stream(stream),
        _ => Err(format!("未知样本 {}", sample.id)),
    };
    match result {
        Ok(detail) => StreamOutcome::Pass { detail },
        Err(detail) => StreamOutcome::Violation { detail },
    }
}

/// 文本流：组装内容含唯一首尾标记且有实质增量。
fn validate_text(stream: &StreamResponse) -> Result<String, String> {
    let deltas = stream
        .events
        .iter()
        .filter(|event| event.content_delta.is_some())
        .count();
    if deltas == 0 {
        return Err("未收到任何内容增量".into());
    }
    if !stream.content.contains(TEXT_BEGIN) || !stream.content.contains(TEXT_END) {
        return Err(format!(
            "组装内容缺少首尾标记：{}",
            context_probe::excerpt(&stream.content, 120)
        ));
    }
    Ok(format!("{deltas} 个内容增量，组装后首尾标记完整"))
}

/// 工具流：增量按 index 归组拼接后必须是合法调用且参数合 schema。
fn validate_tool_stream(stream: &StreamResponse) -> Result<String, String> {
    let calls = assemble_tool_calls(&stream.events);
    if calls.is_empty() {
        return Err("工具场景未收到 tool_calls 增量（纯文本不能替代）".into());
    }
    for (position, call) in calls.iter().enumerate() {
        if call.name != "lookup_weather" {
            return Err(format!("第 {position} 个调用名为 {}", call.name));
        }
        let arguments: Value = serde_json::from_str(&call.arguments)
            .map_err(|e| format!("第 {position} 个调用参数拼接后不是合法 JSON：{e}"))?;
        tool_probe::check_weather_core(&arguments, position)?;
    }
    Ok(format!(
        "{} 个工具调用增量归组拼接成功且参数合规",
        calls.len()
    ))
}

/// 非 2xx 报文是否明确指向流式模式被拒。
fn is_stream_rejection(body: &str) -> bool {
    let lower = body.to_lowercase();
    let mentions_stream = lower.contains("stream") || lower.contains("流式");
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
    ];
    mentions_stream && REJECTION.iter().any(|signal| lower.contains(signal))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::ChatCompletionsResponse;

    fn sample(id: &str) -> StreamSample {
        samples().into_iter().find(|s| s.id == id).unwrap()
    }

    fn event(data: &str) -> StreamEvent {
        let parsed: Value = serde_json::from_str(data).unwrap();
        let choice = &parsed["choices"][0];
        let delta = choice.get("delta");
        StreamEvent {
            at_ms: 0,
            raw: data.into(),
            content_delta: delta
                .and_then(|d| d.get("content"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            reasoning_delta: None,
            tool_calls_delta: delta.and_then(|d| d.get("tool_calls")).cloned(),
            finish_reason: choice
                .get("finish_reason")
                .and_then(Value::as_str)
                .map(str::to_owned),
            done: data == "[DONE]",
        }
    }

    fn stream(events: Vec<StreamEvent>, content: &str, terminated: bool) -> StreamResponse {
        StreamResponse {
            response: ChatCompletionsResponse {
                status: Some(200),
                body: String::new(),
                parsed: None,
                elapsed_ms: 1,
                error: None,
                attempts: 1,
                retry_reasons: Vec::new(),
            },
            events,
            content: content.into(),
            terminated,
            parse_errors: Vec::new(),
        }
    }

    #[test]
    fn text_stream_requires_markers_and_termination() {
        let ok = stream(
            vec![
                event(r#"{"choices":[{"delta":{"content":"STREAM_B"}}]}"#),
                event(
                    r#"{"choices":[{"delta":{"content":"EGIN hi STREAM_END"},"finish_reason":"stop"}]}"#,
                ),
            ],
            "STREAM_BEGIN hi STREAM_END",
            true,
        );
        assert!(matches!(
            classify(&sample("stream-text"), &ok),
            StreamOutcome::Pass { .. }
        ));

        let no_end = stream(
            vec![event(
                r#"{"choices":[{"delta":{"content":"STREAM_BEGIN hi"},"finish_reason":"stop"}]}"#,
            )],
            "STREAM_BEGIN hi",
            true,
        );
        assert!(matches!(
            classify(&sample("stream-text"), &no_end),
            StreamOutcome::Violation { .. }
        ));

        let unterminated = stream(
            vec![event(
                r#"{"choices":[{"delta":{"content":"STREAM_BEGIN hi STREAM_END"}}]}"#,
            )],
            "STREAM_BEGIN hi STREAM_END",
            false,
        );
        assert!(matches!(
            classify(&sample("stream-text"), &unterminated),
            StreamOutcome::Violation { .. }
        ));
    }

    #[test]
    fn tool_stream_assembles_arguments_split_mid_string() {
        // STT02：arguments 在字符串中间切片，单片非法 JSON，拼接后合法。
        let events = vec![
            event(
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","type":"function","function":{"name":"lookup_","arguments":"{\"city\":\"北"}}]}}]}"#,
            ),
            event(
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"weather","arguments":"京\",\"days\":1,\"unit\":\"celsius\",\"include_alert\":false,\"locations\":[\"海淀\"],\"options\":{\"lang\":\"zh\"}}"}}]}}]}"#,
            ),
            event(r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#),
        ];
        let stream = stream(events, "", true);
        assert!(matches!(
            classify(&sample("stream-tool"), &stream),
            StreamOutcome::Pass { .. }
        ));
    }

    #[test]
    fn tool_stream_groups_interleaved_calls_by_index() {
        // STT03：两个调用交错返回，按 index 分别拼接。
        let events = vec![
            event(
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","type":"function","function":{"name":"lookup_weather","arguments":"{\"city\":\"北京\",\"days\":1"}}]}}]}"#,
            ),
            event(
                r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"c2","type":"function","function":{"name":"lookup_weather","arguments":"{\"city\":\"上海\",\"days\":1"}}]}}]}"#,
            ),
            event(
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":",\"unit\":\"celsius\",\"include_alert\":false,\"locations\":[\"海淀\"],\"options\":{\"lang\":\"zh\"}}"}}]}}]}"#,
            ),
            event(
                r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":",\"unit\":\"celsius\",\"include_alert\":false,\"locations\":[\"浦东\"],\"options\":{\"lang\":\"zh\"}}"}}]}}]}"#,
            ),
            event(r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#),
        ];
        let stream = stream(events, "", true);
        assert!(matches!(
            classify(&sample("stream-tool"), &stream),
            StreamOutcome::Pass { .. }
        ));
    }

    #[test]
    fn tool_stream_rejects_text_only_and_bad_calls() {
        // STT06：纯文本不能替代工具增量。
        let text_only = stream(
            vec![
                event(r#"{"choices":[{"delta":{"content":"北京晴"}}]}"#),
                event(r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#),
            ],
            "北京晴",
            true,
        );
        assert!(matches!(
            classify(&sample("stream-tool"), &text_only),
            StreamOutcome::Violation { .. }
        ));

        // 参数拼接后非法 JSON。
        let bad_args = stream(
            vec![
                event(
                    r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"lookup_weather","arguments":"{broken"}}]}}]}"#,
                ),
                event(r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#),
            ],
            "",
            true,
        );
        assert!(matches!(
            classify(&sample("stream-tool"), &bad_args),
            StreamOutcome::Violation { .. }
        ));
    }

    #[test]
    fn stream_rejection_is_unsupported() {
        let mut s = stream(vec![], "", false);
        s.response.status = Some(400);
        s.response.body = r#"{"error":{"message":"stream parameter is not supported"}}"#.into();
        assert!(matches!(
            classify(&sample("stream-text"), &s),
            StreamOutcome::Unsupported { .. }
        ));
    }

    #[test]
    fn tool_payload_carries_stream_and_tools() {
        let payload = request_payload(&sample("stream-tool"), "m");
        assert_eq!(payload["stream"], true);
        assert_eq!(payload["tool_choice"], "required");
        assert_eq!(payload["tools"].as_array().map(Vec::len), Some(2));
    }
}
