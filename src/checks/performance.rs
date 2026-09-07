use crate::protocol::basic_request;

use super::{
    CheckError, CheckPlan, ManifestRefs, PlanContext, PlannedRequest, RequestGroup, from_spec,
};

pub(super) fn plan(id: &str, context: &PlanContext<'_>) -> Result<CheckPlan, CheckError> {
    let plan = match id {
        "052" | "054" => CheckPlan::executed(vec![basic(
            &format!("test-{id}"),
            &format!("Reply only MODEL_DOCTOR_CASE_{id}_OK"),
            false,
            context,
        )]),
        "053" => CheckPlan::executed(vec![basic(
            "test-053",
            "Reply only MODEL_DOCTOR_CASE_053_OK",
            true,
            context,
        )]),
        "055" | "056" => {
            let requests = (1..=5)
                .map(|index| {
                    basic(
                        &format!("test-055-repeat-{index}"),
                        "Reply only MODEL_DOCTOR_CASE_055_SAMPLE_OK",
                        false,
                        context,
                    )
                })
                .collect();
            CheckPlan {
                groups: vec![RequestGroup::Sequential(requests)],
                manifest_refs: ManifestRefs::SharedRepeatSamples,
            }
        }
        "057" => {
            let groups = [4_usize, 8, 16]
                .into_iter()
                .map(|concurrency| {
                    let requests = (1..=concurrency)
                        .map(|index| {
                            basic(
                                &format!("test-057-c{concurrency}-{index}"),
                                "Reply with any short non-empty response.",
                                false,
                                context,
                            )
                        })
                        .collect();
                    RequestGroup::Concurrent(requests)
                })
                .collect();
            CheckPlan {
                groups,
                manifest_refs: ManifestRefs::Executed,
            }
        }
        _ => return Err(CheckError::UnsupportedId(id.to_owned())),
    };
    Ok(plan)
}

fn basic(
    request_id: &str,
    prompt: &str,
    stream: bool,
    context: &PlanContext<'_>,
) -> PlannedRequest {
    from_spec(
        request_id,
        basic_request(context.protocol, context.model, prompt, stream),
        context,
    )
}

#[cfg(test)]
mod tests {
    use crate::protocol::{AuthMode, Protocol};

    use super::*;

    #[test]
    fn check_057_requests_any_short_non_empty_response() {
        let context = PlanContext {
            protocol: Protocol::OpenAiChat,
            auth_mode: AuthMode::None,
            model: "fixture-model",
        };

        let plan = plan("057", &context).expect("057 plan");
        assert_eq!(plan.groups.len(), 3);
        for (group, expected_concurrency) in plan.groups.iter().zip([4, 8, 16]) {
            assert!(
                matches!(group, RequestGroup::Concurrent(requests) if requests.len() == expected_concurrency)
            );
        }
        let requests = plan
            .groups
            .iter()
            .flat_map(RequestGroup::requests)
            .collect::<Vec<_>>();

        assert_eq!(requests.len(), 28);
        for request in requests {
            let prompt = request.body.json()["messages"][0]["content"]
                .as_str()
                .expect("OpenAI Chat prompt");
            assert_eq!(prompt, "Reply with any short non-empty response.");
            assert!(!prompt.contains("MODEL_DOCTOR_057"));
        }
    }
}
