use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

use crate::records::DetectionRecord;

pub const CAPABILITY_VERSION: &str = "capability/v1";
pub const CAPABILITY_LANGUAGE: &str = "zh-CN";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum CapabilityCategory {
    #[serde(rename = "C01")]
    InstructionFollowing,
    #[serde(rename = "C02")]
    InformationExtraction,
    #[serde(rename = "C03")]
    ToolSelection,
    #[serde(rename = "C04")]
    MultiTurn,
    #[serde(rename = "C05")]
    LongContext,
    #[serde(rename = "C06")]
    ReasoningAndMath,
}

impl CapabilityCategory {
    pub const ALL: [Self; 6] = [
        Self::InstructionFollowing,
        Self::InformationExtraction,
        Self::ToolSelection,
        Self::MultiTurn,
        Self::LongContext,
        Self::ReasoningAndMath,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::InstructionFollowing => "C01",
            Self::InformationExtraction => "C02",
            Self::ToolSelection => "C03",
            Self::MultiTurn => "C04",
            Self::LongContext => "C05",
            Self::ReasoningAndMath => "C06",
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::InstructionFollowing => "文本理解与指令执行",
            Self::InformationExtraction => "信息提取与结构化填写",
            Self::ToolSelection => "工具选择与参数填写",
            Self::MultiTurn => "多轮对话与条件承接",
            Self::LongContext => "长材料理解与信息利用",
            Self::ReasoningAndMath => "逻辑推理与计算",
        }
    }

    const fn subdomains(self) -> [&'static str; 5] {
        match self {
            Self::InstructionFollowing => [
                "material-facts",
                "condition-filtering",
                "single-constraint",
                "multi-constraint",
                "insufficient-information",
            ],
            Self::InformationExtraction => [
                "single-object-fields",
                "multi-object-relations",
                "field-types",
                "missing-values",
                "conflicting-information",
            ],
            Self::ToolSelection => [
                "tool-selection",
                "argument-filling",
                "argument-types",
                "multiple-tools",
                "correct-no-call",
            ],
            Self::MultiTurn => [
                "condition-retention",
                "condition-update",
                "condition-revocation",
                "object-switching",
                "history-distinction",
            ],
            Self::LongContext => [
                "localization",
                "cross-section-relation",
                "distractor-rejection",
                "length-variation",
                "in-document-calculation",
            ],
            Self::ReasoningAndMath => [
                "condition-judgement",
                "temporal-order",
                "quantity-comparison",
                "basic-calculation",
                "multi-condition-derivation",
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DatasetIdentity {
    Original,
    Adapted,
    ProductOriginal,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScoreLabel {
    Correct,
    Wrong,
    Pending,
    Incomplete,
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AcceptanceRule {
    ExactAny {
        accepted: Vec<String>,
    },
    ContainsAll {
        required: Vec<String>,
        forbidden: Vec<String>,
    },
    Numeric {
        expected: f64,
        tolerance: f64,
        unit: Option<String>,
    },
    ToolDecision {
        allowed_tool_names: Vec<String>,
        required_arguments: BTreeMap<String, String>,
        allow_no_call: bool,
    },
    NoCall {
        accepted_explanations: Vec<String>,
    },
    Pending {
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapabilitySample {
    pub id: String,
    pub category: CapabilityCategory,
    pub subdomain: String,
    pub dataset_identity: DatasetIdentity,
    pub source_ref: String,
    pub language: String,
    pub prompt: String,
    pub expected: String,
    pub acceptance: AcceptanceRule,
    pub generation_settings: BTreeMap<String, String>,
    pub revision: String,
    pub input_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCall {
    pub name: String,
    pub arguments: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ExecutionState {
    Valid,
    Invalid { reason: String },
    NotMeasured { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapabilityResponse {
    pub execution: ExecutionState,
    pub text: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapabilityObservation {
    pub sample_id: String,
    pub label: ScoreLabel,
    pub output: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub reason: Option<String>,
    pub evidence_refs: Vec<String>,
    pub executed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapabilitySettings {
    pub temperature: String,
    pub model: String,
    pub protocol: String,
    pub client_version: String,
    pub validated_input_max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilitySummary {
    pub category: CapabilityCategory,
    pub planned: u32,
    pub correct: u32,
    pub wrong: u32,
    pub pending: u32,
    pub incomplete: u32,
    pub missing: u32,
    pub valid_scored: u32,
    pub coverage: Option<Ratio>,
    pub score: Option<Ratio>,
    pub typical_errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Ratio {
    pub numerator: u32,
    pub denominator: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapabilityScorecard {
    pub version: String,
    pub record_id: String,
    pub language: String,
    pub catalog_fingerprint: String,
    pub settings: CapabilitySettings,
    pub samples: Vec<CapabilitySample>,
    pub observations: Vec<CapabilityObservation>,
    pub summaries: Vec<CapabilitySummary>,
}

pub fn fixed_capability_catalog() -> Vec<CapabilitySample> {
    let mut samples = Vec::with_capacity(240);
    for category in CapabilityCategory::ALL {
        for (subdomain_index, subdomain) in category.subdomains().into_iter().enumerate() {
            for sample_index in 1..=8 {
                let id = format!(
                    "{}-{}-{:02}",
                    category.id(),
                    subdomain_index + 1,
                    sample_index
                );
                samples.push(CapabilitySample {
                    id: id.clone(),
                    category,
                    subdomain: subdomain.into(),
                    dataset_identity: DatasetIdentity::ProductOriginal,
                    source_ref: format!("agentcheck://capability/{id}"),
                    language: CAPABILITY_LANGUAGE.into(),
                    prompt: prompt_template(category, sample_index),
                    expected: format!("样本 {id} 的预设答案"),
                    acceptance: AcceptanceRule::ExactAny {
                        accepted: vec![format!("样本 {id} 的预设答案")],
                    },
                    generation_settings: BTreeMap::from([
                        (String::from("temperature"), String::from("0")),
                        (String::from("language"), String::from(CAPABILITY_LANGUAGE)),
                    ]),
                    revision: CAPABILITY_VERSION.into(),
                    input_tokens: (category == CapabilityCategory::LongContext)
                        .then_some(1024 + sample_index * 128),
                });
            }
        }
    }
    samples
}

pub fn evaluate_response(
    sample: &CapabilitySample,
    response: CapabilityResponse,
) -> CapabilityObservation {
    let evidence_refs = response.evidence_refs.clone();
    match response.execution {
        ExecutionState::Invalid { reason } => CapabilityObservation {
            sample_id: sample.id.clone(),
            label: ScoreLabel::Incomplete,
            output: response.text,
            tool_calls: response.tool_calls,
            reason: Some(reason),
            evidence_refs,
            executed: true,
        },
        ExecutionState::NotMeasured { reason } => CapabilityObservation {
            sample_id: sample.id.clone(),
            label: ScoreLabel::Missing,
            output: None,
            tool_calls: Vec::new(),
            reason: Some(reason),
            evidence_refs,
            executed: false,
        },
        ExecutionState::Valid => {
            let label = match &sample.acceptance {
                AcceptanceRule::ExactAny { accepted } => response
                    .text
                    .as_deref()
                    .map(|text| {
                        if accepted
                            .iter()
                            .any(|value| normalize(text) == normalize(value))
                        {
                            ScoreLabel::Correct
                        } else {
                            ScoreLabel::Wrong
                        }
                    })
                    .unwrap_or(ScoreLabel::Pending),
                AcceptanceRule::ContainsAll {
                    required,
                    forbidden,
                } => response
                    .text
                    .as_deref()
                    .map(|text| {
                        let normalized = normalize(text);
                        if required
                            .iter()
                            .all(|value| normalized.contains(&normalize(value)))
                            && forbidden
                                .iter()
                                .all(|value| !normalized.contains(&normalize(value)))
                        {
                            ScoreLabel::Correct
                        } else {
                            ScoreLabel::Wrong
                        }
                    })
                    .unwrap_or(ScoreLabel::Pending),
                AcceptanceRule::Numeric {
                    expected,
                    tolerance,
                    unit,
                } => response
                    .text
                    .as_deref()
                    .and_then(first_number)
                    .map(|value| {
                        let unit_ok = unit.as_ref().is_none_or(|unit| {
                            response
                                .text
                                .as_deref()
                                .is_some_and(|text| normalize(text).contains(&normalize(unit)))
                        });
                        if (value - expected).abs() <= *tolerance && unit_ok {
                            ScoreLabel::Correct
                        } else {
                            ScoreLabel::Wrong
                        }
                    })
                    .unwrap_or(ScoreLabel::Pending),
                AcceptanceRule::ToolDecision {
                    allowed_tool_names,
                    required_arguments,
                    allow_no_call,
                } => {
                    if response.tool_calls.is_empty() {
                        if *allow_no_call {
                            ScoreLabel::Correct
                        } else {
                            ScoreLabel::Wrong
                        }
                    } else if response.tool_calls.iter().all(|call| {
                        allowed_tool_names.iter().any(|name| name == &call.name)
                            && required_arguments
                                .iter()
                                .all(|(key, value)| call.arguments.get(key) == Some(value))
                    }) {
                        ScoreLabel::Correct
                    } else {
                        ScoreLabel::Wrong
                    }
                }
                AcceptanceRule::NoCall {
                    accepted_explanations,
                } => {
                    if !response.tool_calls.is_empty() {
                        ScoreLabel::Wrong
                    } else if response.text.is_none() {
                        ScoreLabel::Pending
                    } else if accepted_explanations.is_empty()
                        || response.text.as_deref().is_some_and(|text| {
                            accepted_explanations
                                .iter()
                                .any(|value| normalize(text).contains(&normalize(value)))
                        })
                    {
                        ScoreLabel::Correct
                    } else {
                        ScoreLabel::Wrong
                    }
                }
                AcceptanceRule::Pending { .. } => ScoreLabel::Pending,
            };
            CapabilityObservation {
                sample_id: sample.id.clone(),
                label,
                output: response.text.map(|text| redact_output(&text)),
                tool_calls: response.tool_calls,
                reason: None,
                evidence_refs,
                executed: true,
            }
        }
    }
}

pub fn build_scorecard(
    record_id: &str,
    samples: Vec<CapabilitySample>,
    responses: Vec<(String, CapabilityResponse)>,
    settings: CapabilitySettings,
) -> Result<CapabilityScorecard, String> {
    validate_catalog(&samples)?;
    let known = samples
        .iter()
        .map(|sample| sample.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut response_map = BTreeMap::new();
    for (sample_id, response) in responses {
        if !known.contains(sample_id.as_str()) {
            return Err(format!("Unknown capability sample: {sample_id}"));
        }
        if response_map.insert(sample_id.clone(), response).is_some() {
            return Err(format!("Duplicate capability response: {sample_id}"));
        }
    }
    let observations = samples
        .iter()
        .map(|sample| {
            if sample.category == CapabilityCategory::LongContext
                && settings
                    .validated_input_max_tokens
                    .is_some_and(|max| sample.input_tokens.is_some_and(|tokens| tokens > max))
            {
                return CapabilityObservation {
                    sample_id: sample.id.clone(),
                    label: ScoreLabel::Missing,
                    output: None,
                    tool_calls: Vec::new(),
                    reason: Some("Input length exceeds the verified specification range".into()),
                    evidence_refs: Vec::new(),
                    executed: false,
                };
            }
            response_map
                .remove(&sample.id)
                .map(|response| evaluate_response(sample, response))
                .unwrap_or(CapabilityObservation {
                    sample_id: sample.id.clone(),
                    label: ScoreLabel::Missing,
                    output: None,
                    tool_calls: Vec::new(),
                    reason: Some("Sample was not measured".into()),
                    evidence_refs: Vec::new(),
                    executed: false,
                })
        })
        .collect::<Vec<_>>();
    let summaries = CapabilityCategory::ALL
        .into_iter()
        .map(|category| summarize(category, &samples, &observations))
        .collect();
    Ok(CapabilityScorecard {
        version: CAPABILITY_VERSION.into(),
        record_id: record_id.into(),
        language: CAPABILITY_LANGUAGE.into(),
        catalog_fingerprint: fingerprint(&samples),
        settings,
        samples,
        observations,
        summaries,
    })
}

pub fn build_scorecard_for_record(
    record: &DetectionRecord,
    samples: Vec<CapabilitySample>,
    responses: Vec<(String, CapabilityResponse)>,
    settings: CapabilitySettings,
) -> Result<CapabilityScorecard, String> {
    let known_evidence = record
        .evidence
        .iter()
        .map(|evidence| evidence.id.as_str())
        .collect::<BTreeSet<_>>();
    for (_, response) in &responses {
        for evidence_ref in &response.evidence_refs {
            if !known_evidence.contains(evidence_ref.as_str()) {
                return Err(format!(
                    "Unknown capability evidence reference: {evidence_ref}"
                ));
            }
        }
    }
    build_scorecard(&record.id, samples, responses, settings)
}

pub fn scorecard_json(scorecard: &CapabilityScorecard) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(scorecard)
}

fn validate_catalog(samples: &[CapabilitySample]) -> Result<(), String> {
    if samples.len() != 240 {
        return Err(format!(
            "Capability catalog must contain 240 units, got {}",
            samples.len()
        ));
    }
    if samples
        .iter()
        .any(|sample| sample.language != CAPABILITY_LANGUAGE)
    {
        return Err(
            "English or mixed-language samples cannot enter the Chinese denominator".into(),
        );
    }
    let ids = samples
        .iter()
        .map(|sample| sample.id.as_str())
        .collect::<BTreeSet<_>>();
    if ids.len() != samples.len() {
        return Err("Capability catalog contains duplicate sample IDs".into());
    }
    for category in CapabilityCategory::ALL {
        let count = samples
            .iter()
            .filter(|sample| sample.category == category)
            .count();
        if count != 40 {
            return Err(format!(
                "{} must contain 40 logical units, got {count}",
                category.id()
            ));
        }
    }
    Ok(())
}

fn summarize(
    category: CapabilityCategory,
    samples: &[CapabilitySample],
    observations: &[CapabilityObservation],
) -> CapabilitySummary {
    let ids = samples
        .iter()
        .filter(|sample| sample.category == category)
        .map(|sample| sample.id.as_str())
        .collect::<BTreeSet<_>>();
    let category_observations = observations
        .iter()
        .filter(|observation| ids.contains(observation.sample_id.as_str()))
        .collect::<Vec<_>>();
    let count = |label| {
        category_observations
            .iter()
            .filter(|observation| observation.label == label)
            .count() as u32
    };
    let correct = count(ScoreLabel::Correct);
    let wrong = count(ScoreLabel::Wrong);
    let valid_scored = correct + wrong;
    let typical_errors = category_observations
        .iter()
        .filter(|observation| observation.label == ScoreLabel::Wrong)
        .filter_map(|observation| {
            observation
                .reason
                .clone()
                .or_else(|| Some(observation.sample_id.clone()))
        })
        .take(3)
        .collect();
    CapabilitySummary {
        category,
        planned: ids.len() as u32,
        correct,
        wrong,
        pending: count(ScoreLabel::Pending),
        incomplete: count(ScoreLabel::Incomplete),
        missing: count(ScoreLabel::Missing),
        valid_scored,
        coverage: (valid_scored > 0).then_some(Ratio {
            numerator: valid_scored,
            denominator: ids.len() as u32,
        }),
        score: (valid_scored > 0).then_some(Ratio {
            numerator: correct,
            denominator: valid_scored,
        }),
        typical_errors,
    }
}

fn prompt_template(category: CapabilityCategory, sample_index: u32) -> String {
    match category {
        CapabilityCategory::InstructionFollowing => format!(
            "材料：样本材料 {sample_index}\n任务：按约束回答样本 {sample_index}\n只回答任务，不补充材料外事实。"
        ),
        CapabilityCategory::InformationExtraction => format!(
            "材料：记录 {sample_index}\n请按字段表填写：名称、类型、值\n缺失字段写“未提供”。"
        ),
        CapabilityCategory::ToolSelection => format!(
            "任务：完成受控工具任务 {sample_index}\n可用工具：查询、计算\n请给出工具决策和参数；题目要求不调用时明确说明。"
        ),
        CapabilityCategory::MultiTurn => format!(
            "对话历史：历史标记 {sample_index}\n当前请求：依据当前有效条件回答\n只依据当前有效条件回答。"
        ),
        CapabilityCategory::LongContext => format!(
            "材料（长度档位 {sample_index}）：固定中文材料\n问题：只依据材料回答，不使用外部知识。"
        ),
        CapabilityCategory::ReasoningAndMath => {
            format!("已知事实：固定事实 {sample_index}\n规则：固定规则\n问题：给出最终答案和单位。")
        }
    }
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<String>()
        .to_ascii_lowercase()
}

fn first_number(value: &str) -> Option<f64> {
    let mut current = String::new();
    for character in value.chars() {
        if character.is_ascii_digit() || (character == '.' && !current.contains('.')) {
            current.push(character);
        } else if !current.is_empty() {
            break;
        }
    }
    (!current.is_empty())
        .then(|| current.parse().ok())
        .flatten()
}

fn redact_output(value: &str) -> String {
    value
        .split_whitespace()
        .map(|token| {
            if token.starts_with("sk-") || token.starts_with("rk-") {
                "[REDACTED]"
            } else {
                token
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn fingerprint(samples: &[CapabilitySample]) -> String {
    let bytes = serde_json::to_vec(samples).expect("capability samples must serialize");
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(max: Option<u32>) -> CapabilitySettings {
        CapabilitySettings {
            temperature: "0".into(),
            model: "model-a".into(),
            protocol: "chat-completions".into(),
            client_version: "0.1.0".into(),
            validated_input_max_tokens: max,
        }
    }

    #[test]
    fn freezes_six_categories_and_240_chinese_units() {
        let catalog = fixed_capability_catalog();
        assert_eq!(catalog.len(), 240);
        for category in CapabilityCategory::ALL {
            assert_eq!(
                catalog
                    .iter()
                    .filter(|sample| sample.category == category)
                    .count(),
                40
            );
        }
        assert!(
            catalog
                .iter()
                .all(|sample| sample.language == CAPABILITY_LANGUAGE)
        );
    }

    #[test]
    fn accepts_semantic_aliases_numeric_tolerance_and_legal_no_call() {
        let alias = CapabilitySample {
            id: "alias".into(),
            category: CapabilityCategory::InstructionFollowing,
            subdomain: "facts".into(),
            dataset_identity: DatasetIdentity::ProductOriginal,
            source_ref: "test".into(),
            language: CAPABILITY_LANGUAGE.into(),
            prompt: "回答".into(),
            expected: "六十五元".into(),
            acceptance: AcceptanceRule::ExactAny {
                accepted: vec!["65元".into(), "人民币六十五元".into()],
            },
            generation_settings: BTreeMap::new(),
            revision: CAPABILITY_VERSION.into(),
            input_tokens: None,
        };
        let result = evaluate_response(
            &alias,
            CapabilityResponse {
                execution: ExecutionState::Valid,
                text: Some("人民币六十五元".into()),
                tool_calls: Vec::new(),
                evidence_refs: vec!["evidence://alias".into()],
            },
        );
        assert_eq!(result.label, ScoreLabel::Correct);

        let numeric = CapabilitySample {
            acceptance: AcceptanceRule::Numeric {
                expected: 65.0,
                tolerance: 0.1,
                unit: Some("元".into()),
            },
            ..alias.clone()
        };
        assert_eq!(
            evaluate_response(
                &numeric,
                CapabilityResponse {
                    execution: ExecutionState::Valid,
                    text: Some("65.05 元".into()),
                    tool_calls: Vec::new(),
                    evidence_refs: vec!["evidence://numeric".into()],
                },
            )
            .label,
            ScoreLabel::Correct
        );

        let no_call = CapabilitySample {
            acceptance: AcceptanceRule::NoCall {
                accepted_explanations: vec!["无法确定".into()],
            },
            ..alias
        };
        assert_eq!(
            evaluate_response(
                &no_call,
                CapabilityResponse {
                    execution: ExecutionState::Valid,
                    text: Some("无法确定".into()),
                    tool_calls: Vec::new(),
                    evidence_refs: vec!["evidence://no-call".into()],
                },
            )
            .label,
            ScoreLabel::Correct
        );
    }

    #[test]
    fn keeps_c_w_q_i_m_denominators_and_does_not_create_total_score() {
        let catalog = fixed_capability_catalog();
        let samples = catalog.iter().take(5).cloned().collect::<Vec<_>>();
        let mut responses = Vec::new();
        for (index, sample) in samples.iter().enumerate() {
            let response = match index {
                0 => CapabilityResponse {
                    execution: ExecutionState::Valid,
                    text: Some(sample.expected.clone()),
                    tool_calls: Vec::new(),
                    evidence_refs: vec![format!("evidence://{}", sample.id)],
                },
                1 => CapabilityResponse {
                    execution: ExecutionState::Valid,
                    text: Some("错误答案".into()),
                    tool_calls: Vec::new(),
                    evidence_refs: vec![format!("evidence://{}", sample.id)],
                },
                2 => CapabilityResponse {
                    execution: ExecutionState::Valid,
                    text: None,
                    tool_calls: Vec::new(),
                    evidence_refs: vec![format!("evidence://{}", sample.id)],
                },
                3 => CapabilityResponse {
                    execution: ExecutionState::Invalid {
                        reason: "timeout".into(),
                    },
                    text: None,
                    tool_calls: Vec::new(),
                    evidence_refs: vec![format!("evidence://{}", sample.id)],
                },
                _ => CapabilityResponse {
                    execution: ExecutionState::NotMeasured {
                        reason: "not selected".into(),
                    },
                    text: None,
                    tool_calls: Vec::new(),
                    evidence_refs: Vec::new(),
                },
            };
            responses.push((sample.id.clone(), response));
        }
        let scorecard = build_scorecard("run-a", samples, responses, settings(None));
        assert!(
            scorecard.is_err(),
            "partial test catalog must not replace the fixed denominator"
        );

        let mut responses = Vec::new();
        for (index, sample) in fixed_capability_catalog().iter().enumerate() {
            if index < 5 {
                responses.push((
                    sample.id.clone(),
                    CapabilityResponse {
                        execution: if index == 3 {
                            ExecutionState::Invalid {
                                reason: "timeout".into(),
                            }
                        } else {
                            ExecutionState::Valid
                        },
                        text: (index != 2).then(|| {
                            if index == 0 {
                                sample.expected.clone()
                            } else {
                                "错误答案".into()
                            }
                        }),
                        tool_calls: Vec::new(),
                        evidence_refs: vec![format!("evidence://{}", sample.id)],
                    },
                ));
            }
        }
        let scorecard = build_scorecard(
            "run-a",
            fixed_capability_catalog(),
            responses,
            settings(None),
        )
        .unwrap();
        let summary = &scorecard.summaries[0];
        assert_eq!(summary.planned, 40);
        assert_eq!(
            summary.correct
                + summary.wrong
                + summary.pending
                + summary.incomplete
                + summary.missing,
            40
        );
        assert_eq!(summary.valid_scored, summary.correct + summary.wrong);
        assert!(
            scorecard
                .summaries
                .iter()
                .all(|summary| summary.score.is_none()
                    || summary.category == CapabilityCategory::InstructionFollowing)
        );
    }

    #[test]
    fn does_not_execute_long_material_beyond_verified_input_range() {
        let catalog = fixed_capability_catalog();
        let scorecard =
            build_scorecard("run-a", catalog, Vec::new(), settings(Some(1024))).unwrap();
        let long_context = &scorecard.summaries[4];
        assert_eq!(long_context.valid_scored, 0);
        assert_eq!(long_context.missing, 40);
        assert!(long_context.score.is_none());
        assert!(
            scorecard
                .observations
                .iter()
                .filter(|observation| observation
                    .reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("verified specification range")))
                .count()
                > 0
        );
    }

    #[test]
    fn preserves_dataset_identity_version_evidence_and_serialization() {
        let catalog = fixed_capability_catalog();
        let scorecard = build_scorecard("run-a", catalog, Vec::new(), settings(None)).unwrap();
        assert!(
            scorecard
                .samples
                .iter()
                .all(|sample| sample.dataset_identity == DatasetIdentity::ProductOriginal)
        );
        assert_eq!(scorecard.version, CAPABILITY_VERSION);
        let json = scorecard_json(&scorecard).unwrap();
        assert!(json.contains("catalog_fingerprint"));
        assert!(json.contains("C01"));
    }

    #[test]
    fn rejects_evidence_not_owned_by_the_current_record() {
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
        let sample = fixed_capability_catalog()[0].clone();
        assert!(
            build_scorecard_for_record(
                &record,
                fixed_capability_catalog(),
                vec![(
                    sample.id,
                    CapabilityResponse {
                        execution: ExecutionState::Valid,
                        text: Some(sample.expected),
                        tool_calls: Vec::new(),
                        evidence_refs: vec!["evidence://other-run".into()],
                    },
                ),],
                settings(None),
            )
            .is_err()
        );
    }
}
