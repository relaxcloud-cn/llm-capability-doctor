use crate::protocol::{basic_request, multi_turn_request, thinking_request};

use super::{CheckError, CheckPlan, PlanContext, from_spec};

pub(super) fn plan(id: &str, context: &PlanContext<'_>) -> Result<CheckPlan, CheckError> {
    let requests = match id {
        "009" | "010" | "011" | "012" | "013" => {
            vec![basic(id, structured_prompt(id), false, context)]
        }
        "014" | "015" | "016" | "017" | "018" => vec![context_capacity(id, context)],
        "019" | "020" | "021" | "022" | "023" | "024" | "025" | "037" | "038" | "039" => {
            vec![basic(id, text_prompt(id), false, context)]
        }
        "026" | "027" | "028" | "029" | "030" => vec![context_recall(id, context)],
        "031" => vec![from_spec(
            "test-031",
            multi_turn_request(context.protocol, context.model),
            context,
        )],
        "032" | "034" => vec![thinking(
            &format!("test-{id}"),
            "MODEL_DOCTOR_THINKING_OK",
            "low",
            false,
            context,
        )],
        "033" => vec![
            thinking(
                "test-033-low",
                "MODEL_DOCTOR_THINKING_OK",
                "low",
                false,
                context,
            ),
            thinking(
                "test-033-high",
                "MODEL_DOCTOR_THINKING_OK",
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
        "009" => r#"MODEL_DOCTOR_CASE_009. Return exactly this JSON object with no Markdown or prose: {"status":"ok"}"#,
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
        "019" => "MODEL_DOCTOR_CASE_019. Reply only MODEL_DOCTOR_CASE_019_OK.",
        "020" => "MODEL_DOCTOR_CASE_020. Return exactly three lines: [BEGIN] then ALPHA|BETA|GAMMA then [END]. Do not use FORBIDDEN.",
        "021" => "MODEL_DOCTOR_CASE_021. The latest instruction wins: replace OLD_VALUE with NEW_VALUE and reply only NEW_VALUE.",
        "022" => "MODEL_DOCTOR_CASE_022. From time=10:32 source=203.0.113.7 action=allow, return time and source only.",
        "023" => "MODEL_DOCTOR_CASE_023. For urgent database timeout, return all applicable labels from URGENT, DATABASE, NETWORK.",
        "024" => "MODEL_DOCTOR_CASE_024. In at most 12 English words preserve: deployment failed at 14:20, rollback succeeded, no data loss.",
        "025" => "MODEL_DOCTOR_CASE_025. Merge and deduplicate alpha beta; beta gamma. Reply only alpha,beta,gamma.",
        "037" => "MODEL_DOCTOR_CASE_037. Compute (17 * 3) - (28 / 2). Reply only 37.",
        "038" => r#"MODEL_DOCTOR_CASE_038. A is before B. B is 12 minutes after 09:10. C is 5 minutes after B. Reply only compact JSON with exactly these keys: {"order":["A","B","C"],"bTime":"HH:MM","cTime":"HH:MM"}."#,
        "039" => "MODEL_DOCTOR_CASE_039. Return exactly STEP-1, STEP-2, STEP-3 and VERIFY for a three-step validation plan.",
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
    let expected = format!("CTX_{id}_OK");
    let prompt = format!(
        "Read all supplied context and reply only {expected}. Context: {} Hidden value: {expected}.",
        generate_filler(target)
    );
    basic(id, prompt, false, context)
}

fn context_recall(id: &str, context: &PlanContext<'_>) -> super::PlannedRequest {
    let prompt = match id {
        "026" => format!(
            "Hidden value: CTX_026_OK. Read all context and reply only the hidden value. {}",
            generate_filler(32_000)
        ),
        "027" => {
            let half = generate_filler(16_000);
            format!(
                "Read all context. {half} Hidden value: CTX_027_OK. {half} Reply only the hidden value."
            )
        }
        "028" => format!(
            "Read all context and reply only the hidden value. {} Hidden value: CTX_028_OK.",
            generate_filler(32_000)
        ),
        "029" => {
            let half = generate_filler(10_000);
            format!(
                "MODEL_DOCTOR_CASE_029. Reply only <first-marker>;<second-marker>;<prefix>-<suffix> using the labeled values from the full context. First marker: CTX_029_A. Prefix: ALPHA. {half} Second marker: CTX_029_B. {half} Suffix: GAMMA."
            )
        }
        "030" => format!(
            "Target is primary=ZX-7319; distractors primacy=ZX-7318 and primary-old=ZX-7310. {} Return only the primary target.",
            generate_filler(32_000)
        ),
        _ => unreachable!("context recall ID is validated by caller"),
    };
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
