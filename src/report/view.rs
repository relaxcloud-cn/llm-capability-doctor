use crate::{
    judgment::{Outcome, Validity},
    model::{CheckId, CheckResult, FailureOrigin, RunResult, Status, Verdict},
};
use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Component, Path},
};

const TEMPLATE: &str = include_str!("../report-template.html");
const CHECK_ICON: &str =
    "<svg class=\"icon\" aria-hidden=\"true\"><use href=\"#icon-check\"/></svg>";

fn html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn script_json(value: &Value) -> String {
    value
        .to_string()
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}
fn bind(template: &str, values: &BTreeMap<&str, String>) -> Result<String> {
    let mut remaining = template;
    let mut output = String::with_capacity(template.len());
    while let Some(start) = remaining.find("{{") {
        output.push_str(&remaining[..start]);
        let tail = &remaining[start + 2..];
        let end = tail.find("}}").context("报告模板占位符未结束")?;
        output.push_str(
            values
                .get(&tail[..end])
                .with_context(|| format!("报告模板字段缺失：{}", &tail[..end]))?,
        );
        remaining = &tail[end + 2..];
    }
    output.push_str(remaining);
    Ok(output)
}
fn status_key(status: Status) -> &'static str {
    match status {
        Status::Passed => "passed",
        Status::Failed => "failed",
        Status::Incomplete => "incomplete",
        Status::NotExecuted => "not_executed",
    }
}
fn seconds(ms: u64) -> String {
    format!("{:.2} 秒", ms as f64 / 1000.0)
}
fn duration(ms: u64) -> String {
    let seconds = ms / 1000;
    if seconds >= 3600 {
        format!(
            "{} 小时 {} 分 {} 秒",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else if seconds >= 60 {
        format!("{} 分 {} 秒", seconds / 60, seconds % 60)
    } else {
        format!("{seconds} 秒")
    }
}
fn tokens(value: u64) -> String {
    if value > 0 && value % 1024 == 0 {
        format!("{}K", value / 1024)
    } else {
        value.to_string()
    }
}
fn fact_u64(check: &CheckResult, key: &str) -> Option<u64> {
    check.facts.get(key).and_then(Value::as_u64)
}
fn rounds(check: &CheckResult) -> &[Value] {
    check
        .facts
        .get("rounds")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
fn recommended(check: &CheckResult) -> Option<u64> {
    (check.status == Status::Passed)
        .then(|| fact_u64(check, "recommended_concurrency"))
        .flatten()
        .filter(|n| *n > 0)
}
fn verified_pass(check: &CheckResult) -> bool {
    check.status == Status::Passed
        && check.assessment.as_ref().is_some_and(|a| {
            a.status == Status::Passed
                && a.requirements.iter().any(|r| r.required)
                && a.requirements
                    .iter()
                    .filter(|r| r.required)
                    .all(|r| r.validity == Validity::Verified && r.outcome == Outcome::Satisfied)
        })
}
fn conditions(check: &CheckResult) -> &[String] {
    check
        .assessment
        .as_ref()
        .map(|a| a.conditions.as_slice())
        .unwrap_or(&[])
}
fn has_advice(check: &CheckResult) -> bool {
    check.status != Status::Passed || !check.findings.is_empty() || !conditions(check).is_empty()
}
fn display_name(run: &RunResult, id: CheckId) -> &str {
    let registered = run.check_name(id);
    if registered != id.name() {
        return registered;
    }
    match id {
        CheckId::Context => "上下文容量",
        CheckId::Performance => "并发能力",
        _ => registered,
    }
}
fn note(run: &RunResult, check: &CheckResult) -> String {
    match check.id {
        CheckId::Response => "请求收发与历史消息回传".into(),
        CheckId::Tools => "选择工具、填写参数、使用结果".into(),
        CheckId::Schema => "输出格式与字段内容".into(),
        CheckId::Context => check
            .assessment
            .as_ref()
            .and_then(|a| a.requirements.iter().find(|r| r.id == "minimum_context"))
            .and_then(|r| r.expected.as_u64())
            .map(|n| format!("本次最低要求 {} tokens", tokens(n)))
            .unwrap_or_else(|| "本次记录的请求容量".into()),
        CheckId::Performance => "满足本次时延标准的请求档位".into(),
        CheckId::Continuous => "连续完成文件读取与汇总".into(),
        CheckId::Recovery => "遇到可恢复错误后继续任务".into(),
        CheckId::Multiturn => "按新要求新增、修改并保留内容".into(),
        CheckId::Custom(_) => run
            .plan
            .iter()
            .find(|s| s.id == check.id)
            .map(|s| s.scope.clone())
            .unwrap_or_else(|| "按本次注册要求检查".into()),
    }
}
fn row_copy(check: &CheckResult) -> (String, String, bool) {
    if check.status != Status::Passed {
        return (
            check.title.clone(),
            if check.status == Status::NotExecuted {
                "本项尚未执行，不作为通过或失败证据。".into()
            } else {
                "具体原因、处理建议与已保存的证据见下方记录。".into()
            },
            false,
        );
    }
    match check.id {
        CheckId::Context => {
            if let Some(maximum) = fact_u64(check, "maximum_context_tokens")
                .filter(|n| *n > 0 && check.facts.get("maximum_confirmed") == Some(&json!(true)))
            {
                return (
                    format!("已确认最大 {} tokens", tokens(maximum)),
                    "本次计量与容量边界记录见检测附件，不代表长文理解能力。".into(),
                    true,
                );
            }
            if let Some(lower) = fact_u64(check, "verified_lower_bound_tokens").filter(|n| *n > 0) {
                return (
                    format!("已验证 ≥ {} tokens", tokens(lower)),
                    "达到本次容量要求；精确最大值和长文理解能力未确认。".into(),
                    true,
                );
            }
        }
        CheckId::Performance => {
            if let Some(level) = recommended(check) {
                let best = rounds(check)
                    .iter()
                    .find(|r| r["concurrency"].as_u64() == Some(level));
                let detail = best
                    .and_then(|r| {
                        Some((
                            r["mean_elapsed_ms"].as_f64()?,
                            if r["tail_metric"] == "maximum" {
                                ("最慢", r["longest_elapsed_ms"].as_f64()?)
                            } else {
                                ("P95", r["p95_elapsed_ms"].as_f64()?)
                            },
                        ))
                    })
                    .map(|(mean, (label, tail))| {
                        format!(
                            "平均 {:.2} 秒，{label} {:.2} 秒；仅为本次负载下的建议档位。",
                            mean / 1000.0,
                            tail / 1000.0
                        )
                    })
                    .unwrap_or_else(|| "仅为本次负载下的建议档位，不是精确容量极限。".into());
                return (format!("建议 {level} 并发"), detail, true);
            }
        }
        _ => {}
    }
    if verified_pass(check) {
        let (title, detail) = match check.id {
            CheckId::Response => (
                "普通与流式响应完整",
                "前轮消息已正确回传，可完成基本对话交互。",
            ),
            CheckId::Tools => (
                "能够正确调用工具并使用结果",
                "工具、参数、调用 ID 和回传内容均对应正确。",
            ),
            CheckId::Schema
                if check.facts.get("native_format_satisfied") == Some(&json!(false)) =>
            {
                (
                    "内容符合要求，需兼容解析",
                    "本次记录内容正确，但原生 JSON 格式未满足；请查看兼容条件。",
                )
            }
            CheckId::Schema if check.facts.get("native_format_satisfied") == Some(&json!(true)) => {
                (
                    "本次格式与字段内容符合要求",
                    "本次返回的格式和记录内容已按要求核对。",
                )
            }
            CheckId::Continuous => (
                "多步操作连续完成，结果正确",
                "实际读写过程和最终文件均已核对。",
            ),
            CheckId::Recovery => (
                "收到错误后恢复并完成任务",
                "已核对错误实际送达、后续操作及最终结果。",
            ),
            CheckId::Multiturn => (
                "新要求已落实，原有记录保留正确",
                "分别核对修改前后的文件，不仅看模型声称已完成。",
            ),
            _ => return (check.title.clone(), String::new(), false),
        };
        return (title.into(), detail.into(), false);
    }
    (check.title.clone(), String::new(), false)
}
fn render_rows(run: &RunResult) -> String {
    let mut output = String::new();
    for check in &run.checks {
        let id = html(&check.id.key());
        let status = status_key(check.status);
        let icon = if check.status == Status::Passed {
            CHECK_ICON
        } else if check.status == Status::Failed {
            "!"
        } else {
            "—"
        };
        let (title, detail, metric) = row_copy(check);
        let class = if metric {
            "metric-value"
        } else {
            "result-text"
        };
        output.push_str(&format!("<tr data-check=\"{id}\" data-status=\"{status}\" data-advice=\"{}\"><th scope=\"row\">{}<span class=\"item-note\">{}</span></th><td class=\"customer-col\" data-label=\"客户实测\"><div class=\"result-line\"><span class=\"check-status {status}\">{icon}{}</span><span class=\"{class}\">{}</span></div>",has_advice(check),html(display_name(run, check.id)),html(&note(run,check)),check.status.label(),html(&title)));
        if !detail.is_empty() {
            output.push_str(&format!("<p class=\"row-note\">{}</p>", html(&detail)));
        }
        if check.id == CheckId::Tools && check.status == Status::Passed && check.elapsed_ms > 0 {
            output.push_str(&format!("<div class=\"metric-line\"><span>本次交互用时 <strong>{}</strong></span><span>本项实际完成用时</span></div>",seconds(check.elapsed_ms)));
        }
        for condition in conditions(check) {
            output.push_str(&format!(
                "<p class=\"row-note\">适用条件：{}</p>",
                html(condition)
            ));
        }
        if check.status != Status::Passed {
            output.push_str(&format!(
                "<a class=\"read-link\" href=\"#issue-{id}-0\">查看原因与建议</a>"
            ));
        } else if check.id == CheckId::Performance && has_advice(check) {
            output.push_str("<a class=\"read-link\" href=\"#guidance\">查看并发使用建议</a>");
        } else if check.id == CheckId::Tools || matches!(check.id, CheckId::Custom(_)) {
            output.push_str(&format!("<a class=\"read-link\" href=\"#proof-{id}\" data-open=\"proof-{id}\">查看核验记录</a>"));
        }
        output.push_str("</td></tr>\n");
    }
    output
}
fn human_fact(fact: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(fact) {
        if let Some(diff) = value["differences"].as_array().and_then(|v| v.first()) {
            let id = diff["id"].as_str().unwrap_or("未知记录");
            let field = diff["field"].as_str().unwrap_or("字段");
            if field == "record" {
                return if diff["actual"].is_null() {
                    format!("结果遗漏记录 {id}。")
                } else {
                    format!("结果包含未要求的记录 {id}。")
                };
            }
            let label = match field {
                "owner" => "负责人",
                "version" => "版本号",
                _ => field,
            };
            let display = |v: &Value| {
                v.as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| v.to_string())
            };
            return format!(
                "记录 {id} 的{label}应为 {}，实际为 {}。",
                display(&diff["expected"]),
                display(&diff["actual"])
            );
        }
        if let Some(errors) = value["process_errors"].as_array() {
            return errors
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("；");
        }
    }
    fact.into()
}
fn origin(origin: FailureOrigin) -> &'static str {
    match origin {
        FailureOrigin::ModelOutput => "本次模型输出",
        FailureOrigin::Service => "本次服务请求",
        FailureOrigin::Configuration => "连接或鉴权配置",
        FailureOrigin::Runtime => "检测运行环境",
        FailureOrigin::Detector => "检测过程",
    }
}
struct Advice {
    check: CheckId,
    tone: &'static str,
    title: String,
    fact: String,
    impact: String,
    suggestion: String,
    origin: Option<FailureOrigin>,
}
fn advice(run: &RunResult) -> Vec<Advice> {
    let mut output = vec![];
    for check in run.checks.iter().filter(|c| has_advice(c)) {
        if check.id == CheckId::Performance {
            if let Some(level) = recommended(check) {
                let rejected = fact_u64(check, "first_unsuccessful_concurrency");
                let all_completed = rejected
                    .and_then(|n| {
                        rounds(check)
                            .iter()
                            .find(|r| r["concurrency"].as_u64() == Some(n))
                    })
                    .is_some_and(|r| {
                        r["successful"] == r["concurrency"] && r["successful"].as_u64().is_some()
                    });
                let fact = if let Some(next) = rejected {
                    if all_completed {
                        format!(
                            "本次更高的 {next} 并发请求也全部返回，但响应时间超过本次标准；{level} 是建议档位，不是精确极限。"
                        )
                    } else {
                        format!(
                            "下一档 {next} 并发未全部满足本次要求；{level} 是建议档位，不是精确极限。"
                        )
                    }
                } else {
                    format!("仅保留本次已验证的建议档位 {level}，不代表已测得精确极限。")
                };
                output.push(Advice {check:check.id,tone:"passed",title:format!("建议从 {level} 并发档位试用，再按业务负载调整"),fact,impact:"先按建议档位试用。长输入、复杂推理或更长输出的请求，需要用实际业务负载重新测量。".into(),suggestion:"模型、服务配置、硬件、账号配额或业务负载改变后重测。结果反映当时服务条件，不是长期性能保证。".into(),origin:None});
                continue;
            }
        }
        let tone = status_key(check.status);
        if !check.findings.is_empty() {
            for finding in &check.findings {
                output.push(Advice {
                    check: check.id,
                    tone,
                    title: finding.title.clone(),
                    fact: human_fact(&finding.fact),
                    impact: finding.impact.clone(),
                    suggestion: finding.suggestion.clone(),
                    origin: Some(finding.origin),
                });
            }
        } else {
            output.push(Advice {
                check: check.id,
                tone,
                title: check.title.clone(),
                fact: if check.status == Status::Passed {
                    conditions(check).join("；")
                } else if check.status == Status::NotExecuted {
                    "本项尚未执行，不能作为通过或失败依据。".into()
                } else {
                    "本项结果以已保存的判定与证据为准；尚未取得的证据不能补判为成功。".into()
                },
                impact: match check.status {
                    Status::Passed => "请按本次已确认的条件使用。",
                    Status::Failed => "本项存在未满足本次要求的记录；不能外推为所有场景均不可用。",
                    _ => "本项能力尚未确认，不应直接归为模型能力不足。",
                }
                .into(),
                suggestion: if check.status == Status::Passed {
                    "服务配置或使用条件变化后重新检测。"
                } else {
                    "核对本项原始记录和运行条件，处理后重新检测。"
                }
                .into(),
                origin: None,
            });
        }
    }
    output
}
fn guidance(cards: &[Advice], verdict: Verdict) -> String {
    if cards.is_empty() {
        let (title, text) = if verdict == Verdict::Passed {
            (
                "保持本次验证的接入条件",
                "目前没有额外处理建议。模型、配置或业务负载变化后重新检测。",
            )
        } else {
            (
                "请先核对检测计划与记录",
                "当前没有足够的逐项处理信息。补齐检测记录后，再确认接入结论。",
            )
        };
        return format!(
            "<article class=\"card advice-card\"><div class=\"advice-title\"><span class=\"info-symbol\" aria-hidden=\"true\">i</span><div><h3>{title}</h3><p>{text}</p></div></div></article>"
        );
    }
    let mut output = String::new();
    let mut indexes = BTreeMap::new();
    for card in cards {
        let index = indexes.entry(card.check).or_insert(0);
        let id = format!("issue-{}-{index}", card.check.key());
        *index += 1;
        let performance = card.check == CheckId::Performance && card.tone == "passed";
        let labels = if performance {
            ("接入时怎么用", "什么时候重新检测")
        } else {
            ("会有什么影响", "建议怎么处理")
        };
        output.push_str(&format!("<article class=\"card advice-card {}\" id=\"{}\"><div class=\"advice-title\"><span class=\"info-symbol\" aria-hidden=\"true\">{}</span><div><h3>{}</h3><p>{}</p></div></div><div class=\"advice-grid\"><div><h4>{}</h4><p>{}</p></div><div><h4>{}</h4><p>{}</p></div></div>",card.tone,html(&id),if card.tone=="failed"{"!"}else{"i"},html(&card.title),html(&card.fact),labels.0,html(&card.impact),labels.1,html(&card.suggestion)));
        if let Some(value) = card.origin {
            output.push_str(&format!(
                "<p class=\"origin-note\">记录归属：{}；具体依据见本项证据。</p>",
                origin(value)
            ));
        }
        if performance {
            output.push_str("<a class=\"read-link\" href=\"#load-details\" data-open=\"load-details\">查看本次并发测量记录</a>");
        } else {
            output.push_str(&format!("<a class=\"read-link\" href=\"#proof-{}\" data-open=\"proof-{}\">查看本项检测记录</a>",html(&card.check.key()),html(&card.check.key())));
        }
        output.push_str("</article>\n");
    }
    output
}
fn evidence_href(value: &str) -> Option<String> {
    if value.is_empty()
        || value.contains('\\')
        || Path::new(value)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return None;
    }
    let mut result = String::from("./");
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._/".contains(&byte) {
            result.push(byte as char);
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
    }
    Some(result)
}
fn evidence_links(run: &RunResult, ids: &[String]) -> String {
    let mut output = String::new();
    for id in ids {
        if let Some(reference) = run.evidence.get(id) {
            let link = evidence_href(&reference.path)
                .map(|path| {
                    format!(
                        "<a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\">{}</a>",
                        html(&path),
                        html(id)
                    )
                })
                .unwrap_or_else(|| html(id));
            output.push_str(&format!(
                "<li>{link}<small>ZIP 路径：{} · SHA-256：{}</small></li>",
                html(&reference.path),
                html(&reference.sha256)
            ));
        } else {
            output.push_str(&format!("<li>{}：本次索引中没有这份证据</li>", html(id)));
        }
    }
    if output.is_empty() {
        return "<p>本项没有可展示的证据引用。</p>".into();
    }
    format!(
        "<details><summary>查看 {} 处证据引用</summary><ul class=\"evidence-list\">{output}</ul></details>",
        ids.len()
    )
}
fn proofs(run: &RunResult) -> String {
    let requirements = run
        .checks
        .iter()
        .filter_map(|c| c.assessment.as_ref())
        .map(|a| a.requirements.len())
        .sum::<usize>();
    let references = run
        .checks
        .iter()
        .filter_map(|c| c.assessment.as_ref())
        .flat_map(|a| &a.requirements)
        .map(|r| r.evidence_ids.len())
        .sum::<usize>();
    let mut output = format!(
        "<div class=\"proof-list\"><div class=\"proof-item\"><h4>本次检测记录</h4><p>{} 个检测项目，{requirements} 个要求记录，{} 份索引证据，{references} 处要求引用。索引数量不代表独立样本量或能力评分。</p></div><div class=\"proof-item\"><h4>完整记录可复核</h4><p>原始请求、响应、任务文件与索引保留在 ZIP 中。单独分享 HTML 时，先下载附件，再按记录路径查看；校验和只用于检查完整性，不代表第三方认证。</p></div></div>",
        run.checks.len(),
        run.evidence.len()
    );
    for check in &run.checks {
        output.push_str(&format!(
            "<details id=\"proof-{}\"><summary>{} · {}</summary><div class=\"details-body\">",
            html(&check.id.key()),
            html(display_name(run, check.id)),
            check.status.label()
        ));
        if let Some(spec) = run.plan.iter().find(|s| s.id == check.id) {
            output.push_str(&format!(
                "<p class=\"proof-scope\">规则版本 {} · {}</p>",
                html(&spec.version),
                html(&spec.scope)
            ));
        }
        if !check.tasks.is_empty() {
            let passed = check
                .tasks
                .iter()
                .filter(|t| t.status == Status::Passed)
                .count();
            output.push_str(&format!("<p>已记录 {} 次任务，其中 {passed} 次通过；次数只说明本次样本，不作为长期稳定性承诺。</p><ul>",check.tasks.len()));
            for task in &check.tasks {
                output.push_str(&format!(
                    "<li>{} · {} · {}：{}</li>",
                    html(&task.id),
                    task.status.label(),
                    seconds(task.elapsed_ms),
                    html(&task.title)
                ));
            }
            output.push_str("</ul>");
        }
        if let Some(assessment) = &check.assessment {
            for requirement in &assessment.requirements {
                let validity = match requirement.validity {
                    Validity::Verified => "证据已核验",
                    Validity::Insufficient => "证据不足",
                    Validity::Conflicting => "证据冲突",
                };
                let outcome = match requirement.outcome {
                    Outcome::Satisfied => "满足",
                    Outcome::Unsatisfied => "不满足",
                    Outcome::Unknown => "未确认",
                };
                output.push_str(&format!(
                    "<div class=\"requirement\"><h4>{}（{}）</h4><p>{validity} · {outcome}；{}</p>",
                    html(&requirement.title),
                    if requirement.required {
                        "必需"
                    } else {
                        "补充观察"
                    },
                    html(&requirement.reason)
                ));
                if let Some(value) = requirement.origin {
                    output.push_str(&format!("<p>记录归属：{}</p>", origin(value)));
                }
                output.push_str(&evidence_links(run, &requirement.evidence_ids));
                output.push_str("</div>");
            }
        } else {
            output.push_str(
                "<p>本记录未提供逐项证据裁决，保留当时检测状态，不补充未核验的结论。</p>",
            );
            output.push_str(&evidence_links(run, &check.evidence_ids));
        }
        for condition in conditions(check) {
            output.push_str(&format!(
                "<p class=\"source-note\">本项适用条件：{}</p>",
                html(condition)
            ));
        }
        output.push_str("</div></details>");
    }
    output
}
fn measured_ms(value: &Value) -> String {
    value
        .as_f64()
        .filter(|v| v.is_finite() && *v >= 0.0)
        .map(|n| format!("{:.2} 秒", n / 1000.0))
        .unwrap_or_else(|| "未记录".into())
}
fn load_details(run: &RunResult) -> String {
    let Some(check) = run
        .checks
        .iter()
        .find(|c| c.id == CheckId::Performance && !rounds(c).is_empty())
    else {
        return String::new();
    };
    let workload = if check.facts.get("workload_type").and_then(Value::as_str) == Some("short_text")
    {
        "普通短文本负载；只核对完整结束且有有效文本，不要求 JSON、固定措辞或原样回显"
    } else {
        "历史短 JSON 回显负载，按当时规则保留记录"
    };
    let mut output = format!(
        "<details id=\"load-details\"><summary>并发档位与耗时明细</summary><div class=\"details-body\"><p>{workload}。耗时从请求发出计到完整响应，不是首 token 延迟。</p>"
    );
    if let (Some(mean), Some(tail)) = (
        fact_u64(check, "max_mean_elapsed_ms"),
        fact_u64(check, "max_tail_elapsed_ms"),
    ) {
        output.push_str(&format!("<p>本次标准：平均 ≤ {} 秒、P95 ≤ {} 秒；少于 20 个请求取最慢用时。未测量精确最大并发。</p>",mean as f64/1000.0,tail as f64/1000.0));
    } else {
        output
            .push_str("<p>历史记录未设置响应时间合格阈值，保留当时结论，不按新标准重新判定。</p>");
    }
    if let Some(timeout) = fact_u64(check, "request_timeout_seconds") {
        output.push_str(&format!(
            "<p>本次单请求硬超时 {timeout} 秒，不是性能合格线。</p>"
        ));
    }
    output.push_str("<div class=\"table-scroll\" role=\"region\" aria-label=\"并发明细，可横向滚动\" tabindex=\"0\"><table class=\"load-table\"><thead><tr><th scope=\"col\">并发档位</th><th scope=\"col\">完成 / 总数</th><th scope=\"col\">平均用时</th><th scope=\"col\">尾部用时</th><th scope=\"col\">记录结果</th></tr></thead><tbody>");
    for row in rounds(check) {
        let n = row["concurrency"].as_u64();
        let show = |value: Option<u64>| {
            value
                .map(|v| v.to_string())
                .unwrap_or_else(|| "未记录".into())
        };
        let tail = match row["tail_metric"].as_str() {
            Some("maximum") => format!("最慢 {}", measured_ms(&row["longest_elapsed_ms"])),
            Some("p95") => format!("P95 {}", measured_ms(&row["p95_elapsed_ms"])),
            _ => "未记录".into(),
        };
        let label = row["failure_reason"]
            .as_str()
            .map(crate::concurrency::failure_label)
            .unwrap_or("未记录");
        let class = if n.is_some() && n == recommended(check) {
            "recommended"
        } else if row["performance_passed"] == false {
            "boundary"
        } else {
            ""
        };
        output.push_str(&format!("<tr class=\"{class}\"><td>{}</td><td>{} / {}</td><td>{}</td><td>{}</td><td>{}</td></tr>",show(n),show(row["successful"].as_u64()),show(n),measured_ms(&row["mean_elapsed_ms"]),html(&tail),html(label)));
    }
    output.push_str("</tbody></table></div><p class=\"text-note\">背景负载、网络和配额影响测量结果；缺少环境记录时，不把差异归因于模型本体。</p></div></details>");
    output
}
fn run_details(run: &RunResult) -> String {
    let mut values = vec![
        ("报告编号", run.id.clone()),
        (
            "检测程序版本",
            run.app_version
                .clone()
                .unwrap_or_else(|| "历史记录未保存".into()),
        ),
        ("检测规则版本", run.check_version.clone()),
        ("报告生成器版本", env!("CARGO_PKG_VERSION").into()),
        ("配置模型名", run.target.model.clone()),
        ("接口协议", run.target.protocol.label().into()),
        ("OMP 版本", run.omp_version.clone()),
        (
            "记录状态",
            if run.cancelled {
                "本次提前停止"
            } else if run.finished_at.is_some() {
                "本次记录已结束"
            } else {
                "本次记录尚未结束"
            }
            .into(),
        ),
    ];
    for spec in &run.plan {
        values.push((
            "检测设置",
            format!(
                "{}：阶段预算 {} 秒；{}",
                spec.name, spec.budget_seconds, spec.scope
            ),
        ));
    }
    values
        .into_iter()
        .map(|(key, value)| format!("<div><dt>{}</dt><dd>{}</dd></div>", html(key), html(&value)))
        .collect()
}
pub(super) fn render(run: &RunResult, zip: &[u8]) -> Result<String> {
    let cards = advice(run);
    let passed = run
        .checks
        .iter()
        .filter(|c| c.status == Status::Passed)
        .count();
    let failed = run
        .checks
        .iter()
        .filter(|c| c.status == Status::Failed)
        .count();
    let incomplete = run
        .checks
        .iter()
        .filter(|c| c.status == Status::Incomplete)
        .count();
    let not_executed = run
        .checks
        .iter()
        .filter(|c| c.status == Status::NotExecuted)
        .count();
    let attention = run.checks.iter().filter(|c| has_advice(c)).count();
    let tone = match run.verdict {
        Verdict::Passed => "passed",
        Verdict::Failed => "failed",
        Verdict::Incomplete => "incomplete",
    };
    let verdict = if run.verdict == Verdict::Passed {
        "本次接入检测通过"
    } else {
        run.verdict.title()
    };
    let reason = if run.verdict == Verdict::Passed
        && failed == 0
        && incomplete == 0
        && not_executed == 0
        && !run.checks.is_empty()
    {
        if CheckId::ALL
            .iter()
            .all(|id| run.checks.iter().any(|check| check.id == *id))
        {
            "接口、工具调用和智能体任务均满足本次检测要求，未发现阻止本次接入的问题。".into()
        } else {
            "本次选定检测均满足要求，未发现阻止本次接入的问题。".into()
        }
    } else {
        run.decision
            .as_ref()
            .map(|d| d.reason.clone())
            .unwrap_or_else(|| {
                match run.verdict {
                    Verdict::Passed => "本次结果按已保存的验收规则通过，附加观察见各项记录。",
                    Verdict::Failed => "本次存在未满足要求的记录，请先核对原因与处理建议。",
                    Verdict::Incomplete => "本次检测尚未完成或证据不足，不能据此给出完整接入结论。",
                }
                .into()
            })
    };
    let summary_suggestion = match run.verdict {
        Verdict::Passed => {
            "建议按本次已验证的接入条件试用。结论限于本次服务配置和测试范围，不作为全部业务场景或长期稳定性的保证。"
        }
        Verdict::Failed => {
            "先处理已确认未满足的要求，再重新检测。单次用例异常不等于模型在所有场景下都不可用。"
        }
        Verdict::Incomplete => {
            "补齐未完成项目或缺失证据后，再确认接入结论；不把未知结果当作通过或模型失败。"
        }
    };
    let guidance_intro = match run.verdict {
        Verdict::Passed => "没有需要整改的阻断项，按已验证的条件开始试用。",
        Verdict::Failed => "本次存在未满足要求的项目，请先核对下列原因和处理建议。",
        Verdict::Incomplete => "部分项目未确认或未执行，以下说明需要补齐的条件。",
    };
    let badge = if cards.is_empty() {
        String::new()
    } else {
        format!(
            "<span class=\"pill {}\">{} {}</span>",
            if run.verdict == Verdict::Failed {
                "failed"
            } else {
                "blue"
            },
            cards.len(),
            if run.verdict == Verdict::Passed {
                "条使用建议 · 不影响通过"
            } else {
                "条处理建议"
            }
        )
    };
    let mut counts = format!(
        "<span><strong>{passed}</strong> 项通过</span><span><strong>{failed}</strong> 项未通过</span><span><strong>{incomplete}</strong> 项未确认</span>"
    );
    if not_executed > 0 {
        counts.push_str(&format!(
            "<span><strong>{not_executed}</strong> 项未执行</span>"
        ));
    }
    let mut filters = format!(
        "<button type=\"button\" data-filter=\"all\" aria-pressed=\"true\">全部 {} 项</button><button type=\"button\" data-filter=\"advice\" aria-pressed=\"false\">含建议 {attention} 项</button><button type=\"button\" data-filter=\"failed\" aria-pressed=\"false\">未通过 {failed} 项</button>",
        run.checks.len()
    );
    if incomplete + not_executed > 0 {
        filters.push_str(&format!("<button type=\"button\" data-filter=\"unfinished\" aria-pressed=\"false\">未确认 / 未执行 {} 项</button>",incomplete+not_executed));
    }
    let version = run.app_version.as_deref().unwrap_or("未记录");
    let model = run
        .target
        .model
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(&run.target.model);
    let date = run
        .started_at
        .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
        .format("%Y-%m-%d %H:%M（UTC+8）");
    let meta = format!(
        "<span>模型 <strong>{}</strong></span><span>协议 <strong>{}</strong></span><span>时间 <strong>{date}</strong></span><span>检测版本 <strong>{}</strong></span><span>用时 <strong>{}</strong></span>",
        html(model),
        run.target.protocol.label(),
        html(version),
        duration(run.elapsed_ms)
    );
    let run_note = if run.cancelled {
        "<div class=\"run-note\">本次检测提前停止。已完成结果保留，未执行项目不算通过或失败。</div>"
            .into()
    } else if run.finished_at.is_none() {
        "<div class=\"run-note\">本次记录尚未结束，当前页面只展示已保存的阶段结果。</div>".into()
    } else if run.schema_version < 2 {
        "<div class=\"run-note\">此为历史格式记录，保留当时结论，不按新规则重新判定。</div>".into()
    } else {
        String::new()
    };
    let values = BTreeMap::from([
        ("META", meta),
        ("RUN_ID", html(&run.id)),
        ("RUN_NOTE", run_note),
        ("TONE", tone.into()),
        (
            "VERDICT_ICON",
            if run.verdict == Verdict::Passed {
                CHECK_ICON
            } else if run.verdict == Verdict::Failed {
                "!"
            } else {
                "—"
            }
            .into(),
        ),
        ("VERDICT", html(verdict)),
        ("REASON", html(&reason)),
        ("COUNTS", counts),
        ("GUIDANCE_SECTION_BADGE", badge.replace(" · 不影响通过", "")),
        ("GUIDANCE_BADGE", badge),
        ("SUMMARY_SUGGESTION", html(summary_suggestion)),
        ("CHECK_COUNT", run.checks.len().to_string()),
        ("FILTERS", filters),
        ("MODEL", html(&run.target.model)),
        ("ROWS", render_rows(run)),
        (
            "EMPTY_HIDDEN",
            if run.checks.is_empty() { "" } else { "hidden" }.into(),
        ),
        ("EMPTY_TITLE", "本次没有检测项目".into()),
        ("EMPTY_NOTE", "请核对本次检测计划和运行状态。".into()),
        ("GUIDANCE_INTRO", guidance_intro.into()),
        ("GUIDANCE", guidance(&cards, run.verdict)),
        ("PROOFS", proofs(run)),
        ("LOAD_DETAILS", load_details(run)),
        ("RUN_DETAILS", run_details(run)),
        ("ARCHIVE", script_json(&json!(STANDARD.encode(zip)))),
        (
            "ARCHIVE_NAME",
            script_json(&json!(format!("AgentCheck-{}-检测记录.zip", run.id))),
        ),
    ]);
    bind(TEMPLATE, &values)
}
