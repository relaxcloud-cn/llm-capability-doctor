//! Capacity/performance observations use service accounting; estimates only construct candidate loads.
use crate::{
    dataset::{Record, RecordFile, compare_records, parse_record_payload},
    model::{
        CONCURRENT_REQUESTS, CheckId, CheckResult, FailureOrigin, Finding, INPUT_LOAD_TOKENS,
        MIN_CONTEXT_TOKENS, OUTPUT_LOAD_TOKENS, Status,
    },
    transport::{RequestFailure, RequestOutcome, Transport, messages_body},
};
use anyhow::Result;
use futures_util::future::join_all;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
const RESERVED_OUTPUT: u64 = 1024;
#[derive(Clone, Debug)]
struct Calibration {
    units: usize,
    input: u64,
    slope: f64,
    overhead: f64,
}
impl Calibration {
    fn for_input(&self, input: u64) -> usize {
        (((input as f64 - self.overhead) / self.slope)
            .ceil()
            .max(1.0) as usize)
            .min(2_000_000)
    }
}
fn check(
    id: CheckId,
    started: Instant,
    status: Status,
    title: String,
    evidence: Vec<String>,
    facts: BTreeMap<String, Value>,
    finding: Option<Finding>,
) -> CheckResult {
    CheckResult {
        id,
        finished: true,
        status,
        title,
        elapsed_ms: started.elapsed().as_millis() as u64,
        evidence_ids: evidence,
        facts,
        findings: finding.into_iter().collect(),
        tasks: vec![],
    }
}
fn issue(code: &str, title: &str, fact: &str, origin: FailureOrigin, ids: &[String]) -> Finding {
    let mut finding = Finding {
        code: code.into(),
        title: title.into(),
        fact: fact.into(),
        impact: "这项容量或负载条件尚未得到本次检测的有效验证。".into(),
        suggestion: if origin == FailureOrigin::Detector {
            "请根据记录补齐准确计量或完成负载校准，再重新检测。"
        } else {
            "请根据请求与响应检查模型服务的限制或故障，处理后重新检测。"
        }
        .into(),
        origin,
        evidence_ids: ids.to_vec(),
    };
    if code == "load_unverified" {
        finding.suggestion =
            "请模型服务技术人员补充最终回复长度的统计，再重新运行并发检测。".into();
    }
    finding
}
fn accepted_input(outcome: &RequestOutcome) -> Option<u64> {
    let response = outcome.response.as_ref()?;
    if outcome.failure.is_none()
        && !response.refused
        && matches!(
            response.finish_reason.as_str(),
            "stop" | "completed" | "length"
        )
    {
        response
            .usage
            .input
            .filter(|n| *n > 0 && *n <= u64::MAX - RESERVED_OUTPUT)
    } else {
        None
    }
}
async fn capacity_probe(
    transport: &Transport,
    units: usize,
    deadline: Instant,
) -> Result<RequestOutcome> {
    let text = format!(
        "这是请求容量探测，不需要提取材料信息。收到内容后只需简短确认。\n{}",
        " x".repeat(units)
    );
    let body = messages_body(
        transport.config(),
        json!([{"role":"user","content":text}]),
        RESERVED_OUTPUT,
        false,
    );
    transport.send(body, false, deadline).await
}
/// Only explicitly classified context errors with token accounting can establish an upper bound.
fn rejected_total(outcome: &RequestOutcome) -> Option<u64> {
    let attempt = outcome.evidence.attempts.last()?;
    if !matches!(attempt.http_status, Some(400 | 413)) {
        return None;
    }
    let wire: Value = serde_json::from_str(&attempt.response_body).ok()?;
    let error = &wire["error"];
    let code = error["code"].as_str().or_else(|| error["type"].as_str())?;
    if ![
        "context_length_exceeded",
        "context_window_exceeded",
        "max_context_length_exceeded",
    ]
    .contains(&code)
    {
        return None;
    }
    let input = error["input_tokens"]
        .as_u64()
        .or_else(|| error["prompt_tokens"].as_u64())?;
    input.checked_add(RESERVED_OUTPUT)
}
fn declared_context_hint(outcome: &RequestOutcome) -> Option<u64> {
    let attempt = outcome.evidence.attempts.last()?;
    if !matches!(attempt.http_status, Some(400 | 413)) {
        return None;
    }
    let body: Value = serde_json::from_str(&attempt.response_body).ok()?;
    let error = &body["error"];
    for field in ["maximum_context_length", "max_context_tokens"] {
        if let Some(value) = error[field].as_u64().filter(|n| *n > RESERVED_OUTPUT) {
            return Some(value);
        }
    }
    let message = error["message"].as_str()?;
    let phrase = "maximum context length is ";
    let start = message.to_ascii_lowercase().find(phrase)? + phrase.len();
    let tail = &message[start..];
    let number = tail
        .split_whitespace()
        .next()?
        .replace(',', "")
        .parse::<u64>()
        .ok()?;
    if !tail.contains("tokens") || number <= RESERVED_OUTPUT {
        return None;
    }
    Some(number)
}
pub async fn context(transport: &Transport, stage_deadline: Instant) -> Result<CheckResult> {
    let id = CheckId::Context;
    let started = Instant::now();
    let mut evidence = vec![];
    let mut facts = BTreeMap::new();
    // Leave the concurrency stage its own 60-second request budget and calibration allowance.
    let deadline = stage_deadline
        .checked_sub(Duration::from_secs(80))
        .unwrap_or(stage_deadline);
    if deadline <= started {
        let title = "上下文探测未完成，检测预算不足";
        return Ok(check(
            id,
            started,
            Status::Incomplete,
            title.into(),
            evidence,
            facts,
            None,
        ));
    }
    let first = capacity_probe(transport, 1024, deadline).await?;
    evidence.push(first.evidence.id.clone());
    let second = if accepted_input(&first).is_some() {
        Some(capacity_probe(transport, 2048, deadline).await?)
    } else {
        None
    };
    if let Some(second) = &second {
        evidence.push(second.evidence.id.clone());
    }
    let values = accepted_input(&first).zip(second.as_ref().and_then(accepted_input));
    let Some((a, b)) = values.filter(|(a, b)| b > a) else {
        let title = "上下文容量尚未确认，缺少有效计量记录";
        let finding = issue(
            "capacity_unverified",
            title,
            "校准请求没有取得可用、递增的实际输入 token 计量",
            FailureOrigin::Detector,
            &evidence,
        );
        return Ok(check(
            id,
            started,
            Status::Incomplete,
            title.into(),
            evidence,
            facts,
            Some(finding),
        ));
    };
    let slope = (b - a) as f64 / 1024.0;
    let calibration = Calibration {
        units: 2048,
        input: b,
        slope,
        overhead: a as f64 - slope * 1024.0,
    };
    let mut units = calibration.for_input(MIN_CONTEXT_TOKENS - RESERVED_OUTPUT);
    let mut lower = b + RESERVED_OUTPUT;
    let mut upper = None;
    let mut last_accepted_units = calibration.units;
    let mut rejected_units = None;
    let mut declared_limit = None;
    for _ in 0..8 {
        if Instant::now() >= deadline {
            break;
        }
        let outcome = capacity_probe(transport, units, deadline).await?;
        evidence.push(outcome.evidence.id.clone());
        if let Some(input) = accepted_input(&outcome) {
            lower = lower.max(input + RESERVED_OUTPUT);
            last_accepted_units = units;
            if lower >= MIN_CONTEXT_TOKENS {
                break;
            }
            let missing = (MIN_CONTEXT_TOKENS - lower) as f64;
            units = units
                .saturating_add((missing / slope).ceil().max(1.0) as usize)
                .min(2_000_000);
        } else {
            if let Some(hint) = declared_context_hint(&outcome) {
                declared_limit = Some(hint);
                rejected_units = Some(units);
            }
            if let Some(total) = rejected_total(&outcome) {
                upper = Some(total.saturating_sub(1));
                rejected_units = Some(units);
            }
            // A too-large candidate is not evidence that the required 128K request would fail.
            if upper.is_some_and(|value| value < MIN_CONTEXT_TOKENS) {
                break;
            }
            if units > last_accepted_units + 1 {
                units = last_accepted_units + (units - last_accepted_units) / 2;
            } else {
                break;
            }
        }
    }
    if lower >= MIN_CONTEXT_TOKENS {
        // Continue real probing; do not relabel the configured/successful lower bound as a maximum.
        let mut candidate = units.saturating_mul(2).min(2_000_000);
        for _ in 0..6 {
            if Instant::now() >= deadline || candidate <= units {
                break;
            }
            let outcome = capacity_probe(transport, candidate, deadline).await?;
            evidence.push(outcome.evidence.id.clone());
            if let Some(input) = accepted_input(&outcome) {
                lower = lower.max(input + RESERVED_OUTPUT);
                units = candidate;
                last_accepted_units = candidate;
                candidate = candidate.saturating_mul(2).min(2_000_000);
            } else {
                if let Some(hint) = declared_context_hint(&outcome) {
                    declared_limit = Some(hint);
                    rejected_units = Some(candidate);
                }
                if let Some(total) = rejected_total(&outcome) {
                    upper = Some(total.saturating_sub(1));
                    rejected_units = Some(candidate);
                }
                break;
            }
        }
    }
    // Refine an actually measured boundary while budget remains. A provider hint or
    // an estimated request size alone is never sufficient to publish an exact maximum.
    if let Some(mut high) = rejected_units {
        for _ in 0..20 {
            if Instant::now() >= deadline || high <= last_accepted_units + 1 || upper == Some(lower)
            {
                break;
            }
            let mid = last_accepted_units + (high - last_accepted_units) / 2;
            let outcome = capacity_probe(transport, mid, deadline).await?;
            evidence.push(outcome.evidence.id.clone());
            if let Some(input) = accepted_input(&outcome) {
                lower = lower.max(input + RESERVED_OUTPUT);
                last_accepted_units = mid;
            } else if declared_context_hint(&outcome).is_some() {
                high = mid;
                if let Some(total) = rejected_total(&outcome) {
                    upper = Some(upper.map_or(total.saturating_sub(1), |old| {
                        old.min(total.saturating_sub(1))
                    }));
                }
            } else if let Some(total) = rejected_total(&outcome) {
                upper = Some(upper.map_or(total.saturating_sub(1), |old| {
                    old.min(total.saturating_sub(1))
                }));
                high = mid;
            } else {
                break;
            }
        }
    }
    if upper.is_some_and(|value| value < lower) {
        let title = "上下文计量记录互相矛盾，容量未确认";
        let finding = issue(
            "capacity_accounting_conflict",
            title,
            &format!("已接受下限 {lower}，失败记录上限 {upper:?}"),
            FailureOrigin::Service,
            &evidence,
        );
        return Ok(check(
            id,
            started,
            Status::Incomplete,
            title.into(),
            evidence,
            facts,
            Some(finding),
        ));
    }
    let maximum = upper.filter(|value| *value == lower);
    facts.insert(
        "service_declared_limit_tokens".into(),
        json!(declared_limit),
    );
    facts.insert("verified_lower_bound_tokens".into(), json!(lower));
    facts.insert("verified_upper_bound_tokens".into(), json!(upper));
    facts.insert("maximum_context_tokens".into(), json!(maximum));
    facts.insert("maximum_confirmed".into(), json!(maximum.is_some()));
    facts.insert("output_reservation_tokens".into(), json!(RESERVED_OUTPUT));
    facts.insert("measurement_source".into(), json!("service_usage"));
    facts.insert("calibration_units".into(), json!(calibration.units));
    facts.insert("calibration_input_tokens".into(), json!(calibration.input));
    if lower >= MIN_CONTEXT_TOKENS {
        let title = if let Some(maximum) = maximum {
            format!("模型支持最大上下文为 {maximum} tokens")
        } else {
            format!("已支持至少 {lower} tokens，最大上限待确认")
        };
        return Ok(check(
            id,
            started,
            Status::Passed,
            title,
            evidence,
            facts,
            None,
        ));
    }
    let (status, title, origin) = if upper.is_some_and(|value| value < MIN_CONTEXT_TOKENS) {
        (
            Status::Failed,
            "上下文容量未达到 128K 最低要求",
            FailureOrigin::Service,
        )
    } else {
        (
            Status::Incomplete,
            "尚未确认支持至少 128K 上下文",
            FailureOrigin::Detector,
        )
    };
    let fact = format!("已验证下限 {lower} tokens；可靠上限：{upper:?}");
    let finding = issue("capacity_threshold", title, &fact, origin, &evidence);
    Ok(check(
        id,
        started,
        status,
        title.into(),
        evidence,
        facts,
        Some(finding),
    ))
}
fn within_load(value: u64, target: u64) -> bool {
    value as f64 >= target as f64 * 0.9 && value as f64 <= target as f64 * 1.1
}
fn performance_records() -> RecordFile {
    let owners = ["Lin", "Han", "Sun", "Zhou"];
    RecordFile {
        records: (0..48)
            .map(|index| Record {
                id: format!("R-{:02}", index + 1),
                owner: owners[index % owners.len()].into(),
                version: (index % 4 + 1) as u64,
            })
            .collect(),
    }
}
fn performance_body(transport: &Transport, units: usize, records: &RecordFile) -> Value {
    let text = format!(
        "这是响应负载测试。请完整返回下列记录的 JSON，保留全部字段和值，不添加解释，不丢记录。\n记录：{}\n以下仅用于构造输入负载，无需提取信息：\n{}",
        serde_json::to_string(records).unwrap(),
        " x".repeat(units)
    );
    messages_body(
        transport.config(),
        json!([{"role":"user","content":text}]),
        4096,
        false,
    )
}
fn valid_records(outcome: &RequestOutcome, expected: &RecordFile) -> bool {
    let Some(response) = &outcome.response else {
        return false;
    };
    outcome.failure.is_none()
        && response.normal_end
        && parse_record_payload(&response.text, true)
            .is_ok_and(|actual| compare_records(expected, &actual).passed)
}
pub async fn performance(transport: &Transport, deadline: Instant) -> Result<CheckResult> {
    let id = CheckId::Performance;
    let started = Instant::now();
    let mut evidence = vec![];
    let mut facts = BTreeMap::new();
    let expected = performance_records();
    if deadline <= started {
        return Ok(check(
            id,
            started,
            Status::Incomplete,
            "并发检测未完成，检测预算不足".into(),
            evidence,
            facts,
            None,
        ));
    }
    let calibration = transport
        .send(
            performance_body(transport, 8192, &expected),
            false,
            deadline,
        )
        .await?;
    evidence.push(calibration.evidence.id.clone());
    let input = calibration
        .response
        .as_ref()
        .and_then(|r| r.usage.input)
        .filter(|n| *n > 0);
    let output = calibration
        .response
        .as_ref()
        .and_then(|r| r.usage.delivered_output());
    facts.insert("calibration_input_tokens".into(), json!(input));
    facts.insert("calibration_delivered_output_tokens".into(), json!(output));
    let Some((input, output)) = input.zip(output).filter(|(_, out)| {
        within_load(*out, OUTPUT_LOAD_TOKENS) && valid_records(&calibration, &expected)
    }) else {
        let title = "并发性能尚未确认，缺少有效输出长度记录";
        let finding = issue(
            "load_unverified",
            title,
            "尚未取得测试要求的实际回复长度记录。服务只提供总输出量时，不能确认其中有多少是最终回复内容。",
            FailureOrigin::Detector,
            &evidence,
        );
        return Ok(check(
            id,
            started,
            Status::Incomplete,
            title.into(),
            evidence,
            facts,
            Some(finding),
        ));
    };
    let units = (8192.0 * INPUT_LOAD_TOKENS as f64 / input as f64).ceil() as usize;
    if units > 2_000_000
        || deadline.saturating_duration_since(Instant::now()) < Duration::from_secs(1)
    {
        return Ok(check(
            id,
            started,
            Status::Incomplete,
            "并发检测未完成，无法构造规定负载".into(),
            evidence,
            facts,
            None,
        ));
    }
    let body = performance_body(transport, units, &expected);
    let outcomes =
        join_all((0..CONCURRENT_REQUESTS).map(|_| transport.send(body.clone(), false, deadline)))
            .await;
    let mut elapsed = vec![];
    let mut load_valid = true;
    let mut responses_valid = true;
    let mut cancelled_or_budget = false;
    for outcome in outcomes {
        let outcome = outcome?;
        evidence.push(outcome.evidence.id.clone());
        elapsed.push(outcome.evidence.elapsed_ms);
        let response = outcome.response.as_ref();
        load_valid &= response
            .and_then(|r| r.usage.input)
            .is_some_and(|n| within_load(n, INPUT_LOAD_TOKENS));
        load_valid &= response
            .and_then(|r| r.usage.delivered_output())
            .is_some_and(|n| within_load(n, OUTPUT_LOAD_TOKENS));
        responses_valid &= valid_records(&outcome, &expected);
        cancelled_or_budget |= matches!(
            outcome.failure,
            Some(RequestFailure::Cancelled | RequestFailure::StageBudget)
        );
    }
    let maximum = elapsed.iter().copied().max().unwrap_or(0);
    let mean = elapsed.iter().sum::<u64>() as f64 / CONCURRENT_REQUESTS as f64;
    facts.insert("concurrency".into(), json!(CONCURRENT_REQUESTS));
    facts.insert("longest_elapsed_ms".into(), json!(maximum));
    facts.insert("mean_elapsed_ms".into(), json!(mean));
    facts.insert("actual_request_elapsed_ms".into(), json!(elapsed));
    facts.insert("validated_load".into(), json!(load_valid));
    facts.insert("calibration_output_tokens".into(), json!(output));
    let (status, title, origin) = if cancelled_or_budget {
        (
            Status::Incomplete,
            "并发检测未完成，检测已取消或预算耗尽".into(),
            FailureOrigin::Detector,
        )
    } else if !responses_valid {
        (
            Status::Failed,
            "并发请求未能全部正确完成".into(),
            FailureOrigin::Service,
        )
    } else if !load_valid {
        (
            Status::Incomplete,
            "4 并发请求完成，实际负载未达到测量要求".into(),
            FailureOrigin::Detector,
        )
    } else {
        (
            Status::Passed,
            format!(
                "模型4并发测试通过，最长用时{:.2}秒，平均用时{:.2}秒",
                maximum as f64 / 1000.0,
                mean / 1000.0
            ),
            FailureOrigin::Service,
        )
    };
    let finding = if status == Status::Passed {
        None
    } else {
        Some(issue(
            "concurrency_measurement",
            &title,
            &format!("4 次请求实际用时：{elapsed:?} ms，负载验证：{load_valid}"),
            origin,
            &evidence,
        ))
    };
    Ok(check(id, started, status, title, evidence, facts, finding))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Protocol,
        transport::{HttpAttempt, HttpEvidence},
    };
    #[test]
    fn capacity_errors_require_actual_counting_not_a_hint_or_network_error() {
        let mut outcome=RequestOutcome{evidence:HttpEvidence{id:"x".into(),endpoint:"x".into(),protocol:Protocol::ChatCompletions,request:json!({}),stream:false,elapsed_ms:1,attempts:vec![HttpAttempt{number:1,elapsed_ms:1,http_status:Some(400),headers:BTreeMap::new(),response_body:json!({"error":{"code":"context_length_exceeded","maximum_context_length":65536}}).to_string(),error:None}],parse_error:None},response:None,failure:Some(RequestFailure::Http(400))};
        assert_eq!(rejected_total(&outcome), None);
        assert_eq!(declared_context_hint(&outcome), Some(65536));
        outcome.evidence.attempts[0].response_body =
            json!({"error":{"code":"context_length_exceeded","input_tokens":130048}}).to_string();
        assert_eq!(rejected_total(&outcome), Some(131072));
        outcome.evidence.attempts[0].http_status = Some(503);
        assert_eq!(rejected_total(&outcome), None);
    }
    #[test]
    fn workload_tolerance_is_actual_not_max_token_configuration() {
        assert!(within_load(1024, OUTPUT_LOAD_TOKENS));
        assert!(within_load(922, OUTPUT_LOAD_TOKENS));
        assert!(!within_load(100, OUTPUT_LOAD_TOKENS));
        assert!(!within_load(1127, OUTPUT_LOAD_TOKENS));
    }
}
