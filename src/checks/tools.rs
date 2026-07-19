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
