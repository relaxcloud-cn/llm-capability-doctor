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
            let concurrency_levels: &[usize] = if context.onsite {
                &[8]
            } else {
                &[4, 8, 16, 32]
            };
            let groups = concurrency_levels
                .iter()
                .copied()
                .map(|concurrency| {
                    let requests = (1..=concurrency)
                        .map(|index| {
                            basic(
                                &format!("test-057-c{concurrency}-{index}"),
                                &format!("Reply only MODEL_DOCTOR_057_C{concurrency}_OK"),
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
