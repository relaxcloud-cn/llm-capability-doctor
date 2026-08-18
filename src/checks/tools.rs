use crate::protocol::tools::{tool_prompt, tool_request};

use super::{CheckError, CheckPlan, PlanContext, from_spec};

pub(super) fn plan(id: &str, context: &PlanContext<'_>) -> Result<CheckPlan, CheckError> {
    let prompt = tool_prompt(id).ok_or_else(|| CheckError::UnsupportedId(id.to_owned()))?;
    let request = from_spec(
        format!("test-{id}"),
        tool_request(context.protocol, context.model, id, prompt),
        context,
    );
    Ok(CheckPlan::executed(vec![request]))
}

#[cfg(test)]
mod tests {
    use url::Url;

    use crate::protocol::tools::{ToolRequestPhase, validate_tool_request};
    use crate::protocol::{AuthMode, Protocol, normalize_request_url};

    use super::*;

    #[test]
    fn check_046_is_a_streaming_tool_loop_seed() {
        let context = PlanContext {
            protocol: Protocol::OpenAiChat,
            auth_mode: AuthMode::Bearer,
            model: "test-model",
        };

        let plan = super::super::plan("046", &context).expect("check 046 should be supported");
        let request = &plan.groups[0].requests()[0];
        let body = request.body.json();

        assert_eq!(request.id, "test-046");
        assert!(request.stream);
        assert!(
            body.pointer("/messages/0/content")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|prompt| prompt.contains("MODEL_DOCTOR_CASE_046"))
        );
        assert!(
            body.get("tools")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|tools| tools.iter().any(|tool| {
                    tool.pointer("/function/name")
                        .and_then(serde_json::Value::as_str)
                        == Some("get_weather")
                }))
        );
    }

    #[test]
    fn tool_loop_seeds_validate_for_all_official_protocols() {
        for id in ["046", "047", "048", "049"] {
            for protocol in [
                Protocol::OpenAiChat,
                Protocol::OpenAiResponses,
                Protocol::AnthropicMessages,
                Protocol::GeminiGenerateContent,
                Protocol::OllamaChat,
            ] {
                let context = PlanContext {
                    protocol,
                    auth_mode: AuthMode::Bearer,
                    model: "test-model",
                };
                let plan = super::super::plan(id, &context).expect("tool check is supported");
                let request = &plan.groups[0].requests()[0];
                let configured = if protocol == Protocol::GeminiGenerateContent {
                    Url::parse("https://example.com/v1/models/gemini:generateContent?key=test")
                } else {
                    Url::parse("https://example.com/v1/chat")
                }
                .expect("valid endpoint");
                let endpoint = normalize_request_url(protocol, &configured, request.stream);

                assert!(request.stream, "{protocol:?} check {id}");
                assert_eq!(
                    validate_tool_request(
                        protocol,
                        &endpoint,
                        request.stream,
                        request.body.json(),
                        ToolRequestPhase::Initial,
                    ),
                    Vec::<String>::new(),
                    "{protocol:?} check {id}"
                );
            }
        }
    }
}
