use crate::{
    config::Protocol,
    dataset::{
        RecordFile, SourceSnapshot, compare_records, parse_record_payload, references_path,
        source_observed, validate_task,
    },
    judgment::{CheckSpec, Claim, Evaluation, EvidenceIndex, Outcome, Validity, verify_reference},
    model::{CheckId, CheckResult, FailureOrigin, MIN_CONTEXT_TOKENS, TaskResult, ToolEvent},
    transport::{EvidenceConflict, HttpEvidence, ModelResponse, parse_json, parse_stream},
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

fn claim(
    rule: &str,
    outcome: Outcome,
    reason: impl Into<String>,
    expected: Value,
    observed: Value,
    ids: Vec<String>,
) -> Claim {
    Claim {
        requirement: rule.into(),
        validity: Validity::Verified,
        outcome,
        reason: reason.into(),
        expected,
        observed,
        evidence_ids: ids,
        origin: if outcome == Outcome::Unsatisfied {
            Some(FailureOrigin::ModelOutput)
        } else {
            None
        },
        condition: None,
    }
}
fn unknown(
    rule: &str,
    reason: impl Into<String>,
    ids: Vec<String>,
    origin: FailureOrigin,
) -> Claim {
    let mut c = claim(
        rule,
        Outcome::Unknown,
        reason,
        Value::Null,
        Value::Null,
        ids,
    );
    c.validity = Validity::Insufficient;
    c.origin = Some(origin);
    c
}
fn conflicting(rule: &str, reason: impl Into<String>, ids: Vec<String>) -> Claim {
    let mut c = unknown(rule, reason, ids, FailureOrigin::Detector);
    c.validity = Validity::Conflicting;
    c
}
fn http(directory: &Path, index: &EvidenceIndex, id: &str) -> Result<HttpEvidence> {
    let reference = index.get(id).context("缺少 HTTP 证据索引")?;
    verify_reference(directory, reference)?;
    let row: HttpEvidence =
        serde_json::from_slice(&std::fs::read(directory.join(&reference.path))?)?;
    if row.id != id || row.stream != row.request["stream"].as_bool().unwrap_or(false) {
        bail!("HTTP 证据身份或流式标记冲突");
    }
    Ok(row)
}
fn response(row: &HttpEvidence) -> Result<ModelResponse> {
    let last = row.attempts.last().context("尚无实际请求尝试")?;
    if !last.http_status.is_some_and(|n| (200..300).contains(&n)) {
        bail!("HTTP 请求未成功");
    }
    if row.stream {
        parse_stream(row.protocol, last.response_body.as_bytes())
    } else {
        parse_json(row.protocol, last.response_body.as_bytes())
    }
}
fn failure(rule: &str, row: &HttpEvidence) -> Option<Claim> {
    let ids = vec![row.id.clone()];
    let Some(last) = row.attempts.last() else {
        return Some(unknown(
            rule,
            "尚无实际请求尝试，不能确认能力",
            ids,
            FailureOrigin::Detector,
        ));
    };
    if let Some(error) = &last.error {
        let guarded =
            error.contains("取消") || error.contains("预算") || error.contains("安全大小限制");
        return Some(if guarded {
            unknown(rule, error, ids, FailureOrigin::Detector)
        } else {
            let mut c = claim(
                rule,
                Outcome::Unsatisfied,
                format!("请求未完整完成：{error}"),
                json!("请求完整完成"),
                json!({"error":error,"attempts":row.attempts.len()}),
                ids,
            );
            c.origin = Some(FailureOrigin::Service);
            c
        });
    }
    if let Some(status) = last.http_status.filter(|n| !(200..300).contains(n)) {
        return Some(if matches!(status, 401 | 403 | 404) {
            unknown(
                rule,
                format!("HTTP {status} 阻断配置或鉴权，未确认能力"),
                ids,
                FailureOrigin::Configuration,
            )
        } else {
            let mut c = claim(
                rule,
                Outcome::Unsatisfied,
                format!("服务请求返回 HTTP {status}，未满足本次接口要求"),
                json!("HTTP 成功响应"),
                json!({"http_status":status,"attempts":row.attempts.len()}),
                ids,
            );
            c.origin = Some(FailureOrigin::Service);
            c
        });
    }
    match response(row) {
        Err(error) => Some(if error.is::<EvidenceConflict>() {
            conflicting(rule, error.to_string(), ids)
        } else {
            let mut c = claim(
                rule,
                Outcome::Unsatisfied,
                format!("服务响应不符合所选协议：{error}"),
                json!("完整且可解析的协议响应"),
                json!(error.to_string()),
                ids,
            );
            c.origin = Some(FailureOrigin::Service);
            c
        }),
        Ok(r) if !r.normal_end => {
            let mut c = claim(
                rule,
                Outcome::Unsatisfied,
                if r.refused {
                    "服务拒绝本次测试请求"
                } else {
                    "响应截断或未正常结束"
                },
                json!("正常完成"),
                json!({"finish_reason":r.finish_reason,"refused":r.refused}),
                ids,
            );
            c.origin = Some(FailureOrigin::Service);
            Some(c)
        }
        _ => None,
    }
}
fn messages(row: &HttpEvidence) -> Result<&Vec<Value>> {
    row.request
        .get("messages")
        .or_else(|| row.request.get("input"))
        .and_then(Value::as_array)
        .context("请求消息缺失")
}
fn returned_items(protocol: Protocol, response: &ModelResponse) -> Vec<Value> {
    match protocol {
        Protocol::ChatCompletions => vec![response.wire["choices"][0]["message"].clone()],
        Protocol::Responses => response.wire["output"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
    }
}
fn ids(check: &CheckResult) -> Vec<String> {
    check
        .evidence_ids
        .iter()
        .filter(|id| id.starts_with("http-"))
        .cloned()
        .collect()
}
fn load_all(
    check: &CheckResult,
    directory: &Path,
    index: &EvidenceIndex,
) -> Result<Vec<HttpEvidence>> {
    let rows = ids(check)
        .iter()
        .map(|id| http(directory, index, id))
        .collect::<Result<Vec<_>>>()?;
    if let Some(first) = rows.first() {
        if rows.iter().any(|r| {
            r.endpoint != first.endpoint
                || r.protocol != first.protocol
                || r.request["model"] != first.request["model"]
        }) {
            bail!("同一检测混入不同目标或协议的证据");
        }
    }
    Ok(rows)
}
fn evaluation(claims: Vec<Claim>, facts: Value) -> Evaluation {
    Evaluation {
        claims,
        facts: facts
            .as_object()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default(),
    }
}
pub fn builtin_claims(
    spec: &CheckSpec,
    check: &CheckResult,
    directory: &Path,
    index: &EvidenceIndex,
) -> Result<Vec<Claim>> {
    Ok(builtin_evaluation(spec, check, directory, index)?.claims)
}
pub fn builtin_evaluation(
    spec: &CheckSpec,
    check: &CheckResult,
    directory: &Path,
    index: &EvidenceIndex,
) -> Result<Evaluation> {
    if check.id.is_agent() {
        return Ok(evaluation(
            agent_claims(spec, check, directory, index),
            json!({}),
        ));
    }
    if check.id == CheckId::Response {
        return Ok(response_claims(check, directory, index));
    }
    let rows = load_all(check, directory, index)?;
    if rows.is_empty() {
        return Ok(evaluation(
            spec.requirements
                .iter()
                .map(|r| {
                    unknown(
                        &r.id,
                        "尚未取得本项请求证据",
                        vec![format!("file:observations/{}.json", check.id.key())],
                        FailureOrigin::Detector,
                    )
                })
                .collect(),
            json!({}),
        ));
    }
    match check.id {
        CheckId::Tools => tools_claims(&rows),
        CheckId::Schema => schema_claims(&rows[0]),
        CheckId::Context => context_claims(&rows),
        CheckId::Performance => performance_claims(check, directory, index),
        _ => bail!("内置验证器不处理扩展检测"),
    }
}
fn response_claims(check: &CheckResult, directory: &Path, index: &EvidenceIndex) -> Evaluation {
    let references = ids(check);
    let rows = references
        .iter()
        .map(|id| http(directory, index, id))
        .collect::<Vec<_>>();
    let mut claims = vec![];
    for (n, rule, label, stream) in [
        (0, "nonstream", "普通响应", false),
        (1, "stream", "流式响应", true),
    ] {
        let proof = references.get(n).cloned().into_iter().collect();
        let Some(Ok(row)) = rows.get(n) else {
            claims.push(unknown(
                rule,
                format!("{label}尚无完整证据"),
                proof,
                FailureOrigin::Detector,
            ));
            continue;
        };
        if row.stream != stream {
            claims.push(conflicting(rule, "请求阶段与协议模式不一致", proof));
            continue;
        }
        if let Some(problem) = failure(rule, row) {
            claims.push(problem);
            continue;
        }
        let r = response(row).unwrap();
        let good = !r.text.trim().is_empty() && r.calls.is_empty();
        claims.push(claim(
            rule,
            if good {
                Outcome::Satisfied
            } else {
                Outcome::Unsatisfied
            },
            if good {
                format!("{label}完整返回文本")
            } else {
                format!("{label}为空或含未请求的工具调用")
            },
            json!("完整非空文本，不含未请求的工具调用"),
            json!({"text":r.text,"calls":r.calls,"attempts":row.attempts.len()}),
            proof,
        ));
    }
    let history = match (rows.first(), rows.get(1)) {
        (Some(Ok(first)), Some(Ok(next))) => response(first).ok().and_then(|r| {
            let mut prefix = messages(first).ok()?.clone();
            prefix.extend(returned_items(first.protocol, &r));
            Some(
                first.protocol == next.protocol
                    && first.endpoint == next.endpoint
                    && messages(next).ok()?.starts_with(&prefix),
            )
        }),
        _ => None,
    };
    let history_claim = match history {
        Some(true) => claim(
            "history",
            Outcome::Satisfied,
            "前轮完整消息已按所选协议回传",
            json!(true),
            json!(true),
            references.clone(),
        ),
        Some(false) => conflicting(
            "history",
            "请求未完整保留前轮消息，检测交互本身不可信",
            references.clone(),
        ),
        None => unknown(
            "history",
            "尚无完整的消息回传证据",
            references.clone(),
            FailureOrigin::Detector,
        ),
    };
    claims.push(history_claim);
    let facts = json!({"nonstream_complete":claims[0].outcome==Outcome::Satisfied,"stream_complete":claims[1].outcome==Outcome::Satisfied,"message_history_accepted":claims[2].outcome==Outcome::Satisfied});
    evaluation(claims, facts)
}
fn tools_claims(rows: &[HttpEvidence]) -> Result<Evaluation> {
    let proof = rows.iter().map(|r| r.id.clone()).collect::<Vec<_>>();
    let prompt = messages(&rows[0])?[0]["content"]
        .as_str()
        .context("工具问题缺失")?;
    let wanted: BTreeSet<String> = prompt
        .rsplit_once('：')
        .context("工具问题缺少记录范围")?
        .1
        .trim_end_matches('。')
        .split('、')
        .map(str::to_owned)
        .collect();
    let mut queried = BTreeSet::new();
    let mut delivered = BTreeMap::new();
    let mut count = 0;
    for (n, row) in rows.iter().enumerate() {
        if let Some(problem) = failure("roundtrip", row) {
            return Ok(evaluation(vec![problem], json!({"tool_call_count":count})));
        }
        let r = response(row)?;
        if r.calls.is_empty() {
            if queried != wanted {
                return Ok(evaluation(
                    vec![claim(
                        "roundtrip",
                        Outcome::Unsatisfied,
                        "任务正常结束前没有查询完整指定记录",
                        json!(wanted),
                        json!(queried),
                        proof,
                    )],
                    json!({"tool_call_count":count}),
                ));
            }
            let Some(actual) = parse_record_payload(&r.text, true).ok() else {
                return Ok(evaluation(
                    vec![claim(
                        "roundtrip",
                        Outcome::Unknown,
                        "工具交互完成，但最终回答没有可独立比较的记录内容",
                        json!("可比较的最终记录"),
                        json!(r.text),
                        proof,
                    )],
                    json!({"tool_call_count":count}),
                ));
            };
            let expected = RecordFile {
                records: delivered.into_values().collect(),
            };
            let valid = compare_records(&expected, &actual).passed;
            return Ok(evaluation(
                vec![claim(
                    "roundtrip",
                    if valid {
                        Outcome::Satisfied
                    } else {
                        Outcome::Unsatisfied
                    },
                    if valid {
                        "工具选择、参数、调用关联和最终记录均正确"
                    } else {
                        "最终记录与实际工具回传内容不一致"
                    },
                    json!(expected),
                    actual,
                    proof,
                )],
                json!({"tool_call_count":count,"validated_ids":queried,"interaction_rounds":n+1}),
            ));
        }
        for call in &r.calls {
            let query = &call.arguments["query"];
            let requested = query["ids"].as_array();
            let valid = call.name == "get_records"
                && call.arguments.as_object().is_some_and(|m| m.len() == 1)
                && query.as_object().is_some_and(|m| m.len() == 2)
                && query["kind"] == "records"
                && requested.is_some_and(|items| {
                    !items.is_empty()
                        && items
                            .iter()
                            .all(|v| v.as_str().is_some_and(|s| wanted.contains(s)))
                        && items
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<BTreeSet<_>>()
                            .len()
                            == items.len()
                });
            if !valid {
                return Ok(evaluation(
                    vec![claim(
                        "roundtrip",
                        Outcome::Unsatisfied,
                        "实际工具选择或参数不满足请求定义",
                        json!({"name":"get_records","ids":wanted,"kind":"records"}),
                        json!(call),
                        vec![row.id.clone()],
                    )],
                    json!({"tool_call_count":count}),
                ));
            }
        }
        let Some(next) = rows.get(n + 1) else {
            return Ok(evaluation(
                vec![claim(
                    "roundtrip",
                    Outcome::Unknown,
                    "仍有未完成的工具调用，未取得最终结果",
                    json!("完成工具回传并取得最终记录"),
                    json!({"pending_calls":r.calls}),
                    proof,
                )],
                json!({"tool_call_count":count}),
            ));
        };
        let mut prefix = messages(row)?.clone();
        prefix.extend(returned_items(row.protocol, &r));
        let next_messages = messages(next)?;
        if !next_messages.starts_with(&prefix)
            || next_messages.len() != prefix.len() + r.calls.len()
        {
            bail!("工具往返未保持原始消息与回传数量");
        }
        let outputs = &next_messages[prefix.len()..];
        let mut seen = BTreeSet::new();
        for item in outputs {
            let (call_id, content) = match row.protocol {
                Protocol::ChatCompletions if item["role"] == "tool" => {
                    (item["tool_call_id"].as_str(), item["content"].as_str())
                }
                Protocol::Responses if item["type"] == "function_call_output" => {
                    (item["call_id"].as_str(), item["output"].as_str())
                }
                _ => bail!("工具回传角色或类型不正确"),
            };
            let call_id = call_id.context("工具回传缺少 ID")?;
            if !seen.insert(call_id) {
                bail!("同一批工具结果重复回传");
            }
            let call = r
                .calls
                .iter()
                .find(|c| c.id == call_id)
                .context("原始工具调用 ID 与回传证据不一致")?;
            let output: RecordFile = serde_json::from_str(content.context("工具结果内容缺失")?)?;
            let expected_ids: BTreeSet<_> = call.arguments["query"]["ids"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect();
            let actual_ids: BTreeSet<_> = output.records.iter().map(|v| v.id.as_str()).collect();
            if actual_ids != expected_ids || actual_ids.len() != output.records.len() {
                bail!("检测器提供的工具结果与查询参数不对应");
            }
            queried.extend(actual_ids.iter().map(|s| s.to_string()));
            for record in output.records {
                delivered.insert(record.id.clone(), record);
            }
            count += 1;
        }
    }
    Ok(Evaluation::default())
}
fn schema_claims(row: &HttpEvidence) -> Result<Evaluation> {
    if let Some(problem) = failure("content", row) {
        return Ok(evaluation(vec![problem], json!({})));
    }
    let r = response(row)?;
    let prompt = messages(row)?[0]["content"]
        .as_str()
        .context("结构化问题缺失")?;
    let expected: RecordFile =
        serde_json::from_str(prompt.split_once('：').context("结构化问题缺少预期记录")?.1)?;
    let native = serde_json::from_str::<Value>(&r.text).is_ok();
    let actual = parse_record_payload(&r.text, true).ok();
    let valid = r.calls.is_empty()
        && actual
            .as_ref()
            .is_some_and(|a| compare_records(&expected, a).passed);
    let mut content = claim(
        "content",
        if valid {
            Outcome::Satisfied
        } else {
            Outcome::Unsatisfied
        },
        if !valid {
            "结构化内容或字段不满足要求"
        } else if native {
            "本次结构化正文与字段均正确"
        } else {
            "内容正确，兼容解析通过；原生 JSON 格式未满足"
        },
        json!(expected),
        json!({"parsed":actual,"text":r.text,"unsolicited_calls":r.calls}),
        vec![row.id.clone()],
    );
    if valid && !native {
        content.condition = Some(
            "需要校验并提取唯一 JSON 代码块；本次未满足原生格式，不证明严格 Schema 支持".into(),
        );
    }
    let mut format = claim(
        "native_format",
        if native {
            Outcome::Satisfied
        } else {
            Outcome::Unsatisfied
        },
        if native {
            "本次正文为有效 JSON；不据单一样本声称严格约束始终生效"
        } else {
            "本次正文不是原生 JSON，仅可尝试兼容解析"
        },
        json!("正文直接为 JSON"),
        json!(native),
        vec![row.id.clone()],
    );
    format.origin = Some(FailureOrigin::Service);
    Ok(evaluation(
        vec![content, format],
        json!({"native_schema_request":row.request.get("response_format").is_some() || row.request.pointer("/text/format").is_some(),"native_format_satisfied":native,"compatibility_extraction":valid&&!native,"record_count":expected.records.len()}),
    ))
}
fn context_claims(rows: &[HttpEvidence]) -> Result<Evaluation> {
    let mut lower = 0u64;
    let mut upper: Option<u64> = None;
    let mut proof = vec![];
    for row in rows {
        let reservation = row
            .request
            .get("max_tokens")
            .or_else(|| row.request.get("max_output_tokens"))
            .and_then(Value::as_u64)
            .context("缺少上下文输出额度")?;
        if let Ok(r) = response(row) {
            if !r.refused && matches!(r.finish_reason.as_str(), "stop" | "completed" | "length") {
                if let Some(total) = r
                    .usage
                    .input
                    .filter(|n| *n > 0)
                    .and_then(|n| n.checked_add(reservation))
                {
                    lower = lower.max(total);
                    proof.push(row.id.clone());
                }
            }
        }
        if let Some(last) = row
            .attempts
            .last()
            .filter(|a| matches!(a.http_status, Some(400 | 413)))
        {
            if let Ok(value) = serde_json::from_str::<Value>(&last.response_body) {
                let error = &value["error"];
                let code = error["code"]
                    .as_str()
                    .or_else(|| error["type"].as_str())
                    .unwrap_or("");
                if matches!(
                    code,
                    "context_length_exceeded"
                        | "context_window_exceeded"
                        | "max_context_length_exceeded"
                ) {
                    if let Some(total) = error
                        .get("input_tokens")
                        .or_else(|| error.get("prompt_tokens"))
                        .and_then(Value::as_u64)
                        .filter(|n| *n > 0)
                        .and_then(|n| n.checked_add(reservation))
                    {
                        upper = Some(upper.map_or(total.saturating_sub(1), |old| {
                            old.min(total.saturating_sub(1))
                        }));
                        proof.push(row.id.clone());
                    }
                }
            }
        }
    }
    if proof.is_empty() {
        proof = rows.iter().map(|r| r.id.clone()).collect();
    }
    let conflict = upper.is_some_and(|n| n < lower);
    let maximum = upper.filter(|n| *n == lower && !conflict && lower > 0);
    let outcome = if conflict {
        Outcome::Unknown
    } else if lower >= MIN_CONTEXT_TOKENS {
        Outcome::Satisfied
    } else if upper.is_some_and(|n| n < MIN_CONTEXT_TOKENS) {
        Outcome::Unsatisfied
    } else {
        Outcome::Unknown
    };
    let reason = if conflict {
        "上下文计量证据互相矛盾，容量未确认".into()
    } else if outcome == Outcome::Satisfied {
        format!(
            "已支持至少 {lower} tokens，{}",
            if maximum.is_some() {
                "精确边界已确认"
            } else {
                "最大上限待确认"
            }
        )
    } else if outcome == Outcome::Unsatisfied {
        "可靠拒绝边界低于 128K，未满足本次容量要求".into()
    } else {
        "尚无足够计量证明至少 128K 上下文".into()
    };
    let mut minimum = claim(
        "minimum_context",
        outcome,
        reason,
        json!(MIN_CONTEXT_TOKENS),
        json!({"lower":lower,"upper":upper,"measurement_source":"service_usage"}),
        proof.clone(),
    );
    minimum.origin = Some(FailureOrigin::Service);
    if conflict {
        minimum.validity = Validity::Conflicting;
    }
    let exact = claim(
        "exact_maximum",
        if maximum.is_some() {
            Outcome::Satisfied
        } else {
            Outcome::Unknown
        },
        if maximum.is_some() {
            "成功与可靠拒绝的边界一致"
        } else {
            "精确最大容量未确认"
        },
        json!("相邻成功与拒绝边界"),
        json!(maximum),
        proof,
    );
    Ok(evaluation(
        vec![minimum, exact],
        json!({"verified_lower_bound_tokens":lower,"verified_upper_bound_tokens":upper,"maximum_context_tokens":maximum,"maximum_confirmed":maximum.is_some(),"output_reservation_tokens":1024,"measurement_source":"service_usage"}),
    ))
}
fn performance_claims(
    check: &CheckResult,
    directory: &Path,
    index: &EvidenceIndex,
) -> Result<Evaluation> {
    use crate::transport::RequestFailure;

    let waves = check
        .facts
        .get("rounds")
        .and_then(Value::as_array)
        .context("缺少逐档实验分组记录")?;
    let current_protocol =
        check.facts.get("search_method") == Some(&json!("doubling_network_recheck"));
    let mut rounds = vec![];
    let mut proof = vec![];
    let mut seen = BTreeSet::new();
    let mut lower = 0usize;
    let mut total = 0;
    let mut correct = 0;
    let mut rejected = None;
    let mut rejection = None;
    let mut expected = Some((1usize, "expand"));
    let mut previous_end = 0u64;
    let mut network_rechecks = 0usize;
    let mut connection_errors = 0usize;
    let mut recovered_levels = vec![];
    let mut network_levels = 0usize;
    let mut terminal_network_only = false;
    let mut non_capacity_reason = None;
    let mut request_guard = None;
    for wave in waves {
        let requests = wave["requests"].as_array().context("缺少逐档请求分组")?;
        let n = requests.len();
        if n == 0 || wave["concurrency"].as_u64() != Some(n as u64) {
            bail!("并发档位与实际请求分组不一致");
        }
        let mut times = vec![];
        let mut successes = 0;
        let mut limited = 0;
        let mut network = 0usize;
        let mut wave_connection_errors = 0usize;
        let mut wave_non_capacity = None;
        let mut wave_guard = None;
        let mut records = vec![];
        for original in requests {
            let id = original["evidence_id"]
                .as_str()
                .context("请求缺少证据编号")?;
            if !seen.insert(id) {
                bail!("并发分组重复引用同一请求");
            }
            let row = http(directory, index, id)?;
            let unsent_guard = row.attempts.is_empty()
                && matches!(
                    row.failure,
                    Some(RequestFailure::Cancelled | RequestFailure::StageBudget)
                );
            if !unsent_guard && (row.attempts.len() != 1 || row.attempts[0].number != 1) {
                bail!("并发请求不是单次尝试");
            }
            let input = messages(&row)?;
            if input.len() != 1
                || input[0]["role"] != "user"
                || input[0]["content"] != crate::concurrency::PROMPT
                || row.stream
                || row
                    .request
                    .get("max_tokens")
                    .or_else(|| row.request.get("max_output_tokens"))
                    .and_then(Value::as_u64)
                    != Some(crate::concurrency::OUTPUT_TOKENS)
                || row.request.get("tools").is_some()
                || row.request.get("response_format").is_some()
                || row.request.pointer("/text/format").is_some()
            {
                bail!("并发请求不符合当前普通短文本负载定义，不能按新规则重判旧负载");
            }
            let last = row.attempts.first();
            let status = last.and_then(|attempt| attempt.http_status);
            let parsed = response(&row);
            let good = row.failure.is_none()
                && row.parse_error.is_none()
                && last.is_some_and(|attempt| attempt.error.is_none())
                && parsed
                    .as_ref()
                    .is_ok_and(crate::concurrency::usable_text_response);
            let reason = match &row.failure {
                Some(RequestFailure::Cancelled) => "cancelled",
                Some(RequestFailure::StageBudget) => "stage_budget",
                Some(RequestFailure::Timeout) => "timeout",
                Some(RequestFailure::Network) if status.is_none() && parsed.is_err() => "network",
                Some(RequestFailure::Network) => "evidence_conflict",
                Some(RequestFailure::Http(code)) if status != Some(*code) => "evidence_conflict",
                Some(RequestFailure::Http(429)) => "rate_limited",
                Some(RequestFailure::Http(401 | 403 | 404)) => "configuration",
                Some(RequestFailure::Http(_)) => "http_error",
                Some(RequestFailure::InvalidResponse(_)) => "invalid_response",
                Some(RequestFailure::EvidenceConflict(_)) => "evidence_conflict",
                None if good => "passed",
                None if parsed
                    .as_ref()
                    .is_err_and(|error| error.is::<EvidenceConflict>()) =>
                {
                    "evidence_conflict"
                }
                None if status == Some(429) => "rate_limited",
                None if matches!(status, Some(401 | 403 | 404)) => "configuration",
                None if status.is_some_and(|code| !(200..300).contains(&code)) => "http_error",
                None => "invalid_response",
            };
            if current_protocol {
                let dispatch = original["dispatch_offset_ms"]
                    .as_u64()
                    .context("请求缺少相对本轮的派发时间")?;
                let elapsed = wave["elapsed_ms"].as_u64().context("缺少本轮实际耗时")?;
                if dispatch > elapsed
                    || dispatch
                        .checked_add(row.elapsed_ms)
                        .is_none_or(|end| end > elapsed.saturating_add(1))
                    || last.is_some_and(|attempt| attempt.elapsed_ms > row.elapsed_ms)
                {
                    bail!("请求耗时或派发时间超出本轮记录");
                }
            }
            limited += usize::from(reason == "rate_limited");
            network += usize::from(reason == "network");
            wave_connection_errors +=
                usize::from(matches!(row.failure, Some(RequestFailure::Network)));
            if !matches!(reason, "passed" | "rate_limited" | "timeout") {
                wave_non_capacity.get_or_insert(reason);
            }
            if matches!(reason, "cancelled" | "stage_budget" | "configuration") {
                // The probe gives cancellation, then stage budget, then configuration priority.
                if wave_guard.is_none()
                    || reason == "cancelled"
                    || (reason == "stage_budget" && wave_guard == Some("configuration"))
                {
                    wave_guard = Some(reason);
                }
            }
            times.push(row.elapsed_ms);
            successes += usize::from(good);
            proof.push(id.to_owned());
            records.push(json!({"evidence_id":id,"success":good,"reason":reason,
                "elapsed_ms":row.elapsed_ms,"dispatch_offset_ms":original["dispatch_offset_ms"],
                "http_status":status,"attempts":row.attempts.len(),
                "input_tokens":parsed.as_ref().ok().and_then(|r| r.usage.input),
                "completion_tokens":parsed.as_ref().ok().and_then(|r| r.usage.output)}));
        }
        // Preserve the actionable legacy workload error before rejecting its older policy.
        if !current_protocol {
            bail!("缺少当前并发网络复测协议标记，不能按新规则重判旧实验");
        }
        let phase = wave["phase"].as_str().context("缺少并发阶段")?;
        if expected != Some((n, phase)) {
            bail!("实验分组不符合翻倍及仅一次纯网络失败同档复测规则");
        }
        let started = wave["started_offset_ms"]
            .as_u64()
            .context("缺少本轮起始时间")?;
        let elapsed = wave["elapsed_ms"].as_u64().context("缺少本轮实际耗时")?;
        let minimum_start = previous_end
            .checked_add(if phase == "recheck" { 1000 } else { 0 })
            .context("并发阶段时间溢出")?;
        if started < minimum_start {
            bail!("并发轮次重叠或网络复测未等待 1000 毫秒");
        }
        previous_end = started.checked_add(elapsed).context("并发阶段时间溢出")?;
        if previous_end > check.elapsed_ms.saturating_add(1) {
            bail!("并发轮次超出完整探测耗时");
        }
        times.sort_unstable();
        let mean = times.iter().map(|n| *n as f64).sum::<f64>() / n as f64;
        let p95 = times[(n * 95).div_ceil(100) - 1];
        let maximum = *times.last().unwrap();
        let tail = if n < 20 { maximum } else { p95 };
        let latency = mean <= 10000.0 && tail <= 20000;
        let passed = successes == n && latency && wave_guard.is_none();
        let reason = if let Some(guard) = wave_guard {
            guard
        } else if successes != n {
            "request_failure"
        } else if mean > 10000.0 {
            "mean_latency"
        } else if !latency {
            if n < 20 { "max_latency" } else { "p95_latency" }
        } else {
            "passed"
        };
        let network_only = network > 0 && successes + network == n;
        if phase == "recheck" {
            network_rechecks += 1;
        }
        if phase == "expand" && wave_connection_errors > 0 {
            network_levels += 1;
        }
        if passed {
            lower = n;
            rejected = None;
            rejection = None;
            expected = n.checked_mul(2).map(|next| (next, "expand"));
            if phase == "recheck" {
                recovered_levels.push(n);
            }
        } else {
            rejected = wave_guard.is_none().then_some(n);
            rejection = wave_guard.is_none().then_some(reason);
            expected = if phase == "expand" && network_only {
                Some((n, "recheck"))
            } else {
                None
            };
        }
        terminal_network_only = network_only;
        non_capacity_reason = wave_non_capacity;
        request_guard = wave_guard;
        connection_errors += wave_connection_errors;
        total += n;
        correct += successes;
        let latency_summary = format!(
            "平均 {:.2} 秒，{} {:.2} 秒",
            mean / 1000.0,
            if n < 20 { "最慢" } else { "P95" },
            tail as f64 / 1000.0
        );
        rounds.push(json!({"phase":phase,"concurrency":n,"successful":successes,
            "failed":n-successes,"rate_limited":limited,"mean_elapsed_ms":mean,
            "p95_elapsed_ms":p95,"longest_elapsed_ms":maximum,"latency_summary":latency_summary,
            "tail_metric":if n<20{"maximum"}else{"p95"},"performance_passed":passed,
            "latency_passed":latency,"failure_reason":reason,"requests":records,
            "started_offset_ms":started,"elapsed_ms":elapsed}));
    }
    if !current_protocol {
        bail!("缺少当前并发网络复测协议标记，不能按新规则重判旧实验");
    }
    let listed: BTreeSet<_> = check
        .evidence_ids
        .iter()
        .filter(|id| id.starts_with("http-"))
        .map(String::as_str)
        .collect();
    if seen != listed {
        bail!("并发分组未完整覆盖原始请求证据");
    }
    let stop_reason = request_guard
        .or_else(|| check.facts.get("stop_reason").and_then(Value::as_str))
        .unwrap_or("stage_budget");
    let completed = rejected.is_some()
        && non_capacity_reason.is_none()
        && stop_reason == "first_unsuccessful_wave";
    let trial = terminal_network_only
        && request_guard.is_none()
        && matches!(
            stop_reason,
            "first_unsuccessful_wave"
                | "request_budget"
                | "round_budget"
                | "stage_budget"
                | "local_resource_limit"
        );
    let recommended = ((completed || trial) && lower > 0).then_some(lower);
    let basis = if completed {
        Some("capacity_stop")
    } else if trial && recommended.is_some() {
        Some("verified_trial")
    } else {
        None
    };
    let stability = if connection_errors == 0 {
        "no_error_observed"
    } else if network_levels != recovered_levels.len() {
        "unresolved"
    } else {
        "recovered"
    };
    let reason = if trial && recommended.is_some() {
        format!("并发 {lower} 仅为已验证的试跑起点；仍有连接风险，容量边界未知，不是精确最大值")
    } else if let (Some(n), Some(failed)) = (recommended, rejected) {
        format!("建议并发 {n}：下一档 {failed} 在本次负载下未达标；不是精确最大值")
    } else if non_capacity_reason.is_some() {
        format!("未取得完整有效文本或发生服务异常；已验证档位 {lower}，容量边界未确认")
    } else {
        format!("仅取得已验证档位 {lower}，尚无完整并发建议")
    };
    if proof.is_empty() {
        proof.push(format!("file:observations/{}.json", check.id.key()));
    }
    let best = rounds
        .iter()
        .rev()
        .find(|round| round["concurrency"] == lower && round["performance_passed"] == true);
    let facts = json!({"probe_mode":"concurrent_requests","workload_type":"short_text",
        "workload":crate::concurrency::WORKLOAD,"search_method":"doubling_network_recheck",
        "network_recheck_limit":1,"network_recheck_delay_ms":1000,
        "network_rechecks":network_rechecks,"connection_errors":connection_errors,
        "recovered_levels":recovered_levels,"connection_stability":stability,
        "concurrency":lower,"verified_concurrency_lower_bound":lower,
        "recommended_concurrency":recommended,"estimate_complete":completed,
        "recommendation_basis":basis,"maximum_confirmed":false,"maximum_concurrency":null,
        "request_count":total,"request_retries":0,"request_timeout_seconds":180,
        "max_mean_elapsed_ms":10000,"max_tail_elapsed_ms":20000,
        "latency_measurement":"request_to_complete_response",
        "p95_method":"nearest_rank","small_sample_tail":"maximum_below_20",
        "first_unsuccessful_concurrency":rejected,"first_unsuccessful_reason":rejection,
        "capacity_boundary_unconfirmed_reason":if completed { None } else {
            non_capacity_reason.or(Some(stop_reason))
        },"stop_reason":stop_reason,"probe_elapsed_ms":check.elapsed_ms,
        "longest_elapsed_ms":best.map(|round| &round["longest_elapsed_ms"]),
        "mean_elapsed_ms":best.map(|round| &round["mean_elapsed_ms"]),
        "p95_elapsed_ms":best.map(|round| &round["p95_elapsed_ms"]),
        "tail_metric":best.map(|round| &round["tail_metric"]),"rounds":rounds});
    let mut recommendation = claim(
        "recommendation",
        if recommended.is_some_and(|n| n >= 2) {
            Outcome::Satisfied
        } else {
            Outcome::Unknown
        },
        reason,
        json!("至少 2 并发正常完成普通短文本请求且有允许建议的停止证据"),
        facts.clone(),
        proof.clone(),
    );
    if recommended.is_some() || connection_errors > 0 {
        let mut condition = "仅为本次普通短文本负载及耗时标准下的粗略建议；不检验 JSON 格式或内容准确性，不是容量极限或长期保证".to_owned();
        if connection_errors > 0 {
            condition.push_str(if stability == "recovered" {
                "；曾出现网络错误，虽同档复测恢复，仍须关注连接稳定性"
            } else if trial && recommended.is_some() {
                "；连接错误尚未解决，已验证档位仅可作为试跑起点，容量边界未知"
            } else {
                "；网络错误档位未通过同档复测，连接稳定性风险仍未解除"
            });
        }
        recommendation.condition = Some(condition);
    }
    let mut content = claim(
        "batch_completion",
        if total == 0 {
            Outcome::Unknown
        } else if correct == total {
            Outcome::Satisfied
        } else {
            Outcome::Unsatisfied
        },
        format!("{total} 个请求中 {correct} 个正常结束并返回非空文本；不校验正文措辞或 JSON 格式"),
        json!(total),
        json!(correct),
        proof,
    );
    content.origin = Some(FailureOrigin::Service);
    Ok(evaluation(vec![recommendation, content], facts))
}
fn artifact<T: serde::de::DeserializeOwned>(
    directory: &Path,
    index: &EvidenceIndex,
    path: &str,
) -> Result<T> {
    let id = format!("file:{path}");
    let reference = index.get(&id).context("缺少任务证据文件")?;
    verify_reference(directory, reference)?;
    Ok(serde_json::from_slice(&std::fs::read(
        directory.join(path),
    )?)?)
}
fn result_artifact(directory: &Path, index: &EvidenceIndex, path: &str) -> Result<Option<Value>> {
    let Some(reference) = index.get(&format!("file:{path}")) else {
        if directory.join(path).try_exists()? {
            bail!("结果文件缺少证据索引");
        }
        return Ok(None);
    };
    verify_reference(directory, reference)?;
    Ok(serde_json::from_slice(&std::fs::read(directory.join(path))?).ok())
}
fn turn_normal_end(frames: &[Value]) -> bool {
    let Some(end) = frames
        .iter()
        .rposition(|f| f["type"] == "agent_end" && f["isTerminal"] != false)
    else {
        return false;
    };
    frames[end]["messages"]
        .as_array()
        .and_then(|messages| messages.iter().rfind(|m| m["role"] == "assistant"))
        .or_else(|| {
            frames[..end]
                .iter()
                .rfind(|f| f["type"] == "message_end" && f["message"]["role"] == "assistant")
                .map(|f| &f["message"])
        })
        .is_some_and(|message| message["stopReason"] == "stop")
}
fn trace_events(frames: &[Value]) -> Result<Vec<(usize, usize, ToolEvent)>> {
    let mut pending = BTreeMap::new();
    let mut events = vec![];
    for (position, frame) in frames.iter().enumerate() {
        if frame["type"] == "tool_execution_start" {
            let id = frame["toolCallId"]
                .as_str()
                .context("开始事件没有调用 ID")?;
            if pending.insert(id, (position, frame)).is_some() {
                bail!("同一调用在结束前被重复启动");
            }
        }
        if frame["type"] == "tool_execution_end" {
            let id = frame["toolCallId"]
                .as_str()
                .context("结束事件没有调用 ID")?;
            let error = frame["isError"].as_bool().context("结束事件缺少错误标记")?;
            let start = pending.remove(id);
            if start.is_none() && !error {
                bail!("成功工具执行没有开始事件");
            }
            let name = frame["toolName"].as_str().context("工具事件缺少名称")?;
            if start.is_some_and(|(_, s)| s["toolName"] != name) {
                bail!("工具开始和结束名称不一致");
            }
            let arguments = start
                .map(|(_, s)| s["args"].clone())
                .unwrap_or_else(|| frame["args"].clone());
            events.push((
                start.map(|(p, _)| p).unwrap_or(position),
                position,
                ToolEvent {
                    call_id: id.into(),
                    name: name.into(),
                    arguments,
                    result: frame["result"].clone(),
                    is_error: error,
                },
            ));
        }
    }
    if !pending.is_empty() {
        bail!("工具执行未完整结束");
    }
    Ok(events)
}
fn agent_claims(
    spec: &CheckSpec,
    check: &CheckResult,
    directory: &Path,
    index: &EvidenceIndex,
) -> Vec<Claim> {
    (1..=spec.repetitions)
        .map(|repetition| {
            let rule = format!("run_{repetition}");
            let Some(task) = check.tasks.iter().find(|t| t.repetition == repetition) else {
                let mut c = unknown(
                    &rule,
                    "本次任务尚未执行",
                    vec![format!("file:observations/{}.json", check.id.key())],
                    FailureOrigin::Detector,
                );
                c.observed = json!({"failure_stage":"evidence"});
                return c;
            };
            match agent_claim(check.id, task, directory, index) {
                Ok(c) => c,
                Err(error) => {
                    let mut c = conflicting(
                        &rule,
                        format!("本次任务的证据尚不能可靠关联：{error}"),
                        vec![format!("file:observations/{}.json", check.id.key())],
                    );
                    c.observed = json!({"failure_stage":"evidence"});
                    c
                }
            }
        })
        .collect()
}
fn agent_claim(
    kind: CheckId,
    task: &TaskResult,
    directory: &Path,
    index: &EvidenceIndex,
) -> Result<Claim> {
    let rule = format!("run_{}", task.repetition);
    let prefix = format!("agent/{}", task.id);
    if task.id != format!("{}-{}", kind.key(), task.repetition) || task.kind != kind {
        bail!("任务身份与检测项不一致");
    }
    let mut proof = task
        .evidence_ids
        .iter()
        .filter(|id| id.starts_with("http-"))
        .cloned()
        .collect::<Vec<_>>();
    for name in [
        "rpc.json",
        "source-snapshot.json",
        "workspace/result.json",
        "stage1-result.json",
    ] {
        let id = format!("file:{prefix}/{name}");
        if index.contains_key(&id) {
            proof.push(id);
        }
    }
    let frames: Vec<Value> = artifact(directory, index, &format!("{prefix}/rpc.json"))?;
    let snapshot: SourceSnapshot =
        artifact(directory, index, &format!("{prefix}/source-snapshot.json"))?;
    if format!("{:x}", Sha256::digest(serde_json::to_vec(&snapshot.files)?)) != snapshot.sha256 {
        bail!("来源快照校验不一致");
    }
    let reconstructed = trace_events(&frames)?;
    let events: Vec<_> = reconstructed.iter().map(|(_, _, e)| e.clone()).collect();
    if task.tool_events.len() != events.len()
        || task.tool_events.iter().zip(&events).any(|(a, b)| {
            a.call_id != b.call_id
                || a.name != b.name
                || a.arguments != b.arguments
                || a.result != b.result
                || a.is_error != b.is_error
        })
    {
        bail!("观察层工具事件与原始执行轨迹不一致");
    }
    let mut delivered = false;
    let mut recovered = false;
    if kind == CheckId::Recovery {
        for (_, error_position, error) in reconstructed
            .iter()
            .filter(|(_, _, e)| e.name == "read" && e.is_error)
        {
            let marker = error.result["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v["text"].as_str())
                .find_map(|s| {
                    s.split_once(':')
                        .map(|p| p.0)
                        .filter(|v| v.starts_with("AGENTCHECK_RECOVERABLE_READ_"))
                });
            let Some(marker) = marker else {
                continue;
            };
            for id in task
                .evidence_ids
                .iter()
                .filter(|id| id.starts_with("http-"))
            {
                let row = http(directory, index, id)?;
                if !messages(&row)?.iter().any(|m| {
                    ((m["role"] == "tool" && m["tool_call_id"] == error.call_id)
                        || (m["type"] == "function_call_output"
                            && m["call_id"]
                                == crate::omp::wire_call_id(row.protocol, &error.call_id)))
                        && m.get("content")
                            .or_else(|| m.get("output"))
                            .is_some_and(|content| content.to_string().contains(marker))
                }) {
                    continue;
                }
                if !row
                    .attempts
                    .last()
                    .is_some_and(|a| a.http_status.is_some_and(|s| (200..300).contains(&s)))
                {
                    continue;
                }
                delivered = true;
                if let Ok(reply) = response(&row) {
                    for call in &reply.calls {
                        for (message_position, frame) in
                            frames.iter().enumerate().filter(|(i, f)| {
                                *i > *error_position
                                    && f["type"] == "message_end"
                                    && f["message"]["role"] == "assistant"
                            })
                        {
                            let matches =
                                frame["message"]["content"].as_array().is_some_and(|parts| {
                                    parts.iter().any(|p| {
                                        p["type"] == "toolCall"
                                            && p["id"].as_str().is_some_and(|id| {
                                                response_call_matches(
                                                    row.protocol,
                                                    id,
                                                    &call.id,
                                                    &reply,
                                                )
                                            })
                                            && p["name"] == call.name
                                            && p["arguments"] == call.arguments
                                    })
                                });
                            if matches
                                && reconstructed.iter().any(|(start, end, e)| {
                                    *start > message_position
                                        && *end > *start
                                        && response_call_matches(
                                            row.protocol,
                                            &e.call_id,
                                            &call.id,
                                            &reply,
                                        )
                                        && e.name == call.name
                                        && execution_arguments_match(&call.arguments, &e.arguments)
                                        && !e.is_error
                                })
                            {
                                recovered = true;
                            }
                        }
                    }
                }
            }
        }
    }
    let starts: Vec<_> = frames
        .iter()
        .enumerate()
        .filter(|(_, f)| f["type"] == "agent_start")
        .map(|(i, _)| i)
        .collect();
    let normal = starts
        .last()
        .is_some_and(|start| turn_normal_end(&frames[*start..]));
    let execution =
        json!({"normal_end":normal,"fault_delivered":delivered,"recovery_observed":recovered});
    let continuation = starts.len() >= 2;
    let first_end = starts.get(1).copied().unwrap_or(frames.len());
    let first_normal = starts
        .first()
        .is_some_and(|start| turn_normal_end(&frames[*start..first_end]));
    let same_session = starts
        .first()
        .is_some_and(|start| !frames[*start..].iter().any(|f| f["type"] == "ready"));
    let result_write = |event: &ToolEvent| {
        !event.is_error
            && matches!(event.name.as_str(), "write" | "edit")
            && references_path(event, "result.json")
    };
    let write_position = reconstructed
        .iter()
        .rfind(|(_, _, e)| result_write(e))
        .map(|(start, _, _)| *start);
    let source_reads_before = |position: usize| {
        snapshot.files.iter().all(|(name, source)| {
            reconstructed.iter().any(|(_, end, e)| {
                *end < position && source_observed(e, &format!("inputs/{name}"), source)
            })
        })
    };
    let source_read = source_reads_before(write_position.unwrap_or(frames.len()));
    let first_write = reconstructed
        .iter()
        .rfind(|(_, end, e)| *end < first_end && result_write(e))
        .map(|(start, _, _)| *start);
    let first_source_read = source_reads_before(first_write.unwrap_or(first_end));
    let second_write = starts.get(1).is_some_and(|position| {
        let end = frames[*position..]
            .iter()
            .position(|f| f["type"] == "agent_end" && f["isTerminal"] != false)
            .map(|end| end + position)
            .unwrap_or(frames.len());
        reconstructed
            .iter()
            .any(|(start, finished, e)| *start > *position && *finished < end && result_write(e))
    });
    let mut observed = json!({
        "execution":execution,
        "source_reads_verified":source_read,
        "result_write_verified":write_position.is_some(),
        "fault_delivery_verified":delivered,
        "recovery_verified":recovered,
    });
    if kind == CheckId::Multiturn {
        observed["first_turn_normal_end"] = json!(first_normal);
        observed["first_turn_source_reads_verified"] = json!(first_source_read);
        observed["first_turn_write_verified"] = json!(first_write.is_some());
        observed["continuation_started"] = json!(continuation);
        observed["same_session_verified"] = json!(same_session);
        observed["second_turn_write_verified"] = json!(second_write);
    }
    if !normal || (kind == CheckId::Multiturn && !first_normal) {
        let mut c = unknown(
            &rule,
            if !normal {
                "任务未正常结束，不把部分文件判为结果失败"
            } else {
                "第一轮未正常结束，续接任务的前提尚未确认"
            },
            proof,
            FailureOrigin::Runtime,
        );
        observed["failure_stage"] = json!(if !normal { "runtime" } else { "first_turn" });
        c.observed = observed;
        return Ok(c);
    }
    let output_path = format!("{prefix}/workspace/result.json");
    let output_present = index.contains_key(&format!("file:{output_path}"));
    let output = result_artifact(directory, index, &output_path)?;
    let changes = (kind == CheckId::Multiturn && continuation).then(|| snapshot.changes());
    let validation = output
        .as_ref()
        .map(|output| {
            validate_task(
                &snapshot,
                changes.as_ref(),
                output,
                kind,
                &events,
                normal,
                delivered,
            )
        })
        .transpose()?;
    let first_validation = if kind == CheckId::Multiturn {
        result_artifact(directory, index, &format!("{prefix}/stage1-result.json"))?
            .map(|first| {
                snapshot
                    .expected(None)
                    .map(|expected| compare_records(&expected, &first))
            })
            .transpose()?
    } else {
        None
    };
    observed["result_file_present"] = json!(output_present);
    observed["result_json_valid"] = json!(output.is_some());
    observed["independent_validation"] = json!(validation);
    if kind == CheckId::Multiturn {
        observed["first_turn_correct"] = json!(first_validation.as_ref().map(|v| v.content_valid));
        observed["first_turn_validation"] = json!(first_validation);
    }
    let (outcome, stage, reason) = if !output_present {
        (
            Outcome::Unsatisfied,
            Some("result_write"),
            if recovered {
                "故障已送达且后续操作成功，但任务正常结束后没有结果文件"
            } else {
                "任务正常结束，但没有结果文件"
            },
        )
    } else if output.is_none() || validation.as_ref().is_some_and(|v| !v.content_valid) {
        (
            Outcome::Unsatisfied,
            Some("result_content"),
            if recovered {
                "故障已送达且后续操作成功，但最终结果的记录、字段或格式不符合要求"
            } else {
                "最终结果的记录、字段或格式不符合要求"
            },
        )
    } else if first_validation.as_ref().is_some_and(|v| !v.content_valid) {
        (
            Outcome::Unsatisfied,
            Some("first_turn"),
            "第一轮结果未正确合并全部来源记录",
        )
    } else if kind == CheckId::Recovery && !delivered {
        (
            Outcome::Unknown,
            Some("fault_delivery"),
            "尚未确认临时读取故障实际送达模型",
        )
    } else if kind == CheckId::Recovery && !recovered {
        (
            Outcome::Unknown,
            Some("recovery"),
            "故障已送达，但未关联到模型随后新发起并成功执行的工具操作",
        )
    } else if kind == CheckId::Multiturn && first_validation.is_none() {
        (
            Outcome::Unknown,
            Some("evidence"),
            "缺少可核验的第一轮结果快照",
        )
    } else if !source_read || (kind == CheckId::Multiturn && !first_source_read) {
        (
            Outcome::Unknown,
            Some("source_read"),
            "未确认写入结果前实际读取全部来源记录",
        )
    } else if write_position.is_none() || (kind == CheckId::Multiturn && first_write.is_none()) {
        (
            Outcome::Unknown,
            Some("result_write"),
            "未确认实际写入结果文件",
        )
    } else if kind == CheckId::Multiturn && (!continuation || !same_session || !second_write) {
        (
            Outcome::Unknown,
            Some("continuation"),
            "第一轮结果正确，但未确认在同一会话的第二轮实际写入更新",
        )
    } else if validation.as_ref().is_some_and(|v| v.passed) {
        (
            Outcome::Satisfied,
            None,
            match kind {
                CheckId::Recovery => "临时读取故障已送达，后续操作成功，最终合并结果正确",
                CheckId::Multiturn => {
                    "第一轮合并正确且正常结束，同一会话第二轮仅更新指定记录并保留其余内容"
                }
                _ => "已实际读取全部来源并写入正确的合并结果，任务正常结束",
            },
        )
    } else {
        (
            Outcome::Unknown,
            Some("evidence"),
            "尚未核验全部任务过程证据",
        )
    };
    observed["failure_stage"] = json!(stage);
    let mut c = claim(
        &rule,
        outcome,
        reason,
        json!("内容正确、实际来源与结果操作、正常结束及各任务前提"),
        observed,
        proof,
    );
    if outcome == Outcome::Unknown {
        c.origin = Some(FailureOrigin::Detector);
    }
    Ok(c)
}

fn response_call_matches(
    protocol: Protocol,
    runtime_id: &str,
    wire_id: &str,
    response: &ModelResponse,
) -> bool {
    if runtime_id == wire_id {
        return true;
    }
    let Some((call_id, item_id)) = runtime_id.split_once('|') else {
        return false;
    };
    protocol == Protocol::Responses
        && call_id == wire_id
        && response.wire["output"].as_array().is_some_and(|items| {
            items.iter().any(|item| {
                item["type"] == "function_call"
                    && item["call_id"] == wire_id
                    && item["id"] == item_id
            })
        })
}

fn execution_arguments_match(supplied: &Value, executed: &Value) -> bool {
    if supplied == executed {
        return true;
    }
    let (Some(supplied), Some(executed)) = (supplied.as_object(), executed.as_object()) else {
        return false;
    };
    // OMP carries concise intent in the assistant call, but strips it before tool execution.
    supplied.get("i").is_some_and(Value::is_string)
        && supplied.len() == executed.len() + 1
        && supplied
            .iter()
            .filter(|(key, _)| key.as_str() != "i")
            .all(|(key, value)| executed.get(key) == Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_intent_removal_preserves_recovery_proof_but_changed_arguments_do_not() {
        let supplied = json!({"i":"Reading task input","path":"inputs/source-1.json"});
        assert!(execution_arguments_match(
            &supplied,
            &json!({"path":"inputs/source-1.json"})
        ));
        assert!(execution_arguments_match(&supplied, &supplied));
        for executed in [
            json!({"path":"inputs/source-2.json"}),
            json!({}),
            json!({"path":"inputs/source-1.json","extra":true}),
        ] {
            assert!(!execution_arguments_match(&supplied, &executed));
        }
    }

    #[test]
    fn responses_runtime_ids_require_both_call_and_output_item_identity() {
        let wire = json!({"status":"completed","output":[{"type":"function_call","id":"fc_second","call_id":"second","name":"read","arguments":"{\"path\":\"input.json\"}"}]});
        let response = parse_json(Protocol::Responses, wire.to_string().as_bytes()).unwrap();
        assert!(response_call_matches(
            Protocol::Responses,
            "second|fc_second",
            "second",
            &response
        ));
        assert!(!response_call_matches(
            Protocol::Responses,
            "second|fc_wrong",
            "second",
            &response
        ));
        assert!(!response_call_matches(
            Protocol::Responses,
            "wrong|fc_second",
            "second",
            &response
        ));
        assert!(!response_call_matches(
            Protocol::ChatCompletions,
            "second|fc_second",
            "second",
            &response
        ));
    }
}
