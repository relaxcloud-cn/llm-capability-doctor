use serde::Serialize;
use serde_json::{Value, json};
use thiserror::Error;
use url::Url;

use super::evidence_reader::{ParsedEvidence, ParsedRequest};
use super::projection::{RequestProjection, parse_protocol, project};
use super::rules::rule_for;
use crate::protocol::stream::AssistantTurn;
use crate::protocol::tools::{ExecutedToolResult, ToolRequestPhase, validate_tool_request};
use crate::protocol::{Protocol, normalize_request_url};

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
    pub tool_loop_turn: usize,
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

    let mut projections = Vec::with_capacity(requests.len());
    for request in &requests {
        projections.push(project(request).await);
    }
    let hard_failures = deterministic_failures(rule, &requests, &projections);
    let mut excerpt_limit =
        (MAX_PACKET_EXCERPT_BYTES / requests.len().max(1) / 3).clamp(128, MAX_EXCERPT_BYTES);
    loop {
        let packet = EvidencePacket {
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
                .iter()
                .map(|request| packet_request(request, excerpt_limit))
                .collect(),
            deterministic_facts: DeterministicFacts {
                request_count: test.request_refs.len(),
                hard_failures: hard_failures.clone(),
            },
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
    let mut packets = Vec::with_capacity(crate::catalog::CATALOG.len());
    for test in crate::catalog::CATALOG {
        packets.push(build_packet(evidence, test.id).await?);
    }
    Ok(packets)
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
        request_body_excerpt: bounded_excerpt(&request.request_body, excerpt_limit),
        response_body_excerpt: bounded_excerpt(&request.response_body, excerpt_limit),
        stderr_excerpt: bounded_excerpt(&request.stderr, excerpt_limit),
    }
}

fn deterministic_failures(
    rule: &super::rules::AnalysisRule,
    requests: &[&ParsedRequest],
    projections: &[RequestProjection],
) -> Vec<String> {
    let mut failures = Vec::new();
    if rule.require_all_transport_success
        && requests.iter().any(|request| {
            is_capability_request(rule.test_id, request) && !transport_succeeded(request)
        })
    {
        failures.push("at least one required request did not complete with HTTP 2xx".into());
    }
    if rule.require_all_transport_success
        && projections
            .iter()
            .zip(requests)
            .any(|(projection, request)| {
                is_capability_request(rule.test_id, request) && !projection.protocol_valid
            })
    {
        failures.push("at least one required response has an invalid native envelope".into());
    }
    if rule.require_complete_streams
        && requests.iter().any(|request| {
            is_capability_request(rule.test_id, request)
                && metadata(request, "stream", "0") == "1"
                && metadata(request, "stream_termination", "unknown") != "completed"
        })
    {
        failures.push("at least one required stream is incomplete".into());
    }
    if rule.require_tool_conformance
        && requests.iter().any(|request| {
            is_capability_request(rule.test_id, request)
                && metadata(request, "tool_contract_status", "unknown") != "conformant"
        })
    {
        failures.push("at least one required tool turn is non-conformant".into());
    }
    if requires_visible_text(rule.test_id)
        && projections
            .iter()
            .zip(requests)
            .any(|(projection, request)| {
                is_capability_request(rule.test_id, request)
                    && projection
                        .visible_text
                        .as_deref()
                        .is_none_or(|text| text.trim().is_empty())
            })
    {
        failures.push("at least one required response has no model-visible text".into());
    }
    marker_failures(rule, projections, &mut failures);
    structured_output_failures(rule.test_id, requests, projections, &mut failures);
    metric_failures(rule.test_id, requests, &mut failures);
    interface_failures(rule.test_id, requests, projections, &mut failures);
    tool_loop_failures(rule.test_id, requests, projections, &mut failures);
    failures
}

fn marker_failures(
    rule: &super::rules::AnalysisRule,
    projections: &[RequestProjection],
    failures: &mut Vec<String>,
) {
    let Some(marker) = rule.required_marker else {
        return;
    };
    let matches = |projection: &RequestProjection| {
        projection.visible_text.as_deref().is_some_and(|text| {
            if requires_exact_marker(rule.test_id) {
                text.trim() == marker
            } else {
                text.contains(marker)
            }
        })
    };
    let matched = if matches!(rule.test_id, "046" | "047" | "048" | "049") {
        projections.last().is_some_and(matches)
    } else if matches!(rule.test_id, "055" | "056") {
        !projections.is_empty() && projections.iter().all(matches)
    } else {
        projections.iter().any(matches)
    };
    if !matched {
        failures.push(format!(
            "required marker {marker} is absent from protocol-native visible text"
        ));
    }
}

fn structured_output_failures(
    test_id: &str,
    requests: &[&ParsedRequest],
    projections: &[RequestProjection],
    failures: &mut Vec<String>,
) {
    if !matches!(
        test_id,
        "009" | "010" | "011" | "012" | "013" | "022" | "038" | "059" | "060"
    ) {
        return;
    }
    let relevant = requests
        .iter()
        .zip(projections)
        .filter(|(request, _)| is_capability_request(test_id, request))
        .collect::<Vec<_>>();
    if matches!(test_id, "059" | "060") && relevant.len() != 1 {
        failures.push(format!(
            "check {test_id} does not contain exactly one experiment request"
        ));
    }
    for (request, projection) in relevant {
        let valid = projection
            .visible_text
            .as_deref()
            .and_then(|text| parse_structured_output(test_id, text))
            .is_some_and(|value| valid_structured_value(test_id, &value));
        if !valid {
            failures.push(format!(
                "request {} does not contain the required model-visible JSON structure",
                request.request_id
            ));
        }
    }
}

fn valid_structured_value(test_id: &str, value: &Value) -> bool {
    match test_id {
        "009" => {
            value
                == &json!({
                    "status": "ok",
                    "count": 7,
                    "enabled": true,
                    "profile": {"name": "Ada"},
                    "tags": ["red", "blue"],
                    "note": null
                })
        }
        "010" => value == &json!({"name": "alpha", "count": 7, "enabled": true}),
        "011" => {
            value == &json!({"profile": {"name": "Ada"}, "tags": ["red", "blue"], "note": null})
        }
        "012" => {
            value.pointer("/result/verdict").and_then(Value::as_str) == Some("risk")
                && value.pointer("/result/impact").and_then(Value::as_str) == Some("high")
                && value.pointer("/result/nextMove").and_then(Value::as_str) == Some("verify")
        }
        "013" => {
            value
                .get("investigationStages")
                .and_then(Value::as_array)
                .is_some_and(|stages| {
                    stages.iter().any(|stage| {
                        stage.get("stageId").and_then(Value::as_str) == Some("STAGE-001")
                            && stage
                                .get("evidenceRefs")
                                .and_then(Value::as_array)
                                .is_some_and(|refs| refs.iter().any(|item| item == "EVID-001"))
                    })
                })
                && value
                    .get("evidence")
                    .and_then(Value::as_array)
                    .is_some_and(|items| {
                        items.iter().any(|item| {
                            item.get("evidenceId").and_then(Value::as_str) == Some("EVID-001")
                        })
                    })
        }
        "022" => {
            value
                == &json!({
                    "time": "10:32",
                    "source": "203.0.113.7",
                    "action": "allow",
                    "labels": ["URGENT", "DATABASE"]
                })
        }
        "038" => value == &json!({"order": ["A", "B", "C"], "bTime": "09:22", "cTime": "09:27"}),
        "059" => {
            value
                == &json!({
                    "determination": "确认攻击",
                    "outcome": "已得手",
                    "nextAction": "隔离主机并封禁C2"
                })
        }
        "060" => {
            value
                == &json!({
                    "determination": "confirmed-attack",
                    "outcome": "host-compromised",
                    "nextAction": "isolate-host-and-block-c2"
                })
        }
        _ => true,
    }
}

fn metric_failures(test_id: &str, requests: &[&ParsedRequest], failures: &mut Vec<String>) {
    let metric_names: &[&str] = match test_id {
        "052" | "053" => &["time_starttransfer"],
        "054" | "056" => &["time_total"],
        "057" => &["time_starttransfer", "time_total"],
        _ => &[],
    };
    for request in requests {
        for metric_name in metric_names {
            let valid = metric(request, metric_name)
                .and_then(|value| value.parse::<f64>().ok())
                .is_some_and(|value| value.is_finite() && value > 0.0);
            if !valid {
                failures.push(format!(
                    "request {} has invalid positive metric {metric_name}",
                    request.request_id
                ));
            }
        }
    }
    let expected_count = match test_id {
        "055" | "056" => Some(5),
        "057" => Some(60),
        _ => None,
    };
    if expected_count.is_some_and(|expected| requests.len() != expected) {
        failures.push(format!(
            "check {test_id} has {} requests instead of {}",
            requests.len(),
            expected_count.unwrap()
        ));
    }
}

fn interface_failures(
    test_id: &str,
    requests: &[&ParsedRequest],
    projections: &[RequestProjection],
    failures: &mut Vec<String>,
) {
    match test_id {
        "001" => {
            if !requests.iter().any(|request| {
                metric(request, "http_status")
                    .and_then(|value| value.parse::<u16>().ok())
                    .is_some_and(|status| (100..600).contains(&status))
            }) {
                failures.push("no observable HTTP status was recorded".into());
            }
        }
        "002" | "003" => {
            if !requests
                .iter()
                .zip(projections)
                .any(|(request, projection)| {
                    transport_succeeded(request) && projection.text_response_valid
                })
            {
                failures.push("no protocol-native successful response was observed".into());
            }
        }
        "007" => {
            if !projections
                .iter()
                .any(|projection| projection.text_response_valid && projection.usage_valid)
            {
                failures.push("protocol-native token usage is missing or invalid".into());
            }
        }
        "008" => {
            let valid = requests.iter().any(|request| {
                metric(request, "http_status")
                    .and_then(|value| value.parse::<u16>().ok())
                    .is_some_and(|status| matches!(status, 400 | 422))
                    && native_structured_request_error(request)
            });
            if !valid {
                failures
                    .push("malformed request did not produce a structured request error".into());
            }
        }
        _ => {}
    }
}

fn tool_loop_failures(
    test_id: &str,
    requests: &[&ParsedRequest],
    projections: &[RequestProjection],
    failures: &mut Vec<String>,
) {
    let expected_count = match test_id {
        "046" | "048" => 2,
        "047" | "049" => 3,
        _ => return,
    };
    let mut valid = requests.len() == expected_count;
    for (index, request) in requests.iter().enumerate() {
        let turn = index + 1;
        valid &= request.request_id == format!("test-{test_id}-turn-{turn}");
        valid &= metadata(request, "stream", "0") == "1";
        valid &= metadata(request, "tool_loop_turn", "0") == turn.to_string();
        let expected_outcome = if turn == expected_count {
            "completed"
        } else {
            "continued"
        };
        valid &= metadata(request, "tool_loop_outcome", "unknown") == expected_outcome;
    }
    valid &= projections
        .last()
        .is_some_and(|projection| projection.protocol_valid);
    let call_sequence_valid = projections
        .iter()
        .enumerate()
        .all(|(index, projection)| expected_tool_call(test_id, index, projection));
    if !call_sequence_valid {
        failures.push(format!(
            "check {test_id} does not contain the required tool call sequence"
        ));
    }
    if !tool_result_chain_valid(test_id, requests, projections) {
        failures.push(format!(
            "check {test_id} has an invalid protocol-native tool result correlation"
        ));
    }
    if test_id == "048" {
        valid &= projections
            .last()
            .and_then(|projection| projection.visible_text.as_deref())
            .is_some_and(|text| text.contains("WEATHER_SUNNY"));
    }
    if !valid {
        failures.push(format!(
            "check {test_id} does not contain the required complete tool loop"
        ));
    }
}

fn expected_tool_call(test_id: &str, index: usize, projection: &RequestProjection) -> bool {
    let expected = match (test_id, index) {
        ("046" | "048", 0) | ("047" | "049", 0) | ("049", 1) => {
            Some(("get_weather", json!({"city": "Beijing"})))
        }
        ("047", 1) => Some(("get_time", json!({"zone": "UTC"}))),
        _ => None,
    };
    match expected {
        Some((name, arguments)) => {
            projection.tool_calls.len() == 1
                && logical_tool_name(test_id, &projection.tool_calls[0].name) == name
                && projection.tool_calls[0].arguments == arguments
        }
        None => projection.tool_calls.is_empty(),
    }
}

fn logical_tool_name<'a>(test_id: &str, name: &'a str) -> &'a str {
    if test_id == "047" && matches!(name, "doctor__get_weather" | "doctor/get_weather") {
        "get_weather"
    } else {
        name
    }
}

fn tool_result_chain_valid(
    test_id: &str,
    requests: &[&ParsedRequest],
    projections: &[RequestProjection],
) -> bool {
    let Some(protocol) = requests
        .first()
        .and_then(|request| request.metadata.get("protocol"))
        .and_then(|value| parse_protocol(value))
    else {
        return false;
    };
    if requests.iter().any(|request| {
        request
            .metadata
            .get("protocol")
            .and_then(|value| parse_protocol(value))
            != Some(protocol)
    }) {
        return false;
    }
    let endpoint = validation_endpoint(protocol);
    for (index, request) in requests.iter().enumerate() {
        let Ok(body) = serde_json::from_str::<Value>(&request.request_body) else {
            return false;
        };
        let errors = if index == 0 {
            validate_tool_request(protocol, &endpoint, true, &body, ToolRequestPhase::Initial)
        } else {
            let Some(previous) = projections[index - 1].assistant_turn.as_ref() else {
                return false;
            };
            let Some(results) = expected_tool_results(test_id, index - 1, previous) else {
                return false;
            };
            validate_tool_request(
                protocol,
                &endpoint,
                true,
                &body,
                ToolRequestPhase::FollowUp {
                    previous,
                    results: &results,
                },
            )
        };
        if !errors.is_empty() {
            return false;
        }
    }
    true
}

fn validation_endpoint(protocol: Protocol) -> Url {
    let configured = if protocol == Protocol::GeminiGenerateContent {
        Url::parse("https://example.test/v1/models/fixture:generateContent")
    } else {
        Url::parse("https://example.test/v1/chat")
    }
    .expect("static validation endpoint is valid");
    normalize_request_url(protocol, &configured, true)
}

fn expected_tool_results(
    test_id: &str,
    turn_index: usize,
    previous: &AssistantTurn,
) -> Option<Vec<ExecutedToolResult>> {
    previous
        .tool_calls
        .iter()
        .cloned()
        .map(|call| {
            let (output, is_error) = match logical_tool_name(test_id, &call.name) {
                "get_weather" if test_id == "049" && turn_index == 0 => ("ERROR: timeout", true),
                "get_weather" => ("WEATHER_SUNNY", false),
                "get_time" => ("TIME_UTC_12:00", false),
                _ => return None,
            };
            Some(ExecutedToolResult {
                call,
                output: output.into(),
                is_error,
            })
        })
        .collect()
}

fn native_structured_request_error(request: &ParsedRequest) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(&request.response_body) else {
        return false;
    };
    let Some(descriptor) =
        native_error_descriptor(&metadata(request, "protocol", "unknown"), &value)
    else {
        return false;
    };
    let lower = descriptor.to_ascii_lowercase();
    let authentication_or_quota = [
        "api key",
        "apikey",
        "authenticat",
        "unauthoriz",
        "credential",
        "permission",
        "quota",
        "rate limit",
    ];
    if authentication_or_quota
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return false;
    }
    [
        "json",
        "parse",
        "parsing",
        "malformed",
        "syntax",
        "decode",
        "request body",
        "body format",
        "invalid body",
        "unexpected end",
        "unexpected eof",
        "invalid character",
        "looking for beginning",
        "cannot unmarshal",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn native_error_descriptor(protocol: &str, value: &Value) -> Option<String> {
    match protocol {
        "openai_chat" | "openai_responses" => {
            let message = non_empty_value_string(value.pointer("/error/message"))?;
            let kind = non_empty_value_string(value.pointer("/error/type"))
                .or_else(|| non_empty_value_string(value.pointer("/error/code")))?;
            Some(format!("{kind} {message}"))
        }
        "anthropic_messages" => {
            (value.get("type").and_then(Value::as_str) == Some("error")).then_some(())?;
            let kind = non_empty_value_string(value.pointer("/error/type"))?;
            let message = non_empty_value_string(value.pointer("/error/message"))?;
            Some(format!("{kind} {message}"))
        }
        "gemini_generate_content" => {
            value.pointer("/error/code").and_then(Value::as_u64)?;
            let status = non_empty_value_string(value.pointer("/error/status"))?;
            let message = non_empty_value_string(value.pointer("/error/message"))?;
            Some(format!("{status} {message}"))
        }
        "ollama_chat" => non_empty_value_string(value.get("error")).map(str::to_owned),
        _ => None,
    }
}

fn non_empty_value_string(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
}

fn parse_structured_output(test_id: &str, text: &str) -> Option<Value> {
    let trimmed = text.trim();
    let json_text = if matches!(test_id, "059" | "060") {
        strip_single_json_fence(trimmed).unwrap_or(trimmed)
    } else {
        trimmed
    };
    serde_json::from_str(json_text).ok()
}

fn strip_single_json_fence(text: &str) -> Option<&str> {
    let body = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```JSON"))
        .or_else(|| text.strip_prefix("```"))?
        .strip_suffix("```")?
        .trim();
    (!body.contains("```")).then_some(body)
}

fn is_capability_request(test_id: &str, request: &ParsedRequest) -> bool {
    !matches!(test_id, "059" | "060") || request.request_id.ends_with("-experiment")
}

fn requires_visible_text(test_id: &str) -> bool {
    !matches!(
        test_id,
        "001"
            | "002"
            | "003"
            | "007"
            | "008"
            | "040"
            | "041"
            | "042"
            | "043"
            | "044"
            | "045"
            | "046"
            | "047"
            | "048"
            | "049"
            | "050"
    )
}

fn requires_exact_marker(test_id: &str) -> bool {
    matches!(
        test_id,
        "004"
            | "005"
            | "006"
            | "019"
            | "031"
            | "035"
            | "036"
            | "042"
            | "046"
            | "047"
            | "049"
            | "052"
            | "053"
            | "054"
            | "055"
            | "056"
    )
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
    use crate::protocol::Protocol;
    use crate::protocol::stream::parse_stream;
    use crate::protocol::tools::{ExecutedToolResult, ToolConversation, tool_prompt, tool_request};

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
    async fn high_fanout_check_is_compacted_below_one_batch() {
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
    async fn high_fanout_control_characters_fit_after_json_escaping() {
        let mut parsed = parsed_fixture_with_two_requests();
        parsed.requests.clear();
        let response = "\0".repeat(10_000);
        let request_refs: Vec<String> = (0..60)
            .map(|index| {
                let request_id = format!("test-057-control-{index}");
                parsed
                    .requests
                    .insert(request_id.clone(), parsed_request(&request_id, &response));
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

        assert!(serde_json::to_vec(&[packet]).unwrap().len() <= MAX_BATCH_BYTES);
    }

    #[tokio::test]
    async fn marker_gate_uses_reconstructed_stream_text() {
        let mut body =
            include_str!("../protocol/fixtures/openai_chat_final.sse").replace("046_OK", "005_OK");
        body.push('\n');
        let mut request = parsed_request("test-005", &body);
        request.metadata.insert("stream".into(), "1".into());
        request
            .metadata
            .insert("stream_termination".into(), "completed".into());
        let evidence = single_test_evidence("005", request);

        let packet = build_packet(&evidence, "005").await.unwrap();

        assert!(!body.contains("MODEL_DOCTOR_CASE_005_OK"));
        assert!(
            !packet
                .deterministic_facts
                .hard_failures
                .iter()
                .any(|failure| failure.contains("required marker"))
        );
    }

    #[tokio::test]
    async fn malformed_required_json_is_a_hard_failure() {
        let response = r#"{"choices":[{"message":{"content":"not-json"}}]}"#;
        let evidence = single_test_evidence("009", parsed_request("test-009", response));

        let packet = build_packet(&evidence, "009").await.unwrap();

        assert!(
            packet
                .deterministic_facts
                .hard_failures
                .iter()
                .any(|failure| failure.contains("required model-visible JSON structure"))
        );
    }

    #[tokio::test]
    async fn missing_native_usage_is_a_hard_failure() {
        let response = r#"{"choices":[{"message":{"content":"ok"}}]}"#;
        let evidence = single_test_evidence("007", parsed_request("protocol-1", response));

        let packet = build_packet(&evidence, "007").await.unwrap();

        assert!(
            packet
                .deterministic_facts
                .hard_failures
                .iter()
                .any(|failure| failure.contains("token usage"))
        );
    }

    #[tokio::test]
    async fn interface_reachability_requires_an_observable_http_status() {
        let mut request = parsed_request("test-001", "");
        request.metrics.remove("http_status");
        let evidence = single_test_evidence("001", request);

        let packet = build_packet(&evidence, "001").await.unwrap();

        assert!(has_failure(&packet, "observable HTTP status"));
    }

    #[tokio::test]
    async fn protocol_detection_rejects_empty_native_content() {
        let response = r#"{"choices":[{"message":{"content":""}}]}"#;
        let evidence = single_test_evidence("002", parsed_request("protocol-1", response));

        let packet = build_packet(&evidence, "002").await.unwrap();

        assert!(has_failure(&packet, "protocol-native successful response"));
    }

    #[tokio::test]
    async fn malformed_request_rejects_unrelated_server_error() {
        let mut request = parsed_request(
            "test-008",
            r#"{"error":{"message":"temporary upstream outage","type":"server_error"}}"#,
        );
        request.metrics.insert("http_status".into(), "500".into());
        let evidence = single_test_evidence("008", request);

        let packet = build_packet(&evidence, "008").await.unwrap();

        assert!(has_failure(&packet, "structured request error"));
    }

    #[tokio::test]
    async fn malformed_request_accepts_native_client_error() {
        let mut request = parsed_request(
            "test-008",
            r#"{"error":{"message":"invalid JSON request body","type":"invalid_request_error"}}"#,
        );
        request.metrics.insert("http_status".into(), "400".into());
        let evidence = single_test_evidence("008", request);

        let packet = build_packet(&evidence, "008").await.unwrap();

        assert!(!has_failure(&packet, "structured request error"));
    }

    #[tokio::test]
    async fn malformed_request_rejects_authentication_error_with_http_400() {
        let mut request = parsed_request(
            "test-008",
            r#"{"error":{"message":"invalid API key","type":"invalid_request_error"}}"#,
        );
        request.metrics.insert("http_status".into(), "400".into());
        let evidence = single_test_evidence("008", request);

        let packet = build_packet(&evidence, "008").await.unwrap();

        assert!(has_failure(&packet, "structured request error"));
    }

    #[tokio::test]
    async fn direct_marker_does_not_complete_a_required_tool_loop() {
        let response = r#"{"choices":[{"message":{"content":"MODEL_DOCTOR_CASE_046_OK"}}]}"#;
        let mut request = parsed_request("test-046-turn-1", response);
        request
            .metadata
            .insert("tool_contract_status".into(), "conformant".into());
        request.metadata.insert("tool_loop_turn".into(), "1".into());
        request
            .metadata
            .insert("tool_loop_outcome".into(), "completed".into());
        let evidence = single_test_evidence("046", request);

        let packet = build_packet(&evidence, "046").await.unwrap();

        assert!(has_failure(&packet, "tool loop"));
    }

    #[tokio::test]
    async fn complete_two_turn_tool_loop_passes_deterministic_gates() {
        let mut first = parsed_request(
            "test-046-turn-1",
            &format!(
                "{}\n",
                include_str!("../protocol/fixtures/openai_chat_tool.sse")
            ),
        );
        let mut final_turn = parsed_request(
            "test-046-turn-2",
            &format!(
                "{}\n",
                include_str!("../protocol/fixtures/openai_chat_final.sse")
            ),
        );
        for (request, turn, outcome) in [
            (&mut first, "1", "continued"),
            (&mut final_turn, "2", "completed"),
        ] {
            request.metadata.insert("stream".into(), "1".into());
            request
                .metadata
                .insert("stream_termination".into(), "completed".into());
            request
                .metadata
                .insert("tool_contract_status".into(), "conformant".into());
            request
                .metadata
                .insert("tool_loop_turn".into(), turn.into());
            request
                .metadata
                .insert("tool_loop_outcome".into(), outcome.into());
        }
        populate_tool_request_bodies(Protocol::OpenAiChat, "046", [&mut first, &mut final_turn])
            .await;
        let evidence = evidence_for_requests("046", vec![first, final_turn]);

        let packet = build_packet(&evidence, "046").await.unwrap();

        assert!(
            packet.deterministic_facts.hard_failures.is_empty(),
            "{:?}",
            packet.deterministic_facts.hard_failures
        );
    }

    #[tokio::test]
    async fn tool_loop_rejects_reversed_call_sequence_even_when_metadata_is_conformant() {
        let mut first = parsed_request(
            "test-047-turn-1",
            &openai_chat_tool_stream("get_time", json!({"zone": "UTC"}), "call-time"),
        );
        let mut second = parsed_request(
            "test-047-turn-2",
            &openai_chat_tool_stream(
                "doctor__get_weather",
                json!({"city": "Beijing"}),
                "call-weather",
            ),
        );
        let mut final_turn = parsed_request(
            "test-047-turn-3",
            &format!(
                "{}\n",
                include_str!("../protocol/fixtures/openai_chat_final.sse")
                    .replace("046_OK", "047_OK")
            ),
        );
        for (request, turn, outcome) in [
            (&mut first, "1", "continued"),
            (&mut second, "2", "continued"),
            (&mut final_turn, "3", "completed"),
        ] {
            request.metadata.insert("stream".into(), "1".into());
            request
                .metadata
                .insert("stream_termination".into(), "completed".into());
            request
                .metadata
                .insert("tool_contract_status".into(), "conformant".into());
            request
                .metadata
                .insert("tool_loop_turn".into(), turn.into());
            request
                .metadata
                .insert("tool_loop_outcome".into(), outcome.into());
        }
        populate_tool_request_bodies(
            Protocol::OpenAiChat,
            "047",
            [&mut first, &mut second, &mut final_turn],
        )
        .await;
        let evidence = evidence_for_requests("047", vec![first, second, final_turn]);

        let packet = build_packet(&evidence, "047").await.unwrap();

        assert!(has_failure(&packet, "tool call sequence"));
    }

    #[tokio::test]
    async fn tool_loop_rejects_mismatched_follow_up_correlation() {
        let mut first = parsed_request(
            "test-046-turn-1",
            &format!(
                "{}\n",
                include_str!("../protocol/fixtures/openai_chat_tool.sse")
            ),
        );
        let mut final_turn = parsed_request(
            "test-046-turn-2",
            &format!(
                "{}\n",
                include_str!("../protocol/fixtures/openai_chat_final.sse")
            ),
        );
        for (request, turn, outcome) in [
            (&mut first, "1", "continued"),
            (&mut final_turn, "2", "completed"),
        ] {
            request.metadata.insert("stream".into(), "1".into());
            request
                .metadata
                .insert("stream_termination".into(), "completed".into());
            request
                .metadata
                .insert("tool_contract_status".into(), "conformant".into());
            request
                .metadata
                .insert("tool_loop_turn".into(), turn.into());
            request
                .metadata
                .insert("tool_loop_outcome".into(), outcome.into());
        }
        populate_tool_request_bodies(Protocol::OpenAiChat, "046", [&mut first, &mut final_turn])
            .await;
        let mut body: Value = serde_json::from_str(&final_turn.request_body).unwrap();
        body["messages"][2]["tool_call_id"] = Value::String("forged-call-id".into());
        final_turn.request_body = body.to_string();
        let evidence = evidence_for_requests("046", vec![first, final_turn]);

        let packet = build_packet(&evidence, "046").await.unwrap();

        assert!(has_failure(&packet, "tool result correlation"));
    }

    #[tokio::test]
    async fn responses_namespace_tool_loop_passes_with_native_correlations() {
        let mut first = parsed_request(
            "test-047-turn-1",
            &openai_responses_tool_stream("doctor/get_weather", 1),
        );
        let mut second = parsed_request(
            "test-047-turn-2",
            &openai_responses_tool_stream("get_time", 2),
        );
        let mut final_turn = parsed_request(
            "test-047-turn-3",
            &format!(
                "{}\n",
                include_str!("../protocol/fixtures/openai_responses_final.sse")
                    .replace("046_OK", "047_OK")
            ),
        );
        for (request, turn, outcome) in [
            (&mut first, "1", "continued"),
            (&mut second, "2", "continued"),
            (&mut final_turn, "3", "completed"),
        ] {
            request
                .metadata
                .insert("protocol".into(), "openai_responses".into());
            request.metadata.insert("stream".into(), "1".into());
            request
                .metadata
                .insert("stream_termination".into(), "completed".into());
            request
                .metadata
                .insert("tool_contract_status".into(), "conformant".into());
            request
                .metadata
                .insert("tool_loop_turn".into(), turn.into());
            request
                .metadata
                .insert("tool_loop_outcome".into(), outcome.into());
        }
        populate_tool_request_bodies(
            Protocol::OpenAiResponses,
            "047",
            [&mut first, &mut second, &mut final_turn],
        )
        .await;
        let evidence = evidence_for_requests("047", vec![first, second, final_turn]);

        let packet = build_packet(&evidence, "047").await.unwrap();

        assert!(
            packet.deterministic_facts.hard_failures.is_empty(),
            "{:?}",
            packet.deterministic_facts.hard_failures
        );
    }

    #[tokio::test]
    async fn retry_loop_rejects_missing_timeout_result() {
        let mut first = parsed_request(
            "test-049-turn-1",
            &openai_chat_tool_stream("get_weather", json!({"city": "Beijing"}), "call-weather-1"),
        );
        let mut second = parsed_request(
            "test-049-turn-2",
            &openai_chat_tool_stream("get_weather", json!({"city": "Beijing"}), "call-weather-2"),
        );
        let mut final_turn = parsed_request(
            "test-049-turn-3",
            &format!(
                "{}\n",
                include_str!("../protocol/fixtures/openai_chat_final.sse")
                    .replace("046_OK", "049_OK")
            ),
        );
        for (request, turn, outcome) in [
            (&mut first, "1", "continued"),
            (&mut second, "2", "continued"),
            (&mut final_turn, "3", "completed"),
        ] {
            request.metadata.insert("stream".into(), "1".into());
            request
                .metadata
                .insert("stream_termination".into(), "completed".into());
            request
                .metadata
                .insert("tool_contract_status".into(), "conformant".into());
            request
                .metadata
                .insert("tool_loop_turn".into(), turn.into());
            request
                .metadata
                .insert("tool_loop_outcome".into(), outcome.into());
        }
        populate_tool_request_bodies(
            Protocol::OpenAiChat,
            "049",
            [&mut first, &mut second, &mut final_turn],
        )
        .await;
        second.request_body = second
            .request_body
            .replace("ERROR: timeout", "WEATHER_SUNNY");
        let evidence = evidence_for_requests("049", vec![first, second, final_turn]);

        let packet = build_packet(&evidence, "049").await.unwrap();

        assert!(has_failure(&packet, "tool result correlation"));
    }

    #[tokio::test]
    async fn latency_percentiles_require_successful_shared_sample_markers() {
        let requests = (1..=5)
            .map(|index| {
                parsed_request(
                    &format!("test-055-repeat-{index}"),
                    r#"{"choices":[{"message":{"content":"wrong"}}]}"#,
                )
            })
            .collect();
        let evidence = evidence_for_requests("056", requests);

        let packet = build_packet(&evidence, "056").await.unwrap();

        assert!(has_failure(&packet, "required marker"));
    }

    #[tokio::test]
    async fn guardrail_control_is_diagnostic_and_experiment_allows_one_json_fence() {
        let mut control = parsed_request(
            "test-060-control",
            r#"{"error":{"message":"control unavailable"}}"#,
        );
        control.metrics.insert("http_status".into(), "500".into());
        let experiment = parsed_request(
            "test-060-experiment",
            r#"{"choices":[{"message":{"content":"```json\n{\"determination\":\"confirmed-attack\",\"outcome\":\"host-compromised\",\"nextAction\":\"isolate-host-and-block-c2\"}\n```"}}]}"#,
        );
        let evidence = evidence_for_requests("060", vec![control, experiment]);

        let packet = build_packet(&evidence, "060").await.unwrap();

        assert!(
            packet.deterministic_facts.hard_failures.is_empty(),
            "{:?}",
            packet.deterministic_facts.hard_failures
        );
    }

    #[tokio::test]
    async fn nonpositive_required_metric_is_a_hard_failure() {
        let response = r#"{"choices":[{"message":{"content":"MODEL_DOCTOR_CASE_052_OK"}}]}"#;
        let mut request = parsed_request("test-052", response);
        request
            .metrics
            .insert("time_starttransfer".into(), "0".into());
        let evidence = single_test_evidence("052", request);

        let packet = build_packet(&evidence, "052").await.unwrap();

        assert!(
            packet
                .deterministic_facts
                .hard_failures
                .iter()
                .any(|failure| failure.contains("time_starttransfer"))
        );
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

    fn single_test_evidence(test_id: &str, request: ParsedRequest) -> ParsedEvidence {
        evidence_for_requests(test_id, vec![request])
    }

    fn evidence_for_requests(test_id: &str, requests: Vec<ParsedRequest>) -> ParsedEvidence {
        let request_refs = requests
            .iter()
            .map(|request| request.request_id.clone())
            .collect::<Vec<_>>();
        ParsedEvidence {
            source: EvidenceSource {
                path: PathBuf::from("doctor.log"),
                file_name: "doctor.log".into(),
                size_bytes: 1,
                sha256: "0".repeat(64),
            },
            run: BTreeMap::new(),
            requests: requests
                .into_iter()
                .map(|request| (request.request_id.clone(), request))
                .collect(),
            tests: BTreeMap::from([(
                test_id.into(),
                ParsedTest {
                    id: test_id.into(),
                    name: "check".into(),
                    category: "category".into(),
                    request_refs,
                },
            )]),
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
            request_body: "prompt".into(),
            response_headers: "content-type: application/json".into(),
            stderr: String::new(),
            response_body: response_body.into(),
        }
    }

    fn has_failure(packet: &EvidencePacket, needle: &str) -> bool {
        packet
            .deterministic_facts
            .hard_failures
            .iter()
            .any(|failure| failure.contains(needle))
    }

    fn openai_chat_tool_stream(name: &str, arguments: Value, call_id: &str) -> String {
        let chunk = json!({
            "id": "chatcmpl-tool",
            "object": "chat.completion.chunk",
            "choices": [{
                "index": 0,
                "delta": {
                    "role": "assistant",
                    "tool_calls": [{
                        "index": 0,
                        "id": call_id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": arguments.to_string()
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });
        format!("data: {chunk}\n\ndata: [DONE]\n\n")
    }

    fn openai_responses_tool_stream(name: &str, turn: usize) -> String {
        let mut body = include_str!("../protocol/fixtures/openai_responses_tool.sse")
            .replace("_046", &format!("_047_turn_{turn}"))
            .replace("get_weather", name);
        if name == "get_time" {
            body = body
                .replace(r#"{\"city\":\""#, r#"{\"zone\":\""#)
                .replace(r#"Beijing\"}"#, r#"UTC\"}"#);
        }
        body.push('\n');
        body
    }

    async fn populate_tool_request_bodies<const N: usize>(
        protocol: Protocol,
        test_id: &str,
        requests: [&mut ParsedRequest; N],
    ) {
        let prompt = tool_prompt(test_id).unwrap();
        let initial = tool_request(protocol, "fixture-model", test_id, prompt);
        let mut conversation =
            ToolConversation::from_initial(protocol, initial.body.clone()).unwrap();
        requests[0].request_body = initial.body.to_string();
        for index in 1..requests.len() {
            let parsed = parse_stream(protocol, requests[index - 1].response_body.as_bytes()).await;
            let turn = parsed.assistant_turn.unwrap();
            let results = turn
                .tool_calls
                .iter()
                .cloned()
                .map(|call| fixture_tool_result(test_id, index - 1, call))
                .collect::<Vec<_>>();
            let follow_up = conversation.append_follow_up(&turn, &results).unwrap();
            requests[index].request_body = follow_up.body.to_string();
        }
    }

    fn fixture_tool_result(
        test_id: &str,
        turn_index: usize,
        call: crate::protocol::stream::ToolCall,
    ) -> ExecutedToolResult {
        let logical_name = call
            .name
            .strip_prefix("doctor__")
            .or_else(|| call.name.strip_prefix("doctor/"))
            .unwrap_or(&call.name);
        let (output, is_error) = match logical_name {
            "get_weather" if test_id == "049" && turn_index == 0 => ("ERROR: timeout", true),
            "get_weather" => ("WEATHER_SUNNY", false),
            "get_time" => ("TIME_UTC_12:00", false),
            _ => panic!("unexpected fixture tool {}", call.name),
        };
        ExecutedToolResult {
            call,
            output: output.into(),
            is_error,
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
                tool_loop_turn: 0,
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
