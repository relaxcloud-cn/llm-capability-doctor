use crate::protocol::{AuthMode, Protocol, basic_request};

use super::{Body, CheckError, CheckPlan, ManifestRefs, PlanContext, PlannedRequest, from_spec};

pub(super) fn plan(id: &str, context: &PlanContext<'_>) -> Result<CheckPlan, CheckError> {
    let plan = match id {
        "001" => {
            let spec = basic_request(
                Protocol::OpenAiChat,
                context.model,
                "Reply only MODEL_DOCTOR_OK",
                false,
            );
            let mut request = from_spec("test-001", spec, context);
            request.protocol = Protocol::Unknown;
            request.auth_mode = AuthMode::Bearer;
            CheckPlan::executed(vec![request])
        }
        "002" => CheckPlan::references(ManifestRefs::AllProtocolProbes),
        "003" | "007" => CheckPlan::references(ManifestRefs::SelectedProtocolProbe),
        "004" => one_basic(
            "test-004",
            "Reply only MODEL_DOCTOR_CASE_004_OK",
            false,
            context,
        ),
        "005" => one_basic(
            "test-005",
            "Reply only MODEL_DOCTOR_CASE_005_OK",
            true,
            context,
        ),
        "006" => one_basic(
            "test-006",
            "Reply only MODEL_DOCTOR_CASE_006_OK",
            true,
            context,
        ),
        "008" => CheckPlan::executed(vec![PlannedRequest {
            id: "test-008".into(),
            protocol: context.protocol,
            auth_mode: context.auth_mode,
            body: Body::Raw(br#"{"model":"#.to_vec()),
            stream: false,
        }]),
        _ => return Err(CheckError::UnsupportedId(id.to_owned())),
    };
    Ok(plan)
}

fn one_basic(request_id: &str, prompt: &str, stream: bool, context: &PlanContext<'_>) -> CheckPlan {
    let spec = basic_request(context.protocol, context.model, prompt, stream);
    CheckPlan::executed(vec![from_spec(request_id, spec, context)])
}
