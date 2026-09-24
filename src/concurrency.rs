//! 并发预检：正式检测前先摸清服务能稳定承受多少并发，再按实测并发执行
//! 相互独立的样本（跑分题目等），把「我们自己制造的限流失败」从检测结论里剔除。
//!
//! 探测用最小请求（max_tokens 16）按 1→2→4→8→16 阶梯爬升：
//! 某档错误率 >20%，或 P95 延迟超过首档基线的 3 倍，即停止爬升；
//! 取最高干净档 × 0.75 作为安全系数，夹在 [1, 8]——检测工具不压测人家服务。

use crate::transport::{ChatCompletionsRequest, ChatCompletionsTransport};
use serde_json::{json, Value};
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

pub const PROBE_LADDER: [u32; 5] = [1, 2, 4, 8, 16];
pub const PROBE_CONCURRENCY_CAP: u32 = 8;
const OK_RATE_NUM: u32 = 4;
const OK_RATE_DEN: u32 = 5;
const LATENCY_DEGRADE_FACTOR: u128 = 3;

#[derive(Clone, Debug, PartialEq)]
pub struct LadderStep {
    pub concurrency: u32,
    pub sent: u32,
    pub ok: u32,
    pub p95_ms: u128,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProbeOutcome {
    pub max_clean: u32,
    pub chosen: u32,
    pub ladder: Vec<LadderStep>,
}

fn step_is_clean(step: &LadderStep, baseline_p95: Option<u128>) -> bool {
    let ok_rate = step.ok * OK_RATE_DEN >= step.sent * OK_RATE_NUM;
    let latency_ok = match baseline_p95 {
        None => true,
        Some(baseline) => step.p95_ms <= baseline.max(1) * LATENCY_DEGRADE_FACTOR,
    };
    ok_rate && latency_ok
}

/// 纯决策：给定各档实测，算出可用并发。单测直接打这里。
pub fn decide_from_ladder(ladder: &[LadderStep]) -> ProbeOutcome {
    let mut max_clean = 1;
    let mut baseline: Option<u128> = None;
    for step in ladder {
        if step.sent == 0 {
            continue;
        }
        if !step_is_clean(step, baseline) {
            break;
        }
        max_clean = step.concurrency.max(1);
        if baseline.is_none() {
            baseline = Some(step.p95_ms.max(1));
        }
    }
    let chosen = ((max_clean * 3) / 4).clamp(1, PROBE_CONCURRENCY_CAP);
    ProbeOutcome { max_clean, chosen, ladder: ladder.to_vec() }
}

/// 执行一次完整探测。传输层是克隆友好的（Arc runtime + Clone client），
/// 每档起 N 个线程各发一个最小请求。
pub fn probe(transport: &ChatCompletionsTransport) -> ProbeOutcome {
    // 探测前确认能造出并行 worker；造不出（如配置异常）就按串行结论返回。
    if transport.clone_independent().is_err() {
        return ProbeOutcome { max_clean: 1, chosen: 1, ladder: Vec::new() };
    }
    let mut measured: Vec<LadderStep> = Vec::new();
    let mut baseline: Option<u128> = None;
    for &level in PROBE_LADDER.iter() {
        let step = run_step(transport, level);
        let clean = step_is_clean(&step, baseline);
        measured.push(step);
        if !clean {
            break;
        }
        if baseline.is_none() {
            baseline = Some(measured.last().map(|s| s.p95_ms.max(1)).unwrap_or(1));
        }
    }
    decide_from_ladder(&measured)
}

fn run_step(transport: &ChatCompletionsTransport, level: u32) -> LadderStep {
    let (tx, rx) = mpsc::channel::<(bool, u128)>();
    thread::scope(|scope| {
        for _ in 0..level {
            // 每线程独立传输层：独立 runtime + 连接池，绝不共享 block_on。
            let Ok(transport) = transport.clone_independent() else { continue };
            let tx = tx.clone();
            scope.spawn(move || {
                let started = Instant::now();
                let request = ChatCompletionsRequest {
                    module_id: "preflight".into(),
                    prompt: "回复 ok".into(),
                    messages: None,
                    tools: None,
                    max_tokens: 16,
                    stream: false,
                    // 预检要的是真实成功率，不允许传输层重试掩盖限流
                    allow_retry: false,
                    timeout_ms: Some(20_000),
                };
                let response = transport.send(request);
                let ok = response.error.is_none()
                    && response.status.is_some_and(|status| (200..300).contains(&status));
                let _ = tx.send((ok, started.elapsed().as_millis()));
            });
        }
    });
    // 线程内的发送端随线程结束销毁;外层这份必须显式丢弃,
    // 否则通道永不关闭,recv 收完结果也会永久阻塞。
    drop(tx);
    let mut durations: Vec<u128> = Vec::with_capacity(level as usize);
    let mut ok = 0;
    while let Ok((success, elapsed)) = rx.recv() {
        if success {
            ok += 1;
        }
        durations.push(elapsed);
    }
    durations.sort_unstable();
    let p95 = durations
        .get(((durations.len() as f64 * 0.95).ceil() as usize).saturating_sub(1))
        .copied()
        .unwrap_or(0);
    LadderStep { concurrency: level, sent: level, ok, p95_ms: p95 }
}

/// 探测结果落为证据条目（module=preflight，不参与模块结论，仅供审计与回看）。
pub fn evidence_payload(outcome: &ProbeOutcome) -> Value {
    json!({
        "module": "preflight",
        "summary": format!(
            "并发预检：最高干净档 {}，本次按并发 {} 执行（阶梯 {}）",
            outcome.max_clean,
            outcome.chosen,
            outcome.ladder.iter().map(|s| s.concurrency.to_string()).collect::<Vec<_>>().join("→")
        ),
        "origin": "cli_orchestration",
        "payload": {
            "kind": "concurrency_preflight",
            "max_clean": outcome.max_clean,
            "chosen": outcome.chosen,
            "ladder": outcome
                .ladder
                .iter()
                .map(|step| {
                    json!({
                        "concurrency": step.concurrency,
                        "sent": step.sent,
                        "ok": step.ok,
                        "p95_ms": step.p95_ms,
                    })
                })
                .collect::<Vec<_>>(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(concurrency: u32, sent: u32, ok: u32, p95_ms: u128) -> LadderStep {
        LadderStep { concurrency, sent, ok, p95_ms }
    }

    #[test]
    fn all_clean_caps_at_eight_with_safety_factor() {
        let ladder = [
            step(1, 1, 1, 800),
            step(2, 2, 2, 850),
            step(4, 4, 4, 900),
            step(8, 8, 8, 950),
            step(16, 16, 16, 1000),
        ];
        let outcome = decide_from_ladder(&ladder);
        assert_eq!(outcome.max_clean, 16);
        assert_eq!(outcome.chosen, 8, "16×0.75=12 夹到上限 8");
    }

    #[test]
    fn stops_at_first_failure() {
        let ladder = [step(1, 1, 1, 500), step(2, 2, 2, 520), step(4, 4, 1, 600)];
        let outcome = decide_from_ladder(&ladder);
        assert_eq!(outcome.max_clean, 2);
        assert_eq!(outcome.chosen, 1, "2×0.75=1.5 取整 1");
    }

    #[test]
    fn stops_on_latency_degradation() {
        let ladder = [
            step(1, 1, 1, 400),
            step(2, 2, 2, 450),
            // P95 超过基线 3 倍（400×3=1200），即使全成功也停
            step(4, 4, 4, 3000),
        ];
        let outcome = decide_from_ladder(&ladder);
        assert_eq!(outcome.max_clean, 2);
        assert_eq!(outcome.chosen, 1);
    }

    #[test]
    fn sequential_only_service_stays_serial() {
        let ladder = [step(1, 1, 1, 900), step(2, 2, 0, 0)];
        let outcome = decide_from_ladder(&ladder);
        assert_eq!(outcome.max_clean, 1);
        assert_eq!(outcome.chosen, 1);
    }

    #[test]
    fn ok_rate_boundary_is_eighty_percent() {
        // 5 发 4 中 = 80%，算干净
        let ladder = [step(1, 1, 1, 500), step(2, 2, 2, 500), step(4, 5, 4, 500)];
        let outcome = decide_from_ladder(&ladder);
        assert_eq!(outcome.max_clean, 4);
        assert_eq!(outcome.chosen, 3);
    }
}
