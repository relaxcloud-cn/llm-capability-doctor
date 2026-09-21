use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use crate::records::DetectionRecord;

pub const PERFORMANCE_VERSION: &str = "performance/v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum PerformanceCategory {
    #[serde(rename = "P01")]
    ResponseWaiting,
    #[serde(rename = "P02")]
    GenerationFluency,
    #[serde(rename = "P03")]
    Concurrency,
    #[serde(rename = "P04")]
    ContinuousStability,
    #[serde(rename = "P05")]
    LengthVariation,
}

impl PerformanceCategory {
    pub const ALL: [Self; 5] = [
        Self::ResponseWaiting,
        Self::GenerationFluency,
        Self::Concurrency,
        Self::ContinuousStability,
        Self::LengthVariation,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::ResponseWaiting => "P01",
            Self::GenerationFluency => "P02",
            Self::Concurrency => "P03",
            Self::ContinuousStability => "P04",
            Self::LengthVariation => "P05",
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::ResponseWaiting => "响应等待",
            Self::GenerationFluency => "生成速度与流畅度",
            Self::Concurrency => "并发承载",
            Self::ContinuousStability => "连续运行稳定性",
            Self::LengthVariation => "长输入与长输出性能",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadKind {
    ResponseWaiting,
    GenerationFluency,
    Concurrency,
    Continuous,
    LongInput,
    LongOutput,
    HistoryGrowth,
}

impl WorkloadKind {
    const fn category(self) -> PerformanceCategory {
        match self {
            Self::ResponseWaiting => PerformanceCategory::ResponseWaiting,
            Self::GenerationFluency => PerformanceCategory::GenerationFluency,
            Self::Concurrency => PerformanceCategory::Concurrency,
            Self::Continuous => PerformanceCategory::ContinuousStability,
            Self::LongInput | Self::LongOutput | Self::HistoryGrowth => {
                PerformanceCategory::LengthVariation
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseMode {
    Streaming,
    NonStreaming,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    Warmup,
    Formal,
    Drain,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TerminalState {
    Completed,
    Error,
    Timeout,
    Truncated,
    NaturalEnd,
    Cancelled,
    NotMeasured,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TokenCountSource {
    ServiceUsage,
    ClientCounter,
    Unavailable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Authentication,
    Permission,
    RateLimited,
    Service,
    Network,
    Client,
    Evidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PerformancePlan {
    pub category: PerformanceCategory,
    pub workload: WorkloadKind,
    pub modes: Vec<ResponseMode>,
    pub input_tokens: Vec<u32>,
    pub target_output_tokens: Vec<u32>,
    pub concurrency_targets: Vec<u32>,
    pub warmup_count: u32,
    pub formal_request_limit: u32,
    pub dispatch_window_ms: Option<u64>,
    pub drain_window_ms: Option<u64>,
    pub max_duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Timeline {
    pub send_ms: u64,
    pub first_event_ms: Option<u64>,
    pub first_reasoning_ms: Option<u64>,
    pub first_visible_ms: Option<u64>,
    pub complete_ms: Option<u64>,
    pub error_ms: Option<u64>,
    pub cancel_ms: Option<u64>,
    pub visible_events_ms: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PerformanceSample {
    pub id: String,
    pub workload: WorkloadKind,
    pub mode: ResponseMode,
    pub phase: RunPhase,
    pub input_tokens_target: u32,
    pub target_output_tokens: u32,
    pub actual_input_tokens: Option<u32>,
    pub actual_output_tokens: Option<u32>,
    pub output_chars: Option<u32>,
    pub target_concurrency: u32,
    pub actual_concurrency: u32,
    pub dispatched_at_ms: u64,
    pub terminal_at_ms: Option<u64>,
    pub timeline: Timeline,
    pub terminal_state: TerminalState,
    pub error_kind: Option<ErrorKind>,
    pub length_target_met: bool,
    pub token_count_source: TokenCountSource,
    pub evidence_refs: Vec<String>,
    pub limitation: Option<String>,
    /// 本样本所属派发窗口（闭环负载才有）；用于按维度计算窗口吞吐。
    #[serde(default)]
    pub dispatch_window_start_ms: Option<u64>,
    #[serde(default)]
    pub dispatch_window_end_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PerformanceConditions {
    pub model: String,
    pub protocol: String,
    pub client_version: String,
    pub timeout_ms: u64,
    pub input_range_max_tokens: Option<u32>,
    pub window_start_ms: u64,
    pub window_end_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PerformanceMetrics {
    pub normal_sample_count: u32,
    pub dispatched_count: u32,
    pub terminal_count: u32,
    pub error_count: u32,
    pub timeout_count: u32,
    pub truncated_count: u32,
    pub cancelled_count: u32,
    pub length_shortfall_count: u32,
    pub first_visible_p50_ms: Option<u64>,
    pub first_visible_p95_ms: Option<u64>,
    pub complete_p50_ms: Option<u64>,
    pub complete_p95_ms: Option<u64>,
    pub visible_block_p95_ms: Option<u64>,
    pub max_pause_ms: Option<u64>,
    pub token_rate: Option<Rate>,
    pub character_rate: Option<Rate>,
    pub window_throughput: Option<Rate>,
    pub completion_ratio: Option<Ratio>,
    /// 按派发时间划分的 60 秒桶，用于观察连续运行中的漂移。
    #[serde(default)]
    pub buckets: Vec<PerformanceBucket>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PerformanceBucket {
    /// 距本行测量窗口起点的偏移毫秒数。
    pub start_ms: u64,
    pub dispatched: u32,
    pub completed: u32,
    pub error_count: u32,
    pub timeout_count: u32,
    pub complete_p50_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Rate {
    pub numerator: u64,
    pub denominator_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Ratio {
    pub numerator: u32,
    pub denominator: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PerformanceRow {
    pub category: PerformanceCategory,
    pub workload: WorkloadKind,
    /// 本行的实测维度：不同模式、长度和并发档位不合并。
    #[serde(default = "default_row_mode")]
    pub mode: ResponseMode,
    #[serde(default)]
    pub input_tokens_target: u32,
    #[serde(default)]
    pub target_output_tokens: u32,
    #[serde(default)]
    pub target_concurrency: u32,
    pub conditions: PerformanceConditions,
    pub metrics: PerformanceMetrics,
    pub sample_ids: Vec<String>,
    pub evidence_refs: Vec<String>,
    pub limitations: Vec<String>,
}

fn default_row_mode() -> ResponseMode {
    ResponseMode::NonStreaming
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PerformanceReport {
    pub version: String,
    pub record_id: String,
    pub rows: Vec<PerformanceRow>,
    pub samples: Vec<PerformanceSample>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectionAction {
    Continue,
    CloseCurrentConcurrencyTier,
    StopCurrentWorkload,
}

#[derive(Debug, Default)]
pub struct ProtectionState {
    consecutive_failures: u32,
    terminal_count: u32,
    abnormal_count: u32,
}

impl ProtectionState {
    pub fn observe(&mut self, sample: &PerformanceSample) -> ProtectionAction {
        self.terminal_count += u32::from(sample.terminal_at_ms.is_some());
        // 截断与取消是协议层面的正常终态，不计入服务/网络类异常失败。
        let abnormal = matches!(
            sample.terminal_state,
            TerminalState::Error | TerminalState::Timeout
        );
        self.abnormal_count += u32::from(abnormal);
        if abnormal {
            self.consecutive_failures += 1;
        } else {
            self.consecutive_failures = 0;
        }
        if matches!(
            sample.error_kind,
            Some(ErrorKind::Authentication | ErrorKind::Permission)
        ) {
            return ProtectionAction::StopCurrentWorkload;
        }
        if sample.error_kind == Some(ErrorKind::RateLimited) {
            return ProtectionAction::CloseCurrentConcurrencyTier;
        }
        if self.consecutive_failures >= 5
            || (self.terminal_count >= 20 && self.abnormal_count * 5 >= self.terminal_count)
        {
            return ProtectionAction::CloseCurrentConcurrencyTier;
        }
        ProtectionAction::Continue
    }
}

/// 默认矩阵按 10 分钟内的运行时长设计：样本量少但每个维度仍独立成行；
/// 更慢的的服务由全局时间预算截尾，剩余维度标记为未测而非伪造数据。
pub fn fixed_performance_plan() -> Vec<PerformancePlan> {
    vec![
        PerformancePlan {
            category: PerformanceCategory::ResponseWaiting,
            workload: WorkloadKind::ResponseWaiting,
            modes: vec![ResponseMode::Streaming, ResponseMode::NonStreaming],
            input_tokens: vec![512],
            target_output_tokens: vec![256],
            concurrency_targets: vec![1],
            warmup_count: 1,
            formal_request_limit: 5,
            dispatch_window_ms: None,
            drain_window_ms: None,
            max_duration_ms: None,
        },
        PerformancePlan {
            category: PerformanceCategory::GenerationFluency,
            workload: WorkloadKind::GenerationFluency,
            modes: vec![ResponseMode::Streaming],
            input_tokens: vec![512],
            target_output_tokens: vec![512],
            concurrency_targets: vec![1],
            warmup_count: 1,
            formal_request_limit: 5,
            dispatch_window_ms: None,
            drain_window_ms: None,
            max_duration_ms: None,
        },
        PerformancePlan {
            category: PerformanceCategory::Concurrency,
            workload: WorkloadKind::Concurrency,
            modes: vec![ResponseMode::NonStreaming],
            input_tokens: vec![512],
            target_output_tokens: vec![256],
            concurrency_targets: vec![1, 4, 8],
            warmup_count: 1,
            formal_request_limit: 60,
            dispatch_window_ms: Some(20 * 1000),
            drain_window_ms: Some(15 * 1000),
            max_duration_ms: None,
        },
        PerformancePlan {
            category: PerformanceCategory::ContinuousStability,
            workload: WorkloadKind::Continuous,
            modes: vec![ResponseMode::NonStreaming],
            input_tokens: vec![512],
            target_output_tokens: vec![256],
            concurrency_targets: vec![2],
            warmup_count: 1,
            formal_request_limit: 500,
            dispatch_window_ms: Some(90 * 1000),
            drain_window_ms: Some(15 * 1000),
            max_duration_ms: None,
        },
        PerformancePlan {
            category: PerformanceCategory::LengthVariation,
            workload: WorkloadKind::LongInput,
            modes: vec![ResponseMode::NonStreaming],
            input_tokens: vec![2048, 8192, 32768],
            target_output_tokens: vec![256],
            concurrency_targets: vec![1],
            warmup_count: 1,
            formal_request_limit: 1,
            dispatch_window_ms: None,
            drain_window_ms: None,
            max_duration_ms: None,
        },
        PerformancePlan {
            category: PerformanceCategory::LengthVariation,
            workload: WorkloadKind::HistoryGrowth,
            modes: vec![ResponseMode::NonStreaming],
            input_tokens: vec![1024, 4096],
            target_output_tokens: vec![256],
            concurrency_targets: vec![1],
            warmup_count: 1,
            formal_request_limit: 1,
            dispatch_window_ms: None,
            drain_window_ms: None,
            max_duration_ms: None,
        },
        PerformancePlan {
            category: PerformanceCategory::LengthVariation,
            workload: WorkloadKind::LongOutput,
            modes: vec![ResponseMode::NonStreaming],
            input_tokens: vec![512],
            target_output_tokens: vec![1024],
            concurrency_targets: vec![1],
            warmup_count: 1,
            formal_request_limit: 2,
            dispatch_window_ms: None,
            drain_window_ms: None,
            max_duration_ms: None,
        },
    ]
}

/// 一行结果对应一个实测维度：workload × 模式 × 输入档 × 输出档 × 并发档。
type DimensionKey = (WorkloadKind, ResponseMode, u32, u32, u32);

fn dimension_key(sample: &PerformanceSample) -> DimensionKey {
    (
        sample.workload,
        sample.mode,
        sample.input_tokens_target,
        sample.target_output_tokens,
        sample.target_concurrency,
    )
}

pub fn build_report(
    record_id: &str,
    conditions: PerformanceConditions,
    samples: Vec<PerformanceSample>,
) -> Result<PerformanceReport, String> {
    validate_samples(&samples)?;
    // 按维度分组并保持样本出现顺序，不同长度/并发/模式不合并为一条速度线。
    let mut keys: Vec<DimensionKey> = Vec::new();
    let mut groups: Vec<Vec<&PerformanceSample>> = Vec::new();
    for sample in &samples {
        let key = dimension_key(sample);
        match keys.iter().position(|existing| *existing == key) {
            Some(index) => groups[index].push(sample),
            None => {
                keys.push(key);
                groups.push(vec![sample]);
            }
        }
    }
    let rows = groups
        .into_iter()
        .map(|group| build_row(conditions.clone(), group))
        .collect();
    Ok(PerformanceReport {
        version: PERFORMANCE_VERSION.into(),
        record_id: record_id.into(),
        rows,
        samples,
    })
}

pub fn build_report_for_record(
    record: &DetectionRecord,
    conditions: PerformanceConditions,
    samples: Vec<PerformanceSample>,
) -> Result<PerformanceReport, String> {
    let known_evidence = record
        .evidence
        .iter()
        .map(|evidence| evidence.id.as_str())
        .collect::<BTreeSet<_>>();
    for sample in &samples {
        for evidence_ref in &sample.evidence_refs {
            if !known_evidence.contains(evidence_ref.as_str()) {
                return Err(format!(
                    "Unknown performance evidence reference: {evidence_ref}"
                ));
            }
        }
    }
    build_report(&record.id, conditions, samples)
}

const BUCKET_MS: u64 = 60_000;

fn build_row(
    mut conditions: PerformanceConditions,
    samples: Vec<&PerformanceSample>,
) -> PerformanceRow {
    let first = samples.first().expect("dimension group is non-empty");
    let workload = first.workload;
    // 行级窗口取本维度样本携带的派发窗口；无窗口维度退回正式样本的实测时间跨度。
    let window_start = samples
        .iter()
        .filter_map(|sample| sample.dispatch_window_start_ms)
        .min()
        .or_else(|| {
            samples
                .iter()
                .filter(|sample| sample.phase == RunPhase::Formal)
                .map(|sample| sample.dispatched_at_ms)
                .min()
        })
        .unwrap_or(conditions.window_start_ms);
    let window_end = samples
        .iter()
        .filter_map(|sample| sample.dispatch_window_end_ms)
        .max()
        .or_else(|| {
            samples
                .iter()
                .filter(|sample| sample.phase == RunPhase::Formal)
                .filter_map(|sample| sample.terminal_at_ms)
                .max()
        })
        .unwrap_or(conditions.window_end_ms);
    conditions.window_start_ms = window_start;
    conditions.window_end_ms = window_end;
    let formal = samples
        .iter()
        .filter(|sample| sample.phase == RunPhase::Formal)
        .copied()
        .collect::<Vec<_>>();
    let normal = formal
        .iter()
        .filter(|sample| {
            matches!(
                sample.terminal_state,
                TerminalState::Completed | TerminalState::NaturalEnd
            )
        })
        .collect::<Vec<_>>();
    // 首响统计覆盖所有产生了可见内容的终态（含截断）：截断样本的首 token 延迟
    // 仍是真实测量，推理模型撞输出上限时不应丢掉整档首响数据。
    let measured = formal
        .iter()
        .filter(|sample| {
            matches!(
                sample.terminal_state,
                TerminalState::Completed | TerminalState::NaturalEnd | TerminalState::Truncated
            )
        })
        .collect::<Vec<_>>();
    let first_visible = measured
        .iter()
        .filter_map(|sample| {
            sample
                .timeline
                .first_visible_ms
                .map(|at| at.saturating_sub(sample.timeline.send_ms))
        })
        .collect::<Vec<_>>();
    let complete = normal
        .iter()
        .filter_map(|sample| {
            sample
                .timeline
                .complete_ms
                .map(|at| at.saturating_sub(sample.timeline.send_ms))
        })
        .collect::<Vec<_>>();
    let block_intervals = formal
        .iter()
        .flat_map(|sample| visible_intervals(&sample.timeline))
        .collect::<Vec<_>>();
    let max_pause_ms = block_intervals.iter().copied().max();
    // 速率分母：流式取首正文到完成（解码段），非流式没有可观察首响，取发送到完成（端到端）。
    let normal_token_samples = normal
        .iter()
        .filter_map(|sample| {
            let tokens = sample.actual_output_tokens? as u64;
            let start = sample
                .timeline
                .first_visible_ms
                .unwrap_or(sample.timeline.send_ms);
            let end = sample.timeline.complete_ms?;
            (end > start && sample.token_count_source != TokenCountSource::Unavailable)
                .then_some((tokens, end - start))
        })
        .collect::<Vec<_>>();
    let normal_char_samples = normal
        .iter()
        .filter_map(|sample| {
            let chars = sample.output_chars? as u64;
            let start = sample
                .timeline
                .first_visible_ms
                .unwrap_or(sample.timeline.send_ms);
            let end = sample.timeline.complete_ms?;
            (end > start).then_some((chars, end - start))
        })
        .collect::<Vec<_>>();
    let window_seconds = conditions
        .window_end_ms
        .saturating_sub(conditions.window_start_ms);
    let window_completed = formal
        .iter()
        .filter(|sample| {
            matches!(
                sample.terminal_state,
                TerminalState::Completed | TerminalState::NaturalEnd
            ) && sample.terminal_at_ms.is_some_and(|at| {
                at >= conditions.window_start_ms && at <= conditions.window_end_ms
            })
        })
        .count() as u64;
    let evidence_refs = samples
        .iter()
        .flat_map(|sample| sample.evidence_refs.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let limitations = samples
        .iter()
        .filter_map(|sample| sample.limitation.clone())
        .collect();
    let buckets = build_buckets(&formal, window_start);
    PerformanceRow {
        category: workload.category(),
        workload,
        mode: first.mode,
        input_tokens_target: first.input_tokens_target,
        target_output_tokens: first.target_output_tokens,
        target_concurrency: first.target_concurrency,
        conditions,
        metrics: PerformanceMetrics {
            normal_sample_count: normal.len() as u32,
            dispatched_count: formal.len() as u32,
            terminal_count: formal
                .iter()
                .filter(|sample| sample.terminal_at_ms.is_some())
                .count() as u32,
            error_count: formal
                .iter()
                .filter(|sample| sample.error_kind.is_some())
                .count() as u32,
            timeout_count: formal
                .iter()
                .filter(|sample| sample.terminal_state == TerminalState::Timeout)
                .count() as u32,
            truncated_count: formal
                .iter()
                .filter(|sample| sample.terminal_state == TerminalState::Truncated)
                .count() as u32,
            cancelled_count: formal
                .iter()
                .filter(|sample| sample.terminal_state == TerminalState::Cancelled)
                .count() as u32,
            length_shortfall_count: formal
                .iter()
                .filter(|sample| sample.actual_output_tokens.is_some() && !sample.length_target_met)
                .count() as u32,
            first_visible_p50_ms: percentile(first_visible.clone(), 50),
            first_visible_p95_ms: percentile(first_visible, 95),
            complete_p50_ms: percentile(complete.clone(), 50),
            complete_p95_ms: percentile(complete, 95),
            visible_block_p95_ms: percentile(block_intervals, 95),
            max_pause_ms,
            token_rate: aggregate_rate(&normal_token_samples),
            character_rate: aggregate_rate(&normal_char_samples),
            window_throughput: (window_seconds > 0).then_some(Rate {
                numerator: window_completed,
                denominator_ms: window_seconds,
            }),
            completion_ratio: Some(Ratio {
                numerator: normal.len() as u32,
                denominator: formal.len() as u32,
            }),
            buckets,
        },
        sample_ids: samples.iter().map(|sample| sample.id.clone()).collect(),
        evidence_refs,
        limitations,
    }
}

/// 按派发时间把正式样本切进 60 秒桶，供连续运行等负载观察漂移。
fn build_buckets(formal: &[&PerformanceSample], window_start: u64) -> Vec<PerformanceBucket> {
    let mut grouped: std::collections::BTreeMap<u64, Vec<&PerformanceSample>> =
        std::collections::BTreeMap::new();
    for sample in formal {
        let offset = sample.dispatched_at_ms.saturating_sub(window_start);
        grouped.entry(offset / BUCKET_MS).or_default().push(sample);
    }
    grouped
        .into_iter()
        .map(|(index, bucket_samples)| {
            let latencies = bucket_samples
                .iter()
                .filter_map(|sample| {
                    sample
                        .timeline
                        .complete_ms
                        .map(|at| at.saturating_sub(sample.timeline.send_ms))
                })
                .collect::<Vec<_>>();
            PerformanceBucket {
                start_ms: index * BUCKET_MS,
                dispatched: bucket_samples.len() as u32,
                completed: bucket_samples
                    .iter()
                    .filter(|sample| {
                        matches!(
                            sample.terminal_state,
                            TerminalState::Completed | TerminalState::NaturalEnd
                        )
                    })
                    .count() as u32,
                error_count: bucket_samples
                    .iter()
                    .filter(|sample| sample.terminal_state == TerminalState::Error)
                    .count() as u32,
                timeout_count: bucket_samples
                    .iter()
                    .filter(|sample| sample.terminal_state == TerminalState::Timeout)
                    .count() as u32,
                complete_p50_ms: percentile(latencies, 50),
            }
        })
        .collect()
}

fn validate_samples(samples: &[PerformanceSample]) -> Result<(), String> {
    let ids = samples
        .iter()
        .map(|sample| sample.id.as_str())
        .collect::<BTreeSet<_>>();
    if ids.len() != samples.len() {
        return Err("Performance samples contain duplicate IDs".into());
    }
    for sample in samples {
        if sample.actual_concurrency > sample.target_concurrency {
            return Err(format!(
                "Actual concurrency exceeds target for {}",
                sample.id
            ));
        }
        if sample.token_count_source == TokenCountSource::Unavailable
            && sample.actual_output_tokens.is_some()
        {
            return Err(format!(
                "Unavailable token source cannot carry token count: {}",
                sample.id
            ));
        }
    }
    Ok(())
}

fn visible_intervals(timeline: &Timeline) -> Vec<u64> {
    timeline
        .visible_events_ms
        .windows(2)
        .map(|window| window[1].saturating_sub(window[0]))
        .collect()
}

fn percentile(mut values: Vec<u64>, percentile: u8) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let index = ((values.len() - 1) * percentile as usize).div_ceil(100);
    values.get(index).copied()
}

fn aggregate_rate(values: &[(u64, u64)]) -> Option<Rate> {
    if values.is_empty() {
        return None;
    }
    let numerator = values.iter().map(|(count, _)| *count).sum();
    let denominator_ms = values.iter().map(|(_, duration)| *duration).sum();
    (denominator_ms > 0).then_some(Rate {
        numerator,
        denominator_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn conditions() -> PerformanceConditions {
        PerformanceConditions {
            model: "model-a".into(),
            protocol: "chat-completions".into(),
            client_version: "0.1.0".into(),
            timeout_ms: 120_000,
            input_range_max_tokens: Some(32_768),
            window_start_ms: 0,
            window_end_ms: 10_000,
        }
    }

    fn sample(id: &str, terminal_state: TerminalState) -> PerformanceSample {
        PerformanceSample {
            id: id.into(),
            workload: WorkloadKind::ResponseWaiting,
            mode: ResponseMode::Streaming,
            phase: RunPhase::Formal,
            input_tokens_target: 512,
            target_output_tokens: 256,
            actual_input_tokens: Some(512),
            actual_output_tokens: Some(100),
            output_chars: Some(400),
            target_concurrency: 1,
            actual_concurrency: 1,
            dispatched_at_ms: 0,
            terminal_at_ms: Some(500),
            timeline: Timeline {
                send_ms: 0,
                first_event_ms: Some(100),
                first_reasoning_ms: Some(150),
                first_visible_ms: Some(200),
                complete_ms: Some(500),
                error_ms: None,
                cancel_ms: None,
                visible_events_ms: vec![200, 300, 500],
            },
            terminal_state,
            error_kind: None,
            length_target_met: true,
            token_count_source: TokenCountSource::ServiceUsage,
            evidence_refs: vec![format!("evidence://{id}")],
            limitation: None,
            dispatch_window_start_ms: None,
            dispatch_window_end_ms: None,
        }
    }

    #[test]
    fn freezes_five_categories_and_confirmed_load_limits() {
        let plans = fixed_performance_plan();
        assert_eq!(PerformanceCategory::ALL.len(), 5);
        assert_eq!(
            plans[0].modes,
            vec![ResponseMode::Streaming, ResponseMode::NonStreaming]
        );
        assert_eq!(
            plans
                .iter()
                .filter(|plan| plan.category == PerformanceCategory::Concurrency)
                .count(),
            1
        );
        let concurrency = plans
            .iter()
            .find(|plan| plan.category == PerformanceCategory::Concurrency)
            .unwrap();
        assert_eq!(concurrency.concurrency_targets, vec![1, 4, 8]);
        assert_eq!(concurrency.formal_request_limit, 60);
        assert_eq!(concurrency.drain_window_ms, Some(15_000));
        let continuous = plans
            .iter()
            .find(|plan| plan.category == PerformanceCategory::ContinuousStability)
            .unwrap();
        assert_eq!(continuous.formal_request_limit, 500);
        assert_eq!(continuous.dispatch_window_ms, Some(90_000));
    }

    #[test]
    fn calculates_first_visible_and_complete_latency_without_counting_empty_events() {
        let report = build_report(
            "run-a",
            conditions(),
            vec![sample("s1", TerminalState::Completed)],
        )
        .unwrap();
        let metrics = &report.rows[0].metrics;
        assert_eq!(metrics.first_visible_p50_ms, Some(200));
        assert_eq!(metrics.complete_p50_ms, Some(500));
        assert_eq!(metrics.visible_block_p95_ms, Some(200));
        assert_eq!(metrics.max_pause_ms, Some(200));
    }

    #[test]
    fn excludes_errors_and_timeout_from_success_speed_but_keeps_them_visible() {
        let mut timeout = sample("s2", TerminalState::Timeout);
        timeout.actual_output_tokens = None;
        timeout.error_kind = Some(ErrorKind::Network);
        timeout.timeline.complete_ms = None;
        timeout.token_count_source = TokenCountSource::Unavailable;
        let report = build_report(
            "run-a",
            conditions(),
            vec![sample("s1", TerminalState::Completed), timeout],
        )
        .unwrap();
        let metrics = &report.rows[0].metrics;
        assert_eq!(metrics.normal_sample_count, 1);
        assert_eq!(metrics.error_count, 1);
        assert_eq!(metrics.timeout_count, 1);
        assert_eq!(
            metrics.completion_ratio,
            Some(Ratio {
                numerator: 1,
                denominator: 2
            })
        );
    }

    #[test]
    fn excludes_warmup_and_drain_from_formal_metrics() {
        let mut warmup = sample("warmup", TerminalState::Completed);
        warmup.phase = RunPhase::Warmup;
        warmup.timeline.first_visible_ms = Some(1);
        warmup.timeline.complete_ms = Some(2);
        let mut drain = sample("drain", TerminalState::Completed);
        drain.phase = RunPhase::Drain;
        drain.timeline.first_visible_ms = Some(3);
        drain.timeline.complete_ms = Some(4);
        let formal = sample("formal", TerminalState::Completed);
        let report = build_report("run-a", conditions(), vec![warmup, drain, formal]).unwrap();
        let metrics = &report.rows[0].metrics;
        assert_eq!(metrics.dispatched_count, 1);
        assert_eq!(metrics.normal_sample_count, 1);
        assert_eq!(
            metrics.completion_ratio,
            Some(Ratio {
                numerator: 1,
                denominator: 1
            })
        );
        assert_eq!(metrics.first_visible_p50_ms, Some(200));
    }

    #[test]
    fn does_not_publish_tokens_per_second_without_reliable_token_count() {
        let mut no_tokens = sample("s1", TerminalState::Completed);
        no_tokens.actual_output_tokens = None;
        no_tokens.token_count_source = TokenCountSource::Unavailable;
        let report = build_report("run-a", conditions(), vec![no_tokens]).unwrap();
        assert!(report.rows[0].metrics.token_rate.is_none());
        assert!(report.rows[0].metrics.character_rate.is_some());
    }

    #[test]
    fn preserves_target_and_actual_concurrency_and_length_shortfall() {
        let mut sample = sample("s1", TerminalState::NaturalEnd);
        sample.target_concurrency = 8;
        sample.actual_concurrency = 3;
        sample.length_target_met = false;
        sample.limitation = Some("目标 2048，实际自然结束 100".into());
        let report = build_report("run-a", conditions(), vec![sample]).unwrap();
        assert_eq!(report.rows[0].metrics.length_shortfall_count, 1);
        assert!(report.rows[0].limitations[0].contains("自然结束"));
        assert_eq!(report.samples[0].target_concurrency, 8);
        assert_eq!(report.samples[0].actual_concurrency, 3);
    }

    #[test]
    fn protection_stops_auth_failures_rate_limits_and_repeated_failures() {
        let mut protection = ProtectionState::default();
        let mut auth = sample("auth", TerminalState::Error);
        auth.error_kind = Some(ErrorKind::Authentication);
        assert_eq!(
            protection.observe(&auth),
            ProtectionAction::StopCurrentWorkload
        );
        let mut rate = sample("rate", TerminalState::Error);
        rate.error_kind = Some(ErrorKind::RateLimited);
        assert_eq!(
            protection.observe(&rate),
            ProtectionAction::CloseCurrentConcurrencyTier
        );

        let mut repeated = ProtectionState::default();
        for index in 0..4 {
            let mut failure = sample(&format!("f{index}"), TerminalState::Error);
            failure.error_kind = Some(ErrorKind::Service);
            assert_eq!(repeated.observe(&failure), ProtectionAction::Continue);
        }
        let mut fifth = sample("f5", TerminalState::Error);
        fifth.error_kind = Some(ErrorKind::Service);
        assert_eq!(
            repeated.observe(&fifth),
            ProtectionAction::CloseCurrentConcurrencyTier
        );
    }

    #[test]
    fn separates_rows_by_mode_length_and_concurrency_dimensions() {
        let mut other_tier = sample("s2", TerminalState::Completed);
        other_tier.target_concurrency = 8;
        other_tier.actual_concurrency = 8;
        other_tier.timeline.first_visible_ms = Some(900);
        other_tier.timeline.complete_ms = Some(2_000);
        let report = build_report(
            "run-a",
            conditions(),
            vec![sample("s1", TerminalState::Completed), other_tier],
        )
        .unwrap();
        assert_eq!(report.rows.len(), 2);
        assert_eq!(report.rows[0].target_concurrency, 1);
        assert_eq!(report.rows[1].target_concurrency, 8);
        assert_eq!(report.rows[0].metrics.complete_p50_ms, Some(500));
        assert_eq!(report.rows[1].metrics.complete_p50_ms, Some(2_000));
    }

    #[test]
    fn truncated_samples_stay_out_of_speed_distribution_but_keep_terminal_state() {
        let mut truncated = sample("s2", TerminalState::Truncated);
        truncated.timeline.complete_ms = None;
        let report = build_report(
            "run-a",
            conditions(),
            vec![sample("s1", TerminalState::Completed), truncated],
        )
        .unwrap();
        let metrics = &report.rows[0].metrics;
        assert_eq!(metrics.truncated_count, 1);
        assert_eq!(metrics.normal_sample_count, 1);
        assert_eq!(metrics.complete_p50_ms, Some(500));
    }

    #[test]
    fn truncated_samples_still_contribute_first_visible_latency() {
        let mut truncated = sample("s2", TerminalState::Truncated);
        truncated.timeline.complete_ms = None;
        truncated.timeline.first_visible_ms = Some(300);
        let report = build_report(
            "run-a",
            conditions(),
            vec![sample("s1", TerminalState::Completed), truncated],
        )
        .unwrap();
        let metrics = &report.rows[0].metrics;
        assert_eq!(metrics.first_visible_p50_ms, Some(300));
        assert_eq!(metrics.complete_p50_ms, Some(500));
    }

    #[test]
    fn non_streaming_samples_produce_end_to_end_token_rate() {
        let mut non_stream = sample("s1", TerminalState::Completed);
        non_stream.mode = ResponseMode::NonStreaming;
        non_stream.timeline.first_visible_ms = None;
        non_stream.timeline.visible_events_ms = Vec::new();
        let report = build_report("run-a", conditions(), vec![non_stream]).unwrap();
        let rate = report.rows[0]
            .metrics
            .token_rate
            .clone()
            .expect("token rate");
        assert_eq!(rate.numerator, 100);
        assert_eq!(rate.denominator_ms, 500);
    }

    #[test]
    fn formal_samples_are_bucketed_by_minute_for_drift_observation() {
        let mut later = sample("s2", TerminalState::Completed);
        later.dispatched_at_ms = 61_000;
        later.timeline.send_ms = 61_000;
        later.timeline.first_visible_ms = Some(61_200);
        later.timeline.complete_ms = Some(61_500);
        later.terminal_at_ms = Some(61_500);
        let report = build_report(
            "run-a",
            conditions(),
            vec![sample("s1", TerminalState::Completed), later],
        )
        .unwrap();
        let buckets = &report.rows[0].metrics.buckets;
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[0].start_ms, 0);
        assert_eq!(buckets[1].start_ms, 60_000);
        assert_eq!(buckets[1].complete_p50_ms, Some(500));
    }

    #[test]
    fn binds_performance_evidence_to_the_current_record_and_serializes_shared_report() {
        let record = crate::records::create_run(crate::records::CreateRunInput {
            id: "run-a".into(),
            now: "2026-09-11T00:00:00Z".into(),
            target: crate::records::ServiceSnapshotInput {
                endpoint_fingerprint: "endpoint-a".into(),
                model: "model-a".into(),
                protocol: "chat-completions".into(),
                auth_mode: "bearer".into(),
                client_version: "0.1.0".into(),
                environment: BTreeMap::new(),
            },
            selected_modules: None,
        });
        let observed = sample("s1", TerminalState::Completed);
        assert!(build_report_for_record(&record, conditions(), vec![observed]).is_err());
        let report = build_report(
            "run-a",
            conditions(),
            vec![sample("s1", TerminalState::Completed)],
        )
        .unwrap();
        let serialized = serde_json::to_string(&report).unwrap();
        let restored: PerformanceReport = serde_json::from_str(&serialized).unwrap();
        assert_eq!(restored.version, PERFORMANCE_VERSION);
        assert_eq!(restored.rows[0].metrics.first_visible_p50_ms, Some(200));
    }
}
