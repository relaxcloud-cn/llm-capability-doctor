use crate::{
    config::Protocol,
    dataset::{
        RecordFile, SourceSnapshot, compare_records, parse_record_payload, references_path,
        validate_task,
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
    let waves = check
        .facts
        .get("rounds")
        .and_then(Value::as_array)
        .context("缺少逐档实验分组记录")?;
    let mut rounds = vec![];
    let mut proof = vec![];
    let mut lower = 0usize;
    let mut total = 0;
    let mut correct = 0;
    let mut rejected = None;
    let mut rejection = None;
    let mut non_capacity = false;
    for wave in waves {
        let requests = wave["requests"].as_array().context("缺少逐档请求分组")?;
        let n = requests.len();
        if n == 0
            || wave["concurrency"].as_u64() != Some(n as u64)
            || n != 1usize.checked_shl(rounds.len() as u32).unwrap_or(0)
            || rejected.is_some()
        {
            bail!("实验分组不符合翻倍、首个不合格停止的规则");
        }
        let mut times = vec![];
        let mut successes = 0;
        let mut limited = 0;
        let mut records = vec![];
        for original in requests {
            let id = original["evidence_id"]
                .as_str()
                .context("请求缺少证据编号")?;
            if proof.iter().any(|old| old == id) {
                bail!("并发分组重复引用同一请求");
            }
            let row = http(directory, index, id)?;
            if row.attempts.len() != 1 {
                bail!("并发请求不是单次尝试");
            }
            let expected: Value = serde_json::from_str(
                messages(&row)?[0]["content"]
                    .as_str()
                    .and_then(|s| s.split_once('：'))
                    .context("负载缺少独立标识")?
                    .1,
            )?;
            let good = response(&row).is_ok_and(|r| {
                r.normal_end
                    && r.calls.is_empty()
                    && parse_record_payload(&r.text, true).is_ok_and(|v| v == expected)
            });
            let last = &row.attempts[0];
            let reason = if good {
                "passed"
            } else if last.http_status == Some(429) {
                limited += 1;
                "rate_limited"
            } else if last.error.as_deref() == Some("完整请求超时") {
                "timeout"
            } else if last.http_status.is_some_and(|s| (200..300).contains(&s)) {
                non_capacity = true;
                "invalid_response"
            } else {
                non_capacity = true;
                "service_or_interruption"
            };
            times.push(row.elapsed_ms);
            successes += usize::from(good);
            proof.push(id.to_owned());
            records.push(json!({"evidence_id":id,"success":good,"reason":reason,"elapsed_ms":row.elapsed_ms,"dispatch_offset_ms":original["dispatch_offset_ms"],"http_status":last.http_status,"attempts":1}));
        }
        times.sort_unstable();
        let mean = times.iter().map(|n| *n as f64).sum::<f64>() / n as f64;
        let p95 = times[(n * 95).div_ceil(100) - 1];
        let maximum = *times.last().unwrap();
        let tail = if n < 20 { maximum } else { p95 };
        let latency = mean <= 10000.0 && tail <= 20000;
        let passed = successes == n && latency;
        let reason = if successes != n {
            "request_failure"
        } else if mean > 10000.0 {
            "mean_latency"
        } else if !latency {
            if n < 20 { "max_latency" } else { "p95_latency" }
        } else {
            "passed"
        };
        if passed {
            lower = n;
        } else {
            rejected = Some(n);
            rejection = Some(reason);
        }
        total += n;
        correct += successes;
        rounds.push(json!({"phase":"expand","concurrency":n,"successful":successes,"failed":n-successes,"rate_limited":limited,"mean_elapsed_ms":mean,"p95_elapsed_ms":p95,"longest_elapsed_ms":maximum,"tail_metric":if n<20{"maximum"}else{"p95"},"performance_passed":passed,"latency_passed":latency,"failure_reason":reason,"requests":records}));
    }
    let completed = rejected.is_some() && !non_capacity;
    let recommended = if completed && lower > 0 {
        Some(lower)
    } else {
        None
    };
    let reason = if non_capacity {
        format!("响应内容或服务异常导致停止；已验证档位 {lower}，容量边界未确认")
    } else if let (Some(n), Some(failed)) = (recommended, rejected) {
        format!("建议并发 {n}：下一档 {failed} 在本次负载下未达标；不是精确最大值")
    } else {
        format!("仅取得已验证档位 {lower}，尚无完整并发建议")
    };
    if proof.is_empty() {
        proof.push(format!("file:observations/{}.json", check.id.key()));
    }
    let facts = json!({"probe_mode":"concurrent_requests","search_method":"doubling_first_failure","concurrency":lower,"verified_concurrency_lower_bound":lower,"recommended_concurrency":recommended,"estimate_complete":completed,"maximum_confirmed":false,"maximum_concurrency":null,"request_count":total,"request_retries":0,"request_timeout_seconds":180,"max_mean_elapsed_ms":10000,"max_tail_elapsed_ms":20000,"p95_method":"nearest_rank","small_sample_tail":"maximum_below_20","first_unsuccessful_concurrency":rejected,"first_unsuccessful_reason":rejection,"capacity_boundary_unconfirmed_reason":if non_capacity{json!("content_or_service_error")}else{Value::Null},"rounds":rounds});
    let mut recommendation = claim(
        "recommendation",
        if recommended.is_some_and(|n| n >= 2) {
            Outcome::Satisfied
        } else {
            Outcome::Unknown
        },
        reason,
        json!("至少 2 并发且有允许建议的停止证据"),
        facts.clone(),
        proof.clone(),
    );
    if recommended.is_some() {
        recommendation.condition =
            Some("仅为本次短 JSON 请求及耗时标准下的粗略建议，不是容量极限或长期保证".into());
    }
    let mut content = claim(
        "batch_content",
        if total == 0 {
            Outcome::Unknown
        } else if correct == total {
            Outcome::Satisfied
        } else {
            Outcome::Unsatisfied
        },
        format!("{total} 个请求中 {correct} 个正确完成，错误类型逐请求保留"),
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
                return unknown(
                    &rule,
                    "本次任务尚未执行",
                    vec![format!("file:observations/{}.json", check.id.key())],
                    FailureOrigin::Detector,
                );
            };
            match agent_claim(check.id, task, directory, index) {
                Ok(c) => c,
                Err(error) => conflicting(
                    &rule,
                    format!("本次任务的证据尚不能可靠关联：{error}"),
                    vec![format!("file:observations/{}.json", check.id.key())],
                ),
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
    let normal = frames
        .iter()
        .rfind(|f| f["type"] == "message_end" && f["message"]["role"] == "assistant")
        .is_some_and(|f| f["message"]["stopReason"] == "stop")
        && frames.iter().any(|f| f["type"] == "agent_end");
    let mut execution =
        json!({"normal_end":normal,"fault_delivered":false,"recovery_observed":false});
    if !normal {
        let mut c = unknown(
            &rule,
            "任务未正常结束，不把部分文件判为模型输出失败",
            proof,
            FailureOrigin::Runtime,
        );
        c.observed = json!({"execution":execution});
        return Ok(c);
    }
    let output = artifact::<Value>(directory, index, &format!("{prefix}/workspace/result.json"));
    let output = match output {
        Ok(v) => v,
        Err(_) => {
            return Ok(claim(
                &rule,
                Outcome::Unsatisfied,
                "任务正常结束，但没有有效结果文件",
                json!("有效 JSON 结果文件"),
                json!({"execution":execution}),
                proof,
            ));
        }
    };
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
                    (m["role"] == "tool" || m["type"] == "function_call_output")
                        && m.to_string().contains(marker)
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
                                            && p["id"] == call.id
                                            && p["name"] == call.name
                                    })
                                });
                            if matches
                                && reconstructed.iter().any(|(start, end, e)| {
                                    *start > message_position
                                        && *end > *start
                                        && e.call_id == call.id
                                        && e.name == call.name
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
    execution["fault_delivered"] = json!(delivered);
    execution["recovery_observed"] = json!(recovered);
    let starts: Vec<_> = frames
        .iter()
        .enumerate()
        .filter(|(_, f)| f["type"] == "agent_start")
        .map(|(i, _)| i)
        .collect();
    let continuation = starts.len() >= 2;
    let changes = snapshot.changes();
    let validation = validate_task(
        &snapshot,
        if kind == CheckId::Multiturn && continuation {
            Some(&changes)
        } else {
            None
        },
        &output,
        kind,
        &events,
        normal,
        delivered,
    )?;
    let first_correct = if kind == CheckId::Multiturn {
        let first: Value = artifact(directory, index, &format!("{prefix}/stage1-result.json"))?;
        compare_records(&snapshot.expected(None)?, &first).passed
    } else {
        true
    };
    let second_write = kind != CheckId::Multiturn
        || starts.get(1).is_some_and(|position| {
            reconstructed.iter().any(|(start, _, e)| {
                *start > *position
                    && !e.is_error
                    && matches!(e.name.as_str(), "write" | "edit")
                    && references_path(e, "result.json")
            })
        });
    let outcome = if !validation.content_valid || !first_correct {
        Outcome::Unsatisfied
    } else if validation.passed && second_write && (kind != CheckId::Recovery || recovered) {
        Outcome::Satisfied
    } else {
        Outcome::Unknown
    };
    let reason = match outcome {
        Outcome::Satisfied => format!("第 {} 次任务的完整结果与必要过程均已核验", task.repetition),
        Outcome::Unsatisfied => format!("第 {} 次任务的实际记录未满足内容要求", task.repetition),
        Outcome::Unknown => format!(
            "第 {} 次任务缺少已完成的必要过程证据，不能判断模型能力",
            task.repetition
        ),
    };
    Ok(claim(
        &rule,
        outcome,
        reason,
        json!("内容正确、实际来源与结果操作、正常结束及各任务前提"),
        json!({"execution":execution,"independent_validation":validation,"first_turn_correct":first_correct,"second_turn_write_verified":second_write,"fault_delivery_verified":delivered,"recovery_verified":recovered}),
        proof,
    ))
}
