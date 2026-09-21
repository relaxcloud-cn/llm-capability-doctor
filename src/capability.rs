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
        if category == CapabilityCategory::InstructionFollowing {
            samples.extend(c01_bank());
            continue;
        }
        for (subdomain_index, subdomain) in category.subdomains().into_iter().enumerate() {
            for sample_index in 1..=4 {
                let id = format!(
                    "{}-{}-{:02}",
                    category.id(),
                    subdomain_index + 1,
                    sample_index
                );
                let (prompt, expected) = sample_definition(category, subdomain, sample_index);
                samples.push(CapabilitySample {
                    id: id.clone(),
                    category,
                    subdomain: subdomain.into(),
                    dataset_identity: DatasetIdentity::ProductOriginal,
                    source_ref: format!("agentcheck://capability/{id}"),
                    language: CAPABILITY_LANGUAGE.into(),
                    prompt,
                    expected: expected.clone(),
                    acceptance: AcceptanceRule::ExactAny {
                        accepted: vec![expected],
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

/// C01 文本理解与指令执行题库（44 题，方法借鉴 SQuAD 2.0 与 IFEval，
/// 题目全部为产品原创中文受控材料）。子域序与 `subdomains()` 一致：
/// 1 材料事实 2 条件筛选 3 单项约束 4 多项约束 5 信息不足。
fn c01_bank() -> Vec<CapabilitySample> {
    const UNIFIED: &str = "只依据题目给出的材料和条件回答。先给最终答案；同时满足题目列出的所有输出要求。材料没有提供的信息必须明确说明未知，不要使用外部知识补全。";
    const FACTS_AB: &str =
        "材料：项目甲由小林负责，预算30万，状态进行中；项目乙由小周负责，预算50万，状态已完成。";
    const FACTS_ABCD: &str = "材料：项目甲预算30万、华东、进行中；项目乙预算50万、华北、已完成；项目丙预算20万、华东、进行中；项目丁预算45万、华东、已完成。";
    const FACTS_SC: &str = "材料：项目甲预算30万、华东、进行中，负责人小林；项目乙预算50万、华北、已完成，负责人小周。";

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
                groups(&[&["进行中"], &["已完成", "已经完成", "全部完成"]], &[]),
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
            format!("{FACTS_SC}任务：说明项目乙的验收情况，回答必须以“完毕”结尾。{UNIFIED}"),
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
    const EXCLUSION: [&str; 9] = [
        "除", "不", "未", "非", "排除", "以外", "之外", "不符", "其他",
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
        AcceptanceRule::ContainsAll {
            required,
            forbidden,
        } => (
            answer
                .map(|text| {
                    let normalized = normalize(text);
                    if required
                        .iter()
                        .all(|value| normalized.contains(&normalize(value)))
                        && !forbidden_hit(&normalized, forbidden)
                    {
                        ScoreLabel::Correct
                    } else {
                        ScoreLabel::Wrong
                    }
                })
                .unwrap_or(ScoreLabel::Pending),
            None,
        ),
        AcceptanceRule::ContainsAny { any_of, forbidden } => (
            answer
                .map(|text| {
                    let normalized = normalize(text);
                    if any_of
                        .iter()
                        .any(|value| normalized.contains(&normalize(value)))
                        && !forbidden_hit(&normalized, forbidden)
                    {
                        ScoreLabel::Correct
                    } else {
                        ScoreLabel::Wrong
                    }
                })
                .unwrap_or(ScoreLabel::Pending),
            None,
        ),
        AcceptanceRule::MatchGroups {
            required_groups,
            forbidden,
        } => (
            answer
                .map(|text| {
                    let normalized = normalize(text);
                    if required_groups.iter().all(|group| {
                        group
                            .iter()
                            .any(|alias| normalized.contains(&normalize(alias)))
                    }) && !forbidden_hit(&normalized, forbidden)
                    {
                        ScoreLabel::Correct
                    } else {
                        ScoreLabel::Wrong
                    }
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
                    && required_arguments
                        .iter()
                        .all(|(key, value)| call.arguments.get(key) == Some(value))
            }) {
                ScoreLabel::Correct
            } else {
                ScoreLabel::Wrong
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

fn sample_definition(
    category: CapabilityCategory,
    subdomain: &str,
    sample_index: u32,
) -> (String, String) {
    let item = match sample_index {
        1 => ("甲", "项目甲"),
        2 => ("乙", "项目乙"),
        3 => ("丙", "项目丙"),
        _ => ("丁", "项目丁"),
    };
    match category {
        CapabilityCategory::InstructionFollowing => {
            unreachable!("C01 catalog is served by c01_bank()")
        }
        CapabilityCategory::InformationExtraction => match subdomain {
            "single-object-fields" => (
                format!("材料：{}，类型为服务，数量为{}。任务：只回答数量。", item.1, 10 + sample_index),
                (10 + sample_index).to_string(),
            ),
            "multi-object-relations" => (
                format!("材料：小林负责{}，小周负责其他项目。任务：只回答小林负责的项目。", item.1),
                item.1.into(),
            ),
            "field-types" => (
                "材料：库存数量为 12 件。任务：只回答数量和单位。".into(),
                "12件".into(),
            ),
            "missing-values" => ("材料：项目已登记，但没有填写负责人。任务：只回答负责人。".into(), "未提供".into()),
            _ => ("材料：项目甲旧名称为北区，新名称为东区，最新记录覆盖旧记录。任务：只回答最新名称。".into(), "东区".into()),
        },
        CapabilityCategory::ToolSelection => match subdomain {
            "tool-selection" => ("任务：查询北京今天的天气。可用工具：查询天气、计算。只回答应选择的工具名。".into(), "查询天气".into()),
            "argument-filling" => ("任务：查询北京今天的天气。只回答工具参数。".into(), "城市=北京，日期=今天".into()),
            "argument-types" => ("任务：计算 12 加 8。可用工具：查询、计算。只回答工具名和数字参数。".into(), "计算，数字=12和8".into()),
            "multiple-tools" => ("任务：先查询北京气温，再计算摄氏温度加 2。可用工具：查询天气、计算。只回答调用顺序。".into(), "查询天气→计算".into()),
            _ => ("任务：把一句话改写得更正式。可用工具：查询天气、计算。此任务不需要工具，只回答是否调用。".into(), "不调用工具".into()),
        },
        CapabilityCategory::MultiTurn => match subdomain {
            "condition-retention" => ("第1轮：项目甲预算100元，负责人小林。第2轮：请列出负责人。任务：只回答负责人。".into(), "小林".into()),
            "condition-update" => ("第1轮：项目甲预算100元。第2轮：预算改为120元。任务：只回答当前预算。".into(), "120元".into()),
            "condition-revocation" => ("第1轮：只列出华东项目。第2轮：取消地区限制。任务：说明当前是否还有地区限制。".into(), "没有地区限制".into()),
            "object-switching" => ("第1轮：项目甲负责人小林，项目乙负责人小周。第2轮：现在问项目乙负责人。任务：只回答姓名。".into(), "小周".into()),
            _ => ("第1轮：输出项目名和预算。第2轮：只把项目名改为列表格式，预算要求不变。任务：只回答项目甲及其100元预算。".into(), "项目甲，100元".into()),
        },
        CapabilityCategory::LongContext => match subdomain {
            "localization" => (
                format!(
                    "材料：开头有无关说明。第{}段写着：目标编号为 L{}。结尾有其他说明。任务：只回答目标编号。",
                    sample_index + 1,
                    sample_index
                ),
                format!("L{}", sample_index),
            ),
            "cross-section-relation" => ("材料前段：项目甲负责人小林。材料后段：小林所在团队为 T2。任务：只回答项目甲所在团队。".into(), "T2".into()),
            "distractor-rejection" => ("材料：项目甲团队 T2；项目乙团队 T9。问题：项目甲团队是什么？只回答团队。".into(), "T2".into()),
            "length-variation" => ("材料包含一段重复说明，唯一有效事实是：服务等级为标准。任务：只回答服务等级。".into(), "标准".into()),
            _ => ("材料：项目甲数量 7，项目乙数量 5。任务：只回答两项目数量之和。".into(), "12".into()),
        },
        CapabilityCategory::ReasoningAndMath => match subdomain {
            "condition-judgement" => ("已知：温度高于 30 度才需要预警；今天温度 32 度。任务：只回答是否预警。".into(), "是".into()),
            "temporal-order" => ("事件顺序：提交申请、审核、发布。任务：只回答审核发生在发布之前还是之后。".into(), "之前".into()),
            "quantity-comparison" => ("甲有 8 件，乙有 5 件。任务：只回答谁更多。".into(), "甲".into()),
            "basic-calculation" => (format!("任务：计算 {} + {}。只回答结果。", sample_index + 2, sample_index + 3), (2 * sample_index + 5).to_string()),
            _ => ("已知：甲比乙多 3，乙为 5，丙比甲少 2。任务：只回答丙的数值。".into(), "6".into()),
        },
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
                .any(|sample| sample.prompt.contains("项目甲数量") && sample.expected == "12")
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
}
