use serde_json::{Value, json};

use crate::context_probe;
use crate::transport::ChatCompletionsResponse;

/// S04 工具调用实测的固定样本（5 个，每样本 1 次请求）。
/// 覆盖合并后的最小测点集：全类型单调用、禁止调用、同工具两次、
/// 两工具各一次、强制指定工具。
pub const SAMPLE_IDS: [&str; 5] = [
    "tools-all-types",
    "tools-none",
    "tools-same-twice",
    "tools-two-distinct",
    "tools-forced",
];

#[derive(Debug, Clone)]
pub struct ToolSample {
    pub id: &'static str,
    pub prompt: &'static str,
    pub tool_choice: Value,
}

/// 固定双工具：lookup_weather 埋入 string/integer/enum/boolean/array/嵌套 object
/// 六类参数；lookup_air_quality 用于验证多工具时名称与参数不串位。
pub fn fixed_tools() -> Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "lookup_weather",
                "description": "查询城市天气",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "city": {"type": "string"},
                        "days": {"type": "integer", "minimum": 1, "maximum": 7},
                        "unit": {"type": "string", "enum": ["celsius", "fahrenheit"]},
                        "include_alert": {"type": "boolean"},
                        "locations": {"type": "array", "items": {"type": "string"}},
                        "options": {
                            "type": "object",
                            "properties": {"lang": {"type": "string", "enum": ["zh", "en"]}},
                            "required": ["lang"],
                            "additionalProperties": false
                        }
                    },
                    "required": ["city", "days", "unit", "include_alert", "locations", "options"],
                    "additionalProperties": false
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "lookup_air_quality",
                "description": "查询城市空气质量",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "city": {"type": "string"},
                        "index": {"type": "integer"}
                    },
                    "required": ["city", "index"],
                    "additionalProperties": false
                }
            }
        }
    ])
}

pub fn samples() -> Vec<ToolSample> {
    vec![
        ToolSample {
            id: "tools-all-types",
            prompt: "请调用天气工具查询：城市北京、未来3天、摄氏度、包含警报、覆盖区域海淀和朝阳、语言中文。不要直接回答，只发起工具调用。",
            tool_choice: json!("required"),
        },
        ToolSample {
            id: "tools-none",
            prompt: "北京明天天气怎么样？请直接用文字回答。",
            tool_choice: json!("none"),
        },
        ToolSample {
            id: "tools-same-twice",
            prompt: "请分别发起两次天气工具调用：一次城市北京（1天、摄氏度、不含警报、覆盖区域海淀、语言中文），一次城市上海（1天、摄氏度、不含警报、覆盖区域浦东、语言中文）。",
            tool_choice: json!("required"),
        },
        ToolSample {
            id: "tools-two-distinct",
            prompt: "请调用天气工具查北京明天（1天、摄氏度、不含警报、覆盖区域海淀、语言中文），并调用空气质量工具查上海（指数3）。",
            tool_choice: json!("required"),
        },
        ToolSample {
            id: "tools-forced",
            prompt: "请查询上海的空气质量，指数3。",
            tool_choice: json!({"type": "function", "function": {"name": "lookup_weather"}}),
        },
    ]
}

/// 构造带 tools/tool_choice 的 Chat Completions 请求体。
pub fn request_payload(sample: &ToolSample, model: &str) -> Value {
    json!({
        "model": model,
        "messages": [{"role": "user", "content": sample.prompt}],
        "tools": fixed_tools(),
        "tool_choice": sample.tool_choice,
        "temperature": 0,
        "max_tokens": 512,
        "stream": false,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolOutcome {
    /// 调用形态与参数校验全部满足。
    Pass { detail: String },
    /// 有效响应但违反该样例规则（数量/名称/参数/schema）。
    Violation { detail: String },
    /// 服务明确拒绝工具字段或声明不支持。
    Unsupported { detail: String },
    /// HTTP 2xx 但响应结构不可识别。
    Malformed { detail: String },
    /// 传输失败或不可归因的拒绝。
    Inconclusive { detail: String },
}

struct ParsedCall {
    name: String,
    args: Result<Value, String>,
}

/// 单样本判定：
/// - 传输失败 → Inconclusive；
/// - 非 2xx：报文明确指向工具字段被拒 → Unsupported；其余 → Inconclusive；
/// - 2xx 但 choices/message 缺失 → Malformed；
/// - 其余按各样例规则校验调用形态与参数。
pub fn classify(sample: &ToolSample, response: &ChatCompletionsResponse) -> ToolOutcome {
    if let Some(error) = &response.error {
        return ToolOutcome::Inconclusive {
            detail: format!("传输失败：{error}"),
        };
    }
    let status = response.status.unwrap_or(0);
    if !(200..300).contains(&status) {
        if is_tool_rejection(&response.body) {
            return ToolOutcome::Unsupported {
                detail: format!(
                    "HTTP {status}：服务拒绝工具字段或声明不支持：{}",
                    context_probe::excerpt(&response.body, 200)
                ),
            };
        }
        return ToolOutcome::Inconclusive {
            detail: format!("HTTP {status}，非可归因工具能力拒绝"),
        };
    }
    let Some(message) = message_of(response.parsed.as_ref()) else {
        return ToolOutcome::Malformed {
            detail: "HTTP 成功但响应缺少 choices[0].message".into(),
        };
    };
    let calls = extract_calls(message);
    let result = match sample.id {
        "tools-none" => validate_none(&calls, message),
        "tools-all-types" => validate_all_types(&calls),
        "tools-same-twice" => validate_same_twice(&calls),
        "tools-two-distinct" => validate_two_distinct(&calls),
        "tools-forced" => validate_forced(&calls),
        _ => Err(format!("未知样本 {id}", id = sample.id)),
    };
    match result {
        Ok(detail) => ToolOutcome::Pass { detail },
        Err(detail) => ToolOutcome::Violation { detail },
    }
}

/// 非 2xx 报文是否明确指向工具字段被拒（区分普通参数错误）。
fn is_tool_rejection(body: &str) -> bool {
    let lower = body.to_lowercase();
    let mentions_tool = lower.contains("tool")
        || lower.contains("function")
        || lower.contains("工具")
        || lower.contains("函数");
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
    mentions_tool && REJECTION.iter().any(|signal| lower.contains(signal))
}

fn message_of(parsed: Option<&Value>) -> Option<&Value> {
    parsed?.get("choices")?.as_array()?.first()?.get("message")
}

/// 提取 tool_calls；兼容旧的 function_call 单调用形态。
fn extract_calls(message: &Value) -> Vec<ParsedCall> {
    let mut calls = Vec::new();
    if let Some(items) = message.get("tool_calls").and_then(Value::as_array) {
        for item in items {
            if let Some(function) = item.get("function") {
                calls.push(ParsedCall {
                    name: function
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                    args: parse_arguments(function.get("arguments")),
                });
            }
        }
    } else if let Some(function) = message.get("function_call").filter(|f| f.is_object()) {
        calls.push(ParsedCall {
            name: function
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            args: parse_arguments(function.get("arguments")),
        });
    }
    calls
}

/// arguments 按规范是 JSON 字符串；少数服务直接给对象，两者都接受。
fn parse_arguments(raw: Option<&Value>) -> Result<Value, String> {
    match raw {
        Some(Value::String(text)) => {
            serde_json::from_str::<Value>(text).map_err(|e| format!("arguments 不是合法 JSON：{e}"))
        }
        Some(value @ Value::Object(_)) => Ok(value.clone()),
        Some(_) => Err("arguments 既不是 JSON 字符串也不是对象".into()),
        None => Err("arguments 缺失".into()),
    }
}

fn none_if_empty(calls: &[ParsedCall]) -> Result<(), String> {
    if calls.is_empty() {
        Err("required/指定工具下未返回任何 tool_calls".into())
    } else {
        Ok(())
    }
}

fn validate_none(calls: &[ParsedCall], message: &Value) -> Result<String, String> {
    if !calls.is_empty() {
        return Err(format!(
            "tool_choice=none 仍返回 {} 个 tool_calls",
            calls.len()
        ));
    }
    let has_text = message
        .get("content")
        .and_then(Value::as_str)
        .is_some_and(|text| !text.trim().is_empty());
    if has_text {
        Ok("none 下无调用且返回文本".into())
    } else {
        Err("none 下既无调用也无文本内容".into())
    }
}

/// 全类型单调用：恰好 1 个 lookup_weather，六类参数逐字段校验值。
fn validate_all_types(calls: &[ParsedCall]) -> Result<String, String> {
    none_if_empty(calls)?;
    if calls.len() != 1 {
        return Err(format!("期望恰好 1 个调用，实际 {}", calls.len()));
    }
    let call = &calls[0];
    if call.name != "lookup_weather" {
        return Err(format!(
            "调用了 {name} 而非 lookup_weather",
            name = call.name
        ));
    }
    let args = call.args.clone()?;
    check_value(&args, "city", |v| v.as_str() == Some("北京"))?;
    check_value(&args, "days", |v| v.as_u64() == Some(3))?;
    check_value(&args, "unit", |v| v.as_str() == Some("celsius"))?;
    check_value(&args, "include_alert", |v| v.as_bool() == Some(true))?;
    check_value(&args, "locations", |v| {
        v.as_array().is_some_and(|items| {
            items.iter().all(Value::is_string)
                && items.iter().any(|i| i.as_str() == Some("海淀"))
                && items.iter().any(|i| i.as_str() == Some("朝阳"))
        })
    })?;
    check_value(&args, "options", |v| {
        v.get("lang").and_then(Value::as_str) == Some("zh")
    })?;
    const ALLOWED: [&str; 6] = [
        "city",
        "days",
        "unit",
        "include_alert",
        "locations",
        "options",
    ];
    if let Some(extra) = args
        .as_object()
        .and_then(|map| map.keys().find(|k| !ALLOWED.contains(&k.as_str())))
    {
        return Err(format!("arguments.{extra} 为 schema 外多余字段"));
    }
    Ok("单调用六类参数（含嵌套对象）全部符合 schema 与期望值".into())
}

/// 同一工具调用两次：两个 call 均为 lookup_weather，城市集合为 {北京, 上海}。
fn validate_same_twice(calls: &[ParsedCall]) -> Result<String, String> {
    none_if_empty(calls)?;
    if calls.len() != 2 {
        return Err(format!("期望恰好 2 个调用，实际 {}", calls.len()));
    }
    let mut cities = Vec::new();
    for (index, call) in calls.iter().enumerate() {
        if call.name != "lookup_weather" {
            return Err(format!("第 {index} 个调用了 {name}", name = call.name));
        }
        let args = call
            .args
            .clone()
            .map_err(|e| format!("第 {index} 个调用：{e}"))?;
        check_weather_core(&args, index)?;
        cities.push(
            args.get("city")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        );
    }
    cities.sort();
    if cities != ["上海", "北京"] {
        return Err(format!("两次调用城市应为北京/上海，实际 {cities:?}"));
    }
    Ok("同一工具两次调用，参数按调用分别对应".into())
}

/// 两个不同工具各一次：lookup_weather + lookup_air_quality，参数不串位。
fn validate_two_distinct(calls: &[ParsedCall]) -> Result<String, String> {
    none_if_empty(calls)?;
    if calls.len() != 2 {
        return Err(format!("期望恰好 2 个调用，实际 {}", calls.len()));
    }
    let weather = calls
        .iter()
        .find(|call| call.name == "lookup_weather")
        .ok_or("缺少 lookup_weather 调用")?;
    let air = calls
        .iter()
        .find(|call| call.name == "lookup_air_quality")
        .ok_or("缺少 lookup_air_quality 调用")?;
    let weather_args = weather.args.clone()?;
    check_weather_core(&weather_args, 0)?;
    check_value(&weather_args, "city", |v| v.as_str() == Some("北京"))?;
    let air_args = air.args.clone()?;
    check_value(&air_args, "city", |v| v.as_str() == Some("上海"))?;
    check_value(&air_args, "index", |v| v.as_u64() == Some(3))?;
    Ok("两个不同工具各一次，名称与参数分别对应".into())
}

/// 强制指定工具：≥1 个调用且全部只能是 lookup_weather。
fn validate_forced(calls: &[ParsedCall]) -> Result<String, String> {
    none_if_empty(calls)?;
    if let Some(index) = calls.iter().position(|call| call.name != "lookup_weather") {
        return Err(format!(
            "强制指定 lookup_weather，但第 {index} 个调用了 {name}",
            name = calls[index].name
        ));
    }
    Ok(format!(
        "强制指定生效，{} 个调用均为 lookup_weather",
        calls.len()
    ))
}

/// 天气调用的核心必填字段类型检查（值不逐题核对）。
pub(crate) fn check_weather_core(args: &Value, index: usize) -> Result<(), String> {
    for (key, kind) in [
        ("city", "string"),
        ("days", "integer"),
        ("unit", "enum"),
        ("include_alert", "boolean"),
        ("locations", "string_array"),
        ("options", "object"),
    ] {
        check_kind(args, key, kind).map_err(|e| format!("第 {index} 个调用：{e}"))?;
    }
    Ok(())
}

fn check_kind(args: &Value, key: &str, kind: &str) -> Result<(), String> {
    let path = format!("arguments.{key}");
    let Some(value) = args.get(key) else {
        return Err(format!("{path} 缺失"));
    };
    let ok = match kind {
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "boolean" => value.is_boolean(),
        "enum" => matches!(value.as_str(), Some("celsius") | Some("fahrenheit")),
        "string_array" => value
            .as_array()
            .is_some_and(|items| items.iter().all(Value::is_string)),
        "object" => value.is_object(),
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(format!("{path} 类型/取值不符（期望 {kind}）"))
    }
}

fn check_value(
    args: &Value,
    key: &str,
    predicate: impl FnOnce(&Value) -> bool,
) -> Result<(), String> {
    let path = format!("arguments.{key}");
    let Some(value) = args.get(key) else {
        return Err(format!("{path} 缺失"));
    };
    if predicate(value) {
        Ok(())
    } else {
        Err(format!("{path} 值不符期望，实际 {value}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str) -> ToolSample {
        samples().into_iter().find(|s| s.id == id).unwrap()
    }

    fn ok_response(message: Value) -> ChatCompletionsResponse {
        let parsed = json!({"choices": [{"message": message, "finish_reason": "tool_calls"}]});
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

    fn call(name: &str, args: Value) -> Value {
        json!({"type": "function", "function": {"name": name, "arguments": args.to_string()}})
    }

    fn weather_args(city: &str) -> Value {
        json!({
            "city": city, "days": 1, "unit": "celsius",
            "include_alert": false, "locations": ["海淀"],
            "options": {"lang": "zh"}
        })
    }

    #[test]
    fn all_types_passes_with_full_valid_call() {
        let message = json!({"tool_calls": [call("lookup_weather", json!({
            "city": "北京", "days": 3, "unit": "celsius",
            "include_alert": true, "locations": ["海淀", "朝阳"],
            "options": {"lang": "zh"}
        }))]});
        let outcome = classify(&sample("tools-all-types"), &ok_response(message));
        assert!(matches!(outcome, ToolOutcome::Pass { .. }));
    }

    #[test]
    fn all_types_fails_on_string_boolean() {
        let mut args = weather_args("北京");
        args["days"] = json!(3);
        args["include_alert"] = json!("true");
        args["locations"] = json!(["海淀", "朝阳"]);
        let message = json!({"tool_calls": [call("lookup_weather", args)]});
        let outcome = classify(&sample("tools-all-types"), &ok_response(message));
        assert!(matches!(outcome, ToolOutcome::Violation { .. }));
    }

    #[test]
    fn none_passes_without_calls_and_fails_with_calls() {
        let text = json!({"content": "北京明天晴。"});
        let outcome = classify(&sample("tools-none"), &ok_response(text));
        assert!(matches!(outcome, ToolOutcome::Pass { .. }));

        let with_call = json!({"tool_calls": [call("lookup_weather", weather_args("北京"))]});
        let outcome = classify(&sample("tools-none"), &ok_response(with_call));
        assert!(matches!(outcome, ToolOutcome::Violation { .. }));
    }

    #[test]
    fn same_twice_requires_two_weather_calls_with_cities() {
        let message = json!({"tool_calls": [
            call("lookup_weather", weather_args("北京")),
            call("lookup_weather", weather_args("上海"))
        ]});
        assert!(matches!(
            classify(&sample("tools-same-twice"), &ok_response(message)),
            ToolOutcome::Pass { .. }
        ));

        let same_city = json!({"tool_calls": [
            call("lookup_weather", weather_args("北京")),
            call("lookup_weather", weather_args("北京"))
        ]});
        assert!(matches!(
            classify(&sample("tools-same-twice"), &ok_response(same_city)),
            ToolOutcome::Violation { .. }
        ));
    }

    #[test]
    fn two_distinct_requires_one_of_each_tool() {
        let message = json!({"tool_calls": [
            call("lookup_weather", weather_args("北京")),
            call("lookup_air_quality", json!({"city": "上海", "index": 3}))
        ]});
        assert!(matches!(
            classify(&sample("tools-two-distinct"), &ok_response(message)),
            ToolOutcome::Pass { .. }
        ));

        let swapped_args = json!({"tool_calls": [
            call("lookup_weather", json!({"city": "上海", "index": 3})),
            call("lookup_air_quality", weather_args("北京"))
        ]});
        assert!(matches!(
            classify(&sample("tools-two-distinct"), &ok_response(swapped_args)),
            ToolOutcome::Violation { .. }
        ));
    }

    #[test]
    fn forced_rejects_other_tool_names() {
        let ok = json!({"tool_calls": [call("lookup_weather", weather_args("上海"))]});
        assert!(matches!(
            classify(&sample("tools-forced"), &ok_response(ok)),
            ToolOutcome::Pass { .. }
        ));

        let wrong = json!({"tool_calls": [call("lookup_air_quality", json!({"city": "上海", "index": 3}))]});
        assert!(matches!(
            classify(&sample("tools-forced"), &ok_response(wrong)),
            ToolOutcome::Violation { .. }
        ));
    }

    #[test]
    fn required_without_calls_is_violation() {
        let text = json!({"content": "好的，天气不错。"});
        let outcome = classify(&sample("tools-all-types"), &ok_response(text));
        assert!(matches!(outcome, ToolOutcome::Violation { .. }));
    }

    #[test]
    fn null_function_call_field_is_not_a_call() {
        // 真实服务（deepseek-v4-flash 网关）会在 message 里显式给 "function_call": null，
        // 必须不算作一次调用。
        let message = json!({"content": "北京明天晴。", "function_call": null});
        let outcome = classify(&sample("tools-none"), &ok_response(message));
        assert!(matches!(outcome, ToolOutcome::Pass { .. }));
    }

    #[test]
    fn tool_rejection_is_unsupported() {
        let response = ChatCompletionsResponse {
            status: Some(400),
            body: r#"{"error":{"message":"unknown field tools: this model does not support function calling"}}"#.into(),
            parsed: None,
            elapsed_ms: 1,
            error: None,
            attempts: 1,
            retry_reasons: Vec::new(),
        };
        assert!(matches!(
            classify(&sample("tools-all-types"), &response),
            ToolOutcome::Unsupported { .. }
        ));
    }

    #[test]
    fn auth_error_is_inconclusive_not_unsupported() {
        let response = ChatCompletionsResponse {
            status: Some(401),
            body: r#"{"error":{"message":"invalid api key"}}"#.into(),
            parsed: None,
            elapsed_ms: 1,
            error: None,
            attempts: 1,
            retry_reasons: Vec::new(),
        };
        assert!(matches!(
            classify(&sample("tools-all-types"), &response),
            ToolOutcome::Inconclusive { .. }
        ));
    }

    #[test]
    fn request_payload_carries_tools_and_choice() {
        let payload = request_payload(&sample("tools-forced"), "model-a");
        assert_eq!(payload["tool_choice"]["function"]["name"], "lookup_weather");
        assert_eq!(payload["tools"].as_array().map(Vec::len), Some(2));
        assert_eq!(payload["stream"], false);
    }
}
