//! Coarse concurrency probe: double the load and stop at the first unsuccessful wave.
use crate::{
    dataset::parse_record_payload,
    model::{CheckId, CheckResult, FailureOrigin, Finding, Status},
    transport::{REQUEST_BUDGET, RequestFailure, Transport, messages_body},
};
use anyhow::Result;
use futures_util::future::join_all;
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Instant};

#[derive(Clone, Debug)]
pub struct ProbeOptions {
    // Resource/time guards are never interpreted as a service concurrency ceiling.
    pub max_requests: usize,
    pub max_rounds: usize,
    pub max_mean_ms: u64,
    pub max_tail_ms: u64,
}
impl Default for ProbeOptions {
    fn default() -> Self {
        Self {
            max_requests: 100_000,
            max_rounds: 32,
            max_mean_ms: 10_000,
            max_tail_ms: 20_000,
        }
    }
}
struct Latency {
    mean_ms: f64,
    p95_ms: u64,
    maximum_ms: u64,
    tail_metric: &'static str,
}
impl Latency {
    fn measure(mut samples: Vec<u64>) -> Self {
        assert!(!samples.is_empty());
        samples.sort_unstable();
        Self {
            mean_ms: samples.iter().map(|&ms| ms as f64).sum::<f64>() / samples.len() as f64,
            p95_ms: samples[(samples.len() * 95).div_ceil(100) - 1],
            maximum_ms: samples[samples.len() - 1],
            tail_metric: if samples.len() < 20 { "maximum" } else { "p95" },
        }
    }
    fn tail_ms(&self) -> u64 {
        if self.tail_metric == "maximum" {
            self.maximum_ms
        } else {
            self.p95_ms
        }
    }
    fn failure_reason(&self, options: &ProbeOptions) -> Option<&'static str> {
        if self.mean_ms > options.max_mean_ms as f64 {
            Some("mean_latency")
        } else if self.tail_ms() > options.max_tail_ms {
            Some(if self.tail_metric == "maximum" {
                "max_latency"
            } else {
                "p95_latency"
            })
        } else {
            None
        }
    }
    fn summary(&self) -> String {
        format!(
            "平均 {:.2} 秒，{} {:.2} 秒",
            self.mean_ms / 1000.0,
            if self.tail_metric == "maximum" {
                "最慢"
            } else {
                "P95"
            },
            self.tail_ms() as f64 / 1000.0
        )
    }
}
pub(crate) fn failure_label(reason: &str) -> &str {
    match reason {
        "passed" => "达标",
        "request_failure" => "请求未全部正确完成",
        "invalid_response" => "响应格式或内容错误",
        "network" => "网络错误",
        "http_error" => "服务错误",
        "evidence_conflict" => "证据关联冲突",
        "mean_latency" => "响应过慢（平均耗时超标）",
        "p95_latency" => "响应过慢（P95 耗时超标）",
        "max_latency" => "响应过慢（小样本最慢耗时超标）",
        _ => "未完成",
    }
}
fn local_connection_capacity() -> usize {
    #[cfg(unix)]
    {
        let mut limit = std::mem::MaybeUninit::<libc::rlimit>::uninit();
        // A request can consume a connection plus an evidence file; reserve descriptors for the app.
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, limit.as_mut_ptr()) } == 0 {
            return (unsafe { limit.assume_init() }.rlim_cur as usize).saturating_sub(128) / 2;
        }
    }
    usize::MAX
}
type Progress<'a> = Option<&'a (dyn Fn(String) + Send + Sync)>;
struct Search<'a> {
    transport: &'a Transport,
    deadline: Instant,
    progress: Progress<'a>,
    options: ProbeOptions,
    rounds: Vec<Value>,
    evidence: Vec<String>,
    requests: usize,
    stopped: Option<&'static str>,
}
impl Search<'_> {
    fn say(&self, text: String) {
        if let Some(progress) = self.progress {
            progress(text);
        }
    }
    async fn wave(&mut self, level: usize) -> Result<Option<bool>> {
        if self.transport.is_cancelled() {
            self.stopped = Some("cancelled");
            return Ok(None);
        }
        if level > local_connection_capacity() {
            self.stopped = Some("local_resource_limit");
            return Ok(None);
        }
        if level > self.options.max_requests.saturating_sub(self.requests) {
            self.stopped = Some("request_budget");
            return Ok(None);
        }
        if self.rounds.len() >= self.options.max_rounds {
            self.stopped = Some("round_budget");
            return Ok(None);
        }
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining < REQUEST_BUDGET {
            self.stopped = Some("stage_budget");
            return Ok(None);
        }
        self.say(format!(
            "正在测试 {level} 并发：平均 ≤ {} 秒，{} ≤ {} 秒；单请求最长等待 180 秒",
            self.options.max_mean_ms as f64 / 1000.0,
            if level < 20 { "最慢" } else { "P95" },
            self.options.max_tail_ms as f64 / 1000.0,
        ));
        let started = Instant::now();
        let responses=join_all((0..level).map(|_|async {
            let expected=json!({"probe_id":uuid::Uuid::new_v4().simple().to_string(),"ok":true});
            let prompt=format!("这是并发请求测试。请原样返回以下 JSON，不增加字段或解释：{expected}");
            let body=messages_body(self.transport.config(),json!([{"role":"user","content":prompt}]),2048,false);
            let dispatch=started.elapsed().as_millis() as u64;
            let outcome=self.transport.send_once(body,self.deadline).await?;
            let response=outcome.response.as_ref();
            let valid=outcome.failure.is_none() && response.is_some_and(|r|r.normal_end && r.calls.is_empty()
                && parse_record_payload(&r.text,true).is_ok_and(|actual|actual==expected));
            let reason=match &outcome.failure {
                Some(RequestFailure::Cancelled)=>"cancelled",Some(RequestFailure::StageBudget)=>"stage_budget",
                Some(RequestFailure::Timeout)=>"timeout",Some(RequestFailure::Http(429))=>"rate_limited",
                Some(RequestFailure::Http(401|403|404))=>"configuration",Some(RequestFailure::Http(_))=>"http_error",
                Some(RequestFailure::Network)=>"network",Some(RequestFailure::InvalidResponse(_))=>"invalid_response",
                Some(RequestFailure::EvidenceConflict(_))=>"evidence_conflict",
                None if !valid=>"invalid_response",None=>"passed",
            };
            let record=json!({"evidence_id":outcome.evidence.id,"success":valid,"reason":reason,
                "dispatch_offset_ms":dispatch,"elapsed_ms":outcome.evidence.elapsed_ms,
                "http_status":outcome.evidence.attempts.last().and_then(|a|a.http_status),
                "attempts":outcome.evidence.attempts.len(),"input_tokens":response.and_then(|r|r.usage.input),
                "completion_tokens":response.and_then(|r|r.usage.output)});
            Ok::<_,anyhow::Error>((outcome.evidence.id,record))
        })).await;
        let mut records = vec![];
        for response in responses {
            let (id, record) = response?;
            self.evidence.push(id);
            records.push(record);
        }
        self.requests += level;
        let successful = records.iter().filter(|r| r["success"] == true).count();
        let rate_limited = records
            .iter()
            .filter(|r| r["reason"] == "rate_limited")
            .count();
        let latency = Latency::measure(
            records
                .iter()
                .map(|r| r["elapsed_ms"].as_u64().unwrap())
                .collect(),
        );
        let latency_failure = latency.failure_reason(&self.options);
        for reason in ["cancelled", "stage_budget", "configuration"] {
            if records.iter().any(|r| r["reason"] == reason) {
                self.stopped = Some(reason);
                break;
            }
        }
        let failure = self.stopped.or(if successful != level {
            Some("request_failure")
        } else {
            latency_failure
        });
        let passed = failure.is_none();
        let summary = latency.summary();
        self.rounds.push(json!({"phase":"expand","concurrency":level,"successful":successful,
            "failed":level-successful,"rate_limited":rate_limited,
            "elapsed_ms":started.elapsed().as_millis() as u64,"longest_elapsed_ms":latency.maximum_ms,
            "mean_elapsed_ms":latency.mean_ms,"p95_elapsed_ms":latency.p95_ms,
            "tail_metric":latency.tail_metric,"latency_summary":summary,
            "latency_passed":latency_failure.is_none(),"performance_passed":passed,
            "failure_reason":failure.unwrap_or("passed"),"requests":records}));
        self.say(format!(
            "{level} 并发 {successful}/{level} 成功，限流 {rate_limited} 次，{summary}：{}",
            failure_label(failure.unwrap_or("passed"))
        ));
        Ok(if self.stopped.is_some() {
            None
        } else {
            Some(passed)
        })
    }
}

pub async fn probe(
    transport: &Transport,
    deadline: Instant,
    progress: Progress<'_>,
    options: ProbeOptions,
) -> Result<CheckResult> {
    let started = Instant::now();
    let mut search = Search {
        transport,
        deadline,
        progress,
        options,
        rounds: vec![],
        evidence: vec![],
        requests: 0,
        stopped: None,
    };
    let mut low = 0usize;
    let mut candidate = 1usize;
    let mut first_unsuccessful = None;
    loop {
        let Some(passed) = search.wave(candidate).await? else {
            break;
        };
        if !passed {
            first_unsuccessful = Some(candidate);
            search.stopped = Some("first_unsuccessful_wave");
            break;
        }
        low = candidate;
        candidate = match candidate.checked_mul(2) {
            Some(next) if next > candidate => next,
            _ => {
                search.stopped = Some("local_resource_limit");
                break;
            }
        };
    }
    let reason = search.stopped.unwrap_or("stage_budget");
    let rejected = search
        .rounds
        .last()
        .filter(|_| first_unsuccessful.is_some());
    let non_capacity_reason =
        rejected
            .and_then(|r| r["requests"].as_array())
            .and_then(|requests| {
                requests.iter().find_map(|r| {
                    r["reason"].as_str().filter(|reason| {
                        matches!(
                            *reason,
                            "invalid_response" | "network" | "http_error" | "evidence_conflict"
                        )
                    })
                })
            });
    let completed = first_unsuccessful.is_some() && non_capacity_reason.is_none();
    let rejected_reason = rejected.and_then(|r| r["failure_reason"].as_str());
    let rejected_label = failure_label(rejected_reason.unwrap_or("request_failure"));
    let rejected_latency = rejected
        .and_then(|r| r["latency_summary"].as_str())
        .unwrap_or("");
    let title = if let Some(cause) = non_capacity_reason {
        format!(
            "{} 并发出现{}；容量边界未确认（此前成功档位 {low}）",
            first_unsuccessful.unwrap(),
            failure_label(cause)
        )
    } else if let Some(failed) = first_unsuccessful {
        if low > 0 {
            format!("建议并发 {low}（{failed} 并发{rejected_label}）")
        } else {
            format!("单请求{rejected_label}，尚无可用并发建议")
        }
    } else if low > 0 {
        format!(
            "已验证 {low} 并发可用，探测未完成（{}）",
            stop_label(reason)
        )
    } else {
        format!("并发探测未完成（{}）", stop_label(reason))
    };
    let best = search
        .rounds
        .iter()
        .rev()
        .find(|r| r["concurrency"] == low && r["performance_passed"] == true);
    let facts = BTreeMap::from([
        ("probe_mode".into(), json!("concurrent_requests")),
        ("search_method".into(), json!("doubling_first_failure")),
        ("probe_cap".into(), Value::Null),
        ("request_timeout_seconds".into(), json!(180)),
        ("request_retries".into(), json!(0)),
        (
            "max_mean_elapsed_ms".into(),
            json!(search.options.max_mean_ms),
        ),
        (
            "max_tail_elapsed_ms".into(),
            json!(search.options.max_tail_ms),
        ),
        (
            "latency_measurement".into(),
            json!("request_to_complete_response"),
        ),
        ("p95_method".into(), json!("nearest_rank")),
        ("small_sample_tail".into(), json!("maximum_below_20")),
        ("first_unsuccessful_reason".into(), json!(rejected_reason)),
        (
            "workload".into(),
            json!(
                "短 JSON 回显；全部请求正确完成，且完整响应的平均和尾部耗时均达标；180 秒仅为单请求硬超时"
            ),
        ),
        ("verified_concurrency_lower_bound".into(), json!(low)),
        ("concurrency".into(), json!(low)),
        (
            "recommended_concurrency".into(),
            if completed && low > 0 {
                json!(low)
            } else {
                Value::Null
            },
        ),
        ("estimate_complete".into(), json!(completed)),
        (
            "capacity_boundary_unconfirmed_reason".into(),
            json!(non_capacity_reason),
        ),
        ("maximum_confirmed".into(), json!(false)),
        ("maximum_concurrency".into(), Value::Null),
        (
            "first_unsuccessful_concurrency".into(),
            json!(first_unsuccessful),
        ),
        ("stop_reason".into(), json!(reason)),
        ("request_count".into(), json!(search.requests)),
        ("request_budget".into(), json!(search.options.max_requests)),
        (
            "longest_elapsed_ms".into(),
            best.map(|r| r["longest_elapsed_ms"].clone())
                .unwrap_or(Value::Null),
        ),
        (
            "mean_elapsed_ms".into(),
            best.map(|r| r["mean_elapsed_ms"].clone())
                .unwrap_or(Value::Null),
        ),
        (
            "p95_elapsed_ms".into(),
            best.map(|r| r["p95_elapsed_ms"].clone())
                .unwrap_or(Value::Null),
        ),
        (
            "tail_metric".into(),
            best.map(|r| r["tail_metric"].clone())
                .unwrap_or(Value::Null),
        ),
        ("rounds".into(), json!(search.rounds)),
    ]);
    let status = if completed && low >= 2 {
        Status::Passed
    } else {
        Status::Incomplete
    };
    let fact = match first_unsuccessful {
        Some(failed) if non_capacity_reason.is_some() => format!(
            "{failed} 并发出现{}；本次在此停止。已验证档位 {low} 保留，但不能把内容、网络或证据问题解释为服务容量边界。",
            failure_label(non_capacity_reason.unwrap())
        ),
        Some(failed) if low > 0 => {
            format!(
                "{low} 并发全部正确完成且耗时达标（{}）；下一档 {failed} 并发{rejected_label}（{rejected_latency}），因此建议并发 {low}。",
                best.and_then(|r| r["latency_summary"].as_str())
                    .unwrap_or("")
            )
        }
        Some(_) => format!("1 并发{rejected_label}（{rejected_latency}），无法给出可用档位。"),
        None => format!(
            "保护条件触发或测试取消：{}；保留已完成档位，未给出完整估计。",
            stop_label(reason)
        ),
    };
    let finding = Finding {
        code: "concurrency_estimate".into(),
        title: title.clone(),
        fact,
        impact: format!(
            "这是短 JSON 请求在平均 ≤ {} 秒、P95 ≤ {} 秒（少于 20 个请求时用最慢耗时）标准下的粗略建议档位；180 秒仅为硬超时，不代表性能合格线，也没有测量精确极限。",
            search.options.max_mean_ms as f64 / 1000.0,
            search.options.max_tail_ms as f64 / 1000.0
        ),
        suggestion: if completed {
            "按建议档位试用；账号配额、部署或负载变化后重新检测。"
        } else {
            "先核对停止原因；本次不能据此给出完整容量建议，处理后重新检测。"
        }
        .into(),
        origin: if completed {
            FailureOrigin::Service
        } else {
            FailureOrigin::Detector
        },
        evidence_ids: search.evidence.clone(),
    };
    Ok(CheckResult {
        id: CheckId::Performance,
        finished: true,
        status,
        title,
        elapsed_ms: started.elapsed().as_millis() as u64,
        evidence_ids: search.evidence,
        facts,
        findings: vec![finding],
        tasks: vec![],
        assessment: None,
    })
}
fn stop_label(reason: &str) -> &str {
    match reason {
        "first_unsuccessful_wave" => "遇到首个未全部通过的档位",
        "cancelled" => "用户取消",
        "stage_budget" => "时间预算不足",
        "request_budget" => "请求预算已到",
        "round_budget" => "探测轮数已到",
        "local_resource_limit" => "本机连接资源不足",
        "configuration" => "配置或鉴权受阻",
        _ => "探测未完成",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_limits_are_inclusive_and_mean_can_fail_independently() {
        let options = ProbeOptions::default();
        assert_eq!(options.max_mean_ms, 10_000);
        assert_eq!(options.max_tail_ms, 20_000);
        let latency = Latency::measure(vec![0, 20_000]);
        assert_eq!(latency.mean_ms, 10_000.0);
        assert_eq!(latency.failure_reason(&options), None);
        let latency = Latency::measure(vec![10_001; 32]);
        assert_eq!(latency.failure_reason(&options), Some("mean_latency"));
    }

    #[test]
    fn p95_uses_nearest_rank_and_detects_slow_tail_despite_a_fast_mean() {
        let mut samples = vec![1_000; 30];
        samples.extend([20_001, 30_000]);
        samples.reverse();
        let latency = Latency::measure(samples);
        assert!(latency.mean_ms < 10_000.0);
        assert_eq!(latency.p95_ms, 20_001);
        assert_eq!(latency.maximum_ms, 30_000);
        assert_eq!(latency.tail_metric, "p95");
        assert_eq!(
            latency.failure_reason(&ProbeOptions::default()),
            Some("p95_latency")
        );
    }

    #[test]
    fn small_samples_use_maximum_and_twenty_samples_use_p95() {
        for count in [1, 16, 19, 20, 32] {
            let mut samples = vec![1_000; count - 1];
            samples.push(20_001);
            let latency = Latency::measure(samples);
            let options = ProbeOptions {
                max_mean_ms: 30_000,
                ..Default::default()
            };
            if count < 20 {
                assert_eq!(latency.tail_metric, "maximum");
                assert_eq!(latency.failure_reason(&options), Some("max_latency"));
            } else {
                assert_eq!(latency.p95_ms, 1_000);
                assert_eq!(latency.failure_reason(&options), None);
            }
        }
    }
}
