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
    use crate::protocol::{AuthMode, Protocol};

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
}
