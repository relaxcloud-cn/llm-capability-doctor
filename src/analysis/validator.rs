use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::packet::EvidencePacket;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum CandidateStatus {
    Pass,
    Fail,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CandidateReview {
    pub test_id: String,
    pub candidate_status: CandidateStatus,
    pub observations: Vec<String>,
    pub failure_cause: Option<String>,
    pub evidence_refs: Vec<String>,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateEnvelope {
    pub reviews: Vec<CandidateReview>,
}

#[derive(Clone, Copy, Debug, Serialize, Eq, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum ValidatedStatus {
    Pass,
    Fail,
}

#[derive(Clone, Copy, Debug, Serialize, Eq, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DecisionSource {
    TargetModel,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidatedReview {
    pub test_id: String,
    pub candidate_status: CandidateStatus,
    pub validated_status: ValidatedStatus,
    pub decision_source: DecisionSource,
    pub observations: Vec<String>,
    pub failure_cause: Option<String>,
    pub evidence_refs: Vec<String>,
    pub limitations: Vec<String>,
    pub validation_notes: Vec<String>,
}

pub fn validate_candidates(
    packets: &[EvidencePacket],
    envelope: CandidateEnvelope,
) -> Result<Vec<ValidatedReview>, Vec<String>> {
    let expected: HashMap<_, _> = packets
        .iter()
        .map(|packet| (packet.test_id.as_str(), packet))
        .collect();
    let mut candidates = HashMap::new();
    let mut errors = Vec::new();

    for candidate in envelope.reviews {
        let test_id = candidate.test_id.clone();
        let Some(packet) = expected.get(test_id.as_str()) else {
            errors.push(format!("unknown candidate test ID {test_id}"));
            continue;
        };
        if candidates
            .insert(test_id.clone(), candidate.clone())
            .is_some()
        {
            errors.push(format!("duplicate candidate test ID {test_id}"));
            continue;
        }
        validate_candidate(packet, &candidate, &mut errors);
    }
    for packet in packets {
        if !candidates.contains_key(&packet.test_id) {
            errors.push(format!("missing candidate test ID {}", packet.test_id));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(packets
        .iter()
        .map(|packet| {
            let candidate = candidates
                .remove(&packet.test_id)
                .expect("candidate completeness checked above");
            accept_candidate(candidate)
        })
        .collect())
}

fn validate_candidate(
    packet: &EvidencePacket,
    candidate: &CandidateReview,
    errors: &mut Vec<String>,
) {
    if candidate.observations.is_empty()
        || candidate
            .observations
            .iter()
            .any(|value| value.trim().is_empty())
    {
        errors.push(format!(
            "candidate {} observations must be non-empty strings",
            candidate.test_id
        ));
    }
    match candidate.candidate_status {
        CandidateStatus::Pass if candidate.failure_cause.is_some() => errors.push(format!(
            "candidate {} PASS must not include failureCause",
            candidate.test_id
        )),
        CandidateStatus::Fail
            if candidate
                .failure_cause
                .as_deref()
                .is_none_or(|value| value.trim().is_empty()) =>
        {
            errors.push(format!(
                "candidate {} FAIL requires failureCause",
                candidate.test_id
            ));
        }
        _ => {}
    }
    if candidate
        .limitations
        .iter()
        .any(|value| value.trim().is_empty())
    {
        errors.push(format!(
            "candidate {} limitations must be non-empty strings",
            candidate.test_id
        ));
    }

    let allowed: HashSet<_> = packet.allowed_evidence_refs.iter().collect();
    let supplied: HashSet<_> = candidate.evidence_refs.iter().collect();
    if candidate.evidence_refs.is_empty() {
        errors.push(format!(
            "candidate {} requires at least one evidence reference",
            candidate.test_id
        ));
    }
    if supplied.len() != candidate.evidence_refs.len() {
        errors.push(format!(
            "candidate {} contains duplicate evidence references",
            candidate.test_id
        ));
    }
    for reference in &candidate.evidence_refs {
        if !allowed.contains(reference) {
            errors.push(format!(
                "candidate {} uses foreign evidence reference {reference}",
                candidate.test_id
            ));
        }
    }
}

fn accept_candidate(candidate: CandidateReview) -> ValidatedReview {
    let validated_status = match candidate.candidate_status {
        CandidateStatus::Pass => ValidatedStatus::Pass,
        CandidateStatus::Fail => ValidatedStatus::Fail,
    };
    ValidatedReview {
        test_id: candidate.test_id,
        candidate_status: candidate.candidate_status,
        validated_status,
        decision_source: DecisionSource::TargetModel,
        observations: candidate.observations,
        failure_cause: candidate.failure_cause,
        evidence_refs: candidate.evidence_refs,
        limitations: candidate.limitations,
        validation_notes: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::packet::EvidencePacket;

    #[test]
    fn rejects_candidate_with_foreign_reference() {
        let packet = packet_fixture();
        let candidate = candidate_fixture(CandidateStatus::Pass, &["request:other-test"]);

        let errors = validate_candidates(
            &[packet],
            CandidateEnvelope {
                reviews: vec![candidate],
            },
        )
        .unwrap_err();

        assert!(
            errors
                .iter()
                .any(|error| error.contains("foreign evidence reference"))
        );
    }

    #[test]
    fn target_model_pass_remains_authoritative() {
        let packet = packet_fixture();
        let candidate = candidate_fixture(CandidateStatus::Pass, &["request:test-006"]);

        let validated = validate_candidates(
            &[packet],
            CandidateEnvelope {
                reviews: vec![candidate],
            },
        )
        .unwrap();

        assert_eq!(validated[0].candidate_status, CandidateStatus::Pass);
        assert_eq!(validated[0].validated_status, ValidatedStatus::Pass);
        assert_eq!(validated[0].decision_source, DecisionSource::TargetModel);
    }

    #[test]
    fn target_model_fail_remains_authoritative() {
        let packet = packet_fixture();
        let mut candidate = candidate_fixture(CandidateStatus::Fail, &["request:test-006"]);
        candidate.failure_cause = Some("model reported an incomplete answer".into());

        let validated = validate_candidates(
            &[packet],
            CandidateEnvelope {
                reviews: vec![candidate],
            },
        )
        .unwrap();

        assert_eq!(validated[0].candidate_status, CandidateStatus::Fail);
        assert_eq!(validated[0].validated_status, ValidatedStatus::Fail);
        assert_eq!(validated[0].decision_source, DecisionSource::TargetModel);
    }

    fn packet_fixture() -> EvidencePacket {
        EvidencePacket {
            test_id: "006".into(),
            name: "流结束完整性".into(),
            category: "接口与协议".into(),
            pass_criteria: "流完整结束".into(),
            allowed_evidence_refs: vec!["request:test-006".into()],
            requests: Vec::new(),
        }
    }

    fn candidate_fixture(
        candidate_status: CandidateStatus,
        evidence_refs: &[&str],
    ) -> CandidateReview {
        CandidateReview {
            test_id: "006".into(),
            candidate_status,
            observations: vec!["observable fact".into()],
            failure_cause: None,
            evidence_refs: evidence_refs
                .iter()
                .map(|reference| (*reference).to_owned())
                .collect(),
            limitations: Vec::new(),
        }
    }
}
