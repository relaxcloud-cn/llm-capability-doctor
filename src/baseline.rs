use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

use crate::records::DetectionRecord;

pub const BASELINE_VERSION: &str = "chat-completions-baseline/v1";
pub const BASELINE_SOURCE_URL: &str = "https://developers.openai.com/api/reference/chat";
pub const BASELINE_DESIGN_VERSION: &str = "design/issue-20-chat-completions-baseline/v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum BaselineScenario {
    #[serde(rename = "BC01")]
    ResponseEnvelope,
    #[serde(rename = "BC02")]
    ChoiceContainer,
    #[serde(rename = "BC03")]
    ResponseMessage,
    #[serde(rename = "BC04")]
    UsageSummary,
    #[serde(rename = "BC05")]
    UsageDetails,
    #[serde(rename = "BC06")]
    ServiceMetadata,
    #[serde(rename = "BC07")]
    ToolCallContainer,
    #[serde(rename = "BC08")]
    FunctionArguments,
    #[serde(rename = "BC09")]
    StreamEnvelope,
    #[serde(rename = "BC10")]
    StreamChoiceContainer,
    #[serde(rename = "BC11")]
    StreamDelta,
    #[serde(rename = "BC12")]
    StreamToolDelta,
    #[serde(rename = "BC13")]
    StreamUsage,
    #[serde(rename = "BC14")]
    ErrorEnvelope,
}

impl BaselineScenario {
    pub const ALL: [Self; 14] = [
        Self::ResponseEnvelope,
        Self::ChoiceContainer,
        Self::ResponseMessage,
        Self::UsageSummary,
        Self::UsageDetails,
        Self::ServiceMetadata,
        Self::ToolCallContainer,
        Self::FunctionArguments,
        Self::StreamEnvelope,
        Self::StreamChoiceContainer,
        Self::StreamDelta,
        Self::StreamToolDelta,
        Self::StreamUsage,
        Self::ErrorEnvelope,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::ResponseEnvelope => "BC01",
            Self::ChoiceContainer => "BC02",
            Self::ResponseMessage => "BC03",
            Self::UsageSummary => "BC04",
            Self::UsageDetails => "BC05",
            Self::ServiceMetadata => "BC06",
            Self::ToolCallContainer => "BC07",
            Self::FunctionArguments => "BC08",
            Self::StreamEnvelope => "BC09",
            Self::StreamChoiceContainer => "BC10",
            Self::StreamDelta => "BC11",
            Self::StreamToolDelta => "BC12",
            Self::StreamUsage => "BC13",
            Self::ErrorEnvelope => "BC14",
        }
    }

    pub const fn title(self) -> &'static str {
        match self {
            Self::ResponseEnvelope => "响应外层",
            Self::ChoiceContainer => "候选结果容器",
            Self::ResponseMessage => "回复消息",
            Self::UsageSummary => "用量汇总",
            Self::UsageDetails => "用量明细",
            Self::ServiceMetadata => "服务附加信息",
            Self::ToolCallContainer => "函数调用容器",
            Self::FunctionArguments => "函数名称与参数承载",
            Self::StreamEnvelope => "分块外层",
            Self::StreamChoiceContainer => "分块候选容器",
            Self::StreamDelta => "消息增量",
            Self::StreamToolDelta => "函数调用增量",
            Self::StreamUsage => "流式用量返回",
            Self::ErrorEnvelope => "JSON 错误对象",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseMode {
    NonStreaming,
    Streaming,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StreamPhase {
    Initial,
    Content,
    Tool,
    Terminal,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FieldState {
    Required,
    Optional,
    Nullable,
    Variant,
    Forbidden,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ValueType {
    Object,
    Array,
    String,
    Number,
    Boolean,
    Null,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StructuralStatus {
    SameStructure,
    Different,
    NotObserved,
    Inconclusive,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DifferenceKind {
    MissingRequiredField,
    ForbiddenField,
    TypeMismatch,
    ShapeMismatch,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineFieldRule {
    pub path: String,
    pub state: FieldState,
    pub value_type: ValueType,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BaselineVariant {
    pub scenario: BaselineScenario,
    pub title: String,
    pub mode: ResponseMode,
    pub phase: Option<StreamPhase>,
    pub applicability: String,
    pub reference: Value,
    pub fields: Vec<BaselineFieldRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineProvenance {
    pub source_url: String,
    pub snapshot_version: String,
    pub design_version: String,
    pub retrieved_at: String,
    pub scenario: BaselineScenario,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BaselineCatalog {
    pub version: String,
    pub source_url: String,
    pub retrieved_at: String,
    pub design_version: String,
    pub scope: String,
    pub variants: Vec<BaselineVariant>,
    pub catalog_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActualSnapshot {
    pub raw: String,
    pub parsed: Option<Value>,
    pub source: ActualSource,
    pub phase: Option<StreamPhase>,
    pub truncated: bool,
    pub parse_error: Option<String>,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActualSource {
    RealService,
    ControlledFixture,
}

impl ActualSnapshot {
    pub fn from_raw(
        raw: impl Into<String>,
        source: ActualSource,
        phase: Option<StreamPhase>,
        truncated: bool,
        evidence_refs: Vec<String>,
    ) -> Self {
        let raw = raw.into();
        match serde_json::from_str::<Value>(&raw) {
            Ok(parsed) => Self {
                raw,
                parsed: Some(parsed),
                source,
                phase,
                truncated,
                parse_error: None,
                evidence_refs,
            },
            Err(error) => Self {
                raw,
                parsed: None,
                source,
                phase,
                truncated,
                parse_error: Some(error.to_string()),
                evidence_refs,
            },
        }
    }

    pub fn from_value(
        value: Value,
        source: ActualSource,
        phase: Option<StreamPhase>,
        evidence_refs: Vec<String>,
    ) -> Self {
        Self {
            raw: serde_json::to_string(&value).expect("JSON values must serialize"),
            parsed: Some(value),
            source,
            phase,
            truncated: false,
            parse_error: None,
            evidence_refs,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuralDifference {
    pub path: String,
    pub actual_path: Option<String>,
    pub reference_path: Option<String>,
    pub kind: DifferenceKind,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineComparison {
    pub scenario: BaselineScenario,
    pub title: String,
    pub mode: ResponseMode,
    pub phase: Option<StreamPhase>,
    pub applicability: String,
    pub reference: BaselineProvenance,
    pub actual_source: ActualSource,
    pub status: StructuralStatus,
    pub differences: Vec<StructuralDifference>,
    pub notes: Vec<String>,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BaselineObservation {
    pub scenario: BaselineScenario,
    pub actual: ActualSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BaselineReport {
    pub version: String,
    pub record_id: String,
    pub catalog: BaselineCatalog,
    pub comparisons: Vec<BaselineComparison>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct SelectionContext {
    pub mode: ResponseMode,
    pub tool_call: bool,
    pub has_usage: bool,
    pub has_usage_details: bool,
    pub has_service_metadata: bool,
    pub phase: Option<StreamPhase>,
}

pub fn fixed_baseline_catalog() -> BaselineCatalog {
    let variants = BaselineScenario::ALL
        .into_iter()
        .map(build_variant)
        .collect::<Vec<_>>();
    let fingerprint = digest_json(&variants);
    BaselineCatalog {
        version: BASELINE_VERSION.into(),
        source_url: BASELINE_SOURCE_URL.into(),
        retrieved_at: "2026-09-11".into(),
        design_version: BASELINE_DESIGN_VERSION.into(),
        scope: "Chat Completions 首批 BC01-BC14；不包含 Responses、SDK、业务正确性或整体可用性"
            .into(),
        variants,
        catalog_fingerprint: fingerprint,
    }
}

pub fn select_variants(
    catalog: &BaselineCatalog,
    context: SelectionContext,
) -> Vec<BaselineScenario> {
    if context.mode == ResponseMode::Error {
        return vec![BaselineScenario::ErrorEnvelope];
    }
    let mut selected = if context.mode == ResponseMode::Streaming {
        vec![
            BaselineScenario::StreamEnvelope,
            BaselineScenario::StreamChoiceContainer,
            BaselineScenario::StreamDelta,
        ]
    } else {
        vec![
            BaselineScenario::ResponseEnvelope,
            BaselineScenario::ChoiceContainer,
            BaselineScenario::ResponseMessage,
        ]
    };
    if context.mode == ResponseMode::Streaming && context.tool_call {
        selected.push(BaselineScenario::StreamToolDelta);
    } else if context.mode == ResponseMode::NonStreaming && context.tool_call {
        selected.extend([
            BaselineScenario::ToolCallContainer,
            BaselineScenario::FunctionArguments,
        ]);
    }
    if context.has_usage {
        selected.push(if context.mode == ResponseMode::Streaming {
            BaselineScenario::StreamUsage
        } else {
            BaselineScenario::UsageSummary
        });
    }
    if context.mode == ResponseMode::NonStreaming && context.has_usage_details {
        selected.push(BaselineScenario::UsageDetails);
    }
    if context.mode == ResponseMode::NonStreaming && context.has_service_metadata {
        selected.push(BaselineScenario::ServiceMetadata);
    }
    selected
        .into_iter()
        .filter(|scenario| {
            catalog
                .variants
                .iter()
                .any(|variant| variant.scenario == *scenario)
        })
        .collect()
}

pub fn compare_snapshot(variant: &BaselineVariant, actual: &ActualSnapshot) -> BaselineComparison {
    let reference = BaselineProvenance {
        source_url: BASELINE_SOURCE_URL.into(),
        snapshot_version: BASELINE_VERSION.into(),
        design_version: BASELINE_DESIGN_VERSION.into(),
        retrieved_at: "2026-09-11".into(),
        scenario: variant.scenario,
    };
    let (status, differences, notes) = match (&actual.parsed, actual.truncated) {
        (_, true) => (
            StructuralStatus::Inconclusive,
            Vec::new(),
            vec!["响应被截断，未使用部分文本拼接结构".into()],
        ),
        (None, false) => (
            StructuralStatus::Inconclusive,
            Vec::new(),
            vec!["响应不是可解析 JSON，保留原始片段但不判为结构差异".into()],
        ),
        (Some(actual), false) => {
            let mut context = CompareContext {
                rules: &variant.fields,
                differences: Vec::new(),
                notes: Vec::new(),
                not_observed: false,
            };
            compare_value(&variant.reference, actual, "", &mut context);
            let status = if !context.differences.is_empty() {
                StructuralStatus::Different
            } else if context.not_observed {
                StructuralStatus::NotObserved
            } else {
                StructuralStatus::SameStructure
            };
            (status, context.differences, context.notes)
        }
    };
    BaselineComparison {
        scenario: variant.scenario,
        title: variant.title.clone(),
        mode: variant.mode,
        phase: variant.phase,
        applicability: variant.applicability.clone(),
        reference,
        actual_source: actual.source,
        status,
        differences,
        notes,
        evidence_refs: actual.evidence_refs.clone(),
    }
}

pub fn build_report_for_record(
    record: &DetectionRecord,
    catalog: BaselineCatalog,
    observations: Vec<BaselineObservation>,
) -> Result<BaselineReport, String> {
    let known_evidence = record
        .evidence
        .iter()
        .map(|item| item.id.as_str())
        .collect::<BTreeSet<_>>();
    let variants = catalog
        .variants
        .iter()
        .map(|variant| (variant.scenario, variant))
        .collect::<BTreeMap<_, _>>();
    let mut comparisons = Vec::with_capacity(observations.len());
    for observation in observations {
        let variant = variants
            .get(&observation.scenario)
            .ok_or_else(|| format!("Unknown baseline scenario: {}", observation.scenario.id()))?;
        for evidence_ref in &observation.actual.evidence_refs {
            if !known_evidence.contains(evidence_ref.as_str()) {
                return Err(format!(
                    "Unknown baseline evidence reference: {evidence_ref}"
                ));
            }
        }
        comparisons.push(compare_snapshot(variant, &observation.actual));
    }
    comparisons.sort_by_key(|comparison| comparison.scenario);
    Ok(BaselineReport {
        version: BASELINE_VERSION.into(),
        record_id: record.id.clone(),
        catalog,
        comparisons,
        limitations: vec![
            "结构差异不等于协议合规、任务成功、Agent 可用或客户服务整体不可用。".into(),
            "动态值、对象键顺序、合法数组数量/顺序及函数 arguments 字符串内部业务内容不参与结构比较。".into(),
            "没有可解析 JSON、响应截断或缺少适用样本时保持 inconclusive/not_observed。".into(),
        ],
    })
}

pub fn report_json(report: &BaselineReport) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(report)
}

struct CompareContext<'a> {
    rules: &'a [BaselineFieldRule],
    differences: Vec<StructuralDifference>,
    notes: Vec<String>,
    not_observed: bool,
}

fn compare_value(reference: &Value, actual: &Value, path: &str, context: &mut CompareContext<'_>) {
    match (reference, actual) {
        (Value::Object(reference_object), Value::Object(actual_object)) => {
            for (key, reference_child) in reference_object {
                let child_path = join_path(path, key);
                match actual_object.get(key) {
                    Some(actual_child) => {
                        compare_value(reference_child, actual_child, &child_path, context)
                    }
                    None => match rule_for(context.rules, &child_path).map(|rule| rule.state) {
                        Some(FieldState::Optional | FieldState::Variant) => {
                            context.notes.push(format!("可选字段未返回：{child_path}"))
                        }
                        Some(FieldState::Forbidden) => {}
                        _ => context.differences.push(StructuralDifference {
                            path: child_path.clone(),
                            actual_path: None,
                            reference_path: Some(child_path),
                            kind: DifferenceKind::MissingRequiredField,
                            detail: "参考目录要求字段存在".into(),
                        }),
                    },
                }
            }
            for (key, actual_child) in actual_object {
                if reference_object.contains_key(key) {
                    continue;
                }
                let child_path = join_path(path, key);
                match rule_for(context.rules, &child_path).map(|rule| rule.state) {
                    Some(FieldState::Forbidden) => context.differences.push(StructuralDifference {
                        path: child_path.clone(),
                        actual_path: Some(child_path),
                        reference_path: None,
                        kind: DifferenceKind::ForbiddenField,
                        detail: "当前变体禁止该字段".into(),
                    }),
                    _ => {
                        context.notes.push(format!(
                            "未收录字段未作为结构红线：{child_path}（类型 {:?}）",
                            value_type(actual_child)
                        ));
                    }
                }
            }
        }
        (Value::Array(reference_items), Value::Array(actual_items)) => {
            if reference_items.is_empty() {
                if actual_items.is_empty() {
                    context.not_observed = true;
                    context
                        .notes
                        .push(format!("数组没有可观察元素 schema：{path}"));
                } else {
                    context.not_observed = true;
                    context
                        .notes
                        .push(format!("参考目录未声明数组元素 schema：{path}"));
                }
                return;
            }
            if actual_items.is_empty() {
                return;
            }
            for actual_item in actual_items {
                compare_value(
                    &reference_items[0],
                    actual_item,
                    &format!("{path}[]"),
                    context,
                );
            }
        }
        (Value::Null, Value::Null) => {}
        (Value::Null, actual) => {
            if !allows_null(context.rules, path) {
                context.differences.push(StructuralDifference {
                    path: path.into(),
                    actual_path: Some(path.into()),
                    reference_path: Some(path.into()),
                    kind: DifferenceKind::TypeMismatch,
                    detail: format!("实际类型 {:?} 不符合可空规则", value_type(actual)),
                });
            }
        }
        (reference, Value::Null) => {
            if !allows_null(context.rules, path) {
                context.differences.push(StructuralDifference {
                    path: path.into(),
                    actual_path: Some(path.into()),
                    reference_path: Some(path.into()),
                    kind: DifferenceKind::TypeMismatch,
                    detail: format!("实际 null 不符合参考类型 {:?}", value_type(reference)),
                });
            }
        }
        (reference, actual) if value_type(reference) != value_type(actual) => {
            context.differences.push(StructuralDifference {
                path: path.into(),
                actual_path: Some(path.into()),
                reference_path: Some(path.into()),
                kind: DifferenceKind::TypeMismatch,
                detail: format!(
                    "实际类型 {:?} 不符合参考类型 {:?}",
                    value_type(actual),
                    value_type(reference)
                ),
            });
        }
        _ => {}
    }
}

fn build_variant(scenario: BaselineScenario) -> BaselineVariant {
    match scenario {
        BaselineScenario::ResponseEnvelope => variant(
            scenario,
            ResponseMode::NonStreaming,
            None,
            "非流式普通文本或工具响应",
            json!({"object":"chat.completion","id":"id","created":0,"model":"model","choices":[{"index":0,"message":{"role":"assistant","content":"text","refusal":null},"finish_reason":"stop","logprobs":null}],"usage":{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0}}),
            vec![
                required("object", ValueType::String),
                required("id", ValueType::String),
                required("created", ValueType::Number),
                required("model", ValueType::String),
                required("choices", ValueType::Array),
                required("choices[]", ValueType::Object),
                required("choices[].index", ValueType::Number),
                required("choices[].message", ValueType::Object),
                required_nullable("choices[].message.content", ValueType::String),
                optional_nullable("choices[].message.refusal", ValueType::String),
                required("choices[].message.role", ValueType::String),
                required("choices[].finish_reason", ValueType::String),
                optional("choices[].logprobs", ValueType::Object),
                optional("usage", ValueType::Object),
                optional("usage.prompt_tokens", ValueType::Number),
                optional("usage.completion_tokens", ValueType::Number),
                optional("usage.total_tokens", ValueType::Number),
            ],
        ),
        BaselineScenario::ChoiceContainer => variant(
            scenario,
            ResponseMode::NonStreaming,
            None,
            "BC01 的 choices[] 容器",
            json!({"choices":[{"index":0,"message":{"role":"assistant","content":"text"},"finish_reason":"stop","logprobs":null}]}),
            vec![
                required("choices", ValueType::Array),
                required("choices[]", ValueType::Object),
                required("choices[].index", ValueType::Number),
                required("choices[].message", ValueType::Object),
                required("choices[].finish_reason", ValueType::String),
                optional("choices[].logprobs", ValueType::Object),
            ],
        ),
        BaselineScenario::ResponseMessage => variant(
            scenario,
            ResponseMode::NonStreaming,
            None,
            "普通文本或工具消息分支",
            json!({"choices":[{"message":{"role":"assistant","content":"text","refusal":null,"tool_calls":[],"function_call":null}}]}),
            vec![
                required("choices", ValueType::Array),
                required("choices[]", ValueType::Object),
                required("choices[].message", ValueType::Object),
                required("choices[].message.role", ValueType::String),
                optional_nullable("choices[].message.content", ValueType::String),
                optional_nullable("choices[].message.refusal", ValueType::String),
                optional("choices[].message.tool_calls", ValueType::Array),
                optional("choices[].message.function_call", ValueType::Object),
            ],
        ),
        BaselineScenario::UsageSummary => variant(
            scenario,
            ResponseMode::NonStreaming,
            None,
            "响应带 usage 的非流式请求",
            json!({"usage":{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0}}),
            vec![
                required("usage", ValueType::Object),
                required("usage.prompt_tokens", ValueType::Number),
                required("usage.completion_tokens", ValueType::Number),
                required("usage.total_tokens", ValueType::Number),
            ],
        ),
        BaselineScenario::UsageDetails => variant(
            scenario,
            ResponseMode::NonStreaming,
            None,
            "usage 带明细的响应",
            json!({"usage":{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"prompt_tokens_details":{"cached_tokens":0},"completion_tokens_details":{"reasoning_tokens":0}}}),
            vec![
                required("usage", ValueType::Object),
                required("usage.prompt_tokens", ValueType::Number),
                required("usage.completion_tokens", ValueType::Number),
                required("usage.total_tokens", ValueType::Number),
                optional("usage.prompt_tokens_details", ValueType::Object),
                optional(
                    "usage.prompt_tokens_details.cached_tokens",
                    ValueType::Number,
                ),
                optional("usage.completion_tokens_details", ValueType::Object),
                optional(
                    "usage.completion_tokens_details.reasoning_tokens",
                    ValueType::Number,
                ),
            ],
        ),
        BaselineScenario::ServiceMetadata => variant(
            scenario,
            ResponseMode::NonStreaming,
            None,
            "服务返回附加字段的响应",
            json!({"system_fingerprint":"fp","service_tier":"default","request_id":"request","metadata":{}}),
            vec![
                optional("system_fingerprint", ValueType::String),
                optional("service_tier", ValueType::String),
                optional("request_id", ValueType::String),
                optional("metadata", ValueType::Object),
            ],
        ),
        BaselineScenario::ToolCallContainer => variant(
            scenario,
            ResponseMode::NonStreaming,
            None,
            "非流式响应包含工具调用",
            json!({"choices":[{"message":{"tool_calls":[{"id":"call","type":"function","function":{"name":"lookup","arguments":"{}"}}]}}]}),
            vec![
                required("choices", ValueType::Array),
                required("choices[]", ValueType::Object),
                required("choices[].message", ValueType::Object),
                required("choices[].message.tool_calls", ValueType::Array),
                required("choices[].message.tool_calls[]", ValueType::Object),
                required("choices[].message.tool_calls[].id", ValueType::String),
                required("choices[].message.tool_calls[].type", ValueType::String),
                required("choices[].message.tool_calls[].function", ValueType::Object),
            ],
        ),
        BaselineScenario::FunctionArguments => variant(
            scenario,
            ResponseMode::NonStreaming,
            None,
            "工具 function 对象",
            json!({"choices":[{"message":{"tool_calls":[{"function":{"name":"lookup","arguments":"{\"query\":\"business\"}"}}]}}]}),
            vec![
                required("choices", ValueType::Array),
                required("choices[]", ValueType::Object),
                required("choices[].message", ValueType::Object),
                required("choices[].message.tool_calls", ValueType::Array),
                required("choices[].message.tool_calls[]", ValueType::Object),
                required("choices[].message.tool_calls[].function", ValueType::Object),
                required(
                    "choices[].message.tool_calls[].function.name",
                    ValueType::String,
                ),
                required(
                    "choices[].message.tool_calls[].function.arguments",
                    ValueType::String,
                ),
            ],
        ),
        BaselineScenario::StreamEnvelope => variant(
            scenario,
            ResponseMode::Streaming,
            Some(StreamPhase::Initial),
            "stream=true 的每个 chunk",
            json!({"object":"chat.completion.chunk","id":"id","created":0,"model":"model","choices":[{"index":0,"delta":{},"finish_reason":null}],"usage":null}),
            vec![
                required("object", ValueType::String),
                required("id", ValueType::String),
                required("created", ValueType::Number),
                required("model", ValueType::String),
                required("choices", ValueType::Array),
                required("choices[]", ValueType::Object),
                required("choices[].index", ValueType::Number),
                required("choices[].delta", ValueType::Object),
                optional_nullable("choices[].finish_reason", ValueType::String),
                optional_nullable("usage", ValueType::Object),
            ],
        ),
        BaselineScenario::StreamChoiceContainer => variant(
            scenario,
            ResponseMode::Streaming,
            Some(StreamPhase::Content),
            "chunk 的候选数组",
            json!({"choices":[{"index":0,"delta":{"content":"text"},"finish_reason":null,"logprobs":null}]}),
            vec![
                required("choices", ValueType::Array),
                required("choices[]", ValueType::Object),
                required("choices[].index", ValueType::Number),
                required("choices[].delta", ValueType::Object),
                optional_nullable("choices[].finish_reason", ValueType::String),
                optional("choices[].logprobs", ValueType::Object),
            ],
        ),
        BaselineScenario::StreamDelta => variant(
            scenario,
            ResponseMode::Streaming,
            Some(StreamPhase::Content),
            "文本、角色或拒答增量",
            json!({"choices":[{"delta":{"role":"assistant","content":"text","refusal":null}}]}),
            vec![
                required("choices", ValueType::Array),
                required("choices[]", ValueType::Object),
                required("choices[].delta", ValueType::Object),
                optional("choices[].delta.role", ValueType::String),
                optional_nullable("choices[].delta.content", ValueType::String),
                optional_nullable("choices[].delta.refusal", ValueType::String),
            ],
        ),
        BaselineScenario::StreamToolDelta => variant(
            scenario,
            ResponseMode::Streaming,
            Some(StreamPhase::Tool),
            "工具调用增量事件",
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call","type":"function","function":{"name":"lookup","arguments":"{}"}}]}}]}),
            vec![
                required("choices", ValueType::Array),
                required("choices[]", ValueType::Object),
                required("choices[].delta", ValueType::Object),
                required("choices[].delta.tool_calls", ValueType::Array),
                required("choices[].delta.tool_calls[]", ValueType::Object),
                required("choices[].delta.tool_calls[].index", ValueType::Number),
                optional("choices[].delta.tool_calls[].id", ValueType::String),
                optional("choices[].delta.tool_calls[].type", ValueType::String),
                required("choices[].delta.tool_calls[].function", ValueType::Object),
                optional(
                    "choices[].delta.tool_calls[].function.name",
                    ValueType::String,
                ),
                optional(
                    "choices[].delta.tool_calls[].function.arguments",
                    ValueType::String,
                ),
            ],
        ),
        BaselineScenario::StreamUsage => variant(
            scenario,
            ResponseMode::Streaming,
            Some(StreamPhase::Terminal),
            "终止或约定 usage chunk",
            json!({"usage":{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0,"prompt_tokens_details":{"cached_tokens":0},"completion_tokens_details":{"reasoning_tokens":0}}}),
            vec![
                required("usage", ValueType::Object),
                required("usage.prompt_tokens", ValueType::Number),
                required("usage.completion_tokens", ValueType::Number),
                required("usage.total_tokens", ValueType::Number),
                optional("usage.prompt_tokens_details", ValueType::Object),
                optional(
                    "usage.prompt_tokens_details.cached_tokens",
                    ValueType::Number,
                ),
                optional("usage.completion_tokens_details", ValueType::Object),
                optional(
                    "usage.completion_tokens_details.reasoning_tokens",
                    ValueType::Number,
                ),
            ],
        ),
        BaselineScenario::ErrorEnvelope => variant(
            scenario,
            ResponseMode::Error,
            None,
            "HTTP 错误且 body 可解析为 JSON",
            json!({"error":{"message":"message","type":"invalid_request_error","param":null,"code":null}}),
            vec![
                required("error", ValueType::Object),
                required("error.message", ValueType::String),
                required("error.type", ValueType::String),
                optional_nullable("error.param", ValueType::String),
                optional_nullable("error.code", ValueType::String),
            ],
        ),
    }
}

fn variant(
    scenario: BaselineScenario,
    mode: ResponseMode,
    phase: Option<StreamPhase>,
    applicability: &str,
    reference: Value,
    fields: Vec<BaselineFieldRule>,
) -> BaselineVariant {
    BaselineVariant {
        scenario,
        title: scenario.title().into(),
        mode,
        phase,
        applicability: applicability.into(),
        reference,
        fields,
    }
}

fn required(path: &str, value_type: ValueType) -> BaselineFieldRule {
    rule(path, FieldState::Required, value_type)
}

fn required_nullable(path: &str, value_type: ValueType) -> BaselineFieldRule {
    rule(path, FieldState::Nullable, value_type)
}

fn optional(path: &str, value_type: ValueType) -> BaselineFieldRule {
    rule(path, FieldState::Optional, value_type)
}

fn optional_nullable(path: &str, value_type: ValueType) -> BaselineFieldRule {
    rule(path, FieldState::Optional, value_type)
}

fn rule(path: &str, state: FieldState, value_type: ValueType) -> BaselineFieldRule {
    BaselineFieldRule {
        path: path.into(),
        state,
        value_type,
    }
}

fn rule_for<'a>(rules: &'a [BaselineFieldRule], path: &str) -> Option<&'a BaselineFieldRule> {
    rules.iter().find(|rule| rule.path == path)
}

fn allows_null(rules: &[BaselineFieldRule], path: &str) -> bool {
    rule_for(rules, path)
        .is_some_and(|rule| matches!(rule.state, FieldState::Nullable | FieldState::Optional))
}

fn join_path(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        key.into()
    } else {
        format!("{parent}.{key}")
    }
}

fn value_type(value: &Value) -> ValueType {
    match value {
        Value::Object(_) => ValueType::Object,
        Value::Array(_) => ValueType::Array,
        Value::String(_) => ValueType::String,
        Value::Number(_) => ValueType::Number,
        Value::Bool(_) => ValueType::Boolean,
        Value::Null => ValueType::Null,
    }
}

fn digest_json<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).expect("baseline values must serialize");
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::{CreateRunInput, ServiceSnapshotInput, add_evidence, create_run};
    use std::collections::BTreeMap;

    fn record() -> crate::records::DetectionRecord {
        create_run(CreateRunInput {
            id: "run-baseline".into(),
            now: "2026-09-11T00:00:00Z".into(),
            target: ServiceSnapshotInput {
                endpoint_fingerprint: "api.example.test".into(),
                model: "model-a".into(),
                protocol: "chat-completions".into(),
                auth_mode: "bearer".into(),
                client_version: "test".into(),
                environment: BTreeMap::new(),
            },
            selected_modules: None,
        })
    }

    fn variant(scenario: BaselineScenario) -> BaselineVariant {
        fixed_baseline_catalog()
            .variants
            .into_iter()
            .find(|variant| variant.scenario == scenario)
            .unwrap()
    }

    #[test]
    fn freezes_fourteen_variants_and_official_snapshot_provenance() {
        let catalog = fixed_baseline_catalog();
        assert_eq!(catalog.variants.len(), 14);
        assert_eq!(catalog.source_url, BASELINE_SOURCE_URL);
        assert_eq!(catalog.version, BASELINE_VERSION);
        assert!(catalog.catalog_fingerprint.len() == 64);
        assert!(catalog.scope.contains("Chat Completions"));
    }

    #[test]
    fn selects_stream_tool_usage_and_error_variants_without_merging_phases() {
        let catalog = fixed_baseline_catalog();
        assert_eq!(
            select_variants(
                &catalog,
                SelectionContext {
                    mode: ResponseMode::Streaming,
                    tool_call: true,
                    has_usage: true,
                    has_usage_details: false,
                    has_service_metadata: false,
                    phase: Some(StreamPhase::Terminal),
                }
            ),
            vec![
                BaselineScenario::StreamEnvelope,
                BaselineScenario::StreamChoiceContainer,
                BaselineScenario::StreamDelta,
                BaselineScenario::StreamToolDelta,
                BaselineScenario::StreamUsage,
            ]
        );
        assert_eq!(
            select_variants(
                &catalog,
                SelectionContext {
                    mode: ResponseMode::Error,
                    tool_call: false,
                    has_usage: false,
                    has_usage_details: false,
                    has_service_metadata: false,
                    phase: None,
                }
            ),
            vec![BaselineScenario::ErrorEnvelope]
        );
    }

    #[test]
    fn ignores_dynamic_values_order_and_business_arguments() {
        let base = variant(BaselineScenario::FunctionArguments);
        let actual = ActualSnapshot::from_value(
            json!({"choices":[{"message":{"tool_calls":[{"function":{"name":"other","arguments":"{\"business_key\":\"changed\"}"}}]}}]}),
            ActualSource::RealService,
            None,
            Vec::new(),
        );
        let comparison = compare_snapshot(&base, &actual);
        assert_eq!(comparison.status, StructuralStatus::SameStructure);
        let envelope = variant(BaselineScenario::ResponseEnvelope);
        let reordered = ActualSnapshot::from_value(
            json!({"usage":{"total_tokens":8,"completion_tokens":5,"prompt_tokens":3},"choices":[{"index":0,"finish_reason":"length","message":{"content":"different","role":"assistant"}}],"model":"model","created":9,"id":"id","object":"chat.completion"}),
            ActualSource::RealService,
            None,
            Vec::new(),
        );
        let comparison = compare_snapshot(&envelope, &reordered);
        assert_eq!(comparison.status, StructuralStatus::SameStructure);
    }

    #[test]
    fn reports_field_nested_and_scalar_type_changes_without_claiming_usability() {
        let base = variant(BaselineScenario::ResponseEnvelope);
        let actual = ActualSnapshot::from_value(
            json!({"object":"chat.completion","id":"id","created":"wrong","model":"model","choices":[{"index":0,"message":{"role":"assistant","text":"wrong"},"finish_reason":"stop"}]}),
            ActualSource::RealService,
            None,
            Vec::new(),
        );
        let comparison = compare_snapshot(&base, &actual);
        assert_eq!(comparison.status, StructuralStatus::Different);
        assert!(
            comparison
                .differences
                .iter()
                .any(|difference| difference.path == "created")
        );
        assert!(
            comparison
                .differences
                .iter()
                .any(|difference| difference.path == "choices[].message.content")
        );
        assert!(!comparison.notes.iter().any(|note| note.contains("可用")));
    }

    #[test]
    fn optional_fields_and_empty_arrays_follow_catalog_rules() {
        let base = variant(BaselineScenario::ResponseMessage);
        let actual = ActualSnapshot::from_value(
            json!({"choices":[{"message":{"role":"assistant"}}]}),
            ActualSource::ControlledFixture,
            None,
            Vec::new(),
        );
        assert_eq!(
            compare_snapshot(&base, &actual).status,
            StructuralStatus::SameStructure
        );
        let envelope = variant(BaselineScenario::ResponseEnvelope);
        let empty = ActualSnapshot::from_value(
            json!({"object":"chat.completion","id":"id","created":0,"model":"model","choices":[]}),
            ActualSource::ControlledFixture,
            None,
            Vec::new(),
        );
        assert_eq!(
            compare_snapshot(&envelope, &empty).status,
            StructuralStatus::SameStructure
        );
    }

    #[test]
    fn distinguishes_unknown_json_from_structure_difference() {
        let base = variant(BaselineScenario::ChoiceContainer);
        let actual = ActualSnapshot::from_raw(
            "{\"choices\":",
            ActualSource::RealService,
            None,
            false,
            Vec::new(),
        );
        assert_eq!(
            compare_snapshot(&base, &actual).status,
            StructuralStatus::Inconclusive
        );
        let empty_schema = BaselineVariant {
            scenario: BaselineScenario::ResponseMessage,
            title: "无元素 schema 的数组".into(),
            mode: ResponseMode::NonStreaming,
            phase: None,
            applicability: "测试边界".into(),
            reference: json!({"items": []}),
            fields: vec![required("items", ValueType::Array)],
        };
        let actual = ActualSnapshot::from_value(
            json!({"items": []}),
            ActualSource::RealService,
            None,
            Vec::new(),
        );
        assert_eq!(
            compare_snapshot(&empty_schema, &actual).status,
            StructuralStatus::NotObserved
        );
    }

    #[test]
    fn binds_actual_evidence_and_serializes_left_actual_right_reference_report() {
        let mut current = record();
        add_evidence(
            &mut current,
            "baseline-log",
            "response",
            "2026-09-11T00:01:00Z",
            json!({"scenario":"BC01"}),
        );
        let observation = BaselineObservation {
            scenario: BaselineScenario::ResponseEnvelope,
            actual: ActualSnapshot::from_value(
                json!({"object":"chat.completion","id":"id","created":0,"model":"model","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}),
                ActualSource::RealService,
                None,
                vec!["baseline-log".into()],
            ),
        };
        let report =
            build_report_for_record(&current, fixed_baseline_catalog(), vec![observation]).unwrap();
        assert_eq!(
            report.comparisons[0].actual_source,
            ActualSource::RealService
        );
        assert_eq!(
            report.comparisons[0].reference.scenario,
            BaselineScenario::ResponseEnvelope
        );
        let serialized = report_json(&report).unwrap();
        let restored: BaselineReport = serde_json::from_str(&serialized).unwrap();
        assert_eq!(restored.record_id, "run-baseline");
        let unbound = BaselineObservation {
            scenario: BaselineScenario::ResponseEnvelope,
            actual: ActualSnapshot::from_value(
                json!({"object":"chat.completion"}),
                ActualSource::RealService,
                None,
                vec!["evidence://other-run".into()],
            ),
        };
        assert!(
            build_report_for_record(&current, fixed_baseline_catalog(), vec![unbound]).is_err()
        );
    }
}
