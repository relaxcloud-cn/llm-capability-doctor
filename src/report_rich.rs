//! 富 HTML 报告：一比一移植 GUI 的导出渲染
//! （`prototypes/agent-check-desktop/Sources/ReportExportHTML.swift` 与
//! `RealReportView.swift` 的目录/分组文案）。
//!
//! CLI 与 GUI 读同一份数据（run.json）、用同一套目录文案与样式，
//! 保证两边产出的报告一致：页面所见（GUI）即导出所得（CLI / GUI）。
//! 入参是整份 run.json 的 JSON 值，与 GUI 的读取路径完全相同。

use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write as _;

pub fn render_rich_html(run: &Value) -> String {
    let modules = test_module_ids()
        .into_iter()
        .filter(|id| selected_modules(run).contains(&id.to_string()))
        .collect::<Vec<_>>();
    let sections = modules.iter().map(|id| export_module(run, id)).collect::<Vec<_>>();

    let mut body = String::new();
    body.push_str(&hero_html(run));
    body.push_str(&toc_html(&sections));
    for section in &sections {
        body.push_str(&module_html(run, section));
    }
    body.push_str(&footer_html(run));

    format!(
        "<!doctype html>\n<html lang=\"zh-CN\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n<title>AgentCheck 检测报告 · {}</title>\n<style>\n{}</style>\n</head>\n<body>\n<main>\n{}</main>\n</body>\n</html>\n",
        escape(&model_name(run)),
        CSS,
        body
    )
}

// ---------- 数据形状（与 Swift ExportModule 对齐） ----------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Pass,
    Fail,
    Unverified,
}

#[derive(Clone)]
struct Group {
    id: String,
    name: String,
    pass_note: String,
    fail_note: String,
    how: String,
    judge: String,
}

struct Catalog {
    groups: Vec<Group>,
    unit_name: &'static str,
    desc: &'static str,
    scope: &'static str,
}

#[derive(Clone)]
struct Task<'a> {
    id: String,
    verdict: Verdict,
    verdict_text: String,
    note: String,
    item: Option<&'a Value>,
}

struct Grouped<'a> {
    group: Group,
    samples: Vec<Task<'a>>,
}

struct ExportModule<'a> {
    backend: &'static str,
    title: &'static str,
    headline: String,
    detail: String,
    meta: Option<String>,
    state: String,
    scope: Option<&'static str>,
    groups: Vec<Grouped<'a>>,
    unit_name: &'static str,
    fail_count: usize,
}

impl ExportModule<'_> {
    // 与页面 stateStyle 同一口径：模块级 fail 用红；有失败项用琥珀；pass 绿；其余灰。
    fn banner_class(&self) -> &'static str {
        if self.state == "fail" {
            return "v-block";
        }
        if self.fail_count > 0 {
            return "v-fail";
        }
        match self.state.as_str() {
            "pass" => "v-pass",
            "unsupported" | "inconclusive" | "invalid_execution" => "v-fail",
            _ => "v-unknown",
        }
    }
}

impl Grouped<'_> {
    fn fail_count(&self) -> usize {
        self.samples.iter().filter(|s| s.verdict == Verdict::Fail).count()
    }
    fn unverified_count(&self) -> usize {
        self.samples.iter().filter(|s| s.verdict == Verdict::Unverified).count()
    }
    fn all_pass(&self) -> bool {
        self.samples.iter().all(|s| s.verdict == Verdict::Pass)
    }
    fn correct(&self) -> usize {
        self.samples.iter().filter(|s| s.verdict == Verdict::Pass).count()
    }
    fn group_verdict(&self) -> Verdict {
        if self.fail_count() > 0 {
            Verdict::Fail
        } else if self.unverified_count() > 0 {
            Verdict::Unverified
        } else {
            Verdict::Pass
        }
    }
    fn note(&self, unit_name: &str) -> String {
        let total = self.samples.len();
        let fails = self.fail_count();
        if fails > 0 {
            let ratio = (total - fails) as f64 / total as f64;
            if ratio >= 0.9 {
                return format!("基本可用，个别{unit_name}没通过（{fails} {unit_name}）");
            }
            if ratio >= 0.7 {
                return format!("有 {fails} {unit_name}未通过，建议关注");
            }
            return self.group.fail_note.clone();
        }
        if self.unverified_count() > 0 {
            return format!("有 {} {unit_name}未能判定", self.unverified_count());
        }
        self.group.pass_note.clone()
    }
    fn count_text(&self, unit_name: &str) -> String {
        if unit_name == "题" {
            return format!("{} / {} 题正确", self.correct(), self.samples.len());
        }
        if self.all_pass() {
            format!("{} / {}", self.samples.len(), self.samples.len())
        } else {
            format!("{} / {}", self.correct(), self.samples.len())
        }
    }
}

// ---------- 顶层取值 ----------

fn record_object(run: &Value) -> &Value {
    run.get("record").unwrap_or(&Value::Null)
}

fn selected_modules(run: &Value) -> Vec<String> {
    run.get("selected_modules")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default()
}

fn test_module_ids() -> [&'static str; 5] {
    ["specification", "capability", "performance", "agent", "baseline"]
}

fn module_title(backend: &str) -> &'static str {
    match backend {
        "specification" => "模型规格实测",
        "capability" => "模型能力跑分",
        "performance" => "模型性能实测",
        "agent" => "智能体实测",
        "baseline" => "模型基线对比",
        _ => "检测模块",
    }
}

fn model_name(run: &Value) -> String {
    record_object(run)
        .pointer("/target/model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn endpoint(run: &Value) -> String {
    record_object(run)
        .pointer("/target/endpointFingerprint")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn created_at(run: &Value) -> String {
    record_object(run)
        .get("createdAt")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn record_evidence(run: &Value) -> Vec<&Value> {
    record_object(run)
        .get("evidence")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .collect()
}

fn module_states(run: &Value) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    if let Some(results) = record_object(run).get("moduleResults").and_then(Value::as_array) {
        for result in results {
            if let (Some(id), Some(state)) = (
                result.get("moduleId").and_then(Value::as_str),
                result.get("state").and_then(Value::as_str),
            ) {
                map.insert(id.to_owned(), state.to_owned());
            }
        }
    }
    map
}

// ---------- 模块数据管道（与 RealModuleView 同一算法） ----------

fn module_payload<'a>(run: &'a Value, backend: &str) -> Option<&'a Value> {
    record_evidence(run)
        .into_iter()
        .find(|item| {
            item.get("payload")
                .and_then(|payload| payload.get("module"))
                .and_then(Value::as_str)
                == Some(backend)
        })
        .and_then(|item| item.get("payload"))
        .and_then(|payload| payload.get("payload"))
}

fn nested_evidence<'a>(payload: &'a Value) -> Vec<&'a Value> {
    payload
        .get("evidence")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .collect()
}

/// 样本号/结构项号 -> 证据条目。智能体嵌套条目只有 evidence_id，要回查
/// record.evidence 里的会话记录；基线一条证据覆盖多个结构项（scenarios 数组）。
fn evidence_by_sample<'a>(run: &'a Value, backend: &str) -> BTreeMap<String, &'a Value> {
    let by_id: BTreeMap<&str, &Value> = record_evidence(run)
        .into_iter()
        .filter_map(|item| item.get("id").and_then(Value::as_str).map(|id| (id, item)))
        .collect();
    let mut map: BTreeMap<String, &Value> = BTreeMap::new();
    if let Some(payload) = module_payload(run, backend) {
        for item in nested_evidence(payload) {
            let mut resolved: &Value = item;
            if let Some(evidence_id) = item.get("evidence_id").and_then(Value::as_str) {
                if let Some(real) = by_id.get(evidence_id) {
                    resolved = real;
                }
            }
            let sample_id = item
                .get("sample_id")
                .and_then(Value::as_str)
                .or_else(|| resolved.get("payload").and_then(|p| p.get("sample_id")).and_then(Value::as_str));
            if let Some(sample_id) = sample_id {
                map.insert(sample_id.to_owned(), resolved);
            }
            if let Some(scenarios) = item.get("scenarios").and_then(Value::as_array) {
                for scenario in scenarios.iter().filter_map(Value::as_str) {
                    map.insert(scenario.to_owned(), resolved);
                }
            }
        }
    }
    for item in record_evidence(run) {
        let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
        let sample_id = item
            .get("payload")
            .and_then(|payload| payload.get("sample_id"))
            .and_then(Value::as_str);
        if !id.contains(&format!("-{backend}-")) {
            continue;
        }
        if let Some(sample_id) = sample_id {
            map.entry(sample_id.to_owned()).or_insert(item);
        }
    }
    map
}

fn verdict_text(verdict: Verdict, text: &str) -> (Verdict, String) {
    (verdict, text.to_owned())
}

fn spec_verdict(status: Option<&str>) -> (Verdict, String) {
    match status {
        Some("accepted") | Some("effective") => verdict_text(Verdict::Pass, "通过"),
        Some("verified_range") => verdict_text(Verdict::Pass, "部分验证"),
        Some("failed") => verdict_text(Verdict::Fail, "未通过"),
        Some("unsupported") => verdict_text(Verdict::Unverified, "不支持"),
        Some("inconclusive") => verdict_text(Verdict::Unverified, "无法判定"),
        Some("not_applicable") => verdict_text(Verdict::Unverified, "不适用"),
        _ => verdict_text(Verdict::Unverified, "已记录"),
    }
}

fn capability_verdict(label: Option<&str>) -> (Verdict, String) {
    match label {
        Some("correct") => verdict_text(Verdict::Pass, "答对"),
        Some("wrong") => verdict_text(Verdict::Fail, "答错"),
        Some("pending") => verdict_text(Verdict::Unverified, "未评分"),
        Some("incomplete") => verdict_text(Verdict::Unverified, "未完成"),
        Some("missing") => verdict_text(Verdict::Unverified, "缺失"),
        _ => verdict_text(Verdict::Unverified, "已记录"),
    }
}

fn performance_verdict(state: Option<&str>) -> (Verdict, String) {
    match state {
        Some("natural_end") | Some("completed") | Some("truncated") => {
            verdict_text(Verdict::Pass, "正常完成")
        }
        Some("error") => verdict_text(Verdict::Fail, "出错"),
        Some("timeout") => verdict_text(Verdict::Fail, "超时"),
        Some("cancelled") => verdict_text(Verdict::Unverified, "已取消"),
        Some("not_measured") => verdict_text(Verdict::Unverified, "未测到"),
        _ => verdict_text(Verdict::Unverified, "已记录"),
    }
}

fn agent_verdict(outcome: Option<&str>) -> (Verdict, String) {
    match outcome {
        Some("pass") => verdict_text(Verdict::Pass, "通过"),
        Some("fail") => verdict_text(Verdict::Fail, "未通过"),
        Some("inconclusive") => verdict_text(Verdict::Unverified, "无法判定"),
        Some("not_applicable") => verdict_text(Verdict::Unverified, "不适用"),
        Some("not_measured") => verdict_text(Verdict::Unverified, "未执行"),
        _ => verdict_text(Verdict::Unverified, "已记录"),
    }
}

fn baseline_verdict(status: Option<&str>) -> (Verdict, String) {
    match status {
        Some("same_structure") => verdict_text(Verdict::Pass, "一致"),
        Some("different") => verdict_text(Verdict::Fail, "有差异"),
        Some("not_observed") => verdict_text(Verdict::Unverified, "未观测"),
        Some("inconclusive") => verdict_text(Verdict::Unverified, "无法判定"),
        Some("not_applicable") => verdict_text(Verdict::Unverified, "不适用"),
        _ => verdict_text(Verdict::Unverified, "已记录"),
    }
}

fn agent_check_title(id: Option<&str>) -> &'static str {
    match id {
        Some("A1") => "遵守任务规则",
        Some("A2") => "工具与参数正确",
        Some("A3") => "使用工具返回",
        Some("A4") => "多轮状态保持",
        Some("A5") => "工具失败处理",
        Some("A6") => "信息缺失处理",
        Some("A7") => "操作权限边界",
        Some("A8") => "真实交付与结束",
        _ => "检查项",
    }
}

/// 逐条判定以报告权威字段为准：spec.observations / scorecard.observations /
/// perf.samples / agent.samples / baseline.comparisons。
fn task_specs<'a>(run: &'a Value, backend: &str) -> Vec<Task<'a>> {
    let payload = module_payload(run, backend);
    let report = payload.and_then(|p| p.get("report"));
    let scorecard = payload.and_then(|p| p.get("scorecard"));
    let by_sample = evidence_by_sample(run, backend);
    let mut tasks = Vec::new();
    match backend {
        "specification" => {
            let observations = report
                .and_then(|r| r.get("observations"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for observation in &observations {
                let (verdict, text) = spec_verdict(observation.get("status").and_then(Value::as_str));
                let mut note = observation
                    .get("limitation")
                    .and_then(Value::as_str)
                    .or_else(|| observation.get("verified_scope").and_then(Value::as_str))
                    .unwrap_or_default()
                    .to_owned();
                if observation.get("attempt").and_then(Value::as_str) == Some("recheck") {
                    note = if note.is_empty() {
                        "复核后判定".to_owned()
                    } else {
                        format!("复核：{note}")
                    };
                }
                let id = observation.get("sample_id").and_then(Value::as_str).unwrap_or_default();
                tasks.push(Task {
                    id: id.to_owned(),
                    verdict,
                    verdict_text: text,
                    note,
                    item: by_sample.get(id).copied(),
                });
            }
        }
        "capability" => {
            let observations = scorecard
                .and_then(|s| s.get("observations"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for observation in &observations {
                let (verdict, text) = capability_verdict(observation.get("label").and_then(Value::as_str));
                let id = observation.get("sample_id").and_then(Value::as_str).unwrap_or_default();
                tasks.push(Task {
                    id: id.to_owned(),
                    verdict,
                    verdict_text: text,
                    note: observation.get("reason").and_then(Value::as_str).unwrap_or_default().to_owned(),
                    item: by_sample.get(id).copied(),
                });
            }
        }
        "performance" => {
            let samples = report
                .and_then(|r| r.get("samples"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for sample in &samples {
                let warmup = sample.get("phase").and_then(Value::as_str) == Some("warmup");
                let (verdict, text) = if warmup {
                    (Verdict::Unverified, "预热".to_owned())
                } else {
                    performance_verdict(sample.get("terminal_state").and_then(Value::as_str))
                };
                let mut parts: Vec<String> = Vec::new();
                if warmup {
                    parts.push("预热请求，不计入正式结论".to_owned());
                }
                if let Some(limitation) = sample.get("limitation").and_then(Value::as_str) {
                    parts.push(limitation.to_owned());
                }
                if let Some(concurrency) = sample.get("target_concurrency").and_then(Value::as_i64) {
                    if concurrency > 1 {
                        parts.push(format!("并发档位 {concurrency}"));
                    }
                }
                let id = sample.get("id").and_then(Value::as_str).unwrap_or_default();
                tasks.push(Task {
                    id: id.to_owned(),
                    verdict,
                    verdict_text: text,
                    note: parts.join("；"),
                    item: by_sample.get(id).copied(),
                });
            }
        }
        "agent" => {
            let samples = report
                .and_then(|r| r.get("samples"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for sample in &samples {
                let (verdict, text) = agent_verdict(sample.get("outcome").and_then(Value::as_str));
                let failed_check = sample
                    .get("check_results")
                    .and_then(Value::as_array)
                    .and_then(|checks| {
                        checks
                            .iter()
                            .find(|check| check.get("status").and_then(Value::as_str) == Some("fail"))
                    })
                    .map(|check| {
                        format!(
                            "「{}」{}",
                            agent_check_title(check.get("check").and_then(Value::as_str)),
                            check.get("rationale").and_then(Value::as_str).unwrap_or_default()
                        )
                    });
                let mut note = failed_check
                    .or_else(|| {
                        sample
                            .get("limitations")
                            .and_then(Value::as_array)
                            .and_then(|items| items.first())
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .unwrap_or_default();
                if note.is_empty() && verdict == Verdict::Pass {
                    note = "各检查项均通过".to_owned();
                }
                let id = sample.get("sample_id").and_then(Value::as_str).unwrap_or_default();
                tasks.push(Task {
                    id: id.to_owned(),
                    verdict,
                    verdict_text: text,
                    note,
                    item: by_sample.get(id).copied(),
                });
            }
        }
        "baseline" => {
            let comparisons = report
                .and_then(|r| r.get("comparisons"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for comparison in &comparisons {
                let (verdict, text) = baseline_verdict(comparison.get("status").and_then(Value::as_str));
                let diffs = comparison
                    .get("differences")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|d| {
                                let detail = d.get("detail").and_then(Value::as_str)?;
                                let path = d.get("path").and_then(Value::as_str).unwrap_or_default();
                                Some(if path.is_empty() {
                                    detail.to_owned()
                                } else {
                                    format!("{path}：{detail}")
                                })
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let notes = comparison
                    .get("notes")
                    .and_then(Value::as_array)
                    .map(|items| items.iter().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<_>>())
                    .unwrap_or_default();
                let mut merged = diffs;
                merged.extend(notes);
                let note = merged.iter().take(2).cloned().collect::<Vec<_>>().join("；");
                let id = comparison.get("scenario").and_then(Value::as_str).unwrap_or_default();
                tasks.push(Task {
                    id: id.to_owned(),
                    verdict,
                    verdict_text: text,
                    note,
                    item: by_sample.get(id).copied(),
                });
            }
        }
        _ => {}
    }
    tasks
}

// ---------- 目录（分组、名称、检测方式/判定方式文案，与 GUI RealResultCatalog 一致） ----------

fn catalog(backend: &str) -> Option<Catalog> {
    match backend {
        "specification" => Some(Catalog {
            groups: vec![
                group("S01", "协议可接受上限",
                    "不同长度的请求都能被服务接受并正常回答",
                    "服务拒绝了部分长度的请求；超长业务内容需要先拆分或截断",
                    "向服务发送多个真实长度的输入（约 64K–512K token），观察每个档位是否被接受并完整返回。",
                    "全部档位被接受且回复完整 → 通过；任一档位被拒绝或回复截断 → 未通过；未取得有效回复 → 未判定。"),
                group("S04", "工具调用",
                    "都能按格式返回工具调用",
                    "有请求没有按格式返回工具调用；业务依赖工具调用的话需要先处理",
                    "构造五种工具调用场景（填齐各类参数、禁止调用、同一工具连调两次、两个不同工具、强制指定），检查返回的 tool_calls。",
                    "tool_calls 结构完整、参数可解析 → 通过；未按协议格式返回 → 未通过；服务不支持工具调用 → 未判定。"),
                group("S05", "结构化输出",
                    "JSON、按字段模板两种都能按格式输出",
                    "部分格式要求没有满足；程序直接解析返回值的场景需要适配",
                    "分别要求按 JSON 和按字段模板输出，验证返回是否严格符合声明格式。",
                    "返回可被对应格式解析 → 通过；格式不符或混入多余内容 → 未通过。"),
                group("S06", "消息与多轮输入",
                    "单轮、多轮、系统提示都能正确处理",
                    "部分消息形态处理不正确；多轮对话业务建议验证",
                    "发送四种消息组合（system+user、带历史回引、多标记区分、含 tool 角色），检查多轮消息形态是否被正确处理。",
                    "各消息形态均被正确处理 → 通过；报错或语义错乱 → 未通过。"),
                group("S07", "流式输出",
                    "流式返回能正常开始和正常结束",
                    "流式返回未能正常开始或结束；流式场景需要先排查",
                    "以 stream 模式请求文本与工具调用输出，观察分块流能否正常开始、正常结束。",
                    "流式正常开始且正常结束 → 通过；中途断流或协议错误 → 未通过；不支持流式 → 未判定。"),
            ],
            unit_name: "项",
            desc: "检查接口的基本收发能力：不同长度的请求能否被接受、工具调用能否按格式返回、JSON 和字段模板能否按格式输出、单轮与多轮消息能否正确处理、流式返回能否正常开始和结束。",
            scope: "以上结论来自固定检查项，覆盖日常使用的请求形态；它说明「这些常规用法没问题」，不代表该模型的能力上限。",
        }),
        "capability" => {
            let judge = "回答与参考答案一致 → 答对；不一致 → 答错；未作答或无法判分 → 未判定。";
            let category = |id: &str, name: &str, fail: &str| group(id, name, "这类题做得稳", fail,
                &format!("围绕{name}出固定题目，收集模型回答。"), judge);
            Some(Catalog {
                groups: vec![
                    category("C01", "文本理解与指令执行", "错得较多，重要指令建议复核"),
                    category("C02", "信息提取与结构化填写", "错得较多，提取结果建议复核"),
                    category("C03", "工具选择与参数填写", "近半数选择或参数出错，重点复核"),
                    category("C04", "多轮对话与条件承接", "错得较多，条件变化场景建议复核"),
                    category("C05", "长材料理解与信息利用", "错得较多，长材料结论建议复核"),
                    category("C06", "逻辑推理与计算", "错得最多；重要计算务必人工复核"),
                ],
                unit_name: "题",
                desc: "用固定题目逐题判分：文本理解与指令执行、信息提取与结构化填写、工具选择与参数填写、多轮对话与条件承接、长材料理解、逻辑推理与计算。",
                scope: "题目为固定题库，衡量「这类任务它做得稳不稳」，不是通用智力评分；同题不同次作答可能有小幅波动。",
            })
        }
        "performance" => {
            let judge = "测量值在正常范围且无错误 → 通过；出现超时、错误或明显异常 → 未通过；预热样本仅标注，不计入结论。";
            Some(Catalog {
                groups: vec![
                    group("P01", "首字响应时间",
                        "从发出请求到第一个字返回的等待正常", "部分请求的首字等待异常",
                        "实测从发出请求到收到第一个字的等待时间。", judge),
                    group("P02", "完整响应时间",
                        "完整生成一段回答的耗时正常", "部分请求耗时异常",
                        "实测从发出请求到回答完整生成的总耗时。", judge),
                    group("P03", "并发处理能力",
                        "不同并发档位下请求都能完成", "部分并发档位出现失败",
                        "在多个并发档位下同时发起请求，观察各档位完成情况。", judge),
                    group("P04", "持续运行稳定性",
                        "持续运行期间没有出现失败或明显变慢", "持续运行期间出现失败或明显变慢",
                        "持续发起一段时间的请求，观察是否出现失败或明显变慢。", judge),
                    group("P05", "长文本负载",
                        "不同长度的输入材料下表现正常", "长输入场景下表现异常",
                        "在不同长度的输入材料下实测响应表现。", judge),
                ],
                unit_name: "批",
                desc: "在不同负载下实测响应表现：首字等待时间、完整响应耗时、并发处理能力、持续运行稳定性、长文本输入。",
                scope: "结果来自本次检测时段的实际测量；不同时间、不同负载下数值会有波动。",
            })
        }
        "agent" => {
            let scenarios: [(&str, &str); 10] = [
                ("T1-A", "遵守任务规则"), ("T1-B", "处理外部注入"),
                ("T2-A", "选择正确工具"), ("T2-B", "校验路径与参数"),
                ("T3-A", "使用工具返回驱动下一步"), ("T3-B", "处理工具返回的信息缺失"),
                ("T4-A", "跨轮次保留状态"), ("T4-B", "跨轮次响应条件变化"),
                ("T5-A", "处理可恢复工具失败"), ("T5-B", "处理信息不足与提前结束"),
            ];
            Some(Catalog {
                groups: scenarios
                    .iter()
                    .map(|(id, name)| group(id, name,
                        "任务按预期完成", "任务没有按预期完成，展开可看卡在哪一步",
                        &format!("下发固定任务书「{name}」，记录模型的完整执行过程与最终交付。"),
                        "按任务预期完成 → 通过；未按预期完成 → 未通过；无有效执行 → 未判定（无效尝试不计为模型失败）。"))
                    .collect(),
                unit_name: "个",
                desc: "用固定任务考察智能体行为：遵守任务规则、抵御外部注入、选对工具、校验参数、用工具返回驱动下一步、跨轮次保留状态、处理可恢复错误、信息不足时的表现。",
                scope: "任务为固定场景，覆盖常见交付形态；不能穷尽所有真实业务。",
            })
        }
        "baseline" => {
            let items: [(&str, &str); 14] = [
                ("BC01", "响应外层"), ("BC02", "候选回复"), ("BC03", "回复消息"),
                ("BC04", "用量统计"), ("BC05", "细分用量"), ("BC06", "附加信息"),
                ("BC07", "工具调用结构"), ("BC08", "函数名称与参数"), ("BC09", "分块外层"),
                ("BC10", "增量候选"), ("BC11", "消息增量"), ("BC12", "工具调用增量"),
                ("BC13", "流式用量返回"), ("BC14", "错误对象"),
            ];
            Some(Catalog {
                groups: items
                    .iter()
                    .map(|(id, title)| group(id, title,
                        "返回结构与通用规范一致", "结构与通用规范不一致，展开可看差异",
                        &format!("发起真实请求，将返回中「{title}」相关字段与通用规范逐项对照。"),
                        "结构与规范一致 → 一致；存在差异 → 有差异；本次未观测到 → 未判定。"))
                    .collect(),
                unit_name: "项",
                desc: "把服务的实际返回与通用规范逐项对照：消息字段、结束原因、用量统计、工具调用结构、流式分块等——只看格式，不评内容质量。",
                scope: "只检查数据格式是否符合通用规范，不判断内容质量。",
            })
        }
        _ => None,
    }
}

fn group(
    id: &str, name: &str, pass_note: &str, fail_note: &str, how: &str, judge: &str,
) -> Group {
    Group {
        id: id.to_owned(),
        name: name.to_owned(),
        pass_note: pass_note.to_owned(),
        fail_note: fail_note.to_owned(),
        how: how.to_owned(),
        judge: judge.to_owned(),
    }
}

/// 小项号 -> 分组号（与 GUI catalog.sampleGroup 相同的规则）。
fn sample_group(backend: &str, sample_id: &str) -> Option<String> {
    match backend {
        "specification" => [
            ("context-64k", "S01"), ("context-128k", "S01"), ("context-256k", "S01"), ("context-512k", "S01"),
            ("tools-all-types", "S04"), ("tools-none", "S04"), ("tools-same-twice", "S04"),
            ("tools-two-distinct", "S04"), ("tools-forced", "S04"),
            ("json", "S05"), ("schema", "S05"),
            ("M01", "S06"), ("M02", "S06"), ("M03", "S06"), ("M04", "S06"),
            ("stream-text", "S07"), ("stream-tool", "S07"),
        ]
        .iter()
        .find(|(known, _)| *known == sample_id)
        .map(|(_, group)| group.to_string()),
        "capability" => sample_id.split('-').next().map(str::to_owned),
        "performance" => sample_id.split('-').next().map(str::to_owned),
        "agent" => {
            let raw = sample_id.strip_prefix("agent-").unwrap_or(sample_id);
            let upper = raw.to_uppercase();
            if upper.len() == 3 {
                Some(format!("{}-{}", &upper[..2], &upper[2..]))
            } else {
                Some(upper)
            }
        }
        "baseline" => {
            let upper = sample_id.to_uppercase();
            if upper.starts_with("BC") {
                Some(upper.chars().take(4).collect())
            } else {
                None
            }
        }
        _ => None,
    }
}

fn specification_names() -> Vec<(&'static str, &'static str)> {
    vec![
        ("context-64k", "约 64K token 的真实长度输入"),
        ("context-128k", "约 128K token 的真实长度输入"),
        ("context-256k", "约 256K token 的真实长度输入"),
        ("context-512k", "约 512K token 的真实长度输入"),
        ("tools-all-types", "单次调用填齐各类参数"),
        ("tools-none", "禁止工具调用时直接回答"),
        ("tools-same-twice", "同一工具调用两次且参数不串"),
        ("tools-two-distinct", "两个不同工具各调一次"),
        ("tools-forced", "强制调用指定工具"),
        ("M01", "system+user 角色组合"),
        ("M02", "带历史回引暗号"),
        ("M03", "多条历史里区分目标标记"),
        ("M04", "含 tool 角色的消息序列"),
        ("json", "要求 JSON 回答"),
        ("schema", "要求按字段模板回答"),
        ("stream-text", "流式文本输出"),
        ("stream-tool", "流式工具调用输出"),
    ]
}

/// 分组内小项的人话名称（与 GUI catalog.sampleName 相同的规则）。
fn sample_name(backend: &str, index: usize, sample_id: &str) -> String {
    match backend {
        "specification" => specification_names()
            .into_iter()
            .find(|(known, _)| *known == sample_id)
            .map(|(_, name)| name.to_owned())
            .unwrap_or_else(|| format!("第 {index} 项检查")),
        "capability" => format!("第 {index} 题"),
        "performance" => format!("第 {index} 批请求"),
        "agent" => "完整任务过程".to_owned(),
        "baseline" => "该结构项的实测返回".to_owned(),
        _ => format!("第 {index} 项检查"),
    }
}

// ---------- curl / 原始返回（与 GUI 同一算法：请求体取证据 request，响应体取 response.body） ----------

fn request_json(item: &Value) -> Option<&Value> {
    let payload = item.get("payload").unwrap_or(item);
    let nested = payload.get("payload").unwrap_or(payload);
    nested
        .get("request")
        .or_else(|| payload.get("request"))
        .filter(|value| value.is_object())
}

fn response_body(item: &Value) -> Option<&Value> {
    let payload = item.get("payload").unwrap_or(item);
    let nested = payload.get("payload").unwrap_or(payload);
    let response = nested.get("response").or_else(|| payload.get("response"))?;
    response.get("body").filter(|value| value.is_object())
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// 原始 curl 命令：请求体取证据里的 request 对象；智能体任务是任务书不是单次 HTTP，返回空。
fn curl_text(run: &Value, backend: &str, item: &Value) -> String {
    if backend == "agent" {
        return String::new();
    }
    let body = request_json(item).map(pretty_json).unwrap_or_default();
    if body.is_empty() {
        return String::new();
    }
    curl_command(&endpoint(run), &body)
}

fn curl_command(url: &str, body_json: &str) -> String {
    format!(
        "curl -sS -X POST \"{url}\" \\\n  -H \"Content-Type: application/json\" \\\n  -H \"Authorization: Bearer $API_KEY\" \\\n  -d '{body_json}'"
    )
}

// ---------- 模块装配 ----------

fn export_module<'a>(run: &'a Value, backend: &'static str) -> ExportModule<'a> {
    let catalog = catalog(backend);
    let tasks = task_specs(run, backend);
    let state = module_states(run)
        .get(backend)
        .cloned()
        .unwrap_or_else(|| "unverified".to_owned());

    let fail_count = tasks.iter().filter(|t| t.verdict == Verdict::Fail).count();
    let unverified_count = tasks.iter().filter(|t| t.verdict == Verdict::Unverified).count();

    let headline = if fail_count > 0 {
        "有问题".to_owned()
    } else {
        match state.as_str() {
            "pass" => if unverified_count > 0 { "通过，部分项未判定" } else { "通过" }.to_owned(),
            "fail" => "有问题".to_owned(),
            "unsupported" => "不支持".to_owned(),
            "invalid_execution" => "本次检测未完成".to_owned(),
            "inconclusive" => "证据不足，暂不能判断".to_owned(),
            "not_applicable" => "不适用".to_owned(),
            "not_selected" => "本次未选".to_owned(),
            "unverified" => "缺少判定证据".to_owned(),
            _ => "尚未检测".to_owned(),
        }
    };

    let unit_name = catalog.as_ref().map(|c| c.unit_name).unwrap_or("项");
    let banner_meta = if tasks.is_empty() {
        None
    } else if fail_count == 0 && unverified_count == 0 {
        Some(format!("{} {unit_name}全部正常 · 每项都是真实调用", tasks.len()))
    } else {
        let pass_count = tasks.len() - fail_count - unverified_count;
        let mut parts = vec![format!("{pass_count} 项正常")];
        if fail_count > 0 {
            parts.push(format!("{fail_count} 项未通过"));
        }
        if unverified_count > 0 {
            parts.push(format!("{unverified_count} 项未判定"));
        }
        Some(parts.join(" · "))
    };

    let mut grouped: Vec<Grouped> = Vec::new();
    if let Some(catalog) = &catalog {
        let mut buckets: BTreeMap<String, Vec<Task>> = BTreeMap::new();
        for task in tasks {
            let group_id = sample_group(backend, &task.id).unwrap_or_else(|| "_other".to_owned());
            buckets.entry(group_id).or_default().push(task);
        }
        for group in &catalog.groups {
            if let Some(samples) = buckets.get(&group.id) {
                grouped.push(Grouped { group: group.clone(), samples: samples.to_vec() });
            }
        }
        if let Some(others) = buckets.get("_other") {
            if !others.is_empty() {
                grouped.push(Grouped {
                    group: group("_other", "其他检查", "全部正常", "有未通过的检查",
                        "该检查项在本次检测中产生了记录。", "按返回状态判定。"),
                    samples: others.to_vec(),
                });
            }
        }
    }

    ExportModule {
        backend,
        title: module_title(backend),
        headline,
        detail: catalog.as_ref().map(|c| c.desc.to_owned()).unwrap_or_else(|| {
            record_object(run)
                .get("moduleResults")
                .and_then(Value::as_array)
                .and_then(|results| {
                    results.iter().find(|result| result.get("moduleId").and_then(Value::as_str) == Some(backend))
                })
                .and_then(|result| result.get("reason").and_then(Value::as_str))
                .map(str::to_owned)
                .unwrap_or_else(|| "该模块已完成真实执行；详细任务证据见下方。".to_owned())
        }),
        meta: banner_meta,
        state,
        scope: catalog.as_ref().map(|c| c.scope),
        unit_name,
        fail_count,
        groups: grouped,
    }
}

// ---------- HTML 片段（与 GUI ReportExportHTML.swift 相同结构） ----------

fn hero_html(run: &Value) -> String {
    let kind = run
        .pointer("/customer_conclusion/kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let (class, title) = match kind {
        "normal_use" => ("v-pass", "可以正常使用"),
        "limited_use" => ("v-fail", "可以使用，存在限制"),
        "cannot_use" => ("v-block", "目前不能正常使用"),
        _ => ("v-unknown", "暂不能判断是否可用"),
    };
    let explanation = run
        .pointer("/customer_conclusion/text")
        .and_then(Value::as_str)
        .unwrap_or("本页展示检测程序产生的状态摘要；完整请求、事件、模块状态和限制保存在导出报告中。")
        .to_owned();
    let selected = selected_modules(run).len();
    let completed = record_object(run)
        .get("moduleResults")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    format!(
        "<header class=\"hero {class}\">\n  <div class=\"hero-head\">\n    <span class=\"hero-ico\">{icon}</span>\n    <div>\n      <p class=\"eyebrow\">AgentCheck 模型检测报告</p>\n      <h1>{title}</h1>\n      <p class=\"explain\">{explanation}</p>\n    </div>\n  </div>\n  <dl class=\"meta-bar\">\n    <div><dt>模型</dt><dd>{model}</dd></div>\n    <div><dt>服务地址</dt><dd>{endpoint}</dd></div>\n    <div><dt>检测时间</dt><dd>{created}</dd></div>\n    <div><dt>模块完成</dt><dd>{completed}/{selected}</dd></div>\n  </dl>\n</header>\n",
        icon = symbol_for(class),
        title = escape(&title),
        explanation = escape(&explanation),
        model = escape(&model_name(run)),
        endpoint = escape(&endpoint(run)),
        created = escape(&created_at(run)),
    )
}

fn toc_html(sections: &[ExportModule<'_>]) -> String {
    let mut chips = String::new();
    for section in sections {
        let _ = write!(
            chips,
            "<a class=\"chip {klass}\" href=\"#mod-{title}\">\n  <span class=\"chip-ico\">{icon}</span>\n  <span class=\"chip-body\">\n    <span class=\"chip-name\">{title}</span>\n    <span class=\"chip-head\">{headline}</span>\n  </span>\n</a>\n",
            klass = section.banner_class(),
            icon = symbol_for(section.banner_class()),
            title = escape(section.title),
            headline = escape(&section.headline),
        );
    }
    format!(
        "<nav class=\"toc\">\n  <h2>这次检测的结论一览</h2>\n  <div class=\"chips\">\n{chips}  </div>\n</nav>\n"
    )
}

fn module_html(run: &Value, section: &ExportModule<'_>) -> String {
    let mut groups = String::new();
    for grouped in &section.groups {
        let mut samples = String::new();
        for (index, sample) in grouped.samples.iter().enumerate() {
            samples.push_str(&sample_html(run, section, grouped, index, sample));
        }
        let _ = write!(
            groups,
            "      <div class=\"group\">\n        <div class=\"group-head {klass}\">\n          <span class=\"dot\">●</span>\n          <span class=\"gname\">{name}</span>\n          <span class=\"count\">{count}</span>\n        </div>\n        <p class=\"gnote\">{note}</p>\n{samples}      </div>\n",
            klass = verdict_class(grouped.group_verdict()),
            name = escape(&grouped.group.name),
            count = escape(&grouped.count_text(section.unit_name)),
            note = escape(&grouped.note(section.unit_name)),
        );
    }
    if groups.is_empty() {
        groups = "<p class=\"gnote empty\">没有找到该模块的任务级证据。</p>".to_owned();
    }
    let meta = section
        .meta
        .as_ref()
        .map(|m| format!("<span class=\"banner-meta\">{}</span>", escape(m)))
        .unwrap_or_default();
    let scope = section.scope.map(|s| format!("<p class=\"scope\">{}</p>", escape(s))).unwrap_or_default();
    format!(
        "<section class=\"module\" id=\"mod-{title}\">\n  <div class=\"banner {klass}\">\n    <div class=\"banner-head\">\n      <span class=\"banner-ico\">{icon}</span>\n      <h2>{title}：{headline}</h2>\n      {meta}\n    </div>\n    <p class=\"banner-desc\">{detail}</p>\n  </div>\n  <div class=\"card\">\n{groups}  </div>\n{scope}</section>\n",
        title = escape(section.title),
        klass = section.banner_class(),
        icon = symbol_for(section.banner_class()),
        headline = escape(&section.headline),
        detail = escape(&section.detail),
    )
}

fn sample_html(
    run: &Value, section: &ExportModule<'_>, grouped: &Grouped<'_>, index: usize, sample: &Task<'_>,
) -> String {
    let name = sample_name(section.backend, index + 1, &sample.id);
    let judge = if sample.note.is_empty() {
        grouped.group.judge.clone()
    } else {
        format!("{}\n本次结果：{}", grouped.group.judge, sample.note)
    };
    let curl = sample
        .item
        .map(|item| curl_text(run, section.backend, item))
        .unwrap_or_default();
    let raw_response = sample
        .item
        .and_then(response_body)
        .map(pretty_json)
        .unwrap_or_default();
    let is_task_style = curl.is_empty();
    let request_title = if is_task_style { "查看原始请求" } else { "查看原始 curl 请求" };
    let response_title = if is_task_style { "任务执行记录（JSON）" } else { "原始返回（JSON）" };
    let response_value = if raw_response.is_empty() {
        if is_task_style { "本次没有取得任务执行记录" } else { "本次没有取得响应体" }
    } else {
        &raw_response
    };
    let request_value = if is_task_style {
        &sample.request_text()
    } else {
        &curl
    };
    format!(
        "  <div class=\"sample\">\n    <div class=\"sample-head {klass}\">\n      <span class=\"pill\">{verdict}</span>\n      <span class=\"sname\">{name}</span>\n    </div>\n    <div class=\"info\"><h4>检测方式</h4><p>{how}</p></div>\n    <div class=\"info\"><h4>判定方式</h4><p>{judge}</p></div>\n    <details><summary>{request_title}</summary><pre>{request}</pre></details>\n    <details><summary>{response_title}</summary><pre>{response}</pre></details>\n  </div>\n",
        klass = verdict_class(sample.verdict),
        verdict = escape(&sample.verdict_text),
        name = escape(&name),
        how = escape(&grouped.group.how),
        judge = escape(&judge),
        request_title = request_title,
        request = escape(request_value),
        response_title = response_title,
        response = escape(response_value),
    )
}

// 任务式（智能体）行的「原始请求」：证据里的任务书文本。
impl Task<'_> {
    fn request_text(&self) -> String {
        let Some(item) = &self.item else { return String::new() };
        let payload = item.get("payload").unwrap_or(item);
        let nested = payload.get("payload").unwrap_or(payload);
        let request = nested.get("request").or_else(|| payload.get("request"));
        request
            .and_then(|request| request.get("prompt").and_then(Value::as_str))
            .map(str::to_owned)
            .unwrap_or_else(|| "未记录请求内容".to_owned())
    }
}

fn footer_html(run: &Value) -> String {
    format!(
        "<footer>\n  <p>本报告由 AgentCheck 桌面端导出，与报告页同源生成；判定口径以检测程序为准。</p>\n  <p class=\"meta\">检测时间 {created} · 模型 {model}</p>\n</footer>\n",
        created = escape(&created_at(run)),
        model = escape(&model_name(run)),
    )
}

fn verdict_class(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Pass => "v-pass",
        Verdict::Fail => "v-fail",
        Verdict::Unverified => "v-unknown",
    }
}

fn symbol_for(class: &str) -> &'static str {
    match class {
        "v-pass" => "✔",
        "v-fail" => "!",
        "v-block" => "✕",
        _ => "?",
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ---------- 样式（与 GUI ReportExportHTML.swift 的 CSS 一致） ----------

const CSS: &str = r#":root{
  --ink:#1b2733;--muted:#5d6b78;--faint:#8d99a5;--line:#e3e9ef;--canvas:#f5f7f9;
  --accent:#1769aa;
  --pass:#1c7f47;--pass-bg:#e9f6ef;
  --warn:#9c5d08;--warn-bg:#fdf3e3;
  --block:#b3352c;--block-bg:#fdefee;
  --unknown:#6f7b86;--unknown-bg:#eef1f4;
}
*{box-sizing:border-box}
body{margin:0;background:var(--canvas);color:var(--ink);font:14px/1.75 -apple-system,BlinkMacSystemFont,"PingFang SC","Segoe UI",sans-serif;-webkit-font-smoothing:antialiased}
main{max-width:920px;margin:0 auto;padding:36px 22px 56px}
.hero{background:#fff;border:1px solid var(--line);border-radius:14px;overflow:hidden;margin-bottom:16px}
.hero-head{display:flex;gap:18px;align-items:flex-start;padding:28px 30px 22px}
.hero-ico{flex:none;width:46px;height:46px;border-radius:50%;display:flex;align-items:center;justify-content:center;font-size:21px;font-weight:700;margin-top:4px}
.hero h1{margin:2px 0 6px;font-size:25px;line-height:1.4;letter-spacing:.2px}
.eyebrow{margin:0;font-size:11px;font-weight:600;letter-spacing:.22em;color:var(--faint)}
.explain{margin:0;color:var(--muted);font-size:14px;max-width:640px}
.meta-bar{display:grid;grid-template-columns:repeat(4,1fr);gap:0;margin:0;padding:14px 30px;border-top:1px solid var(--line);background:#fafcfd}
.meta-bar div{padding:0 18px;border-left:1px solid var(--line)}
.meta-bar div:first-child{border-left:none;padding-left:0}
.meta-bar dt{font-size:11px;color:var(--faint);letter-spacing:.08em;margin:0 0 2px}
.meta-bar dd{margin:0;font-size:12.5px;font-weight:600;color:var(--ink);word-break:break-all}
.toc{margin-bottom:30px}
.toc h2{margin:0 0 10px;font-size:13px;color:var(--muted);letter-spacing:.06em}
.chips{display:grid;grid-template-columns:repeat(auto-fill,minmax(262px,1fr));gap:10px}
.chip{display:flex;gap:11px;align-items:center;padding:13px 15px;background:#fff;border:1px solid var(--line);border-radius:11px;text-decoration:none;color:var(--ink);transition:border-color .15s}
.chip:hover{border-color:var(--accent)}
.chip-ico{flex:none;width:26px;height:26px;border-radius:50%;display:flex;align-items:center;justify-content:center;font-size:12.5px;font-weight:700}
.chip-body{min-width:0}
.chip-name{display:block;font-size:13px;font-weight:600;line-height:1.4}
.chip-head{display:block;font-size:11.5px;color:var(--muted);line-height:1.5}
.module{margin-bottom:34px}
.banner{border-radius:12px;padding:16px 20px 15px;border:1px solid transparent;border-left-width:4px}
.banner-head{display:flex;align-items:center;gap:10px;flex-wrap:wrap}
.banner-ico{flex:none;width:24px;height:24px;border-radius:50%;display:flex;align-items:center;justify-content:center;font-size:12px;font-weight:700}
.banner h2{margin:0;font-size:16.5px;font-weight:650;letter-spacing:.2px}
.banner-meta{margin-left:auto;font-size:11.5px;padding:3px 10px;border-radius:99px;font-variant-numeric:tabular-nums}
.banner-desc{margin:7px 0 2px 34px;font-size:13px}
.card{background:#fff;border:1px solid var(--line);border-radius:12px;padding:8px 20px 4px}
.group{padding:14px 0 10px;border-bottom:1px solid var(--line)}
.group:last-child{border-bottom:none}
.group-head{display:flex;align-items:center;gap:9px;padding:8px 12px;border-radius:8px;background:#f7fafc}
.dot{font-size:9px;line-height:1}
.gname{font-weight:600;font-size:13.5px}
.count{margin-left:auto;font-size:12px;color:var(--muted);font-variant-numeric:tabular-nums}
.gnote{margin:8px 2px 4px;color:var(--muted);font-size:12px}
.gnote.empty{padding:16px 2px}
.sample{margin:4px 0 12px 5px;padding:2px 0 0 15px;border-left:2px solid #e8edf2}
.sample-head{display:flex;align-items:center;gap:9px;margin:10px 0 9px}
.sname{font-size:13px;font-weight:500}
.pill{flex:none;font-size:11px;font-weight:600;padding:2px 10px;border-radius:99px}
.info{margin:0 0 9px}
.info h4{margin:0 0 2px;font-size:11px;font-weight:600;color:var(--faint);letter-spacing:.1em}
.info p{margin:0;font-size:12.5px;white-space:pre-wrap}
details{margin:7px 0}
summary{list-style:none;cursor:pointer;user-select:none;display:inline-flex;align-items:center;gap:5px;font-size:12px;font-weight:500;color:var(--accent)}
summary::-webkit-details-marker{display:none}
summary::before{content:"▸";font-size:10px;transition:transform .15s}
details[open] summary::before{transform:rotate(90deg)}
pre{margin:7px 0 10px;padding:11px 13px;background:#f8fafb;border:1px solid #e8edf2;border-radius:8px;font:11px/1.6 ui-monospace,SFMono-Regular,Menlo,monospace;overflow-x:auto;white-space:pre-wrap;word-break:break-all}
.scope{margin:9px 2px 0;color:var(--faint);font-size:11.5px}
footer{margin-top:6px;padding-top:14px;border-top:1px solid var(--line);color:var(--faint);font-size:11.5px}
footer p{margin:0 0 3px}
.v-pass .hero-ico,.v-pass .chip-ico,.v-pass .banner-ico{color:var(--pass);background:var(--pass-bg)}
.v-pass .dot{color:var(--pass)}
.v-pass .pill{color:var(--pass);background:var(--pass-bg)}
.banner.v-pass{background:#f4fbf7;border-color:#d5eadf}
.banner.v-pass .banner-meta{color:var(--pass);background:var(--pass-bg)}
.hero.v-pass{background:linear-gradient(180deg,#f2faf5 0%,#fff 100%)}
.hero.v-pass .hero-ico{background:var(--pass);color:#fff}
.v-fail .hero-ico,.v-fail .chip-ico,.v-fail .banner-ico{color:var(--warn);background:var(--warn-bg)}
.v-fail .dot{color:var(--warn)}
.v-fail .pill{color:var(--warn);background:var(--warn-bg)}
.banner.v-fail{background:#fdf9f1;border-color:#f2e3c4}
.banner.v-fail .banner-meta{color:var(--warn);background:var(--warn-bg)}
.hero.v-fail{background:linear-gradient(180deg,#fdf8ef 0%,#fff 100%)}
.hero.v-fail .hero-ico{background:var(--warn);color:#fff}
.v-block .hero-ico,.v-block .chip-ico,.v-block .banner-ico{color:var(--block);background:var(--block-bg)}
.v-block .pill{color:var(--block);background:var(--block-bg)}
.banner.v-block{background:#fdf4f3;border-color:#f1d2cf}
.banner.v-block .banner-meta{color:var(--block);background:var(--block-bg)}
.banner.v-block{border-left-color:var(--block)}
.hero.v-block{background:linear-gradient(180deg,#fdf3f2 0%,#fff 100%)}
.hero.v-block .hero-ico{background:var(--block);color:#fff}
.v-unknown .hero-ico,.v-unknown .chip-ico,.v-unknown .banner-ico{color:var(--unknown);background:var(--unknown-bg)}
.v-unknown .dot{color:var(--unknown)}
.v-unknown .pill{color:var(--unknown);background:var(--unknown-bg)}
.banner.v-unknown{background:#f8fafb;border-color:#e5eaef}
.banner.v-unknown .banner-meta{color:var(--unknown);background:var(--unknown-bg)}
.hero.v-unknown{background:linear-gradient(180deg,#f6f8fa 0%,#fff 100%)}
.hero.v-unknown .hero-ico{background:var(--unknown);color:#fff}
.banner.v-pass{border-left-color:var(--pass)}
.banner.v-fail{border-left-color:var(--warn)}
.banner.v-unknown{border-left-color:#b9c4cd}
@media (max-width:640px){
  .meta-bar{grid-template-columns:1fr 1fr}
  .meta-bar div{padding:6px 14px}
  .meta-bar div:nth-child(odd){border-left:none;padding-left:0}
  .banner-desc{margin-left:0}
}
@media print{
  body{background:#fff}
  main{padding:12px 0}
}
"#;

// ---------- 测试（对齐 GUI 自测的断言） ----------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture_run() -> Value {
        json!({
            "selected_modules": ["specification"],
            "customer_conclusion": {
                "kind": "normal_use",
                "text": "这套模型服务可以正常使用"
            },
            "record": {
                "createdAt": "2026-09-23T09:00:00Z",
                "target": {
                    "model": "mock-model",
                    "endpointFingerprint": "http://127.0.0.1:8931/v1/chat/completions"
                },
                "moduleResults": [
                    {"moduleId": "specification", "state": "pass", "reason": "通过"}
                ],
                "evidence": [
                    {
                        "id": "cli-specification-0",
                        "kind": "real_service_response",
                        "payload": {
                            "module": "specification",
                            "payload": {
                                "report": {
                                    "observations": [
                                        {"sample_id": "context-64k", "status": "accepted", "verified_scope": "64K 档位被接受并完整返回"}
                                    ]
                                },
                                "evidence": [
                                    {"sample_id": "context-64k", "evidence_id": "cli-specification-0"}
                                ],
                                "request": {"prompt": "这是一段很长的输入"},
                                "response": {
                                    "status": 200,
                                    "body": {"id": "chatcmpl-mock", "object": "chat.completion", "choices": [{"index": 0, "message": {"role": "assistant", "content": "mock response"}}], "usage": {"total_tokens": 13}}
                                }
                            }
                        }
                    }
                ]
            }
        })
    }

    #[test]
    fn rich_report_contains_conclusion_and_detail() {
        let html = render_rich_html(&fixture_run());
        assert!(html.contains("可以正常使用"), "含大结论");
        assert!(html.contains("这次检测的结论一览"), "含模块一览");
        assert!(html.contains("检测方式") && html.contains("判定方式"), "含明细口径");
    }

    #[test]
    fn rich_report_folds_curl_and_body() {
        let html = render_rich_html(&fixture_run());
        assert!(
            html.contains("<details><summary>查看原始 curl 请求</summary>"),
            "折叠原始 curl：{}",
            html.len()
        );
        assert!(html.contains("chat.completion"), "含完整响应体");
    }

    #[test]
    fn rich_report_is_self_contained() {
        let html = render_rich_html(&fixture_run());
        assert!(!html.contains("<script"), "自包含且无脚本");
        assert!(html.contains("约 64K token 的真实长度输入"), "内部编号翻译成人话");
    }

    #[test]
    fn renders_preview_from_real_run_json() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("agentcheck-report/run.json");
        let Ok(content) = std::fs::read_to_string(&path) else {
            return; // 本地没有样本 run.json 时跳过
        };
        let run: Value = serde_json::from_str(&content).expect("解析 run.json");
        let html = render_rich_html(&run);
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/report-rich-preview.html");
        std::fs::write(&out, html).expect("写预览");
    }
}
