use crate::protocol::{basic_request, multi_turn_request, thinking_request};

use super::{CheckError, CheckPlan, PlanContext, from_spec};

pub(super) fn plan(id: &str, context: &PlanContext<'_>) -> Result<CheckPlan, CheckError> {
    let requests = match id {
        "009" | "010" | "011" | "012" | "013" => {
            vec![basic(id, structured_prompt(id), false, context)]
        }
        "014" | "015" | "016" | "017" | "018" => vec![context_capacity(id, context)],
        "019" | "020" | "022" | "024" | "038" => {
            vec![basic(id, text_prompt(id), false, context)]
        }
        "031" => vec![from_spec(
            "test-031",
            multi_turn_request(context.protocol, context.model),
            context,
        )],
        "033" => vec![
            thinking(
                "test-033-low",
                "Reply only MODEL_DOCTOR_THINKING_OK.",
                "low",
                false,
                context,
            ),
            thinking(
                "test-033-high",
                "Reply only MODEL_DOCTOR_THINKING_OK.",
                "high",
                false,
                context,
            ),
        ],
        "035" => vec![thinking(
            "test-035",
            "Compute 19 + 23 internally. Reply only MODEL_DOCTOR_CASE_035_OK.",
            "low",
            false,
            context,
        )],
        "036" => vec![thinking(
            "test-036",
            "Reply only MODEL_DOCTOR_CASE_036_OK",
            "low",
            true,
            context,
        )],
        _ => return Err(CheckError::UnsupportedId(id.to_owned())),
    };
    Ok(CheckPlan::executed(requests))
}

fn basic(
    id: &str,
    prompt: String,
    stream: bool,
    context: &PlanContext<'_>,
) -> super::PlannedRequest {
    from_spec(
        format!("test-{id}"),
        basic_request(context.protocol, context.model, &prompt, stream),
        context,
    )
}

fn thinking(
    request_id: &str,
    prompt: &str,
    effort: &str,
    stream: bool,
    context: &PlanContext<'_>,
) -> super::PlannedRequest {
    from_spec(
        request_id,
        thinking_request(context.protocol, context.model, prompt, effort, stream),
        context,
    )
}

fn structured_prompt(id: &str) -> String {
    match id {
        "009" => r#"MODEL_DOCTOR_CASE_009. Return exactly this JSON object with no Markdown or prose: {"status":"ok","count":7,"enabled":true,"profile":{"name":"Ada"},"tags":["red","blue"],"note":null}"#,
        "010" => r#"MODEL_DOCTOR_CASE_010. Return exactly: {"name":"alpha","count":7,"enabled":true}"#,
        "011" => r#"MODEL_DOCTOR_CASE_011. Return exactly: {"profile":{"name":"Ada"},"tags":["red","blue"],"note":null}"#,
        "012" => "MODEL_DOCTOR_CASE_012. Return one JSON object containing result.verdict=risk, result.impact=high and result.nextMove=verify.",
        "013" => "MODEL_DOCTOR_CASE_013. Return one JSON object with investigationStages containing stageId STAGE-001 whose evidenceRefs contains EVID-001, and evidence defining evidenceId EVID-001.",
        _ => unreachable!("structured prompt ID is validated by caller"),
    }
    .to_owned()
}

fn text_prompt(id: &str) -> String {
    match id {
        "019" => "MODEL_DOCTOR_CASE_019. Output exactly the token between the markers below. Do not output the markers, punctuation, whitespace, or any other text.\n<token>MODEL_DOCTOR_CASE_019_OK</token>",
        "020" => "MODEL_DOCTOR_CASE_020. Return exactly these three lines and nothing else:\n[BEGIN]\nALPHA|BETA|GAMMA\n[END]\nThe two ASCII vertical bar characters \"|\" are literal output characters and must both be present. Do not output the word FORBIDDEN.",
        "022" => r#"MODEL_DOCTOR_CASE_022. From time=10:32 source=203.0.113.7 action=allow labels=URGENT,DATABASE excluded_label=NETWORK, return exactly this compact JSON shape: {"time":"10:32","source":"203.0.113.7","action":"allow","labels":["URGENT","DATABASE"]}. Copy labels only from the comma-separated labels field, preserve their order, never copy excluded_label, and never infer labels from any other field."#,
        "024" => "MODEL_DOCTOR_CASE_024. In at most 12 English words preserve: deployment failed at 14:20, rollback succeeded, no data loss.",
        "038" => r#"MODEL_DOCTOR_CASE_038. A is before B. B is 12 minutes after 09:10. C is 5 minutes after B. Reply only compact JSON with exactly these keys: {"order":["A","B","C"],"bTime":"HH:MM","cTime":"HH:MM"}."#,
        _ => unreachable!("text prompt ID is validated by caller"),
    }
    .to_owned()
}

fn context_capacity(id: &str, context: &PlanContext<'_>) -> super::PlannedRequest {
    let target = match id {
        "014" => 32_000,
        "015" => 64_000,
        "016" => 128_000,
        "017" => 256_000,
        "018" => 512_000,
        _ => unreachable!("context capacity ID is validated by caller"),
    };
    let segment = generate_filler(target / 3);
    let prompt = format!(
        "MODEL_DOCTOR_CONTEXT_{id}. This probe measures context acceptance only. \
         Response accuracy and exact wording are not evaluated. {segment}{segment}{segment} \
         The full request is complete. Return any short non-empty response."
    );
    basic(id, prompt, false, context)
}

fn generate_filler(target: usize) -> String {
    const BLOCK: &str = "FILLER_BLOCK_0123456789 ";
    let mut filler = String::with_capacity(target + BLOCK.len());
    while filler.len() < target {
        filler.push_str(BLOCK);
    }
    filler
}

#[cfg(test)]
mod tests {
    use crate::protocol::{AuthMode, Protocol};

    use super::*;

    #[test]
    fn context_capacity_probe_requests_only_a_non_empty_response() {
        let context = PlanContext {
            protocol: Protocol::OpenAiChat,
            auth_mode: AuthMode::None,
            model: "fixture-model",
        };
        let request = context_capacity("014", &context);
        let prompt = request.body.json()["messages"][0]["content"]
            .as_str()
            .expect("OpenAI Chat prompt");

        assert!(prompt.contains("This probe measures context acceptance only."));
        assert!(prompt.contains("Return any short non-empty response."));
        assert!(!prompt.contains("compact JSON shaped exactly"));
        assert!(!prompt.contains("Link prefix"));
        assert!(!prompt.contains("Primary target"));
    }

    #[test]
    fn check_020_requires_literal_ascii_vertical_bars() {
        let prompt = text_prompt("020");

        assert!(prompt.contains("ASCII vertical bar characters"));
        assert!(prompt.contains("literal output characters"));
        assert!(prompt.contains("ALPHA|BETA|GAMMA"));
    }

    #[test]
    fn check_019_delimits_the_exact_token_without_punctuation_ambiguity() {
        let prompt = text_prompt("019");

        assert!(prompt.contains("<token>MODEL_DOCTOR_CASE_019_OK</token>"));
        assert!(prompt.contains("Do not output the markers, punctuation, whitespace"));
        assert!(!prompt.ends_with("MODEL_DOCTOR_CASE_019_OK."));
    }

    #[test]
    fn check_022_copies_only_the_explicit_labels_field() {
        let prompt = text_prompt("022");

        assert!(prompt.contains("labels=URGENT,DATABASE"));
        assert!(prompt.contains("excluded_label=NETWORK"));
        assert!(prompt.contains("Copy labels only"));
        assert!(prompt.contains("never infer labels"));
    }
}
