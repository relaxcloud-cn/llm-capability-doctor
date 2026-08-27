use serde::Serialize;
use thiserror::Error;

use super::evidence_reader::{ParsedEvidence, ParsedRequest};
use super::rules::rule_for;

pub const MAX_EXCERPT_BYTES: usize = 16 * 1024;
pub const MAX_CHECKS_PER_BATCH: usize = 4;
pub const MAX_BATCH_BYTES: usize = 256 * 1024;
const OMISSION_MARKER: &str = "\n...[bytes omitted]...\n";
const MAX_PACKET_EXCERPT_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidencePacket {
    pub test_id: String,
    pub report_test_id: String,
    pub name: String,
    pub category: String,
    pub pass_criteria: String,
    pub allowed_evidence_refs: Vec<String>,
    pub requests: Vec<PacketRequest>,
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
    pub tool_loop_turn: usize,
    pub tool_loop_outcome: String,
    pub http_status: Option<u16>,
    pub time_total_seconds: Option<f64>,
    pub time_starttransfer_seconds: Option<f64>,
    pub context_target_tokens: Option<u64>,
    pub context_measured_tokens: Option<u64>,
    pub request_body_excerpt: String,
    pub response_body_excerpt: String,
    pub stderr_excerpt: String,
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

pub async fn build_packet(
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

    let mut excerpt_limit =
        (MAX_PACKET_EXCERPT_BYTES / requests.len().max(1) / 3).clamp(128, MAX_EXCERPT_BYTES);
    loop {
        let packet = EvidencePacket {
            test_id: test.id.clone(),
            report_test_id: test.id.clone(),
            name: test.name.clone(),
            category: test.category.clone(),
            pass_criteria: rule.pass_criteria.to_owned(),
            allowed_evidence_refs: test
                .request_refs
                .iter()
                .map(|request_id| format!("request:{request_id}"))
                .collect(),
            requests: requests
                .iter()
                .map(|request| packet_request(request, excerpt_limit))
                .collect(),
        };
        if serde_json::to_vec(&[&packet])?.len() <= MAX_BATCH_BYTES {
            return Ok(packet);
        }
        if excerpt_limit == 0 {
            return Err(PacketError::PacketTooLarge {
                test_id: test.id.clone(),
                max_bytes: MAX_BATCH_BYTES,
            });
        }
        excerpt_limit /= 2;
    }
}

pub async fn build_all_packets(
    evidence: &ParsedEvidence,
) -> Result<Vec<EvidencePacket>, PacketError> {
    let degraded: std::collections::HashSet<&str> = evidence
        .degraded_tests
        .iter()
        .map(|entry| entry.id.as_str())
        .collect();
    let mut packets = Vec::with_capacity(crate::catalog::CATALOG.len());
    for test in crate::catalog::CATALOG {
        if degraded.contains(test.id) {
            // 证据损坏的检测项不进入自分析，由 orchestrator 标记为分析不可用。
            continue;
        }
        if test.id == "057" {
            packets.extend(build_concurrency_packets(evidence).await?);
        } else {
            packets.push(build_packet(evidence, test.id).await?);
        }
    }
    Ok(packets)
}

async fn build_concurrency_packets(
    evidence: &ParsedEvidence,
) -> Result<Vec<EvidencePacket>, PacketError> {
    let test = evidence
        .tests
        .get("057")
        .ok_or_else(|| PacketError::UnknownTest("057".into()))?;
    let mut packets = Vec::new();
    for concurrency in [4, 8, 16, 32] {
        let request_refs = test
            .request_refs
            .iter()
            .filter(|request_id| concurrency_level(request_id) == Some(concurrency))
            .cloned()
            .collect::<Vec<_>>();
        let packet_id = format!("057-wave-{concurrency}");
        packets.push(
            build_packet_with_refs(
                evidence,
                "057",
                &packet_id,
                format!("{}（{concurrency} 并发）", test.name),
                request_refs,
            )
            .await?,
        );
    }
    if packets
        .iter()
        .any(|packet| packet.allowed_evidence_refs.is_empty())
    {
        return Ok(vec![build_packet(evidence, "057").await?]);
    }
    Ok(packets)
}

async fn build_packet_with_refs(
    evidence: &ParsedEvidence,
    report_test_id: &str,
    packet_test_id: &str,
    name: String,
    request_refs: Vec<String>,
) -> Result<EvidencePacket, PacketError> {
    let test = evidence
        .tests
        .get(report_test_id)
        .ok_or_else(|| PacketError::UnknownTest(report_test_id.to_owned()))?;
    let rule = rule_for(report_test_id)
        .ok_or_else(|| PacketError::UnknownTest(report_test_id.to_owned()))?;
    let requests: Vec<&ParsedRequest> = request_refs
        .iter()
        .map(|request_id| {
            evidence
                .requests
                .get(request_id)
                .ok_or_else(|| PacketError::MissingRequest {
                    test_id: report_test_id.to_owned(),
                    request_id: request_id.clone(),
                })
        })
        .collect::<Result<_, _>>()?;
    let mut excerpt_limit =
        (MAX_PACKET_EXCERPT_BYTES / requests.len().max(1) / 3).clamp(128, MAX_EXCERPT_BYTES);
    loop {
        let packet = EvidencePacket {
            test_id: packet_test_id.to_owned(),
            report_test_id: report_test_id.to_owned(),
            name: name.clone(),
            category: test.category.clone(),
            pass_criteria: rule.pass_criteria.to_owned(),
            allowed_evidence_refs: request_refs
                .iter()
                .map(|request_id| format!("request:{request_id}"))
                .collect(),
            requests: requests
                .iter()
                .map(|request| packet_request(request, excerpt_limit))
                .collect(),
        };
        if serde_json::to_vec(&[&packet])?.len() <= MAX_BATCH_BYTES {
            return Ok(packet);
        }
        if excerpt_limit == 0 {
            return Err(PacketError::PacketTooLarge {
                test_id: packet_test_id.to_owned(),
                max_bytes: MAX_BATCH_BYTES,
            });
        }
        excerpt_limit /= 2;
    }
}

fn concurrency_level(request_id: &str) -> Option<usize> {
    let suffix = request_id.strip_prefix("test-057-c")?;
    let (concurrency, index) = suffix.split_once('-')?;
    let concurrency = concurrency.parse::<usize>().ok()?;
    let index = index.parse::<usize>().ok()?;
    (matches!(concurrency, 4 | 8 | 16 | 32) && index > 0).then_some(concurrency)
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

pub async fn default_batches(
    evidence: &ParsedEvidence,
) -> Result<Vec<Vec<EvidencePacket>>, PacketError> {
    batch_packets(
        build_all_packets(evidence).await?,
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
    let context_target_tokens = crate::context_capacity::parse_probe_target(&request.request_id);
    let context_measured_tokens = context_target_tokens.and_then(|_| {
        crate::context_capacity::extract_prompt_tokens(
            crate::context_capacity::protocol_from_wire_name(
                request
                    .metadata
                    .get("protocol")
                    .map(String::as_str)
                    .unwrap_or(""),
            ),
            request.response_body.as_bytes(),
        )
    });
    PacketRequest {
        request_id: request.request_id.clone(),
        protocol: metadata(request, "protocol", "unknown"),
        stream: metadata(request, "stream", "0") == "1",
        transport_outcome: metadata(request, "transport_outcome", "unknown"),
        stream_termination: metadata(request, "stream_termination", "unknown"),
        stream_end_signal: metadata(request, "stream_end_signal", "none"),
        tool_contract_status: metadata(request, "tool_contract_status", "not_applicable"),
        tool_loop_turn: metadata(request, "tool_loop_turn", "0")
            .parse()
            .unwrap_or_default(),
        tool_loop_outcome: metadata(request, "tool_loop_outcome", "not_applicable"),
        http_status: metric(request, "http_status").and_then(|value| value.parse().ok()),
        time_total_seconds: metric(request, "time_total").and_then(|value| value.parse().ok()),
        time_starttransfer_seconds: metric(request, "time_starttransfer")
            .and_then(|value| value.parse().ok()),
        context_target_tokens,
        context_measured_tokens,
        request_body_excerpt: bounded_excerpt(&request.request_body, excerpt_limit),
        response_body_excerpt: bounded_excerpt(&request.response_body, excerpt_limit),
        stderr_excerpt: bounded_excerpt(&request.stderr, excerpt_limit),
    }
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

    #[tokio::test]
    async fn packet_contains_only_manifest_owned_requests() {
        let parsed = parsed_fixture_with_two_requests();

        let packet = build_packet(&parsed, "019").await.unwrap();

        assert_eq!(packet.requests.len(), 1);
        assert_eq!(packet.requests[0].request_id, "test-019");
        assert!(
            !serde_json::to_string(&packet)
                .unwrap()
                .contains("unrelated-secret-marker")
        );
    }

    #[tokio::test]
    async fn packet_contains_criteria_and_observations_without_authoritative_verdicts() {
        let parsed = parsed_fixture_with_two_requests();

        let packet = build_packet(&parsed, "019").await.unwrap();
        let serialized = serde_json::to_string(&packet).unwrap();

        assert!(serialized.contains("passCriteria"));
        assert!(serialized.contains("requestBodyExcerpt"));
        assert!(!serialized.contains("hardFailures"));
    }

    #[tokio::test]
    async fn packet_preserves_diagnostic_observations_for_the_model() {
        let mut request = parsed_request("test-019", "response body");
        request
            .metadata
            .insert("transport_outcome".into(), "timeout".into());
        request
            .metadata
            .insert("tool_contract_status".into(), "non_conformant".into());
        request.metrics.insert("http_status".into(), "500".into());
        let evidence = single_test_evidence("019", request);

        let packet = build_packet(&evidence, "019").await.unwrap();

        assert_eq!(packet.requests[0].transport_outcome, "timeout");
        assert_eq!(packet.requests[0].tool_contract_status, "non_conformant");
        assert_eq!(packet.requests[0].http_status, Some(500));
        assert_eq!(packet.requests[0].response_body_excerpt, "response body");
    }

    #[tokio::test]
    async fn packet_preserves_a_045_sized_single_response_body() {
        let response_body = "tool-stream-event".repeat(392);
        assert!(response_body.len() > 4_096);
        assert!(response_body.len() < 10 * 1024);
        let evidence = single_test_evidence("019", parsed_request("test-019", &response_body));

        let packet = build_packet(&evidence, "019").await.unwrap();

        assert_eq!(packet.requests[0].response_body_excerpt, response_body);
    }

    #[tokio::test]
    async fn context_probe_packets_expose_target_and_measured_tokens() {
        let mut request = parsed_request("test-018-probe-124000-a1", "expected");
        request
            .metadata
            .insert("protocol".into(), "openai_chat".into());
        request.response_body =
            r#"{"choices":[{"message":{"content":"ok"}}],"usage":{"prompt_tokens":124012}}"#.into();
        let evidence = single_test_evidence("018", request);

        let packet = build_packet(&evidence, "018").await.unwrap();

        assert_eq!(packet.requests[0].context_target_tokens, Some(124_000));
        assert_eq!(packet.requests[0].context_measured_tokens, Some(124_012));
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

    #[tokio::test]
    async fn high_fanout_packet_is_compacted_below_one_batch() {
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

        let packet = build_packet(&parsed, "057").await.unwrap();

        assert_eq!(packet.requests.len(), 60);
        assert!(serde_json::to_vec(&[packet]).unwrap().len() <= MAX_BATCH_BYTES);
    }

    #[tokio::test]
    async fn concurrency_check_is_split_into_four_wave_packets() {
        let mut parsed = parsed_fixture_with_two_requests();
        parsed.requests.clear();
        let mut request_refs = Vec::new();
        for concurrency in [4, 8, 16, 32] {
            for index in 1..=concurrency {
                let request_id = format!("test-057-c{concurrency}-{index}");
                parsed.requests.insert(
                    request_id.clone(),
                    parsed_request(&request_id, &"response".repeat(100)),
                );
                request_refs.push(request_id);
            }
        }
        parsed.tests = BTreeMap::from([(
            "057".into(),
            ParsedTest {
                id: "057".into(),
                name: "4-32 并发响应时间".into(),
                category: "性能与稳定性".into(),
                request_refs,
            },
        )]);

        let packets = build_concurrency_packets(&parsed).await.unwrap();
        let waves = packets
            .iter()
            .filter(|packet| packet.report_test_id == "057")
            .collect::<Vec<_>>();

        assert_eq!(waves.len(), 4);
        assert_eq!(
            waves
                .iter()
                .map(|packet| packet.requests.len())
                .collect::<Vec<_>>(),
            [4, 8, 16, 32]
        );
        assert_eq!(
            waves
                .iter()
                .map(|packet| packet.test_id.as_str())
                .collect::<Vec<_>>(),
            ["057-wave-4", "057-wave-8", "057-wave-16", "057-wave-32"]
        );
        assert!(waves.iter().all(|packet| {
            packet
                .requests
                .iter()
                .all(|request| !request.response_body_excerpt.contains("bytes omitted"))
        }));
    }

    #[tokio::test]
    async fn concurrency_check_builds_one_packet_per_wave() {
        let mut parsed = parsed_fixture_with_two_requests();
        parsed.requests.clear();
        let mut request_refs = Vec::new();
        for concurrency in [4, 8, 16, 32] {
            for index in 1..=concurrency {
                let request_id = format!("test-057-c{concurrency}-{index}");
                parsed
                    .requests
                    .insert(request_id.clone(), parsed_request(&request_id, "response"));
                request_refs.push(request_id);
            }
        }
        parsed.tests = BTreeMap::from([(
            "057".into(),
            ParsedTest {
                id: "057".into(),
                name: "4-32 并发响应时间".into(),
                category: "性能与稳定性".into(),
                request_refs,
            },
        )]);

        let packets = build_concurrency_packets(&parsed).await.unwrap();

        assert_eq!(packets.len(), 4);
        assert_eq!(
            packets
                .iter()
                .map(|packet| (packet.test_id.as_str(), packet.requests.len()))
                .collect::<Vec<_>>(),
            [
                ("057-wave-4", 4),
                ("057-wave-8", 8),
                ("057-wave-16", 16),
                ("057-wave-32", 32),
            ]
        );
        assert!(packets.iter().all(|packet| packet.report_test_id == "057"));
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
            degraded_requests: Vec::new(),
            degraded_tests: Vec::new(),
        }
    }

    fn single_test_evidence(test_id: &str, request: ParsedRequest) -> ParsedEvidence {
        ParsedEvidence {
            source: EvidenceSource {
                path: PathBuf::from("doctor.log"),
                file_name: "doctor.log".into(),
                size_bytes: 1,
                sha256: "0".repeat(64),
            },
            run: BTreeMap::new(),
            requests: BTreeMap::from([(request.request_id.clone(), request.clone())]),
            tests: BTreeMap::from([(
                test_id.into(),
                ParsedTest {
                    id: test_id.into(),
                    name: "check".into(),
                    category: "category".into(),
                    request_refs: vec![request.request_id],
                },
            )]),
            degraded_requests: Vec::new(),
            degraded_tests: Vec::new(),
        }
    }

    fn parsed_request(request_id: &str, response_body: &str) -> ParsedRequest {
        ParsedRequest {
            request_id: request_id.into(),
            metadata: BTreeMap::from([
                ("protocol".into(), "openai_chat".into()),
                ("stream".into(), "0".into()),
                ("transport_outcome".into(), "completed_eof".into()),
                ("stream_termination".into(), "not_applicable".into()),
                ("stream_end_signal".into(), "none".into()),
                ("tool_contract_status".into(), "not_applicable".into()),
                ("tool_loop_turn".into(), "0".into()),
                ("tool_loop_outcome".into(), "not_applicable".into()),
            ]),
            metrics: BTreeMap::from([
                ("http_status".into(), "200".into()),
                ("time_total".into(), "1.0".into()),
                ("time_starttransfer".into(), "0.5".into()),
            ]),
            curl_command: String::new(),
            request_body: "prompt".into(),
            response_headers: "content-type: application/json".into(),
            stderr: String::new(),
            response_body: response_body.into(),
        }
    }

    fn packet_fixture(test_id: &str, response_size: usize) -> EvidencePacket {
        EvidencePacket {
            test_id: test_id.into(),
            report_test_id: test_id.into(),
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
                tool_loop_turn: 0,
                tool_loop_outcome: "not_applicable".into(),
                http_status: Some(200),
                time_total_seconds: Some(1.0),
                time_starttransfer_seconds: Some(0.5),
                context_target_tokens: None,
                context_measured_tokens: None,
                request_body_excerpt: "prompt".into(),
                response_body_excerpt: "x".repeat(response_size),
                stderr_excerpt: String::new(),
            }],
        }
    }
}
