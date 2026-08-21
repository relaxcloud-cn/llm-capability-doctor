use serde::Serialize;
use thiserror::Error;

use super::evidence_reader::{ParsedEvidence, ParsedRequest};
use super::rules::rule_for;

pub const MAX_EXCERPT_BYTES: usize = 4_096;
pub const MAX_CHECKS_PER_BATCH: usize = 4;
pub const MAX_BATCH_BYTES: usize = 65_536;
const OMISSION_MARKER: &str = "\n...[bytes omitted]...\n";
const MAX_PACKET_EXCERPT_BYTES: usize = 32_768;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidencePacket {
    pub test_id: String,
    pub name: String,
    pub category: String,
    pub pass_criteria: String,
    pub allowed_evidence_refs: Vec<String>,
    pub requests: Vec<PacketRequest>,
    pub deterministic_facts: DeterministicFacts,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PacketRequest {
    pub request_id: String,
    pub protocol: String,
    pub stream: bool,
    pub transport_outcome: String,
    pub stream_termination: String,
    pub stream_end_signal: String,
    pub tool_contract_status: String,
    pub tool_loop_outcome: String,
    pub http_status: Option<u16>,
    pub time_total_seconds: Option<f64>,
    pub time_starttransfer_seconds: Option<f64>,
    pub request_body_excerpt: String,
    pub response_body_excerpt: String,
    pub stderr_excerpt: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeterministicFacts {
    pub request_count: usize,
    pub hard_failures: Vec<String>,
}

#[derive(Debug, Error)]
pub enum PacketError {
    #[error("unknown test ID {0}")]
    UnknownTest(String),
    #[error("test {test_id} references missing request {request_id}")]
    MissingRequest { test_id: String, request_id: String },
    #[error("packet {test_id} is larger than the {max_bytes}-byte batch limit")]
    PacketTooLarge { test_id: String, max_bytes: usize },
    #[error("batch limits must be positive")]
    InvalidBatchLimit,
    #[error("failed to measure packet JSON: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn build_packet(
    evidence: &ParsedEvidence,
    test_id: &str,
) -> Result<EvidencePacket, PacketError> {
    let test = evidence
        .tests
        .get(test_id)
        .ok_or_else(|| PacketError::UnknownTest(test_id.to_owned()))?;
    let rule = rule_for(test_id).ok_or_else(|| PacketError::UnknownTest(test_id.to_owned()))?;
    let requests: Vec<&ParsedRequest> = test
        .request_refs
        .iter()
        .map(|request_id| {
            evidence
                .requests
                .get(request_id)
                .ok_or_else(|| PacketError::MissingRequest {
                    test_id: test_id.to_owned(),
                    request_id: request_id.clone(),
                })
        })
        .collect::<Result<_, _>>()?;

    let hard_failures = deterministic_failures(rule, &requests);
    let excerpt_limit =
        (MAX_PACKET_EXCERPT_BYTES / requests.len().max(1) / 3).clamp(128, MAX_EXCERPT_BYTES);
    Ok(EvidencePacket {
        test_id: test.id.clone(),
        name: test.name.clone(),
        category: test.category.clone(),
        pass_criteria: rule.pass_criteria.to_owned(),
        allowed_evidence_refs: test
            .request_refs
            .iter()
            .map(|request_id| format!("request:{request_id}"))
            .collect(),
        requests: requests
            .into_iter()
            .map(|request| packet_request(request, excerpt_limit))
            .collect(),
        deterministic_facts: DeterministicFacts {
            request_count: test.request_refs.len(),
            hard_failures,
        },
    })
}

pub fn build_all_packets(evidence: &ParsedEvidence) -> Result<Vec<EvidencePacket>, PacketError> {
    crate::catalog::CATALOG
        .iter()
        .map(|test| build_packet(evidence, test.id))
        .collect()
}

pub fn batch_packets(
    packets: Vec<EvidencePacket>,
    max_checks: usize,
    max_bytes: usize,
) -> Result<Vec<Vec<EvidencePacket>>, PacketError> {
    if max_checks == 0 || max_bytes == 0 {
        return Err(PacketError::InvalidBatchLimit);
    }
    let mut batches = Vec::new();
    let mut current = Vec::new();
    for packet in packets {
        if serde_json::to_vec(&[&packet])?.len() > max_bytes {
            return Err(PacketError::PacketTooLarge {
                test_id: packet.test_id,
                max_bytes,
            });
        }
        let would_exceed_bytes = !current.is_empty() && {
            let mut candidate: Vec<&EvidencePacket> = current.iter().collect();
            candidate.push(&packet);
            serde_json::to_vec(&candidate)?.len() > max_bytes
        };
        if current.len() == max_checks || would_exceed_bytes {
            batches.push(std::mem::take(&mut current));
        }
        current.push(packet);
    }
    if !current.is_empty() {
        batches.push(current);
    }
    Ok(batches)
}

pub fn default_batches(evidence: &ParsedEvidence) -> Result<Vec<Vec<EvidencePacket>>, PacketError> {
    batch_packets(
        build_all_packets(evidence)?,
        MAX_CHECKS_PER_BATCH,
        MAX_BATCH_BYTES,
    )
}

pub fn bounded_excerpt(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    if max_bytes <= OMISSION_MARKER.len() {
        let end = floor_char_boundary(value, max_bytes);
        return value[..end].to_owned();
    }

    let available = max_bytes - OMISSION_MARKER.len();
    let head_end = floor_char_boundary(value, available / 2);
    let tail_budget = available - head_end;
    let tail_start = ceil_char_boundary(value, value.len().saturating_sub(tail_budget));
    format!(
        "{}{}{}",
        &value[..head_end],
        OMISSION_MARKER,
        &value[tail_start..]
    )
}

fn packet_request(request: &ParsedRequest, excerpt_limit: usize) -> PacketRequest {
    PacketRequest {
        request_id: request.request_id.clone(),
        protocol: metadata(request, "protocol", "unknown"),
        stream: metadata(request, "stream", "0") == "1",
        transport_outcome: metadata(request, "transport_outcome", "unknown"),
        stream_termination: metadata(request, "stream_termination", "unknown"),
        stream_end_signal: metadata(request, "stream_end_signal", "none"),
        tool_contract_status: metadata(request, "tool_contract_status", "not_applicable"),
        tool_loop_outcome: metadata(request, "tool_loop_outcome", "not_applicable"),
        http_status: metric(request, "http_status").and_then(|value| value.parse().ok()),
        time_total_seconds: metric(request, "time_total").and_then(|value| value.parse().ok()),
        time_starttransfer_seconds: metric(request, "time_starttransfer")
            .and_then(|value| value.parse().ok()),
        request_body_excerpt: bounded_excerpt(&request.request_body, excerpt_limit),
        response_body_excerpt: bounded_excerpt(&request.response_body, excerpt_limit),
        stderr_excerpt: bounded_excerpt(&request.stderr, excerpt_limit),
    }
}

fn deterministic_failures(
    rule: &super::rules::AnalysisRule,
    requests: &[&ParsedRequest],
) -> Vec<String> {
    let mut failures = Vec::new();
    if rule.require_all_transport_success
        && requests.iter().any(|request| !transport_succeeded(request))
    {
        failures.push("at least one required request did not complete with HTTP 2xx".into());
    }
    if rule.require_complete_streams
        && requests.iter().any(|request| {
            metadata(request, "stream", "0") == "1"
                && metadata(request, "stream_termination", "unknown") != "completed"
        })
    {
        failures.push("at least one required stream is incomplete".into());
    }
    if rule.require_tool_conformance
        && requests
            .iter()
            .any(|request| metadata(request, "tool_contract_status", "unknown") != "conformant")
    {
        failures.push("at least one required tool turn is non-conformant".into());
    }
    if let Some(marker) = rule.required_marker
        && !requests
            .iter()
            .any(|request| request.response_body.contains(marker))
    {
        failures.push(format!(
            "required marker {marker} is absent from response evidence"
        ));
    }
    failures
}

fn transport_succeeded(request: &ParsedRequest) -> bool {
    matches!(
        metadata(request, "transport_outcome", "unknown").as_str(),
        "completed_eof" | "protocol_terminated"
    ) && metric(request, "http_status")
        .and_then(|value| value.parse::<u16>().ok())
        .is_some_and(|status| (200..300).contains(&status))
}

fn metadata(request: &ParsedRequest, key: &str, fallback: &str) -> String {
    request
        .metadata
        .get(key)
        .map_or_else(|| fallback.to_owned(), Clone::clone)
}

fn metric<'a>(request: &'a ParsedRequest, key: &str) -> Option<&'a str> {
    request.metrics.get(key).map(String::as_str)
}

fn floor_char_boundary(value: &str, mut index: usize) -> usize {
    index = index.min(value.len());
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(value: &str, mut index: usize) -> usize {
    index = index.min(value.len());
    while index < value.len() && !value.is_char_boundary(index) {
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::*;
    use crate::analysis::evidence_reader::{
        EvidenceSource, ParsedEvidence, ParsedRequest, ParsedTest,
    };

    #[test]
    fn packet_contains_only_manifest_owned_requests() {
        let parsed = parsed_fixture_with_two_requests();

        let packet = build_packet(&parsed, "019").unwrap();

        assert_eq!(packet.requests.len(), 1);
        assert_eq!(packet.requests[0].request_id, "test-019");
        assert!(
            !serde_json::to_string(&packet)
                .unwrap()
                .contains("unrelated-secret-marker")
        );
    }

    #[test]
    fn compaction_keeps_head_and_tail_inside_limit() {
        let input = format!("HEAD{}TAIL", "x".repeat(20_000));

        let compact = bounded_excerpt(&input, 4_096);

        assert!(compact.starts_with("HEAD"));
        assert!(compact.ends_with("TAIL"));
        assert!(compact.len() <= 4_096);
        assert!(compact.contains("bytes omitted"));
    }

    #[test]
    fn compaction_preserves_utf8_boundaries() {
        let input = format!("开头{}结尾", "测".repeat(10_000));

        let compact = bounded_excerpt(&input, 4_096);

        assert!(compact.starts_with("开头"));
        assert!(compact.ends_with("结尾"));
        assert!(compact.len() <= 4_096);
    }

    #[test]
    fn batches_respect_count_and_serialized_byte_limits() {
        let packets = (0..9)
            .map(|index| packet_fixture(&format!("{index:03}"), 20_000))
            .collect();

        let batches = batch_packets(packets, 4, 65_536).unwrap();

        assert!(batches.iter().all(|batch| batch.len() <= 4));
        assert!(
            batches
                .iter()
                .all(|batch| serde_json::to_vec(batch).unwrap().len() <= 65_536)
        );
        assert_eq!(batches.iter().map(Vec::len).sum::<usize>(), 9);
    }

    #[test]
    fn high_fanout_check_is_compacted_below_one_batch() {
        let mut parsed = parsed_fixture_with_two_requests();
        parsed.requests.clear();
        let request_refs: Vec<String> = (0..60)
            .map(|index| {
                let request_id = format!("test-057-wave-{index}");
                parsed.requests.insert(
                    request_id.clone(),
                    parsed_request(&request_id, &"response".repeat(2_000)),
                );
                request_id
            })
            .collect();
        parsed.tests = BTreeMap::from([(
            "057".into(),
            ParsedTest {
                id: "057".into(),
                name: "并发".into(),
                category: "性能与稳定性".into(),
                request_refs,
            },
        )]);

        let packet = build_packet(&parsed, "057").unwrap();

        assert_eq!(packet.requests.len(), 60);
        assert!(serde_json::to_vec(&[packet]).unwrap().len() <= MAX_BATCH_BYTES);
    }

    fn parsed_fixture_with_two_requests() -> ParsedEvidence {
        ParsedEvidence {
            source: EvidenceSource {
                path: PathBuf::from("doctor.log"),
                file_name: "doctor.log".into(),
                size_bytes: 1,
                sha256: "0".repeat(64),
            },
            run: BTreeMap::new(),
            requests: BTreeMap::from([
                ("test-019".into(), parsed_request("test-019", "expected")),
                (
                    "test-other".into(),
                    parsed_request("test-other", "unrelated-secret-marker"),
                ),
            ]),
            tests: BTreeMap::from([(
                "019".into(),
                ParsedTest {
                    id: "019".into(),
                    name: "精确输出".into(),
                    category: "指令与文本".into(),
                    request_refs: vec!["test-019".into()],
                },
            )]),
        }
    }

    fn parsed_request(request_id: &str, response_body: &str) -> ParsedRequest {
        ParsedRequest {
            request_id: request_id.into(),
            metadata: BTreeMap::from([
                ("stream".into(), "0".into()),
                ("transport_outcome".into(), "completed_eof".into()),
                ("stream_termination".into(), "not_applicable".into()),
                ("stream_end_signal".into(), "none".into()),
                ("tool_contract_status".into(), "not_applicable".into()),
                ("tool_loop_outcome".into(), "not_applicable".into()),
            ]),
            metrics: BTreeMap::from([
                ("http_status".into(), "200".into()),
                ("time_total".into(), "1.0".into()),
                ("time_starttransfer".into(), "0.5".into()),
            ]),
            request_body: "prompt".into(),
            response_headers: "content-type: application/json".into(),
            stderr: String::new(),
            response_body: response_body.into(),
        }
    }

    fn packet_fixture(test_id: &str, response_size: usize) -> EvidencePacket {
        EvidencePacket {
            test_id: test_id.into(),
            name: "check".into(),
            category: "category".into(),
            pass_criteria: "pass".into(),
            allowed_evidence_refs: vec![format!("request:test-{test_id}")],
            requests: vec![PacketRequest {
                request_id: format!("test-{test_id}"),
                protocol: "openai_chat".into(),
                stream: false,
                transport_outcome: "completed_eof".into(),
                stream_termination: "not_applicable".into(),
                stream_end_signal: "none".into(),
                tool_contract_status: "not_applicable".into(),
                tool_loop_outcome: "not_applicable".into(),
                http_status: Some(200),
                time_total_seconds: Some(1.0),
                time_starttransfer_seconds: Some(0.5),
                request_body_excerpt: "prompt".into(),
                response_body_excerpt: "x".repeat(response_size),
                stderr_excerpt: String::new(),
            }],
            deterministic_facts: DeterministicFacts {
                request_count: 1,
                hard_failures: Vec::new(),
            },
        }
    }
}
