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
                    360,
                    "本次工具选择、参数、调用关联及工具结果使用；每次请求最多输出 20000 tokens",
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
                    "当前账号与普通短文本负载的并发表现；不校验正文格式或措辞，不证明通用内容质量或精确容量上限",
                    vec![
                        ("recommendation", "并发建议有正常完成及耗时证据", true),
                        ("batch_completion", "各档请求是否正常结束并返回非空文本", false),
                    ],
                ),
                _ => (
                    240,
                    "本次指定文件任务的真实执行、来源与结果对应；不代表所有未来任务",
                    vec![
                        ("run_1", "第一次任务满足规定要求", true),
                        ("run_2", "第二次任务满足规定要求", true),
                    ],
                ),
            };
            CheckSpec {
                id,
                name: id.name().into(),
                version: if matches!(id, CheckId::Tools | CheckId::Performance) { "2" } else { "1" }.into(),
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
                    CheckId::Continuous => Some("两次连续工具执行的结果与过程均正确".into()),
                    CheckId::Recovery => {
                        Some("两次故障均实际送达，之后继续执行并得到正确结果".into())
                    }
                    CheckId::Multiturn => Some("两次多轮任务均完成首轮结果、次轮新增与修改".into()),
                    _ => None,
                },
            }
        })
        .collect()
}
