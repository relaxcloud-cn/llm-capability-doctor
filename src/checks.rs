use crate::{
    config::Protocol,
    dataset::{RecordFile, SourceSnapshot, compare_records, parse_record_payload},
    model::{CheckId, CheckResult, FailureOrigin, Finding, Status},
    transport::{RequestFailure, RequestOutcome, Transport, add_schema, add_tools, messages_body},
};
use anyhow::Result;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

fn result(
    id: CheckId,
    started: Instant,
    status: Status,
    title: &str,
    evidence: Vec<String>,
    facts: BTreeMap<String, Value>,
    findings: Vec<Finding>,
) -> CheckResult {
    CheckResult {
        id,
        finished: true,
        status,
        title: title.into(),
        elapsed_ms: started.elapsed().as_millis() as u64,
        evidence_ids: evidence,
        facts,
        findings,
        tasks: vec![],
        assessment: None,
    }
}
fn problem(
    code: &str,
    title: &str,
    fact: &str,
    origin: FailureOrigin,
    evidence: &[String],
) -> Finding {
    Finding {
        code: code.into(),
        title: title.into(),
        fact: fact.into(),
        origin,
        evidence_ids: evidence.to_vec(),
        impact: "这项能力尚未满足本次平台使用要求。".into(),
        suggestion: if origin == FailureOrigin::Configuration {
            "请核对连接配置，修正后重新检测。"
        } else {
            "请根据请求和原始响应排查问题，处理后重新检测。"
        }
        .into(),
    }
}
fn rejected(
    id: CheckId,
    started: Instant,
    outcome: &RequestOutcome,
    evidence: Vec<String>,
) -> Option<CheckResult> {
    let (status, title, origin) = if let Some(failure) = &outcome.failure {
        match failure {
            RequestFailure::EvidenceConflict(_) => {
                (Status::Incomplete, failure.title(), FailureOrigin::Detector)
            }
            RequestFailure::StageBudget => (
                Status::Incomplete,
                "检测阶段预算已耗尽".into(),
                FailureOrigin::Detector,
            ),
            RequestFailure::Cancelled => (
                Status::Incomplete,
                "检测已取消".into(),
                FailureOrigin::Detector,
            ),
            RequestFailure::Timeout if outcome.evidence.attempts.is_empty() => (
                Status::Incomplete,
                "检测预算已耗尽".into(),
                FailureOrigin::Detector,
            ),
            RequestFailure::Http(401 | 403 | 404) => (
                Status::Incomplete,
                failure.title(),
                FailureOrigin::Configuration,
            ),
            _ => (Status::Failed, failure.title(), FailureOrigin::Service),
        }
    } else if let Some(response) = &outcome.response {
        if response.normal_end {
            return None;
        }
        (
            Status::Failed,
            if response.refused {
                "模型拒绝了本次请求"
            } else {
                "响应未正常完成，结果被截断或发生错误"
            }
            .into(),
            FailureOrigin::ModelOutput,
        )
    } else {
        (
            Status::Incomplete,
            "没有取得有效响应".into(),
            FailureOrigin::Detector,
        )
    };
    let finding = problem(
        "request_failed",
        &title,
        &format!("请求 {}：{}", outcome.evidence.id, title),
        origin,
        &evidence,
    );
    let mut check = result(
        id,
        started,
        status,
        &title,
        evidence,
        BTreeMap::new(),
        vec![finding],
    );
    check.finished = origin != FailureOrigin::Configuration
        && !matches!(
            outcome.failure,
            Some(RequestFailure::Cancelled | RequestFailure::StageBudget)
        );
    Some(check)
}
fn semantic_failure(
    id: CheckId,
    started: Instant,
    title: &str,
    fact: String,
    evidence: Vec<String>,
) -> CheckResult {
    let mut finding = problem(
        "output_validation",
        title,
        &fact,
        FailureOrigin::ModelOutput,
        &evidence,
    );
    if id == CheckId::Schema {
        finding.impact = "平台无法直接读取这份结构化结果，后续流程可能中断。".into();
        finding.suggestion="请技术人员核对服务的格式支持情况，并根据请求和原始响应排查结果损坏的原因；处理后重新检测。".into();
    }
    result(
        id,
        started,
        Status::Failed,
        title,
        evidence,
        BTreeMap::new(),
        vec![finding],
    )
}
pub async fn response(transport: &Transport, deadline: Instant) -> Result<CheckResult> {
    let id = CheckId::Response;
    let started = Instant::now();
    let cfg = transport.config();
    let mut evidence = vec![];
    let mut messages = json!([{"role":"user","content":"这是模型连接检查，请用中文简短回复。"}]);
    let first = transport
        .send(
            messages_body(cfg, messages.clone(), 2048, false),
            false,
            deadline,
        )
        .await?;
    evidence.push(first.evidence.id.clone());
    if let Some(result) = rejected(id, started, &first, evidence.clone()) {
        return Ok(result);
    }
    let first_response = first.response.as_ref().unwrap();
    if first_response.text.trim().is_empty() || !first_response.calls.is_empty() {
        return Ok(semantic_failure(
            id,
            started,
            "普通响应没有返回完整文本",
            "文本为空或返回了未请求的工具调用".into(),
            evidence,
        ));
    }
    match cfg.protocol {
        Protocol::ChatCompletions => messages
            .as_array_mut()
            .unwrap()
            .push(first_response.wire["choices"][0]["message"].clone()),
        Protocol::Responses => messages.as_array_mut().unwrap().extend(
            first_response.wire["output"]
                .as_array()
                .into_iter()
                .flatten()
                .cloned(),
        ),
    }
    messages
        .as_array_mut()
        .unwrap()
        .push(json!({"role":"user","content":"请继续用中文简短回复连接确认。"}));
    let second = transport
        .send(messages_body(cfg, messages, 2048, true), true, deadline)
        .await?;
    evidence.push(second.evidence.id.clone());
    if let Some(result) = rejected(id, started, &second, evidence.clone()) {
        return Ok(result);
    }
    let response = second.response.as_ref().unwrap();
    if response.text.trim().is_empty() || !response.calls.is_empty() {
        return Ok(semantic_failure(
            id,
            started,
            "流式响应没有返回完整文本",
            "流式响应为空或返回了未请求的工具调用".into(),
            evidence,
        ));
    }
    let facts = BTreeMap::from([
        ("nonstream_complete".into(), json!(true)),
        ("stream_complete".into(), json!(true)),
        ("message_history_accepted".into(), json!(true)),
        ("utf8_valid".into(), json!(true)),
    ]);
    Ok(result(
        id,
        started,
        Status::Passed,
        "普通响应、流式响应均正常",
        evidence,
        facts,
        vec![],
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LookupArgs {
    query: LookupQuery,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LookupQuery {
    ids: Vec<String>,
    kind: ResourceKind,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ResourceKind {
    Records,
}
pub async fn tools(transport: &Transport, deadline: Instant) -> Result<CheckResult> {
    let transport = transport.with_request_budget(Duration::from_secs(600));
    let id = CheckId::Tools;
    let started = Instant::now();
    let cfg = transport.config();
    let mut evidence = vec![];
    let snapshot = SourceSnapshot::generate(crate::model::CHECK_VERSION);
    let records = snapshot.records();
    let selected = vec![records[0].clone(), records[6].clone(), records[11].clone()];
    let expected = RecordFile {
        records: selected.clone(),
    };
    let ids = selected
        .iter()
        .map(|r| r.id.clone())
        .collect::<BTreeSet<_>>();
    let mut definitions = vec![
        json!({"name":"get_records","description":"根据记录 ID 查询负责人和版本，返回 records 数组","parameters":{"type":"object","properties":{"query":{"type":"object","properties":{"ids":{"type":"array","items":{"type":"string","enum":records.iter().map(|r|&r.id).collect::<Vec<_>>()}},"kind":{"type":"string","enum":["records"]}},"required":["ids","kind"],"additionalProperties":false}},"required":["query"],"additionalProperties":false}}),
        json!({"name":"get_service_time","description":"查询服务当前时间，不能查询记录","parameters":{"type":"object","properties":{"timezone":{"type":"string","enum":["UTC"]}},"required":["timezone"],"additionalProperties":false}}),
    ];
    if cfg.protocol == Protocol::Responses {
        for definition in &mut definitions {
            definition["strict"] = json!(false);
        }
    }
    let mut messages = json!([{"role":"user","content":format!("请使用适合的工具查询这些记录的负责人和版本，并把查询到的原始记录以 {{\"records\":[...]}} JSON 输出，不新增其他字段：{}。",ids.iter().cloned().collect::<Vec<_>>().join("、"))}]);
    let mut queried = BTreeSet::new();
    let mut call_count = 0usize;
    for turn in 0..32 {
        if transport.is_cancelled() || Instant::now() >= deadline {
            break;
        }
        let stream = turn == 0;
        let mut body = messages_body(cfg, messages.clone(), 20_000, stream);
        add_tools(&mut body, cfg.protocol, &definitions);
        let outcome = transport.send(body, stream, deadline).await?;
        evidence.push(outcome.evidence.id.clone());
        if let Some(check) = rejected(id, started, &outcome, evidence.clone()) {
            return Ok(check);
        }
        let response = outcome.response.as_ref().unwrap();
        if response.calls.is_empty() {
            if queried != ids {
                return Ok(semantic_failure(
                    id,
                    started,
                    "任务结束前未查询完整的指定记录",
                    "没有实际查询全部指定记录的工具证据".into(),
                    evidence,
                ));
            }
            let Some(actual) = parse_record_payload(&response.text, true).ok() else {
                let title = "工具结果使用情况尚未确认";
                let finding = problem(
                    "tool_answer_unverified",
                    title,
                    "工具选择、参数和回传正常，但最终回答没有可独立比较的记录内容",
                    FailureOrigin::ModelOutput,
                    &evidence,
                );
                return Ok(result(
                    id,
                    started,
                    Status::Incomplete,
                    title,
                    evidence,
                    BTreeMap::new(),
                    vec![finding],
                ));
            };
            if !compare_records(&expected, &actual).passed {
                return Ok(semantic_failure(
                    id,
                    started,
                    "工具结果没有被正确用于后续回答",
                    "最终输出没有完整保留正确的记录字段".into(),
                    evidence,
                ));
            }
            return Ok(result(
                id,
                started,
                Status::Passed,
                "工具选择、参数和结果使用均正确",
                evidence,
                BTreeMap::from([
                    ("tool_call_count".into(), json!(call_count)),
                    ("validated_ids".into(), json!(ids)),
                    ("interaction_rounds".into(), json!(turn + 1)),
                ]),
                vec![],
            ));
        }
        let mut outputs = vec![];
        for call in &response.calls {
            if call.name != "get_records" {
                return Ok(semantic_failure(
                    id,
                    started,
                    "选择的工具不符合任务要求",
                    format!("调用了 {}，本次需要查询记录", call.name),
                    evidence,
                ));
            }
            let args = match serde_json::from_value::<LookupArgs>(call.arguments.clone()) {
                Ok(args) => args,
                Err(_) => {
                    return Ok(semantic_failure(
                        id,
                        started,
                        "工具参数结构或字段不正确",
                        format!("调用 {} 的参数未满足工具定义", call.id),
                        evidence,
                    ));
                }
            };
            let _ = args.query.kind;
            if args.query.ids.is_empty()
                || args.query.ids.iter().any(|value| !ids.contains(value))
                || args.query.ids.iter().collect::<BTreeSet<_>>().len() != args.query.ids.len()
            {
                return Ok(semantic_failure(
                    id,
                    started,
                    "工具参数没有对应正确的记录 ID",
                    format!("调用 {} 的查询范围与要求不符", call.id),
                    evidence,
                ));
            }
            let output = RecordFile {
                records: selected
                    .iter()
                    .filter(|r| args.query.ids.contains(&r.id))
                    .cloned()
                    .collect(),
            };
            queried.extend(args.query.ids);
            call_count += 1;
            outputs.push((call.id.clone(), serde_json::to_string(&output)?));
        }
        match cfg.protocol {
            Protocol::ChatCompletions => {
                messages
                    .as_array_mut()
                    .unwrap()
                    .push(response.wire["choices"][0]["message"].clone());
                for (call_id, output) in outputs {
                    messages
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"role":"tool","tool_call_id":call_id,"content":output}));
                }
            }
            Protocol::Responses => {
                // Preserve reasoning and function-call items exactly as the API requires.
                messages.as_array_mut().unwrap().extend(
                    response.wire["output"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .cloned(),
                );
                for (call_id, output) in outputs {
                    messages.as_array_mut().unwrap().push(
                        json!({"type":"function_call_output","call_id":call_id,"output":output}),
                    );
                }
            }
        }
    }
    let title = "工具交互预算已到或检测已取消，完整结果未确认";
    let finding = problem(
        "tool_interaction_budget",
        title,
        "最多 32 轮交互，并共享本项时间预算；未把尚未完成的调用序列判为模型失败",
        FailureOrigin::Detector,
        &evidence,
    );
    Ok(result(
        id,
        started,
        Status::Incomplete,
        title,
        evidence,
        BTreeMap::from([("tool_call_count".into(), json!(call_count))]),
        vec![finding],
    ))
}
pub fn record_schema(records: &RecordFile) -> Value {
    json!({"type":"object","properties":{"records":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string","enum":records.records.iter().map(|r|&r.id).collect::<Vec<_>>()},"owner":{"type":"string","enum":records.records.iter().map(|r|&r.owner).collect::<BTreeSet<_>>()},"version":{"type":"integer"}},"required":["id","owner","version"],"additionalProperties":false}}},"required":["records"],"additionalProperties":false})
}
pub async fn schema(transport: &Transport, deadline: Instant) -> Result<CheckResult> {
    let id = CheckId::Schema;
    let started = Instant::now();
    let cfg = transport.config();
    let expected = SourceSnapshot::generate(crate::model::CHECK_VERSION).expected(None)?;
    let prompt = format!(
        "请按接口指定的原生 JSON 格式约束返回这份记录，保持每个 ID 对应的负责人和版本，不新增字段：{}",
        serde_json::to_string(&expected)?
    );
    let mut body = messages_body(cfg, json!([{"role":"user","content":prompt}]), 4096, false);
    add_schema(&mut body, cfg.protocol, record_schema(&expected));
    let outcome = transport.send(body, false, deadline).await?;
    let evidence = vec![outcome.evidence.id.clone()];
    if let Some(result) = rejected(id, started, &outcome, evidence.clone()) {
        return Ok(result);
    }
    let response = outcome.response.as_ref().unwrap();
    Ok(evaluate_schema(
        &response.text,
        &expected,
        started,
        evidence,
    ))
}

pub fn evaluate_schema(
    text: &str,
    expected: &RecordFile,
    started: Instant,
    evidence: Vec<String>,
) -> CheckResult {
    let id = CheckId::Schema;
    let native = serde_json::from_str::<Value>(text).is_ok();
    let actual = parse_record_payload(text, true).ok();
    if let Some(actual) = actual {
        let validation = compare_records(expected, &actual);
        if validation.passed {
            let findings = if native {
                vec![]
            } else {
                vec![Finding {
                code: "schema_compatibility".into(),
                title: "内容正确，需兼容解析代码块；原生 JSON 约束未满足".into(),
                fact: "返回正文包含 Markdown JSON 代码块；提取唯一代码块后，全部字段与预期一致。当前证据不能区分包装来自模型还是网关。".into(),
                impact: "兼容解析后可读取；直接把完整正文当 JSON 的调用方仍会解析失败。本项兼容通过不代表服务支持原生严格 JSON Schema。".into(),
                suggestion: "接入时使用经过校验的代码块提取；需要原生严格 JSON 时，请服务方检查网关透传和格式约束。".into(),
                origin: FailureOrigin::Service,
                evidence_ids: evidence.clone(),
            }]
            };
            return result(
                id,
                started,
                Status::Passed,
                if native {
                    "输出格式和字段内容符合要求"
                } else {
                    "内容正确，兼容解析通过；原生 JSON 格式未满足"
                },
                evidence,
                BTreeMap::from([
                    ("native_schema_request".into(), json!(true)),
                    ("native_format_satisfied".into(), json!(native)),
                    ("compatibility_extraction".into(), json!(!native)),
                    ("record_count".into(), json!(expected.records.len())),
                ]),
                findings,
            );
        }
        return semantic_failure(
            id,
            started,
            "输出字段或内容不符合约束",
            serde_json::to_string(&validation).unwrap_or_default(),
            evidence,
        );
    }
    let mut check = semantic_failure(
        id,
        started,
        "返回结果格式错误，平台无法直接读取",
        "请求包含原生 JSON 格式约束，但正文和可独立提取的唯一 JSON 代码块均无法解析；需排查模型服务及网关，不能仅凭此定位到模型本体。".into(),
        evidence,
    );
    for finding in &mut check.findings {
        finding.origin = FailureOrigin::Service;
    }
    check
}
