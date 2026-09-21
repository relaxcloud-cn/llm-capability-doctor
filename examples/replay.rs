use llm_capability_doctor::capability::*;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: replay <capability.input.json>");
    let raw = std::fs::read_to_string(path).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let payload = &doc["evidence"][0]["payload"]["payload"];
    let card = &payload["scorecard"];
    let catalog = fixed_capability_catalog();
    let mut mism = 0;
    for o in card["observations"].as_array().unwrap() {
        let id = o["sample_id"].as_str().unwrap();
        let Some(sample) = catalog.iter().find(|s| s.id == id) else {
            continue;
        };
        let Some(ev) = payload["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["sample_id"] == id)
        else {
            continue;
        };
        let body = &ev["payload"]["response"]["body"];
        let text = body["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_string);
        let tool_calls = body["choices"][0]["message"]["tool_calls"]
            .as_array()
            .map(|calls| {
                calls
                    .iter()
                    .filter_map(|call| {
                        let function = call.get("function")?;
                        let name = function.get("name")?.as_str()?.to_string();
                        let arguments = function
                            .get("arguments")
                            .and_then(serde_json::Value::as_str)
                            .and_then(|raw| {
                                serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(
                                    raw,
                                )
                                .ok()
                            })
                            .map(|object| {
                                object
                                    .into_iter()
                                    .map(|(key, value)| {
                                        let rendered = match value {
                                            serde_json::Value::String(t) => t,
                                            other => other.to_string(),
                                        };
                                        (key, rendered)
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        Some(ToolCall { name, arguments })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let truncated = body["choices"][0]["finish_reason"].as_str() == Some("length");
        let status = ev["payload"]["response"]["status"].as_u64();
        let has_error = !ev["payload"]["response"]["error"].is_null();
        let execution =
            if has_error || !status.is_some_and(|code| (200..300).contains(&(code as u16))) {
                ExecutionState::Invalid {
                    reason: "真实服务请求未得到可评分响应".into(),
                }
            } else {
                ExecutionState::Valid
            };
        let obs = evaluate_response(
            sample,
            CapabilityResponse {
                execution,
                text,
                tool_calls,
                truncated,
                evidence_refs: vec![],
            },
        );
        let stored = o["label"].as_str().unwrap();
        let mine = format!("{:?}", obs.label).to_lowercase();
        if mine != stored {
            mism += 1;
            println!(
                "{} mine={} stored={} reason={:?}",
                id, mine, stored, obs.reason
            );
        }
    }
    println!("total mismatches: {}", mism);
}
