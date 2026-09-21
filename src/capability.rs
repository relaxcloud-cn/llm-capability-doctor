use serde::{Deserialize, Serialize};
use serde_json::Value;
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
pub enum OutputConstraint {
    /// 归一化后回答字符数 ≤ n。
    MaxChars(u32),
    /// 回答须包含子串（归一化匹配）。
    MustContain(String),
    /// 回答不得包含子串。
    MustNotContain(String),
    /// 关键词出现次数 ≥ n。
    KeywordMinCount(String, u32),
    /// 全回答为可解析 JSON 对象。
    JsonObject,
    /// JSON 顶层字段数 = n。
    JsonFieldCount(u32),
    /// JSON 必含指定字段名。
    JsonRequiredFields(Vec<String>),
    /// 非空行数 = n。
    LineCount(u32),
    /// 空行分段数 = n。
    ParagraphCount(u32),
    /// 含 [[...]] 形式标题。
    WrappedTitle,
    /// 回答以指定串开头。
    StartsWith(String),
    /// 回答以指定串结尾。
    EndsWith(String),
    /// 不含指定字符。
    NoChar(char),
    /// 首尾为配对引号。
    QuotedOutput,
    /// 给定项按顺序出现。
    ExactOrder(Vec<String>),
}

impl OutputConstraint {
    pub fn describe(&self) -> String {
        match self {
            Self::MaxChars(n) => format!("不超过{n}字"),
            Self::MustContain(s) => format!("必须包含“{s}”"),
            Self::MustNotContain(s) => format!("不得包含“{s}”"),
            Self::KeywordMinCount(s, n) => format!("“{s}”至少出现{n}次"),
            Self::JsonObject => "整体为可解析JSON对象".into(),
            Self::JsonFieldCount(n) => format!("JSON顶层字段数为{n}"),
            Self::JsonRequiredFields(fields) => format!("JSON必含字段{}", fields.join(",")),
            Self::LineCount(n) => format!("恰好{n}行"),
            Self::ParagraphCount(n) => format!("恰好{n}段"),
            Self::WrappedTitle => "含[[标题]]包裹".into(),
            Self::StartsWith(s) => format!("以“{s}”开头"),
            Self::EndsWith(s) => format!("以“{s}”结尾"),
            Self::NoChar(c) => format!("不含字符“{c}”"),
            Self::QuotedOutput => "首尾为配对引号".into(),
            Self::ExactOrder(items) => format!("按序出现{}", items.join("→")),
        }
    }
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
    /// 任一等义表达命中即过，且不得含禁含值。
    ContainsAny {
        any_of: Vec<String>,
        forbidden: Vec<String>,
    },
    /// 每组别名至少命中一个，且不得含禁含值（集合型答案）。
    MatchGroups {
        required_groups: Vec<Vec<String>>,
        forbidden: Vec<String>,
    },
    /// 内容规则 + 输出约束清单，逐约束独立判定（IFEval 式）。
    ConstraintSet {
        content: Box<AcceptanceRule>,
        constraints: Vec<OutputConstraint>,
    },
    /// 多次工具调用集合匹配（BFCL 式）：每条预期调用须在响应中出现，
    /// 函数名一致且预期参数为实际参数子集；实际不得多出未预期调用。
    ToolCallsMatch {
        expected_calls: Vec<ToolCall>,
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

/// 多轮样本中的一条消息（C04 用，按真实 messages 数组发送）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SampleMessage {
    pub role: String,
    pub content: String,
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
    /// 多轮样本的完整消息历史；存在时优先于 prompt 发送。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messages: Option<Vec<SampleMessage>>,
    /// 工具选择样本的真实 tools 定义（BFCL 式 JSON Schema）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Value>>,
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
    #[serde(default)]
    pub truncated: bool,
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
    let mut samples = Vec::with_capacity(144);
    for category in CapabilityCategory::ALL {
        let bank = match category {
            CapabilityCategory::InstructionFollowing => c01_bank(),
            CapabilityCategory::InformationExtraction => c02_bank(),
            CapabilityCategory::ToolSelection => c03_bank(),
            CapabilityCategory::MultiTurn => c04_bank(),
            CapabilityCategory::LongContext => c05_bank(),
            CapabilityCategory::ReasoningAndMath => c06_bank(),
        };
        samples.extend(bank);
    }
    samples
}

/// 通用样本构造：题库函数只关心题目内容，其余字段统一补齐。
fn sample(
    category: CapabilityCategory,
    subdomain_index: u32,
    sample_index: u32,
    subdomain: &str,
    prompt: String,
    expected: &str,
    acceptance: AcceptanceRule,
    messages: Option<Vec<SampleMessage>>,
    tools: Option<Vec<Value>>,
    input_tokens: Option<u32>,
) -> CapabilitySample {
    let id = format!("{}-{}-{:02}", category.id(), subdomain_index, sample_index);
    CapabilitySample {
        id: id.clone(),
        category,
        subdomain: subdomain.into(),
        dataset_identity: DatasetIdentity::ProductOriginal,
        source_ref: format!("agentcheck://capability/{id}"),
        language: CAPABILITY_LANGUAGE.into(),
        prompt,
        messages,
        tools,
        expected: expected.into(),
        acceptance,
        generation_settings: BTreeMap::from([
            (String::from("temperature"), String::from("0")),
            (String::from("language"), String::from(CAPABILITY_LANGUAGE)),
        ]),
        revision: CAPABILITY_VERSION.into(),
        input_tokens,
    }
}

/// C01 文本理解与指令执行题库（44 题，方法借鉴 SQuAD 2.0 与 IFEval，
/// 题目全部为产品原创中文受控材料）。子域序与 `subdomains()` 一致：
/// 1 材料事实 2 条件筛选 3 单项约束 4 多项约束 5 信息不足。
fn any(aliases: &[&str], forbidden: &[&str]) -> AcceptanceRule {
    AcceptanceRule::ContainsAny {
        any_of: aliases.iter().map(|value| value.to_string()).collect(),
        forbidden: forbidden.iter().map(|value| value.to_string()).collect(),
    }
}

fn groups(required: &[&[&str]], forbidden: &[&str]) -> AcceptanceRule {
    AcceptanceRule::MatchGroups {
        required_groups: required
            .iter()
            .map(|group| group.iter().map(|value| value.to_string()).collect())
            .collect(),
        forbidden: forbidden.iter().map(|value| value.to_string()).collect(),
    }
}

fn set(content: AcceptanceRule, constraints: Vec<OutputConstraint>) -> AcceptanceRule {
    AcceptanceRule::ConstraintSet {
        content: Box::new(content),
        constraints,
    }
}

fn c01_bank() -> Vec<CapabilitySample> {
    const UNIFIED: &str = "只依据题目给出的材料和条件回答。先给最终答案；同时满足题目列出的所有输出要求。材料没有提供的信息必须明确说明未知，不要使用外部知识补全。";
    const FACTS_AB: &str =
        "材料：项目甲由小林负责，预算30万，状态进行中；项目乙由小周负责，预算50万，状态已完成。";
    const FACTS_ABCD: &str = "材料：项目甲预算30万、华东、进行中；项目乙预算50万、华北、已完成；项目丙预算20万、华东、进行中；项目丁预算45万、华东、已完成。";
    const FACTS_SC: &str = "材料：项目甲预算30万、华东、进行中，负责人小林；项目乙预算50万、华北、已完成，负责人小周。";

    use OutputConstraint as C;

    let defs: Vec<(usize, u32, &str, String, &str, AcceptanceRule)> = vec![
        // ---- 子域 1：材料事实（8 题）----
        (
            1,
            1,
            "material-facts",
            format!("{FACTS_AB}任务：项目乙的预算是多少？{UNIFIED}"),
            "50万",
            any(&["50万", "500000", "五十万"], &[]),
        ),
        (
            1,
            2,
            "material-facts",
            format!("{FACTS_AB}任务：项目乙的验收日期是哪天？{UNIFIED}"),
            "未提供",
            any(&["未提供", "未说明", "未知", "没有提供", "无此信息"], &[]),
        ),
        (
            1,
            3,
            "material-facts",
            format!(
                "材料：产品A负责人王五，产品B负责人王六，产品C负责人王五。任务：产品B的负责人是谁？{UNIFIED}"
            ),
            "王六",
            any(&["王六"], &["王五"]),
        ),
        (
            1,
            4,
            "material-facts",
            format!(
                "材料：项目甲原预算30万，3月调整为45万；项目负责人始终为小林。任务：项目甲当前预算是多少？{UNIFIED}"
            ),
            "45万",
            any(&["45万", "四十五万"], &["30万"]),
        ),
        (
            1,
            5,
            "material-facts",
            format!(
                "材料：本批共三个项目：项目甲、项目乙、项目丙。任务：本批共有几个项目？{UNIFIED}"
            ),
            "3个",
            any(&["3个", "三个", "共3", "共三"], &[]),
        ),
        (
            1,
            6,
            "material-facts",
            format!("材料：库存现有零件12件，另有4件在途未到货。任务：现有库存多少件？{UNIFIED}"),
            "12件",
            any(&["12件", "12"], &["16"]),
        ),
        (
            1,
            7,
            "material-facts",
            format!(
                "材料：季度评审会定于3月15日召开，地点暂定二层会议室。任务：评审会定在哪天？{UNIFIED}"
            ),
            "3月15日",
            any(&["3月15日", "三月十五"], &[]),
        ),
        (
            1,
            8,
            "material-facts",
            format!(
                "材料：项目丁已完成立项，负责人待任命，预算待评审。任务：项目丁的负责人是谁？{UNIFIED}"
            ),
            "未提供",
            any(&["待任命", "未提供", "未知", "未确定", "尚未"], &[]),
        ),
        // ---- 子域 2：条件筛选（8 题，共用 ABCD 材料）----
        (
            2,
            1,
            "condition-filtering",
            format!("{FACTS_ABCD}任务：列出预算超过40万的项目，按金额从高到低排列。{UNIFIED}"),
            "项目乙、项目丁",
            set(
                groups(&[&["项目乙", "乙"], &["项目丁", "丁"]], &["甲", "丙"]),
                vec![C::ExactOrder(vec!["乙".into(), "丁".into()])],
            ),
        ),
        (
            2,
            2,
            "condition-filtering",
            format!("{FACTS_ABCD}任务：列出状态为已完成的项目。{UNIFIED}"),
            "项目乙、项目丁",
            groups(&[&["乙"], &["丁"]], &["甲", "丙", "进行中"]),
        ),
        (
            2,
            3,
            "condition-filtering",
            format!("{FACTS_ABCD}任务：列出华东地区且状态为进行中的项目。{UNIFIED}"),
            "项目甲、项目丙",
            groups(&[&["甲"], &["丙"]], &["乙", "丁", "已完成"]),
        ),
        (
            2,
            4,
            "condition-filtering",
            format!("{FACTS_ABCD}任务：列出预算超过100万的项目。{UNIFIED}"),
            "无",
            any(
                &["无", "没有", "不存在", "空"],
                &["项目甲", "项目乙", "项目丙", "项目丁"],
            ),
        ),
        (
            2,
            5,
            "condition-filtering",
            format!("{FACTS_ABCD}任务：列出除项目乙外状态为已完成的项目。{UNIFIED}"),
            "项目丁",
            groups(&[&["丁"]], &["甲", "乙", "丙", "进行中"]),
        ),
        (
            2,
            6,
            "condition-filtering",
            format!("{FACTS_ABCD}任务：有几个项目未完成？列出名称。{UNIFIED}"),
            "2个：项目甲、项目丙",
            groups(&[&["2", "两"], &["甲"], &["丙"]], &["乙", "丁", "已完成"]),
        ),
        (
            2,
            7,
            "condition-filtering",
            format!("{FACTS_ABCD}任务：列出预算在20万到40万之间（含边界）的项目。{UNIFIED}"),
            "项目甲、项目丙",
            groups(&[&["甲"], &["丙"]], &["乙", "丁"]),
        ),
        (
            2,
            8,
            "condition-filtering",
            format!("{FACTS_ABCD}任务：列出华东项目，按预算从低到高排列。{UNIFIED}"),
            "项目丙、项目甲、项目丁",
            set(
                groups(&[&["丙"], &["甲"], &["丁"]], &["乙"]),
                vec![C::ExactOrder(vec!["丙".into(), "甲".into(), "丁".into()])],
            ),
        ),
        // ---- 子域 3：单项约束（12 题，共用 SC 材料，每题一条约束覆盖 12 种类型）----
        (
            3,
            1,
            "single-constraint",
            format!("{FACTS_SC}任务：总结项目甲当前状态，回答不超过10个字。{UNIFIED}"),
            "进行中",
            set(any(&["进行中"], &[]), vec![C::MaxChars(10)]),
        ),
        (
            3,
            2,
            "single-constraint",
            format!(
                "{FACTS_SC}任务：说明项目推进中需要注意的问题，回答必须包含“风险”一词。{UNIFIED}"
            ),
            "含“风险”",
            set(any(&["风险"], &[]), vec![C::MustContain("风险".into())]),
        ),
        (
            3,
            3,
            "single-constraint",
            format!("{FACTS_SC}任务：介绍项目甲的基本情况，回答不得包含“预算”一词。{UNIFIED}"),
            "提到项目甲事实",
            set(
                any(&["小林", "进行中", "华东"], &[]),
                vec![C::MustNotContain("预算".into())],
            ),
        ),
        (
            3,
            4,
            "single-constraint",
            format!("{FACTS_SC}任务：描述项目的安全管理要求，“安全”一词至少出现2次。{UNIFIED}"),
            "含2次“安全”",
            set(
                any(&["安全"], &[]),
                vec![C::KeywordMinCount("安全".into(), 2)],
            ),
        ),
        (
            3,
            5,
            "single-constraint",
            format!(
                "{FACTS_SC}任务：以JSON对象输出项目甲的负责人和预算，不要输出JSON以外的文字。{UNIFIED}"
            ),
            "小林、30万",
            set(
                groups(&[&["小林"], &["30万", "30"]], &[]),
                vec![C::JsonObject],
            ),
        ),
        (
            3,
            6,
            "single-constraint",
            format!("{FACTS_SC}任务：列出材料中的两个项目，每行一个。{UNIFIED}"),
            "项目甲、项目乙",
            set(groups(&[&["甲"], &["乙"]], &[]), vec![C::LineCount(2)]),
        ),
        (
            3,
            7,
            "single-constraint",
            format!("{FACTS_SC}任务：分别概括两个项目的状态，用两个自然段回答。{UNIFIED}"),
            "进行中、已完成",
            set(
                groups(
                    &[
                        &["进行中", "进行", "推进中"],
                        &["已完成", "已经完成", "全部完成"],
                    ],
                    &[],
                ),
                vec![C::ParagraphCount(2)],
            ),
        ),
        (
            3,
            8,
            "single-constraint",
            format!("{FACTS_SC}任务：为项目甲的进展汇报拟一个标题，标题用[[]]包裹。{UNIFIED}"),
            "含标题",
            set(any(&["甲", "进展", "汇报"], &[]), vec![C::WrappedTitle]),
        ),
        (
            3,
            9,
            "single-constraint",
            format!("{FACTS_SC}任务：回答项目甲由谁负责，回答必须以“结论：”开头。{UNIFIED}"),
            "小林",
            set(any(&["小林"], &[]), vec![C::StartsWith("结论：".into())]),
        ),
        (
            3,
            10,
            "single-constraint",
            format!("{FACTS_SC}任务：说明项目乙的完成状态，回答必须以“完毕”结尾。{UNIFIED}"),
            "已完成",
            set(any(&["已完成"], &[]), vec![C::EndsWith("完毕".into())]),
        ),
        (
            3,
            11,
            "single-constraint",
            format!("{FACTS_SC}任务：介绍两个项目的预算，回答中不得使用逗号。{UNIFIED}"),
            "30万、50万",
            set(
                groups(&[&["30万", "30"], &["50万", "50"]], &[]),
                vec![C::NoChar('，')],
            ),
        ),
        (
            3,
            12,
            "single-constraint",
            format!("{FACTS_SC}任务：给出项目甲的负责人，整个回答用中文引号包裹。{UNIFIED}"),
            "小林",
            set(any(&["小林"], &[]), vec![C::QuotedOutput]),
        ),
        // ---- 子域 4：多项约束（8 题，每题 2-4 条）----
        (
            4,
            1,
            "multi-constraint",
            format!(
                "{FACTS_SC}任务：总结项目推进的风险，回答不超过15字且必须包含“风险”。{UNIFIED}"
            ),
            "含“风险”且≤15字",
            set(
                any(&["风险"], &[]),
                vec![C::MustContain("风险".into()), C::MaxChars(15)],
            ),
        ),
        (
            4,
            2,
            "multi-constraint",
            format!(
                "{FACTS_SC}任务：以JSON对象输出项目乙的负责人和状态，恰好2个字段，不得输出JSON以外的文字。{UNIFIED}"
            ),
            "小周、已完成",
            set(
                groups(&[&["小周"], &["已完成"]], &[]),
                vec![C::JsonObject, C::JsonFieldCount(2)],
            ),
        ),
        (
            4,
            3,
            "multi-constraint",
            format!(
                "{FACTS_SC}任务：列出预算超过40万的项目，每行一个，回答中不得包含“甲”字。{UNIFIED}"
            ),
            "项目乙",
            set(
                groups(&[&["乙"]], &["甲"]),
                vec![C::LineCount(1), C::MustNotContain("甲".into())],
            ),
        ),
        (
            4,
            4,
            "multi-constraint",
            format!(
                "{FACTS_SC}任务：为项目汇报拟标题并给出结论，标题用[[]]包裹，整段回答不超过30字。{UNIFIED}"
            ),
            "含标题且≤30字",
            set(
                any(&["项目", "汇报"], &[]),
                vec![C::WrappedTitle, C::MaxChars(30)],
            ),
        ),
        (
            4,
            5,
            "multi-constraint",
            format!(
                "{FACTS_SC}任务：说明项目甲的情况，回答不得使用逗号，必须以句号结尾。{UNIFIED}"
            ),
            "提到项目甲事实",
            set(
                any(&["进行中", "小林", "华东", "30万"], &[]),
                vec![C::NoChar('，'), C::EndsWith("。".into())],
            ),
        ),
        (
            4,
            6,
            "multi-constraint",
            format!(
                "{FACTS_SC}任务：提示项目资金风险，回答不超过8字且必须包含“预算不足”。{UNIFIED}"
            ),
            "含“预算不足”且≤8字",
            set(
                any(&["预算不足"], &[]),
                vec![C::MustContain("预算不足".into()), C::MaxChars(8)],
            ),
        ),
        (
            4,
            7,
            "multi-constraint",
            format!(
                "{FACTS_SC}任务：以JSON对象输出项目甲信息，必须包含name、owner、budget三个字段，不得输出JSON以外的文字。{UNIFIED}"
            ),
            "字段值正确",
            set(
                groups(&[&["甲"], &["小林"], &["30万", "30"]], &[]),
                vec![
                    C::JsonObject,
                    C::JsonRequiredFields(vec!["name".into(), "owner".into(), "budget".into()]),
                ],
            ),
        ),
        (
            4,
            8,
            "multi-constraint",
            format!(
                "{FACTS_SC}任务：按预算从高到低列出两个项目，每行一个，回答中不得包含“约”字。{UNIFIED}"
            ),
            "乙在甲前",
            set(
                groups(&[&["乙"], &["甲"]], &[]),
                vec![
                    C::ExactOrder(vec!["乙".into(), "甲".into()]),
                    C::LineCount(2),
                    C::MustNotContain("约".into()),
                ],
            ),
        ),
        // ---- 子域 5：信息不足（8 题）----
        (
            5,
            1,
            "insufficient-information",
            format!(
                "材料：项目甲预算30万，状态进行中，负责人待任命。任务：项目甲的负责人是谁？{UNIFIED}"
            ),
            "未提供",
            any(
                &["待任命", "未提供", "未知", "未确定", "尚未确定", "无法确定"],
                &[],
            ),
        ),
        (
            5,
            2,
            "insufficient-information",
            format!(
                "材料：记录一称验收人为王五，记录二称验收人为王六，两条记录均未标注日期。任务：验收人是谁？{UNIFIED}"
            ),
            "无法确定",
            any(&["冲突", "无法确定", "不一致", "无法判断", "两种说法"], &[]),
        ),
        (
            5,
            3,
            "insufficient-information",
            format!(
                "材料：项目甲的旧名称是北区项目，也曾被称为东区项目，未说明哪个是最新名称。任务：项目甲的最新名称是什么？{UNIFIED}"
            ),
            "无法判断",
            any(&["无法判断", "未说明", "无法确定", "未知"], &[]),
        ),
        (
            5,
            4,
            "insufficient-information",
            format!(
                "材料：项目总预算80万，分项预算未列出。任务：项目甲的分项预算是多少？{UNIFIED}"
            ),
            "未提供",
            any(&["未列出", "未提供", "未说明", "未知"], &[]),
        ),
        (
            5,
            5,
            "insufficient-information",
            format!(
                "材料：项目甲于2月立项，预计工期6个月。任务：项目甲的验收日期是哪天？{UNIFIED}"
            ),
            "未提供",
            any(&["未提供", "未说明", "未知", "无法确定"], &[]),
        ),
        (
            5,
            6,
            "insufficient-information",
            format!("材料：产品A本月销量增长20%。任务：产品B本月销量增长多少？{UNIFIED}"),
            "未提供",
            any(&["未提供", "未说明", "未知", "无产品B", "没有产品B"], &[]),
        ),
        (
            5,
            7,
            "insufficient-information",
            format!(
                "材料：华东的项目乙预算50万；华北的项目乙预算70万。任务：项目乙的预算是多少？{UNIFIED}"
            ),
            "需要澄清",
            any(
                &["两个", "哪个", "澄清", "歧义", "无法确定", "华东", "华北"],
                &[],
            ),
        ),
        (
            5,
            8,
            "insufficient-information",
            format!("材料：项目甲今年收入120万。任务：项目甲今年收入同比增长多少？{UNIFIED}"),
            "无法计算",
            any(
                &[
                    "无法计算",
                    "缺少",
                    "未提供",
                    "没有去年",
                    "无法确定",
                    "无法判断",
                    "未知",
                ],
                &[],
            ),
        ),
    ];

    defs.into_iter()
        .map(
            |(sub_index, sample_index, subdomain, prompt, expected, acceptance)| CapabilitySample {
                id: format!("C01-{sub_index}-{sample_index:02}"),
                category: CapabilityCategory::InstructionFollowing,
                subdomain: subdomain.into(),
                dataset_identity: DatasetIdentity::ProductOriginal,
                source_ref: format!("agentcheck://capability/C01-{sub_index}-{sample_index:02}"),
                language: CAPABILITY_LANGUAGE.into(),
                prompt,
                messages: None,
                tools: None,
                expected: expected.into(),
                acceptance,
                generation_settings: BTreeMap::from([
                    (String::from("temperature"), String::from("0")),
                    (String::from("language"), String::from(CAPABILITY_LANGUAGE)),
                ]),
                revision: CAPABILITY_VERSION.into(),
                input_tokens: None,
            },
        )
        .collect()
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
            let stripped = response
                .text
                .as_deref()
                .map(|text| strip_prompt_echo(text, &sample.prompt));
            let (label, reason) = evaluate_acceptance(
                &sample.acceptance,
                stripped.as_deref(),
                response.text.as_deref(),
                response.truncated,
                &response.tool_calls,
            );
            CapabilityObservation {
                sample_id: sample.id.clone(),
                label,
                output: response.text.map(|text| redact_output(&text)),
                tool_calls: response.tool_calls,
                reason,
                evidence_refs,
                executed: true,
            }
        }
    }
}

/// 剥掉响应中对题目的原文复述（部分模型会先引用材料/要求再作答），
/// 避免复述文本里的禁含值或"最终答案"字样污染判分。
fn strip_prompt_echo(text: &str, prompt: &str) -> String {
    let mut out = text.replace(prompt, "");
    if let Some((material, _)) = prompt.split_once("任务：") {
        if material.len() >= 8 {
            out = out.replace(material, "");
        }
    }
    if let Some(index) = prompt.find("只依据题目给出") {
        let suffix = &prompt[index..];
        if suffix.len() >= 8 {
            out = out.replace(suffix, "");
        }
    }
    out
}

/// 禁含值命中检查：命中项处在排除性语境（"除…外""不在""未超过"等）
/// 时不算违规，只统计以断言/列举形式出现的禁含值。
fn forbidden_hit(normalized: &str, forbidden: &[String]) -> bool {
    const EXCLUSION: [&str; 11] = [
        "除", "不", "未", "非", "排除", "以外", "之外", "不符", "其他", "原", "之前",
    ];
    forbidden.iter().any(|raw| {
        let needle = normalize(raw);
        !needle.is_empty()
            && normalized.match_indices(&needle).any(|(position, _)| {
                let head: String = normalized[..position]
                    .chars()
                    .rev()
                    .take(12)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                let tail: String = normalized[position + needle.len()..]
                    .chars()
                    .take(12)
                    .collect();
                let window = format!("{head}{tail}");
                !EXCLUSION.iter().any(|marker| window.contains(marker))
            })
    })
}

/// 提取"最终答案"区域：优先最后一个"最终答案"标记到解释边界；
/// 无标记时取最后一个非空段落；兜底全文。内容规则只对答案区生效，
/// 避免模型复述材料或解释排除项时带出禁含值造成误判。
fn answer_region(text: &str) -> &str {
    const MARKER: &str = "最终答案";
    const BOUNDARIES: [&str; 8] = [
        "说明：",
        "解释：",
        "理由：",
        "注：",
        "（注",
        "推理过程",
        "推理：",
        "分析：",
    ];
    if let Some(position) = text.rfind(MARKER) {
        let rest = &text[position..];
        let end = BOUNDARIES
            .iter()
            .filter_map(|boundary| rest.find(boundary))
            .chain(rest.find("\n\n"))
            .min()
            .unwrap_or(rest.len());
        return rest[..end].trim();
    }
    text.split("\n\n")
        .filter(|part| !part.trim().is_empty())
        .last()
        .unwrap_or(text)
        .trim()
}

/// 统一判分入口：返回（标签, 原因）。`text` 为剥离题目复述后的文本
/// （供答案区提取与内容规则使用），`output` 为完整原始输出
/// （供输出约束使用）；ConstraintSet 递归复用内容规则。
fn evaluate_acceptance(
    rule: &AcceptanceRule,
    text: Option<&str>,
    output: Option<&str>,
    truncated: bool,
    tool_calls: &[ToolCall],
) -> (ScoreLabel, Option<String>) {
    if truncated && !text.is_some_and(|text| text.contains("最终答案")) && tool_calls.is_empty()
    {
        return (
            ScoreLabel::Pending,
            Some("响应在最大输出长度处截断，最终答案未成形".into()),
        );
    }
    let answer = text.map(answer_region);
    match rule {
        AcceptanceRule::ExactAny { accepted } => (
            answer
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
            None,
        ),
        AcceptanceRule::ContainsAll { .. }
        | AcceptanceRule::ContainsAny { .. }
        | AcceptanceRule::MatchGroups { .. } => (
            answer
                .map(|region| match content_verdict(rule, region) {
                    Some(true) => ScoreLabel::Correct,
                    Some(false) => ScoreLabel::Wrong,
                    // 答案区既无别名也无禁含（如答案在中间段、末段是声明），
                    // 回退到剥离复述后的全文再判一次。
                    None => text
                        .map(|full| {
                            if content_verdict(rule, full).unwrap_or(false) {
                                ScoreLabel::Correct
                            } else {
                                ScoreLabel::Wrong
                            }
                        })
                        .unwrap_or(ScoreLabel::Wrong),
                })
                .unwrap_or(ScoreLabel::Pending),
            None,
        ),
        AcceptanceRule::ConstraintSet {
            content,
            constraints,
        } => {
            let (content_label, content_reason) =
                evaluate_acceptance(content, text, output, truncated, tool_calls);
            if content_label != ScoreLabel::Correct {
                return (content_label, content_reason);
            }
            let text = output.or(text).unwrap_or("");
            let failed: Vec<String> = constraints
                .iter()
                .filter(|constraint| !check_constraint(constraint, text))
                .map(OutputConstraint::describe)
                .collect();
            if failed.is_empty() {
                (ScoreLabel::Correct, None)
            } else {
                (
                    ScoreLabel::Incomplete,
                    Some(format!("内容正确但约束未满足：{}", failed.join("；"))),
                )
            }
        }
        AcceptanceRule::Numeric {
            expected,
            tolerance,
            unit,
        } => (
            answer
                .and_then(first_number)
                .map(|value| {
                    let unit_ok = unit.as_ref().is_none_or(|unit| {
                        answer.is_some_and(|text| normalize(text).contains(&normalize(unit)))
                    });
                    if (value - expected).abs() <= *tolerance && unit_ok {
                        ScoreLabel::Correct
                    } else {
                        ScoreLabel::Wrong
                    }
                })
                .unwrap_or(ScoreLabel::Pending),
            None,
        ),
        AcceptanceRule::ToolDecision {
            allowed_tool_names,
            required_arguments,
            allow_no_call,
        } => (
            if tool_calls.is_empty() {
                if *allow_no_call {
                    ScoreLabel::Correct
                } else {
                    ScoreLabel::Wrong
                }
            } else if tool_calls.iter().all(|call| {
                allowed_tool_names.iter().any(|name| name == &call.name)
                    && required_arguments.iter().all(|(key, value)| {
                        call.arguments
                            .get(key)
                            .is_some_and(|actual| normalize(actual) == normalize(value))
                    })
            }) {
                ScoreLabel::Correct
            } else {
                ScoreLabel::Wrong
            },
            None,
        ),
        AcceptanceRule::ToolCallsMatch { expected_calls } => (
            {
                let mut used = vec![false; tool_calls.len()];
                let matched = expected_calls.iter().all(|expected| {
                    tool_calls
                        .iter()
                        .enumerate()
                        .find(|(index, call)| {
                            !used[*index]
                                && call.name == expected.name
                                && expected.arguments.iter().all(|(key, value)| {
                                    call.arguments
                                        .get(key)
                                        .is_some_and(|actual| normalize(actual) == normalize(value))
                                })
                        })
                        .map(|(index, _)| used[index] = true)
                        .is_some()
                });
                if matched && tool_calls.len() == expected_calls.len() {
                    ScoreLabel::Correct
                } else {
                    ScoreLabel::Wrong
                }
            },
            None,
        ),
        AcceptanceRule::NoCall {
            accepted_explanations,
        } => (
            if !tool_calls.is_empty() {
                ScoreLabel::Wrong
            } else if answer.is_none() {
                ScoreLabel::Pending
            } else if accepted_explanations.is_empty()
                || answer.is_some_and(|text| {
                    accepted_explanations
                        .iter()
                        .any(|value| normalize(text).contains(&normalize(value)))
                })
            {
                ScoreLabel::Correct
            } else {
                ScoreLabel::Wrong
            },
            None,
        ),
        AcceptanceRule::Pending { .. } => (ScoreLabel::Pending, None),
    }
}

/// 内容型规则（ContainsAll/ContainsAny/MatchGroups）在一段文本上的判定：
/// Some(true)=通过，Some(false)=违规（命中禁含值），None=无结论（既无别名也无禁含）。
fn content_verdict(rule: &AcceptanceRule, region: &str) -> Option<bool> {
    let normalized = normalize(region);
    let (aliases_ok, forbidden) = match rule {
        AcceptanceRule::ContainsAll {
            required,
            forbidden,
        } => (
            required
                .iter()
                .all(|value| normalized.contains(&normalize(value))),
            forbidden,
        ),
        AcceptanceRule::ContainsAny { any_of, forbidden } => (
            any_of
                .iter()
                .any(|value| normalized.contains(&normalize(value))),
            forbidden,
        ),
        AcceptanceRule::MatchGroups {
            required_groups,
            forbidden,
        } => (
            required_groups.iter().all(|group| {
                group
                    .iter()
                    .any(|alias| normalized.contains(&normalize(alias)))
            }),
            forbidden,
        ),
        _ => return Some(false),
    };
    if forbidden_hit(&normalized, forbidden) {
        return Some(false);
    }
    if aliases_ok { Some(true) } else { None }
}

/// IFEval 式单条可验证约束的程序化校验。
fn check_constraint(constraint: &OutputConstraint, text: &str) -> bool {
    let normalized = normalize(text);
    match constraint {
        OutputConstraint::MaxChars(n) => normalized.chars().count() <= *n as usize,
        OutputConstraint::MustContain(value) => normalized.contains(&normalize(value)),
        OutputConstraint::MustNotContain(value) => !normalized.contains(&normalize(value)),
        OutputConstraint::KeywordMinCount(value, n) => {
            normalized.matches(&normalize(value)).count() >= *n as usize
        }
        OutputConstraint::JsonObject => serde_json::from_str::<Value>(text.trim())
            .map(|value| value.is_object())
            .unwrap_or(false),
        OutputConstraint::JsonFieldCount(n) => serde_json::from_str::<Value>(text.trim())
            .ok()
            .and_then(|value| value.as_object().map(|object| object.len() == *n as usize))
            .unwrap_or(false),
        OutputConstraint::JsonRequiredFields(fields) => serde_json::from_str::<Value>(text.trim())
            .ok()
            .and_then(|value| {
                value
                    .as_object()
                    .map(|object| fields.iter().all(|field| object.contains_key(field)))
            })
            .unwrap_or(false),
        OutputConstraint::LineCount(n) => {
            text.lines().filter(|line| !line.trim().is_empty()).count() == *n as usize
        }
        OutputConstraint::ParagraphCount(n) => {
            text.replace("\r\n", "\n")
                .split("\n\n")
                .filter(|part| !part.trim().is_empty())
                .count()
                == *n as usize
        }
        OutputConstraint::WrappedTitle => text.find("[[").is_some_and(|start| {
            text[start + 2..]
                .find("]]")
                .is_some_and(|end| !text[start + 2..start + 2 + end].trim().is_empty())
        }),
        OutputConstraint::StartsWith(prefix) => text.trim_start().starts_with(prefix.as_str()),
        OutputConstraint::EndsWith(suffix) => text.trim_end().ends_with(suffix.as_str()),
        OutputConstraint::NoChar(c) => !text.contains(*c),
        OutputConstraint::QuotedOutput => {
            const PAIRS: [(&str, &str); 4] = [("「", "」"), ("“", "”"), ("\"", "\""), ("'", "'")];
            let trimmed = text.trim();
            PAIRS.iter().any(|(open, close)| {
                trimmed.starts_with(open) && trimmed.ends_with(close) && trimmed.len() > open.len()
            })
        }
        OutputConstraint::ExactOrder(items) => {
            let mut cursor = 0usize;
            items.iter().all(|item| {
                normalized[cursor..]
                    .find(&normalize(item))
                    .map(|position| {
                        cursor += position + normalize(item).len();
                    })
                    .is_some()
            })
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
    if samples.len() != 144 {
        return Err(format!(
            "Capability catalog must contain 144 units, got {}",
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
        let expected = match category {
            CapabilityCategory::InstructionFollowing => 44,
            _ => 20,
        };
        let count = samples
            .iter()
            .filter(|sample| sample.category == category)
            .count();
        if count != expected {
            return Err(format!(
                "{} must contain {expected} logical units, got {count}",
                category.id()
            ));
        }
    }
    for sample in samples {
        if !sample
            .category
            .subdomains()
            .contains(&sample.subdomain.as_str())
        {
            return Err(format!(
                "{} has unknown subdomain {}",
                sample.id, sample.subdomain
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

/// C02 信息提取与结构化填写题库（20 题，方法借鉴 SGD 的槽位/字段抽取组织方式，
/// 材料为产品原创中文工单与登记记录）。子域序与 `subdomains()` 一致：
/// 1 单对象字段 2 多对象关系 3 字段类型 4 缺失值 5 冲突信息。
fn c02_bank() -> Vec<CapabilitySample> {
    const UNIFIED: &str =
        "只依据材料回答。材料没有提供的信息必须明确说明未知，不要使用外部知识补全。";
    const TICKETS: &str = "材料：【工单W-1024】类型：报修；区域：华东；负责人：陈晨；优先级：高；提交时间：2024-03-15；状态：待处理。【工单W-1025】类型：咨询；区域：华北；负责人：赵敏；优先级：低；提交时间：2024-03-16；状态：已关闭。";
    const REGISTRY: &str = "材料：【登记表A】设备名称：空压机；资产编号：ZC-3301；所属部门：动力车间；购置日期：2023-11-02；启用状态：已启用。";
    const CONFLICT: &str = "材料：登记记录（2024-01-10）：项目甲预算30万。变更记录（2024-02-20）：项目甲预算调整为45万。";

    use OutputConstraint as C;

    let defs: Vec<(u32, u32, &str, String, &str, AcceptanceRule)> = vec![
        // ---- 子域 1：单对象字段（4 题）----
        (
            1,
            1,
            "single-object-fields",
            format!("{TICKETS}任务：工单W-1024的负责人是谁？{UNIFIED}"),
            "陈晨",
            any(&["陈晨"], &["赵敏"]),
        ),
        (
            1,
            2,
            "single-object-fields",
            format!("{TICKETS}任务：工单W-1025的状态是什么？{UNIFIED}"),
            "已关闭",
            any(&["已关闭", "关闭"], &["待处理"]),
        ),
        (
            1,
            3,
            "single-object-fields",
            format!("{TICKETS}任务：工单W-1024的提交时间是哪天？{UNIFIED}"),
            "2024-03-15",
            any(
                &["2024-03-15", "2024年3月15日", "3月15日"],
                &["2024-03-16", "3月16日"],
            ),
        ),
        (
            1,
            4,
            "single-object-fields",
            format!("{REGISTRY}任务：该设备的资产编号是什么？{UNIFIED}"),
            "ZC-3301",
            any(&["ZC-3301", "ZC3301"], &[]),
        ),
        // ---- 子域 2：多对象关系（4 题）----
        (
            2,
            1,
            "multi-object-relations",
            format!("{TICKETS}任务：陈晨负责的工单编号是什么？{UNIFIED}"),
            "W-1024",
            any(&["W-1024", "W1024"], &["W-1025"]),
        ),
        (
            2,
            2,
            "multi-object-relations",
            format!("{TICKETS}任务：华北区域的工单编号是什么？{UNIFIED}"),
            "W-1025",
            any(&["W-1025", "W1025"], &[]),
        ),
        (
            2,
            3,
            "multi-object-relations",
            format!("{TICKETS}任务：状态为“待处理”的工单，其负责人和优先级分别是什么？{UNIFIED}"),
            "陈晨、高",
            groups(&[&["陈晨"], &["高"]], &["赵敏"]),
        ),
        (
            2,
            4,
            "multi-object-relations",
            format!(
                "{REGISTRY}材料补充：动力车间隶属于生产部。任务：空压机所属的上一级部门是什么？{UNIFIED}"
            ),
            "生产部",
            any(&["生产部"], &["动力车间"]),
        ),
        // ---- 子域 3：字段类型（4 题，含格式约束）----
        (
            3,
            1,
            "field-types",
            format!(
                "{TICKETS}任务：提取工单W-1024的提交时间，输出格式为 YYYY-MM-DD，不要输出其他文字。{UNIFIED}"
            ),
            "2024-03-15",
            set(
                any(&["2024-03-15"], &["2024-03-16"]),
                vec![C::MustNotContain("年".into()), C::MaxChars(12)],
            ),
        ),
        (
            3,
            2,
            "field-types",
            format!(
                "{REGISTRY}材料补充：库存记录显示该设备备件现有12件。任务：提取备件数量，只输出阿拉伯数字，不带单位。{UNIFIED}"
            ),
            "12",
            set(
                any(&["12"], &[]),
                vec![C::MustNotContain("件".into()), C::MaxChars(4)],
            ),
        ),
        (
            3,
            3,
            "field-types",
            format!(
                "{TICKETS}任务：用JSON对象输出工单W-1025的负责人和优先级，字段名用中文，不要输出JSON以外的文字。{UNIFIED}"
            ),
            "赵敏、低",
            set(
                groups(&[&["赵敏"], &["低"]], &["陈晨"]),
                vec![
                    C::JsonObject,
                    C::JsonRequiredFields(vec!["负责人".into(), "优先级".into()]),
                ],
            ),
        ),
        (
            3,
            4,
            "field-types",
            format!(
                "{TICKETS}任务：按提交时间从早到晚，每行一个列出工单编号，不要输出其他文字。{UNIFIED}"
            ),
            "W-1024、W-1025",
            set(
                groups(&[&["W-1024"], &["W-1025"]], &[]),
                vec![
                    C::LineCount(2),
                    C::ExactOrder(vec!["W-1024".into(), "W-1025".into()]),
                ],
            ),
        ),
        // ---- 子域 4：缺失值（4 题）----
        (
            4,
            1,
            "missing-values",
            format!("{TICKETS}任务：工单W-1024的验收日期是哪天？{UNIFIED}"),
            "未提供",
            any(
                &["未提供", "未说明", "未知", "没有提供", "无此信息"],
                &["2024"],
            ),
        ),
        (
            4,
            2,
            "missing-values",
            format!(
                "材料：【工单W-2001】类型：报修；区域：华南；负责人：（空）；优先级：中。任务：该工单的负责人是谁？{UNIFIED}"
            ),
            "未提供",
            any(
                &["未提供", "未填写", "为空", "未知", "无"],
                &["陈晨", "赵敏"],
            ),
        ),
        (
            4,
            3,
            "missing-values",
            format!(
                "{REGISTRY}任务：用JSON对象输出该设备的资产编号和报废日期，字段名用中文；材料没有的字段值填 null。{UNIFIED}"
            ),
            "ZC-3301、null",
            set(
                groups(&[&["ZC-3301", "ZC3301"], &["null", "未提供"]], &[]),
                vec![C::JsonObject],
            ),
        ),
        (
            4,
            4,
            "missing-values",
            format!("{TICKETS}任务：工单W-3000的负责人是谁？{UNIFIED}"),
            "未提供",
            any(
                &["不存在", "未提供", "未知", "没有", "无此工单", "材料中没有"],
                &["陈晨", "赵敏"],
            ),
        ),
        // ---- 子域 5：冲突信息（4 题）----
        (
            5,
            1,
            "conflicting-information",
            format!("{CONFLICT}任务：以最新登记为准，项目甲当前预算是多少？{UNIFIED}"),
            "45万",
            any(&["45万", "450000"], &[]),
        ),
        (
            5,
            2,
            "conflicting-information",
            format!(
                "材料：记录一：项目乙验收人是王五。记录二：项目乙验收人是王六。两条记录均有效。任务：项目乙的验收人是谁？{UNIFIED}"
            ),
            "无法确定",
            any(
                &[
                    "无法确定",
                    "冲突",
                    "不一致",
                    "矛盾",
                    "两条记录",
                    "王五和王六",
                    "王五、王六",
                ],
                &[],
            ),
        ),
        (
            5,
            3,
            "conflicting-information",
            format!(
                "材料：2024-03-01登记：设备状态为“停用”。2024-04-10登记：设备状态更新为“在用”。任务：按最新登记回答设备当前状态。{UNIFIED}"
            ),
            "在用",
            any(&["在用"], &["停用"]),
        ),
        (
            5,
            4,
            "conflicting-information",
            format!(
                "材料：有两个同名“项目甲”：华东区项目甲（负责人小林）和华南区项目甲（负责人老周）。任务：项目甲的负责人是谁？{UNIFIED}"
            ),
            "无法确定",
            any(
                &[
                    "无法确定",
                    "两个",
                    "需要明确",
                    "哪一个",
                    "同名",
                    "未知",
                    "未明确",
                    "不明确",
                    "可能是",
                ],
                &[],
            ),
        ),
    ];

    defs.into_iter()
        .map(|(si, qi, sub, prompt, expected, acceptance)| {
            sample(
                CapabilityCategory::InformationExtraction,
                si,
                qi,
                sub,
                prompt,
                expected,
                acceptance,
                None,
                None,
                None,
            )
        })
        .collect()
}

/// C03 工具选择与参数填写题库（20 题，方法借鉴 BFCL：真实 tools 定义随请求发送，
/// 判定函数名与参数值而非提示词复述）。子域序与 `subdomains()` 一致：
/// 1 工具选择 2 参数填写 3 参数类型 4 多工具调用 5 正确不调用。
fn c03_bank() -> Vec<CapabilitySample> {
    use serde_json::json;
    fn tool(name: &str, description: &str, props: Value, required: &[&str]) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": name,
                "description": description,
                "parameters": {
                    "type": "object",
                    "properties": props,
                    "required": required,
                }
            }
        })
    }
    fn s(p: &str) -> Value {
        json!({"type": "string", "description": p})
    }
    fn call(name: &str, args: &[(&str, &str)]) -> ToolCall {
        ToolCall {
            name: name.into(),
            arguments: args
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }
    fn pick(names: &[&str]) -> AcceptanceRule {
        AcceptanceRule::ToolDecision {
            allowed_tool_names: names.iter().map(|n| n.to_string()).collect(),
            required_arguments: BTreeMap::new(),
            allow_no_call: false,
        }
    }
    fn call_with(name: &str, args: &[(&str, &str)]) -> AcceptanceRule {
        AcceptanceRule::ToolCallsMatch {
            expected_calls: vec![call(name, args)],
        }
    }
    fn calls(calls: Vec<ToolCall>) -> AcceptanceRule {
        AcceptanceRule::ToolCallsMatch {
            expected_calls: calls,
        }
    }

    let weather = tool(
        "query_weather",
        "查询指定城市指定日期的天气",
        json!({"city": s("城市名"), "date": s("日期，如“今天”")}),
        &["city", "date"],
    );
    let calc = tool(
        "calculate",
        "计算一个算术表达式的值",
        json!({"expression": s("算术表达式")}),
        &["expression"],
    );
    let inventory = tool(
        "query_inventory",
        "查询某仓库中某物料的库存数量",
        json!({"warehouse": s("仓库名"), "item": s("物料名")}),
        &["warehouse", "item"],
    );
    let notify = tool(
        "send_notification",
        "向指定对象发送一条通知消息",
        json!({"target": s("接收人"), "message": s("通知内容")}),
        &["target", "message"],
    );
    let project = tool(
        "query_project",
        "查询项目基本信息字段",
        json!({"project": s("项目名"), "field": s("字段名")}),
        &["project", "field"],
    );
    let meeting = tool(
        "book_meeting_room",
        "预订会议室",
        json!({"room": s("会议室名称"), "attendees": json!({"type": "integer", "description": "参会人数"}), "duration_hours": json!({"type": "integer", "description": "时长（小时）"})}),
        &["room", "attendees", "duration_hours"],
    );
    let alert = tool(
        "set_alert",
        "设置指标告警开关",
        json!({"metric": s("指标名"), "enabled": json!({"type": "boolean", "description": "是否开启"})}),
        &["metric", "enabled"],
    );

    let all3 = Some(vec![weather.clone(), calc.clone(), inventory.clone()]);
    let all4 = Some(vec![
        weather.clone(),
        calc.clone(),
        inventory.clone(),
        notify.clone(),
    ]);
    let all5 = Some(vec![
        weather.clone(),
        calc.clone(),
        inventory.clone(),
        notify.clone(),
        project.clone(),
    ]);

    let defs: Vec<(
        u32,
        u32,
        &str,
        String,
        &str,
        AcceptanceRule,
        Option<Vec<Value>>,
    )> = vec![
        // ---- 子域 1：工具选择（4 题）----
        (
            1,
            1,
            "tool-selection",
            "帮我查一下北京明天的天气。".into(),
            "query_weather",
            pick(&["query_weather"]),
            all3.clone(),
        ),
        (
            1,
            2,
            "tool-selection",
            "帮我算一下 128 乘以 46 等于多少。".into(),
            "calculate",
            pick(&["calculate"]),
            all3.clone(),
        ),
        (
            1,
            3,
            "tool-selection",
            "华东仓的零件A还剩多少库存？".into(),
            "query_inventory",
            pick(&["query_inventory"]),
            all3.clone(),
        ),
        (
            1,
            4,
            "tool-selection",
            "给小周发一条通知，内容是下午三点开项目例会。".into(),
            "send_notification",
            pick(&["send_notification"]),
            all4.clone(),
        ),
        // ---- 子域 2：参数填写（4 题）----
        (
            2,
            1,
            "argument-filling",
            "查一下上海后天的天气。".into(),
            "city=上海,date=后天",
            call_with("query_weather", &[("city", "上海"), ("date", "后天")]),
            all4.clone(),
        ),
        (
            2,
            2,
            "argument-filling",
            "华北仓的零件B库存还剩多少？".into(),
            "warehouse=华北仓,item=零件B",
            call_with(
                "query_inventory",
                &[("warehouse", "华北仓"), ("item", "零件B")],
            ),
            all4.clone(),
        ),
        (
            2,
            3,
            "argument-filling",
            "查一下项目甲的负责人是谁。".into(),
            "project=项目甲,field=负责人",
            call_with(
                "query_project",
                &[("project", "项目甲"), ("field", "负责人")],
            ),
            all5.clone(),
        ),
        (
            2,
            4,
            "argument-filling",
            "给赵敏发通知，说明天上午十点验收。".into(),
            "target=赵敏",
            call_with("send_notification", &[("target", "赵敏")]),
            all4.clone(),
        ),
        // ---- 子域 3：参数类型（4 题）----
        (
            3,
            1,
            "argument-types",
            "预订3号会议室，8个人参加，用2小时。".into(),
            "attendees=8,duration_hours=2",
            call_with(
                "book_meeting_room",
                &[
                    ("room", "3号会议室"),
                    ("attendees", "8"),
                    ("duration_hours", "2"),
                ],
            ),
            Some(vec![meeting.clone()]),
        ),
        (
            3,
            2,
            "argument-types",
            "把库存告警开关打开。".into(),
            "enabled=true",
            call_with("set_alert", &[("enabled", "true")]),
            Some(vec![alert.clone()]),
        ),
        (
            3,
            3,
            "argument-types",
            "算一下 12 加 30。".into(),
            "expression=12+30",
            call_with("calculate", &[("expression", "12+30")]),
            all4.clone(),
        ),
        (
            3,
            4,
            "argument-types",
            "查项目乙的预算。".into(),
            "field=预算",
            call_with("query_project", &[("project", "项目乙"), ("field", "预算")]),
            all5.clone(),
        ),
        // ---- 子域 4：多工具调用（4 题）----
        (
            4,
            1,
            "multiple-tools",
            "北京和上海明天的天气分别怎么样？".into(),
            "query_weather×2",
            calls(vec![
                call("query_weather", &[("city", "北京"), ("date", "明天")]),
                call("query_weather", &[("city", "上海"), ("date", "明天")]),
            ]),
            all4.clone(),
        ),
        (
            4,
            2,
            "multiple-tools",
            "先查华东仓零件A的库存，然后给小周发个通知告诉他结果。".into(),
            "query_inventory+send_notification",
            calls(vec![
                call(
                    "query_inventory",
                    &[("warehouse", "华东仓"), ("item", "零件A")],
                ),
                call("send_notification", &[("target", "小周")]),
            ]),
            all4.clone(),
        ),
        (
            4,
            3,
            "multiple-tools",
            "查一下项目甲的预算和负责人。".into(),
            "query_project×2",
            calls(vec![
                call("query_project", &[("project", "项目甲"), ("field", "预算")]),
                call(
                    "query_project",
                    &[("project", "项目甲"), ("field", "负责人")],
                ),
            ]),
            all5.clone(),
        ),
        (
            4,
            4,
            "multiple-tools",
            "分别计算 12+30 和 50-8 的结果。".into(),
            "calculate×2",
            calls(vec![
                call("calculate", &[("expression", "12+30")]),
                call("calculate", &[("expression", "50-8")]),
            ]),
            all4.clone(),
        ),
        // ---- 子域 5：正确不调用（4 题）----
        (
            5,
            1,
            "correct-no-call",
            "你好，今天过得怎么样？".into(),
            "不调用工具",
            AcceptanceRule::NoCall {
                accepted_explanations: vec![],
            },
            all3.clone(),
        ),
        (
            5,
            2,
            "correct-no-call",
            "材料：项目甲负责人是小林。任务：项目甲负责人是谁？只依据材料回答，不要调用工具。"
                .into(),
            "小林",
            AcceptanceRule::NoCall {
                accepted_explanations: vec!["小林".into()],
            },
            all5.clone(),
        ),
        (
            5,
            3,
            "correct-no-call",
            "不用调用任何工具，直接告诉我 3+5 等于几。".into(),
            "8",
            AcceptanceRule::NoCall {
                accepted_explanations: vec!["8".into(), "八".into()],
            },
            all3.clone(),
        ),
        (
            5,
            4,
            "correct-no-call",
            "帮我订一张明天去上海的高铁票。".into(),
            "不调用工具",
            AcceptanceRule::NoCall {
                accepted_explanations: vec![],
            },
            all3.clone(),
        ),
    ];

    defs.into_iter()
        .map(|(si, qi, sub, prompt, expected, acceptance, tools)| {
            sample(
                CapabilityCategory::ToolSelection,
                si,
                qi,
                sub,
                prompt,
                expected,
                acceptance,
                None,
                tools,
                None,
            )
        })
        .collect()
}

/// C04 多轮对话与条件承接题库（20 题，方法借鉴 Multi-IF：真实 messages 数组、
/// 条件逐轮叠加/变更/撤销）。子域序与 `subdomains()` 一致：
/// 1 条件保持 2 条件更新 3 条件撤销 4 对象切换 5 新旧区分。
fn c04_bank() -> Vec<CapabilitySample> {
    fn m(role: &str, content: &str) -> SampleMessage {
        SampleMessage {
            role: role.into(),
            content: content.into(),
        }
    }
    fn msgs(turns: &[(&str, &str)]) -> Option<Vec<SampleMessage>> {
        Some(turns.iter().map(|(r, c)| m(r, c)).collect())
    }
    use OutputConstraint as C;

    let defs: Vec<(
        u32,
        u32,
        &str,
        String,
        &str,
        AcceptanceRule,
        Option<Vec<SampleMessage>>,
    )> = vec![
        // ---- 子域 1：条件保持（4 题）----
        (
            1,
            1,
            "condition-retention",
            "那3000元的设备采购报销呢？需要总监签字吗？".into(),
            "需要总监签字",
            any(&["需要", "要", "必须"], &["不需要"]),
            msgs(&[
                ("user", "记住这个规则：报销金额超过2000元需要总监签字。"),
                (
                    "assistant",
                    "好的，已记录：报销金额超过2000元需要总监签字。",
                ),
                ("user", "我有一笔1500元的差旅报销。"),
                ("assistant", "1500元未超过2000元，不需要总监签字。"),
                ("user", "那3000元的设备采购报销呢？需要总监签字吗？"),
            ]),
        ),
        (
            1,
            2,
            "condition-retention",
            "那周二呢？".into(),
            "赵敏",
            any(&["赵敏"], &["陈晨"]),
            msgs(&[
                ("user", "本周值班表：周一陈晨，周二赵敏，周三老钱。"),
                ("assistant", "已记录值班表。"),
                ("user", "周一谁值班？"),
                ("assistant", "陈晨。"),
                ("user", "那周二呢？"),
            ]),
        ),
        (
            1,
            3,
            "condition-retention",
            "项目乙的编号呢？".into(),
            "B-202",
            any(&["B-202", "B202"], &["A-101"]),
            msgs(&[
                (
                    "user",
                    "项目编号对照：项目甲是A-101，项目乙是B-202，项目丙是C-303。",
                ),
                ("assistant", "已记录编号对照。"),
                ("user", "项目甲的编号是什么？"),
                ("assistant", "A-101。"),
                ("user", "项目乙的编号呢？"),
            ]),
        ),
        (
            1,
            4,
            "condition-retention",
            "住宿费报销上限是多少？".into(),
            "5000",
            any(&["5000", "五千"], &["2000", "3000"]),
            msgs(&[
                ("user", "记住：本月报销总额上限是5000元。"),
                ("assistant", "好的，已记录：本月报销总额上限5000元。"),
                ("user", "交通费报销上限是多少？"),
                ("assistant", "交通费适用总额上限5000元。"),
                ("user", "住宿费报销上限是多少？"),
            ]),
        ),
        // ---- 子域 2：条件更新（4 题）----
        (
            2,
            1,
            "condition-update",
            "项目甲当前预算是多少？".into(),
            "45万",
            any(&["45万", "450000"], &[]),
            msgs(&[
                ("user", "记录：项目甲预算30万。"),
                ("assistant", "已记录：项目甲预算30万。"),
                ("user", "更正一下：项目甲预算调整为45万。"),
                ("assistant", "已更新：项目甲预算45万。"),
                ("user", "项目甲当前预算是多少？"),
            ]),
        ),
        (
            2,
            2,
            "condition-update",
            "项目甲现在谁负责？".into(),
            "小周",
            any(&["小周"], &[]),
            msgs(&[
                ("user", "项目甲的负责人是小林。"),
                ("assistant", "好的。"),
                ("user", "更正：负责人改为小周。"),
                ("assistant", "已更新：负责人为小周。"),
                ("user", "项目甲现在谁负责？"),
            ]),
        ),
        (
            2,
            3,
            "condition-update",
            "评审会现在是哪天？".into(),
            "3月20日",
            any(&["3月20日", "三月二十"], &["3月15日"]),
            msgs(&[
                ("user", "评审会定在3月15日。"),
                ("assistant", "已记录：评审会3月15日。"),
                ("user", "通知有变化：评审会改到3月20日。"),
                ("assistant", "已更新：评审会3月20日。"),
                ("user", "评审会现在是哪天？"),
            ]),
        ),
        (
            2,
            4,
            "condition-update",
            "当前库存多少？".into(),
            "60",
            any(&["60"], &["100", "80"]),
            msgs(&[
                ("user", "库存100件。"),
                ("assistant", "已记录：库存100件。"),
                ("user", "出库20件。"),
                ("assistant", "已更新：库存80件。"),
                ("user", "又出库20件。"),
                ("assistant", "已更新：库存60件。"),
                ("user", "当前库存多少？"),
            ]),
        ),
        // ---- 子域 3：条件撤销（4 题）----
        (
            3,
            1,
            "condition-revocation",
            "项目甲负责人是谁？（已知：小林）".into(),
            "小林",
            set(any(&["小林"], &[]), vec![C::MustNotContain("完毕".into())]),
            msgs(&[
                ("user", "从现在开始，你的每条回答末尾都要加“完毕”两个字。"),
                ("assistant", "好的，我会在每条回答末尾加上“完毕”。"),
                ("user", "项目甲预算多少？（已知：30万）"),
                ("assistant", "项目甲预算30万。完毕"),
                ("user", "取消刚才那个结尾要求，正常回答。"),
                ("assistant", "好的，已取消。"),
                ("user", "项目甲负责人是谁？（已知：小林）"),
            ]),
        ),
        (
            3,
            2,
            "condition-revocation",
            "项目甲负责人是谁？（已知：小林）".into(),
            "小林",
            set(any(&["小林"], &[]), vec![C::MustNotContain("{".into())]),
            msgs(&[
                ("user", "从现在开始，你的所有回答都用JSON格式输出。"),
                ("assistant", "好的，之后我会用JSON格式回答。"),
                ("user", "项目甲预算多少？（已知：30万）"),
                ("assistant", "{\"预算\": \"30万\"}"),
                ("user", "取消JSON格式要求，用正常中文回答即可。"),
                ("assistant", "好的。"),
                ("user", "项目甲负责人是谁？（已知：小林）"),
            ]),
        ),
        (
            3,
            3,
            "condition-revocation",
            "项目乙在哪个区域？（材料：项目乙位于华北）".into(),
            "华北",
            set(any(&["华北"], &["华东"]), vec![]),
            msgs(&[
                ("user", "之后提到区域时只能回答“华东”，其他区域都不要提。"),
                ("assistant", "好的。"),
                ("user", "项目甲在哪个区域？（材料：项目甲位于华东）"),
                ("assistant", "华东"),
                ("user", "取消刚才的限制，之后如实回答所有区域。"),
                ("assistant", "好的。"),
                ("user", "项目乙在哪个区域？（材料：项目乙位于华北）"),
            ]),
        ),
        (
            3,
            4,
            "condition-revocation",
            "项目乙状态是什么？（已知：进行中）".into(),
            "进行中",
            set(any(&["进行中"], &[]), vec![C::MaxChars(20)]),
            msgs(&[
                ("user", "从现在开始，回答问题时字数不限，越详细越好。"),
                ("assistant", "好的，我会尽量详细回答。"),
                ("user", "项目甲状态是什么？（已知：进行中）"),
                (
                    "assistant",
                    "项目甲目前处于进行中状态，各项工作按计划推进，尚未完成验收。",
                ),
                ("user", "取消详细要求，之后回答越简短越好，不超过20个字。"),
                ("assistant", "好的。"),
                ("user", "项目乙状态是什么？（已知：进行中）"),
            ]),
        ),
        // ---- 子域 4：对象切换（4 题）----
        (
            4,
            1,
            "object-switching",
            "那项目甲的负责人呢？".into(),
            "小林",
            any(&["小林"], &[]),
            msgs(&[
                ("user", "项目甲负责人是小林，预算30万。"),
                ("assistant", "已记录项目甲信息。"),
                ("user", "项目乙负责人是小周，预算50万。"),
                ("assistant", "已记录项目乙信息。"),
                ("user", "项目乙负责人是谁？"),
                ("assistant", "小周。"),
                ("user", "那项目甲的负责人呢？"),
            ]),
        ),
        (
            4,
            2,
            "object-switching",
            "项目甲的呢？".into(),
            "30万",
            any(&["30万", "300000"], &["50万"]),
            msgs(&[
                ("user", "项目甲预算30万，项目乙预算50万。"),
                ("assistant", "已记录。"),
                ("user", "项目乙预算多少？"),
                ("assistant", "50万。"),
                ("user", "项目甲的呢？"),
            ]),
        ),
        (
            4,
            3,
            "object-switching",
            "W-2的区域？".into(),
            "华北",
            any(&["华北"], &["华东"]),
            msgs(&[
                ("user", "工单W-1区域是华东，工单W-2区域是华北。"),
                ("assistant", "已记录。"),
                ("user", "W-1的区域？"),
                ("assistant", "华东。"),
                ("user", "W-2的区域？"),
            ]),
        ),
        (
            4,
            4,
            "object-switching",
            "设备A呢？".into(),
            "陈晨",
            any(&["陈晨"], &["赵敏"]),
            msgs(&[
                ("user", "设备A由陈晨保管，设备B由赵敏保管。"),
                ("assistant", "已记录。"),
                ("user", "设备B谁保管？"),
                ("assistant", "赵敏。"),
                ("user", "设备A呢？"),
            ]),
        ),
        // ---- 子域 5：新旧区分（4 题）----
        (
            5,
            1,
            "history-distinction",
            "入库之前的库存是多少？".into(),
            "100",
            any(&["100"], &["120"]),
            msgs(&[
                ("user", "库存原本100件。"),
                ("assistant", "已记录：库存100件。"),
                ("user", "入库20件后库存变成多少？"),
                ("assistant", "120件。"),
                ("user", "入库之前的库存是多少？"),
            ]),
        ),
        (
            5,
            2,
            "history-distinction",
            "原来的负责人是谁？".into(),
            "小林",
            any(&["小林"], &[]),
            msgs(&[
                ("user", "项目甲原负责人是小林，现已更换为小周。"),
                ("assistant", "已记录负责人变更。"),
                ("user", "现在的负责人是谁？"),
                ("assistant", "小周。"),
                ("user", "原来的负责人是谁？"),
            ]),
        ),
        (
            5,
            3,
            "history-distinction",
            "最初的预算是多少？".into(),
            "30万",
            any(&["30万"], &["45万"]),
            msgs(&[
                ("user", "项目甲预算最初30万，后来调整到45万。"),
                ("assistant", "已记录。"),
                ("user", "当前预算多少？"),
                ("assistant", "45万。"),
                ("user", "最初的预算是多少？"),
            ]),
        ),
        (
            5,
            4,
            "history-distinction",
            "原定日期是哪天？".into(),
            "3月15日",
            any(&["3月15日"], &["3月20日"]),
            msgs(&[
                ("user", "评审会原定3月15日，后改到3月20日。"),
                ("assistant", "已记录。"),
                ("user", "现在会议是哪天？"),
                ("assistant", "3月20日。"),
                ("user", "原定日期是哪天？"),
            ]),
        ),
    ];

    defs.into_iter()
        .map(|(si, qi, sub, prompt, expected, acceptance, messages)| {
            sample(
                CapabilityCategory::MultiTurn,
                si,
                qi,
                sub,
                prompt,
                expected,
                acceptance,
                messages,
                None,
                None,
            )
        })
        .collect()
}
/// C05 长材料理解与信息利用题库（20 题，方法借鉴 RULER：确定性生成可控长度
/// 台账材料，在指定深度注入"针"事实与干扰项）。子域序与 `subdomains()` 一致：
/// 1 定位 2 跨段关联 3 干扰项排除 4 长度变化 5 材料内计算。
fn c05_bank() -> Vec<CapabilitySample> {
    const WH: [&str; 4] = ["华东仓", "华北仓", "华南仓", "西南仓"];
    const ITEMS: [&str; 6] = ["零件A", "零件B", "零件C", "零件D", "紧固件", "密封圈"];
    const DIR: [&str; 2] = ["入库", "出库"];
    const PPL: [&str; 6] = ["王二", "李四", "张三", "陈晨", "赵敏", "老钱"];

    fn entry(i: usize) -> String {
        format!(
            "第{i:03}条｜2024-03-{day:02}｜{wh}｜{item}｜{dir}｜{qty}件｜经办人：{p}",
            i = i,
            day = (i % 27) + 1,
            wh = WH[i % 4],
            item = ITEMS[(i * 3 + 1) % 6],
            dir = DIR[(i / 2) % 2],
            qty = (i * 7 % 90) + 10,
            p = PPL[(i * 5 + 2) % 6],
        )
    }

    /// 生成台账：entries 条基础记录，按位置升序插入备注行（干扰项同法）。
    fn ledger(entries: usize, inserts: &[(usize, &str)], header: &str) -> String {
        let mut lines: Vec<String> = (1..=entries).map(entry).collect();
        for (pos, text) in inserts.iter().rev() {
            lines.insert((*pos).min(lines.len()), format!("备注：{text}"));
        }
        let body = lines.join("\n");
        format!("材料：以下是一份仓库台账（含{entries}条流水记录及若干备注行）。{header}\n{body}\n")
    }

    fn q(entries: usize, inserts: &[(usize, &str)], header: &str, task: &str) -> (String, u32) {
        let material = ledger(entries, inserts, header);
        let prompt = format!("{material}任务：{task}只依据材料回答，材料没有的信息必须说明未知。");
        let tokens = (prompt.chars().count() / 2) as u32;
        (prompt, tokens)
    }

    // 备注行的"针"：插在 entries 的 ~10%/~35%/~60%/~85% 深度
    let (p1, t1) = q(
        80,
        &[(8, "华东仓零件C盘点亏库7件，已上报仓库主管。")],
        "",
        "华东仓零件C盘点亏库了多少件？",
    );
    let (p2, t2) = q(
        80,
        &[(28, "零件B单价自本月起调整为每件4.5元。")],
        "",
        "零件B调整后的单价是多少？",
    );
    let (p3, t3) = q(
        80,
        &[(48, "华北仓3月起由赵敏统一验收。")],
        "",
        "华北仓3月起由谁统一验收？",
    );
    let (p4, t4) = q(
        80,
        &[(68, "台账第075条记录因重复登记作废。")],
        "",
        "台账中哪条记录因重复登记作废？",
    );

    // 跨段关联：页首规则/编号 + 深处事实组合
    let (p5, t5) = q(
        80,
        &[(40, "本次盘点结果已按页首账本编号归档。")],
        "账本编号：XH-2024-031。\n",
        "本次盘点结果按哪个账本编号归档？",
    );
    let (p6, t6) = q(
        80,
        &[(20, "第030条记录的经办人已变更为老钱。")],
        "",
        "变更后第030条记录的经办人是谁？",
    );
    let (p7, t7) = q(
        80,
        &[
            (15, "零件A安全库存为50件，低于该值须补货。"),
            (60, "华东仓零件A当前库存42件。"),
        ],
        "",
        "华东仓零件A当前是否需要补货？回答「需要」或「不需要」。",
    );
    let (p8, t8) = q(
        80,
        &[(10, "所有出库记录须经李四复核。")],
        "",
        "第058条记录需要谁复核？",
    );

    // 干扰项排除：同物不同仓 / 同仓不同物 / 新旧值 / 相邻字段
    let (p9, t9) = q(
        80,
        &[
            (15, "华东仓零件C盘点亏库7件。"),
            (60, "华北仓零件C盘点亏库12件。"),
        ],
        "",
        "华东仓零件C盘点亏库多少件？",
    );
    let (p10, t10) = q(
        80,
        &[
            (25, "零件B销售单价为每件4.5元。"),
            (55, "零件B采购价为每件5.2元。"),
        ],
        "",
        "零件B的销售单价是多少？",
    );
    let (p11, t11) = q(
        80,
        &[(30, "设备A原保管人为张三。"), (70, "设备A现保管人为陈晨。")],
        "",
        "设备A现在的保管人是谁？",
    );
    let (p12, t12) = q(
        80,
        &[(35, "零件D入库日期2024-03-12，验收日期2024-03-18。")],
        "",
        "零件D的验收日期是哪天？",
    );

    // 长度变化：同一任务在不同规模材料中
    let (p13, t13) = q(
        30,
        &[(22, "华东仓零件C盘点亏库7件。")],
        "",
        "华东仓零件C盘点亏库多少件？",
    );
    let (p14, t14) = q(
        60,
        &[(45, "华东仓零件C盘点亏库7件。")],
        "",
        "华东仓零件C盘点亏库多少件？",
    );
    let (p15, t15) = q(
        120,
        &[(90, "华东仓零件C盘点亏库7件。")],
        "",
        "华东仓零件C盘点亏库多少件？",
    );
    let (p16, t16) = q(
        160,
        &[(130, "华东仓零件C盘点亏库7件。")],
        "",
        "华东仓零件C盘点亏库多少件？",
    );

    // 材料内计算：对注入的针做计数/求和/乘法
    let (p17, t17) = q(
        80,
        &[
            (12, "华东仓零件C盘点亏库7件。"),
            (36, "华北仓零件A盘点亏库12件。"),
            (64, "华南仓零件D盘点亏库5件。"),
        ],
        "",
        "本台账记录的盘点亏库一共多少件？",
    );
    let (p18, t18) = q(
        80,
        &[
            (10, "第010条标记为加急。"),
            (30, "第030条标记为加急。"),
            (50, "第050条标记为加急。"),
            (70, "第070条标记为加急。"),
        ],
        "",
        "本台账中被标记为「加急」的记录共有几条？",
    );
    let (p19, t19) = q(
        80,
        &[(20, "零件B分两批入库：第一批50件，第二批30件。")],
        "",
        "零件B两批入库共多少件？",
    );
    let (p20, t20) = q(
        80,
        &[(40, "零件E出库20件，单价3元。")],
        "",
        "零件E出库金额是多少元？",
    );

    let defs: Vec<(u32, u32, &str, String, &str, AcceptanceRule, u32)> = vec![
        (
            1,
            1,
            "localization",
            p1,
            "7件",
            any(&["7件", "7", "七件"], &["12件", "12"]),
            t1,
        ),
        (
            1,
            2,
            "localization",
            p2,
            "4.5元",
            any(&["4.5", "四点五"], &["5.2"]),
            t2,
        ),
        (
            1,
            3,
            "localization",
            p3,
            "赵敏",
            any(&["赵敏"], &["陈晨", "王二", "李四"]),
            t3,
        ),
        (
            1,
            4,
            "localization",
            p4,
            "第075条",
            any(&["075", "第075条", "75条"], &[]),
            t4,
        ),
        (
            2,
            1,
            "cross-section-relation",
            p5,
            "XH-2024-031",
            any(&["XH-2024-031", "XH2024-031"], &[]),
            t5,
        ),
        (
            2,
            2,
            "cross-section-relation",
            p6,
            "老钱",
            any(&["老钱"], &[]),
            t6,
        ),
        (
            2,
            3,
            "cross-section-relation",
            p7,
            "需要",
            set(any(&["需要"], &["不需要"]), vec![]),
            t7,
        ),
        (
            2,
            4,
            "cross-section-relation",
            p8,
            "李四",
            any(&["李四"], &["王二", "张三"]),
            t8,
        ),
        (
            3,
            1,
            "distractor-rejection",
            p9,
            "7件",
            any(&["7件", "7"], &["12件", "12"]),
            t9,
        ),
        (
            3,
            2,
            "distractor-rejection",
            p10,
            "4.5元",
            any(&["4.5", "四点五"], &["5.2"]),
            t10,
        ),
        (
            3,
            3,
            "distractor-rejection",
            p11,
            "陈晨",
            any(&["陈晨"], &["张三"]),
            t11,
        ),
        (
            3,
            4,
            "distractor-rejection",
            p12,
            "2024-03-18",
            any(&["2024-03-18", "3月18日"], &[]),
            t12,
        ),
        (
            4,
            1,
            "length-variation",
            p13,
            "7件",
            any(&["7件", "7"], &["12"]),
            t13,
        ),
        (
            4,
            2,
            "length-variation",
            p14,
            "7件",
            any(&["7件", "7"], &["12"]),
            t14,
        ),
        (
            4,
            3,
            "length-variation",
            p15,
            "7件",
            any(&["7件", "7"], &["12"]),
            t15,
        ),
        (
            4,
            4,
            "length-variation",
            p16,
            "7件",
            any(&["7件", "7"], &["12"]),
            t16,
        ),
        (
            5,
            1,
            "in-document-calculation",
            p17,
            "24件",
            any(&["24件", "24"], &["19件"]),
            t17,
        ),
        (
            5,
            2,
            "in-document-calculation",
            p18,
            "4条",
            any(&["4条", "4", "四"], &["3条", "5条"]),
            t18,
        ),
        (
            5,
            3,
            "in-document-calculation",
            p19,
            "80件",
            any(&["80件", "80"], &[]),
            t19,
        ),
        (
            5,
            4,
            "in-document-calculation",
            p20,
            "60元",
            any(&["60元", "60"], &[]),
            t20,
        ),
    ];

    defs.into_iter()
        .map(|(si, qi, sub, prompt, expected, acceptance, tokens)| {
            sample(
                CapabilityCategory::LongContext,
                si,
                qi,
                sub,
                prompt,
                expected,
                acceptance,
                None,
                None,
                Some(tokens),
            )
        })
        .collect()
}

/// C06 逻辑推理与计算题库（20 题，方法借鉴 BBH 的确定性题族：每题一个
/// 可程序验证的唯一答案，含干扰事实）。子域序与 `subdomains()` 一致：
/// 1 条件判断 2 时序排序 3 数量比较 4 基础计算 5 多条件推导。
fn c06_bank() -> Vec<CapabilitySample> {
    use OutputConstraint as C;
    const UNIFIED: &str = "只依据题目条件推理，给出最终答案；不要使用外部知识。";

    let defs: Vec<(u32, u32, &str, String, &str, AcceptanceRule)> = vec![
        // ---- 子域 1：条件判断（4 题）----
        (
            1,
            1,
            "condition-judgement",
            format!(
                "条件：温度高于30℃或湿度高于80%时触发预警。今天温度28℃、湿度85%。任务：今天是否触发预警？{UNIFIED}"
            ),
            "触发",
            any(&["触发", "是"], &["不触发", "不预警"]),
        ),
        (
            1,
            2,
            "condition-judgement",
            format!(
                "条件：报销金额不超过2000元免审批。差旅费报销1800元。任务：这笔报销是否需要审批？{UNIFIED}"
            ),
            "不需要",
            any(&["不需要", "免审批", "不用"], &["需要审批", "需要总监"]),
        ),
        (
            1,
            3,
            "condition-judgement",
            format!(
                "条件：同时满足「预算低于50万」且「状态为进行中」的项目才可启动。项目甲预算30万、状态进行中。任务：项目甲是否可启动？{UNIFIED}"
            ),
            "可启动",
            any(&["可启动", "可以", "是"], &["不可启动", "不可以"]),
        ),
        (
            1,
            4,
            "condition-judgement",
            format!(
                "条件：同时满足「预算低于50万」且「状态为进行中」的项目才可启动。项目乙预算50万、状态已完成。任务：项目乙是否可启动？{UNIFIED}"
            ),
            "不可启动",
            any(&["不可启动", "不可以", "否"], &[]),
        ),
        // ---- 子域 2：时序排序（4 题）----
        (
            2,
            1,
            "temporal-order",
            format!(
                "条件：流程顺序固定为 提交→初审→复审→归档。任务：初审在第几步？回答阿拉伯数字。{UNIFIED}"
            ),
            "2",
            set(any(&["2", "二", "第二"], &[]), vec![C::MaxChars(4)]),
        ),
        (
            2,
            2,
            "temporal-order",
            format!("条件：甲先于乙完成，丙晚于乙但早于丁。任务：谁最后完成？{UNIFIED}"),
            "丁",
            any(&["丁"], &[]),
        ),
        (
            2,
            3,
            "temporal-order",
            format!(
                "条件：会议A在周二召开，会议B比A晚一天，会议C比A早一天。任务：会议C在周几召开？{UNIFIED}"
            ),
            "周一",
            any(&["周一", "星期一"], &["周二", "周三"]),
        ),
        (
            2,
            4,
            "temporal-order",
            format!(
                "条件：操作日志顺序为：先登记入库，再登记出库，最后盘点。任务：三个操作中哪个最后执行？{UNIFIED}"
            ),
            "盘点",
            any(&["盘点"], &["入库", "出库"]),
        ),
        // ---- 子域 3：数量比较（4 题）----
        (
            3,
            1,
            "quantity-comparison",
            format!(
                "条件：甲库存30件，乙比甲多15件，丙比乙少10件。任务：丙的库存是多少件？{UNIFIED}"
            ),
            "35",
            any(&["35"], &["25"]),
        ),
        (
            3,
            2,
            "quantity-comparison",
            format!("条件：A卖出12件，B的销量是A的2倍。任务：B卖出多少件？{UNIFIED}"),
            "24",
            any(&["24"], &["36"]),
        ),
        (
            3,
            3,
            "quantity-comparison",
            format!("条件：X队20人，Y队比X队少5人，Z队人数是Y队的2倍。任务：Z队多少人？{UNIFIED}"),
            "30",
            any(&["30"], &["40"]),
        ),
        (
            3,
            4,
            "quantity-comparison",
            format!(
                "条件：甲、乙、丙三个项目预算分别为30万、50万、20万。任务：哪个项目预算最少？{UNIFIED}"
            ),
            "丙",
            any(&["丙", "项目丙"], &[]),
        ),
        // ---- 子域 4：基础计算（4 题）----
        (
            4,
            1,
            "basic-calculation",
            format!("条件：项目预算45万，已支出28万。任务：剩余预算多少万？{UNIFIED}"),
            "17",
            any(&["17万", "17"], &["73"]),
        ),
        (
            4,
            2,
            "basic-calculation",
            format!("条件：3箱备件，每箱24件。任务：共多少件？{UNIFIED}"),
            "72",
            any(&["72"], &["27", "64"]),
        ),
        (
            4,
            3,
            "basic-calculation",
            format!("条件：某物料单价8元，采购15件，按9折结算。任务：应付多少元？{UNIFIED}"),
            "108",
            any(&["108"], &["112"]),
        ),
        (
            4,
            4,
            "basic-calculation",
            format!("条件：总额120万平均分给3个项目。任务：每个项目多少万？{UNIFIED}"),
            "40",
            any(&["40万", "40"], &["60", "30"]),
        ),
        // ---- 子域 5：多条件推导（4 题）----
        (
            5,
            1,
            "multi-condition-derivation",
            format!(
                "条件：甲拿红球，乙拿蓝球，丙拿绿球。甲与乙交换，随后甲与丙交换。任务：现在蓝球在谁手里？{UNIFIED}"
            ),
            "丙",
            any(&["丙"], &[]),
        ),
        (
            5,
            2,
            "multi-condition-derivation",
            format!("条件：A比B高，B比C高，C比D高。任务：四人中谁最矮？{UNIFIED}"),
            "D",
            any(&["D", "d"], &[]),
        ),
        (
            5,
            3,
            "multi-condition-derivation",
            format!(
                "条件：周一值班只能从小林和小周中选；小林不值周一。任务：周一谁值班？{UNIFIED}"
            ),
            "小周",
            any(&["小周"], &[]),
        ),
        (
            5,
            4,
            "multi-condition-derivation",
            format!("条件：一个数加5后再乘2等于20。任务：这个数是多少？{UNIFIED}"),
            "5",
            any(&["5"], &[]),
        ),
    ];

    defs.into_iter()
        .map(|(si, qi, sub, prompt, expected, acceptance)| {
            sample(
                CapabilityCategory::ReasoningAndMath,
                si,
                qi,
                sub,
                prompt,
                expected,
                acceptance,
                None,
                None,
                None,
            )
        })
        .collect()
}
/// 归一化：去空白、去 markdown 强调符（*、_）、小写。
/// 模型常用 "**3** 个项目" 这类加粗写法，装饰符会把 "3个" 隔开导致别名漏判。
fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<String>()
        .replace(['*', '_'], "")
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
    let mut out = String::with_capacity(value.len());
    let mut token = String::new();
    for ch in value.chars() {
        if ch.is_whitespace() {
            if !token.is_empty() {
                out.push_str(if token.starts_with("sk-") || token.starts_with("rk-") {
                    "[REDACTED]"
                } else {
                    token.as_str()
                });
                token.clear();
            }
            out.push(ch);
        } else {
            token.push(ch);
        }
    }
    if !token.is_empty() {
        out.push_str(if token.starts_with("sk-") || token.starts_with("rk-") {
            "[REDACTED]"
        } else {
            token.as_str()
        });
    }
    out
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
    fn freezes_six_categories_and_144_chinese_units() {
        let catalog = fixed_capability_catalog();
        assert_eq!(catalog.len(), 144);
        for category in CapabilityCategory::ALL {
            let expected = match category {
                CapabilityCategory::InstructionFollowing => 44,
                _ => 20,
            };
            assert_eq!(
                catalog
                    .iter()
                    .filter(|sample| sample.category == category)
                    .count(),
                expected
            );
        }
        assert!(
            catalog
                .iter()
                .all(|sample| sample.language == CAPABILITY_LANGUAGE)
        );
        assert!(catalog.iter().all(|sample| {
            !sample.prompt.contains("样本材料")
                && !sample.expected.contains(&sample.id)
                && !sample.prompt.contains("预设答案")
                && !sample.prompt.trim().is_empty()
                && !sample.expected.trim().is_empty()
        }));
        assert!(
            catalog
                .iter()
                .any(|sample| sample.prompt.contains("负责人") && sample.expected == "小林")
        );
        assert!(
            catalog
                .iter()
                .filter(|sample| sample.category == CapabilityCategory::ToolSelection)
                .all(|sample| sample.tools.is_some())
        );
        assert!(
            catalog
                .iter()
                .filter(|sample| sample.category == CapabilityCategory::MultiTurn)
                .all(|sample| sample.messages.as_ref().is_some_and(|m| m.len() >= 3))
        );
        assert!(
            catalog
                .iter()
                .filter(|sample| sample.category == CapabilityCategory::LongContext)
                .all(|sample| sample.input_tokens.is_some())
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
            messages: None,
            tools: None,
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
                truncated: false,
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
                    truncated: false,
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
                    truncated: false,
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
                    truncated: false,
                    evidence_refs: vec![format!("evidence://{}", sample.id)],
                },
                1 => CapabilityResponse {
                    execution: ExecutionState::Valid,
                    text: Some("错误答案".into()),
                    tool_calls: Vec::new(),
                    truncated: false,
                    evidence_refs: vec![format!("evidence://{}", sample.id)],
                },
                2 => CapabilityResponse {
                    execution: ExecutionState::Valid,
                    text: None,
                    tool_calls: Vec::new(),
                    truncated: false,
                    evidence_refs: vec![format!("evidence://{}", sample.id)],
                },
                3 => CapabilityResponse {
                    execution: ExecutionState::Invalid {
                        reason: "timeout".into(),
                    },
                    text: None,
                    tool_calls: Vec::new(),
                    truncated: false,
                    evidence_refs: vec![format!("evidence://{}", sample.id)],
                },
                _ => CapabilityResponse {
                    execution: ExecutionState::NotMeasured {
                        reason: "not selected".into(),
                    },
                    text: None,
                    tool_calls: Vec::new(),
                    truncated: false,
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
                        truncated: false,
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
        assert_eq!(summary.planned, 44);
        assert_eq!(
            summary.correct
                + summary.wrong
                + summary.pending
                + summary.incomplete
                + summary.missing,
            44
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
        assert_eq!(long_context.missing, 20);
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
                        truncated: false,
                        evidence_refs: vec!["evidence://other-run".into()],
                    },
                ),],
                settings(None),
            )
            .is_err()
        );
    }

    fn judged(acceptance: AcceptanceRule, text: Option<&str>) -> CapabilityObservation {
        let sample = CapabilitySample {
            id: "c01-test".into(),
            category: CapabilityCategory::InstructionFollowing,
            subdomain: "single-constraint".into(),
            dataset_identity: DatasetIdentity::ProductOriginal,
            source_ref: "test".into(),
            language: CAPABILITY_LANGUAGE.into(),
            prompt: "回答".into(),
            messages: None,
            tools: None,
            expected: "进行中".into(),
            acceptance,
            generation_settings: BTreeMap::new(),
            revision: CAPABILITY_VERSION.into(),
            input_tokens: None,
        };
        evaluate_response(
            &sample,
            CapabilityResponse {
                execution: ExecutionState::Valid,
                text: text.map(str::to_string),
                tool_calls: Vec::new(),
                truncated: false,
                evidence_refs: vec!["evidence://c01-test".into()],
            },
        )
    }

    #[test]
    fn contains_any_accepts_aliases_and_rejects_forbidden_values() {
        let rule = AcceptanceRule::ContainsAny {
            any_of: vec!["未提供".into(), "未知".into()],
            forbidden: vec!["王五".into()],
        };
        assert_eq!(
            judged(rule.clone(), Some("材料中未提供该信息")).label,
            ScoreLabel::Correct
        );
        assert_eq!(judged(rule.clone(), Some("王五")).label, ScoreLabel::Wrong);
        assert_eq!(judged(rule, None).label, ScoreLabel::Pending);
    }

    #[test]
    fn match_groups_requires_every_group_and_blocks_forbidden() {
        let rule = AcceptanceRule::MatchGroups {
            required_groups: vec![vec!["项目乙".into(), "乙".into()], vec!["项目丁".into()]],
            forbidden: vec!["甲".into()],
        };
        assert_eq!(
            judged(rule.clone(), Some("项目乙、项目丁")).label,
            ScoreLabel::Correct
        );
        assert_eq!(
            judged(rule.clone(), Some("项目乙")).label,
            ScoreLabel::Wrong
        );
        assert_eq!(
            judged(rule, Some("项目甲、项目乙、项目丁")).label,
            ScoreLabel::Wrong
        );
    }

    #[test]
    fn constraint_set_marks_content_right_constraint_missing_as_incomplete() {
        let rule = AcceptanceRule::ConstraintSet {
            content: Box::new(AcceptanceRule::ContainsAny {
                any_of: vec!["进行中".into()],
                forbidden: vec![],
            }),
            constraints: vec![OutputConstraint::MaxChars(10)],
        };
        let pass = judged(rule.clone(), Some("进行中"));
        assert_eq!(pass.label, ScoreLabel::Correct);
        let incomplete = judged(rule.clone(), Some("项目甲目前状态为进行中"));
        assert_eq!(incomplete.label, ScoreLabel::Incomplete);
        assert!(
            incomplete
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("不超过10字"))
        );
        assert_eq!(judged(rule, Some("已完成")).label, ScoreLabel::Wrong);
    }

    #[test]
    fn output_constraints_cover_each_verifiable_check() {
        let cases: Vec<(OutputConstraint, &str, bool)> = vec![
            (OutputConstraint::MaxChars(5), "进行中", true),
            (OutputConstraint::MaxChars(2), "进行中", false),
            (
                OutputConstraint::MustContain("风险".into()),
                "存在风险",
                true,
            ),
            (
                OutputConstraint::MustNotContain("预算".into()),
                "含预算",
                false,
            ),
            (
                OutputConstraint::KeywordMinCount("安全".into(), 2),
                "安全第一安全第二",
                true,
            ),
            (OutputConstraint::JsonObject, "{\"owner\":\"小林\"}", true),
            (OutputConstraint::JsonObject, "结果：{}", false),
            (
                OutputConstraint::JsonFieldCount(2),
                "{\"a\":1,\"b\":2}",
                true,
            ),
            (
                OutputConstraint::JsonRequiredFields(vec!["name".into(), "owner".into()]),
                "{\"name\":\"甲\",\"owner\":\"小林\"}",
                true,
            ),
            (
                OutputConstraint::JsonRequiredFields(vec!["name".into(), "owner".into()]),
                "{\"name\":\"甲\"}",
                false,
            ),
            (OutputConstraint::LineCount(2), "甲\n乙", true),
            (OutputConstraint::LineCount(2), "甲\n乙\n丙", false),
            (
                OutputConstraint::ParagraphCount(2),
                "第一段\n\n第二段",
                true,
            ),
            (OutputConstraint::WrappedTitle, "[[项目汇报]]", true),
            (OutputConstraint::WrappedTitle, "项目汇报", false),
            (
                OutputConstraint::StartsWith("结论：".into()),
                "结论：小林",
                true,
            ),
            (OutputConstraint::EndsWith("完毕".into()), "验收完毕", true),
            (OutputConstraint::NoChar('，'), "甲，乙", false),
            (OutputConstraint::QuotedOutput, "「小林」", true),
            (OutputConstraint::QuotedOutput, "小林", false),
            (
                OutputConstraint::ExactOrder(vec!["丙".into(), "甲".into()]),
                "项目丙、项目甲",
                true,
            ),
            (
                OutputConstraint::ExactOrder(vec!["丙".into(), "甲".into()]),
                "项目甲、项目丙",
                false,
            ),
        ];
        for (constraint, text, expected) in cases {
            assert_eq!(
                check_constraint(&constraint, text),
                expected,
                "{constraint:?} on {text:?}"
            );
        }
    }

    #[test]
    fn c01_bank_covers_five_subdomains_with_real_acceptance_rules() {
        let catalog = fixed_capability_catalog();
        let c01: Vec<_> = catalog
            .iter()
            .filter(|sample| sample.category == CapabilityCategory::InstructionFollowing)
            .collect();
        assert_eq!(c01.len(), 44);
        for (index, expected_count) in [8, 8, 12, 8, 8].iter().enumerate() {
            let prefix = format!("C01-{}-", index + 1);
            assert_eq!(
                c01.iter()
                    .filter(|sample| sample.id.starts_with(&prefix))
                    .count(),
                *expected_count,
                "subdomain {prefix}"
            );
        }
        assert!(c01.iter().all(|sample| matches!(
            sample.acceptance,
            AcceptanceRule::ContainsAny { .. }
                | AcceptanceRule::MatchGroups { .. }
                | AcceptanceRule::ConstraintSet { .. }
        )));
        let constrained = c01
            .iter()
            .filter(|sample| matches!(sample.acceptance, AcceptanceRule::ConstraintSet { .. }))
            .count();
        assert_eq!(constrained, 22);
    }

    fn judged_full(
        acceptance: AcceptanceRule,
        text: Option<&str>,
        truncated: bool,
    ) -> CapabilityObservation {
        let sample = CapabilitySample {
            id: "c01-test".into(),
            category: CapabilityCategory::InstructionFollowing,
            subdomain: "material-facts".into(),
            dataset_identity: DatasetIdentity::ProductOriginal,
            source_ref: "test".into(),
            language: CAPABILITY_LANGUAGE.into(),
            prompt: "回答".into(),
            messages: None,
            tools: None,
            expected: "王六".into(),
            acceptance,
            generation_settings: BTreeMap::new(),
            revision: CAPABILITY_VERSION.into(),
            input_tokens: None,
        };
        evaluate_response(
            &sample,
            CapabilityResponse {
                execution: ExecutionState::Valid,
                text: text.map(str::to_string),
                tool_calls: Vec::new(),
                truncated,
                evidence_refs: vec!["evidence://c01-test".into()],
            },
        )
    }

    #[test]
    fn truncated_response_without_answer_marker_is_pending_not_wrong() {
        let rule = AcceptanceRule::ContainsAny {
            any_of: vec!["王六".into()],
            forbidden: vec!["王五".into()],
        };
        let observation = judged_full(
            rule.clone(),
            Some("The user asks: \"材料：产品A负责人王五...\" So we answer: 产品B的负责人是"),
            true,
        );
        assert_eq!(observation.label, ScoreLabel::Pending);
        assert!(
            observation
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("截断"))
        );
        // 截断但已给出最终答案标记的，照常判分
        let observation = judged_full(rule, Some("推理过程略。最终答案：王六"), true);
        assert_eq!(observation.label, ScoreLabel::Correct);
    }

    #[test]
    fn answer_region_shields_forbidden_values_in_echo_and_explanation() {
        let rule = AcceptanceRule::ContainsAny {
            any_of: vec!["王六".into()],
            forbidden: vec!["王五".into()],
        };
        // 复述材料带出的禁含值不算违规；最终答案正确即通过
        let observation = judged_full(
            rule.clone(),
            Some("The user asks: \"产品A负责人王五，产品B负责人王六...\" 最终答案：王六"),
            false,
        );
        assert_eq!(observation.label, ScoreLabel::Correct);
        // 答案后的说明段提到排除项，不影响判定
        let groups = AcceptanceRule::MatchGroups {
            required_groups: vec![vec!["甲".into()], vec!["丙".into()]],
            forbidden: vec!["乙".into(), "丁".into()],
        };
        let observation = judged_full(
            groups,
            Some("最终答案：项目甲、项目丙。说明：项目乙超支，项目丁不符合。"),
            false,
        );
        assert_eq!(observation.label, ScoreLabel::Correct);
        // 无标记时取最后一个非空段落
        let observation = judged_full(
            rule,
            Some("思考过程提到王五作为干扰项。\n\n产品B的负责人是王六。"),
            false,
        );
        assert_eq!(observation.label, ScoreLabel::Correct);
    }

    fn judged_calls(acceptance: AcceptanceRule, calls: Vec<ToolCall>) -> CapabilityObservation {
        let sample = CapabilitySample {
            id: "c03-test".into(),
            category: CapabilityCategory::ToolSelection,
            subdomain: "multiple-tools".into(),
            dataset_identity: DatasetIdentity::ProductOriginal,
            source_ref: "test".into(),
            language: CAPABILITY_LANGUAGE.into(),
            prompt: "调用工具".into(),
            messages: None,
            tools: None,
            expected: "calls".into(),
            acceptance,
            generation_settings: BTreeMap::new(),
            revision: CAPABILITY_VERSION.into(),
            input_tokens: None,
        };
        evaluate_response(
            &sample,
            CapabilityResponse {
                execution: ExecutionState::Valid,
                text: None,
                tool_calls: calls,
                truncated: false,
                evidence_refs: vec!["evidence://c03-test".into()],
            },
        )
    }

    fn call(name: &str, args: &[(&str, &str)]) -> ToolCall {
        ToolCall {
            name: name.into(),
            arguments: args
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn tool_calls_match_consumes_duplicates_and_rejects_extras() {
        let two_weather = AcceptanceRule::ToolCallsMatch {
            expected_calls: vec![
                call("query_weather", &[("city", "北京")]),
                call("query_weather", &[("city", "上海")]),
            ],
        };
        // 正确：两个不同参数的同函数调用
        assert_eq!(
            judged_calls(
                two_weather.clone(),
                vec![
                    call("query_weather", &[("city", "北京")]),
                    call("query_weather", &[("city", "上海")]),
                ],
            )
            .label,
            ScoreLabel::Correct
        );
        // 错误：同一调用不能重复匹配两个期望
        assert_eq!(
            judged_calls(
                two_weather.clone(),
                vec![call("query_weather", &[("city", "北京")])],
            )
            .label,
            ScoreLabel::Wrong
        );
        // 错误：多出一次预期外的调用
        assert_eq!(
            judged_calls(
                two_weather.clone(),
                vec![
                    call("query_weather", &[("city", "北京")]),
                    call("query_weather", &[("city", "上海")]),
                    call("query_weather", &[("city", "北京")]),
                ],
            )
            .label,
            ScoreLabel::Wrong
        );
        // 错误：函数名不对
        assert_eq!(
            judged_calls(
                two_weather,
                vec![
                    call("query_weather", &[("city", "北京")]),
                    call("calculate", &[("expression", "1+1")]),
                ],
            )
            .label,
            ScoreLabel::Wrong
        );
        // 期望参数是子集：实际参数多出的字段不影响判定
        let subset = AcceptanceRule::ToolCallsMatch {
            expected_calls: vec![call("send_notification", &[("target", "小周")])],
        };
        assert_eq!(
            judged_calls(
                subset,
                vec![call(
                    "send_notification",
                    &[("target", "小周"), ("message", "下午三点开会")],
                )],
            )
            .label,
            ScoreLabel::Correct
        );
    }
}
