use crate::{
    checks,
    judgment::{CheckGroup, CheckSpec, Evaluation, EvidenceIndex, RequirementSpec},
    measure,
    model::{CheckId, CheckResult},
    transport::Transport,
};
use anyhow::{Result, bail};
use futures_util::future::BoxFuture;
use std::{collections::BTreeSet, path::Path, time::Instant};

pub struct CheckContext<'a> {
    pub spec: &'a CheckSpec,
    pub directory: &'a Path,
    pub transport: &'a Transport,
    pub deadline: Instant,
    pub progress: Option<&'a (dyn Fn(String) + Send + Sync)>,
}
pub type Executor = for<'a> fn(CheckContext<'a>) -> BoxFuture<'a, Result<CheckResult>>;
pub type Evaluator = fn(&CheckSpec, &CheckResult, &Path, &EvidenceIndex) -> Result<Evaluation>;

#[derive(Clone)]
pub struct Registration {
    pub spec: CheckSpec,
    pub execute: Option<Executor>,
    pub evaluate: Evaluator,
}
#[derive(Clone)]
pub struct Registry {
    pub entries: Vec<Registration>,
}
impl Registry {
    pub fn new(entries: Vec<Registration>) -> Result<Self> {
        let mut ids = BTreeSet::new();
        for entry in &entries {
            let spec = &entry.spec;
            if !ids.insert(spec.id)
                || spec.name.is_empty()
                || spec.version.is_empty()
                || spec.scope.is_empty()
                || spec.budget_seconds == 0
            {
                bail!("检测注册信息无效或编号重复");
            }
            let mut requirements = BTreeSet::new();
            if !spec.requirements.iter().any(|r| r.required)
                || spec
                    .requirements
                    .iter()
                    .any(|r| r.id.is_empty() || !requirements.insert(&r.id))
            {
                bail!("检测要求缺失或编号重复");
            }
            if entry.execute.is_none() && (!spec.id.is_agent() || spec.repetitions == 0) {
                bail!("缺少检测执行器");
            }
        }
        if entries.is_empty() {
            bail!("检测计划不能为空");
        }
        Ok(Self { entries })
    }
    pub fn builtin() -> Self {
        Self::new(
            builtin_specs()
                .into_iter()
                .map(|spec| {
                    let execute: Option<Executor> = match spec.id {
                        CheckId::Response => {
                            Some(|ctx| Box::pin(checks::response(ctx.transport, ctx.deadline)))
                        }
                        CheckId::Tools => {
                            Some(|ctx| Box::pin(checks::tools(ctx.transport, ctx.deadline)))
                        }
                        CheckId::Schema => {
                            Some(|ctx| Box::pin(checks::schema(ctx.transport, ctx.deadline)))
                        }
                        CheckId::Context => {
                            Some(|ctx| Box::pin(measure::context(ctx.transport, ctx.deadline)))
                        }
                        CheckId::Performance => Some(|ctx| {
                            Box::pin(measure::performance_with_progress(
                                ctx.transport,
                                ctx.deadline,
                                ctx.progress,
                            ))
                        }),
                        _ => None,
                    };
                    Registration {
                        spec,
                        execute,
                        evaluate: crate::assessment::builtin_evaluation,
                    }
                })
                .collect(),
        )
        .expect("valid built-in registry")
    }
    pub fn plan(&self) -> Vec<CheckSpec> {
        self.entries.iter().map(|e| e.spec.clone()).collect()
    }
}

pub fn builtin_specs() -> Vec<CheckSpec> {
    CheckId::ALL
        .into_iter()
        .map(|id| {
            let (budget, scope, rules): (u64, &str, Vec<(&str, &str, bool)>) = match id {
                CheckId::Response => (
                    360,
                    "本次普通、流式响应及消息历史的协议接收；不证明通用理解或记忆能力",
                    vec![
                        ("nonstream", "普通响应完整", true),
                        ("stream", "流式响应完整", true),
                        ("history", "前轮消息完整回传", true),
                    ],
                ),
                CheckId::Tools => (
                    1200,
                    "本次普通工具选择、参数、调用关联及结果使用；不要求服务端 strict 强制约束；每次请求最多输出 20000 tokens，完整请求超时 600 秒",
                    vec![("roundtrip", "工具调用与结果回传正确", true)],
                ),
                CheckId::Schema => (
                    180,
                    "本次记录内容与格式；兼容通过不代表原生严格 Schema 支持",
                    vec![
                        ("content", "结构化内容正确且可解析", true),
                        ("native_format", "本次正文为原生 JSON", false),
                    ],
                ),
                CheckId::Context => (
                    900,
                    "依据服务 token 计量测量请求容量；不测长材料理解能力",
                    vec![
                        ("minimum_context", "已验证至少 131072 tokens 请求容量", true),
                        ("exact_maximum", "精确最大容量已确认", false),
                    ],
                ),
                CheckId::Performance => (
                    3600,
                    "当前账号与普通短文本负载的并发表现；仅连接异常允许整档复核一次，保留首次失败；已验证档位可附稳定性风险作为试用起点，不代表容量极限或长期保证",
                    vec![
                        ("recommendation", "并发建议有正常完成及耗时证据", true),
                        ("batch_completion", "各档请求是否正常结束并返回非空文本", false),
                    ],
                ),
                _ => (
                    240,
                    match id {
                        CheckId::Continuous => "本次服务、模型与固定 OMP 版本组合：使用原生文件工具读取两份各含一条记录的文件并合并结果，独立重复两次；不代表通用任务能力或长期可靠性",
                        CheckId::Recovery => "本次服务、模型与固定 OMP 版本组合：两记录文件任务中，每次仅注入一次且明确提示重试的临时读取错误，核对随后继续执行与结果，独立重复两次；不代表通用容错能力或长期可靠性",
                        CheckId::Multiturn => "本次服务、模型与固定 OMP 版本组合：先合并两条记录，再在同一会话中按明确目标仅更新 R-01.version，保留 R-02 及其他字段，独立重复两次；不代表通用记忆能力或长期可靠性",
                        _ => "本次指定文件任务的真实执行、来源与结果对应；不代表所有未来任务",
                    },
                    vec![
                        ("run_1", "第一次任务满足规定要求", true),
                        ("run_2", "第二次任务满足规定要求", true),
                    ],
                ),
            };
            CheckSpec {
                id,
                name: id.name().into(),
                version: match id {
                    CheckId::Tools => "4",
                    CheckId::Performance => "3",
                    CheckId::Continuous | CheckId::Recovery | CheckId::Multiturn => "2",
                    _ => "1",
                }
                .into(),
                scope: scope.into(),
                group: if id.is_agent() {
                    CheckGroup::Agent
                } else {
                    CheckGroup::Interface
                },
                required: true,
                budget_seconds: budget,
                repetitions: if id.is_agent() { 2 } else { 0 },
                requirements: rules
                    .into_iter()
                    .enumerate()
                    .map(|(i, (key, title, required))| RequirementSpec {
                        id: key.into(),
                        title: title.into(),
                        required,
                        subject: id.is_agent().then(|| format!("{}-{}", id.key(), i + 1)),
                    })
                    .collect(),
                pass_summary: match id {
                    CheckId::Response => Some("普通响应、流式响应和消息回传均已核验".into()),
                    CheckId::Continuous => Some("两次两记录任务的原生工具读写与合并结果均正确".into()),
                    CheckId::Recovery => {
                        Some("两次明确提示的单次临时读取错误均已送达，随后继续执行并得到正确结果".into())
                    }
                    CheckId::Multiturn => Some("两次任务均完成首轮合并、同会话单字段更新与无关内容保留".into()),
                    _ => None,
                },
            }
        })
        .collect()
}
