use agent_check::{
    config::{ModelConfig, Protocol},
    model::{CheckId, Status},
    omp,
    transport::{Transport, messages_body},
};
use anyhow::{Context, Result, bail};
use clap::Parser;
use serde_json::json;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
#[command(name = "agent-check", version, about = "模型接入检测工具")]
struct Cli {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    url: Option<String>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long, value_enum)]
    protocol: Option<Protocol>,
    #[arg(long, hide = true)]
    api_key: Option<String>,
    #[arg(long, conflicts_with = "gui")]
    no_gui: bool,
    #[arg(long, conflicts_with = "no_gui")]
    gui: bool,
    #[arg(long, default_value = "agentcheck-results")]
    report_dir: PathBuf,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    no_color: bool,
    #[arg(long, hide = true)]
    preflight_only: bool,
    #[arg(long, hide = true)]
    dev_interfaces: bool,
    #[arg(long, hide = true)]
    dev_run: bool,
    #[arg(long, hide = true)]
    render_result: Option<PathBuf>,
    #[arg(long, hide = true)]
    dev_task: Option<String>,
    #[arg(long, hide = true, default_value_t = 1)]
    repetition: u8,
}
#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(2);
    }
}
async fn run() -> Result<()> {
    let cli = Cli::parse();
    if let Some(path) = &cli.render_result {
        let mut run: agent_check::model::RunResult = serde_json::from_slice(&std::fs::read(path)?)?;
        let directory = path.parent().context("结果文件缺少目录")?;
        agent_check::report::write(&mut run, directory)?;
        println!("{}", run.report_path.unwrap_or_default());
        return Ok(());
    }

    let config = if let Some(path) = &cli.config {
        ModelConfig::load_with_protocol(path, cli.protocol)?
    } else {
        ModelConfig::new(
            cli.url.as_deref().context("请提供 --url 或 --config")?,
            cli.model.as_deref().context("请提供 --model 或 --config")?,
            cli.api_key
                .or_else(|| std::env::var("MODEL_API_KEY").ok())
                .context("请设置 MODEL_API_KEY 或通过本机配置提供密钥")?,
            cli.protocol,
        )?
    };
    let cancel = CancellationToken::new();
    let signal_cancel = cancel.clone();
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            if let Ok(mut term) =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            {
                tokio::select! {_=tokio::signal::ctrl_c()=>{},_=term.recv()=>{}};
                signal_cancel.cancel();
                return;
            }
        }
        let _ = tokio::signal::ctrl_c().await;
        signal_cancel.cancel();
    });
    if cli.preflight_only {
        let folder = cli
            .report_dir
            .join(format!("preflight-{}", uuid::Uuid::new_v4().simple()));
        let transport = Transport::new(config.clone(), &folder, cancel)?;
        let body = messages_body(
            &config,
            json!([{"role":"user","content":"请回复一个简短的连接确认。"}]),
            512,
            false,
        );
        let outcome = transport
            .send(body, false, Instant::now() + Duration::from_secs(60))
            .await?;
        let valid = outcome
            .response
            .as_ref()
            .is_some_and(|r| r.normal_end && !r.text.trim().is_empty());
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"target":config.public_target(),"connection_ok":valid,"returned_model":outcome.response.as_ref().and_then(|r|r.model.clone()),"elapsed_ms":outcome.evidence.elapsed_ms,"evidence_id":outcome.evidence.id,"evidence_directory":folder,"capability_verdict":"not_evaluated"})
            )?
        );
        if !valid {
            bail!("连接检查未通过，请查看脱敏请求记录");
        }
        return Ok(());
    }
    if cli.dev_run {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let printer = tokio::spawn(async move {
            while let Some(envelope) = receiver.recv().await {
                let agent_check::model::EventEnvelope { event, .. } = envelope;
                match event {
                    agent_check::model::RunEvent::CheckStarted { id } => {
                        eprintln!("正在检查 {}", id.name())
                    }
                    agent_check::model::RunEvent::CheckFinished { check } => {
                        eprintln!("{}：{}", check.status.label(), check.title)
                    }
                    agent_check::model::RunEvent::TaskStarted {
                        kind, repetition, ..
                    } => eprintln!("启动 {} · 第 {} 次", kind.name(), repetition),
                    agent_check::model::RunEvent::TaskStep {
                        kind,
                        repetition,
                        message,
                        ..
                    } => eprintln!("{} · 第 {} 次：{}", kind.name(), repetition, message),
                    agent_check::model::RunEvent::TaskFinished { task } => eprintln!(
                        "{} · 第 {} 次 {}",
                        task.kind.name(),
                        task.repetition,
                        task.status.label()
                    ),
                    _ => {}
                }
            }
        });
        let output =
            agent_check::engine::run(config, &cli.report_dir, cancel, Some(sender)).await?;
        printer.await?;
        println!("{}", serde_json::to_string_pretty(&output.run)?);
        return Ok(());
    }
    if cli.dev_interfaces {
        let folder = cli
            .report_dir
            .join(format!("interfaces-{}", uuid::Uuid::new_v4().simple()));
        let transport = Transport::new(config.clone(), &folder, cancel)?;
        let deadline = Instant::now() + Duration::from_secs(240);
        let mut checks = vec![];
        for id in [
            CheckId::Response,
            CheckId::Tools,
            CheckId::Schema,
            CheckId::Context,
            CheckId::Performance,
        ] {
            eprintln!("正在检查 {}", id.name());
            let check = match id {
                CheckId::Response => agent_check::checks::response(&transport, deadline).await?,
                CheckId::Tools => agent_check::checks::tools(&transport, deadline).await?,
                CheckId::Schema => agent_check::checks::schema(&transport, deadline).await?,
                CheckId::Context => agent_check::measure::context(&transport, deadline).await?,
                _ => agent_check::measure::performance(&transport, deadline).await?,
            };
            eprintln!("{}：{}", check.status.label(), check.title);
            checks.push(check);
        }
        std::fs::write(
            folder.join("checks.json"),
            serde_json::to_vec_pretty(&checks)?,
        )?;
        println!("{}", serde_json::to_string_pretty(&checks)?);
        return Ok(());
    }
    if let Some(kind) = cli.dev_task {
        let kind = match kind.as_str() {
            "continuous" => CheckId::Continuous,
            "recovery" => CheckId::Recovery,
            "multiturn" => CheckId::Multiturn,
            _ => bail!("开发验收任务必须是 continuous、recovery 或 multiturn"),
        };
        let folder = std::path::absolute(
            cli.report_dir
                .join(format!("task-{}", uuid::Uuid::new_v4().simple())),
        )?;
        std::fs::create_dir_all(&folder)?;
        eprintln!(
            "正在执行 {} · 第 {} 次；记录目录 {}",
            kind.name(),
            cli.repetition,
            folder.display()
        );
        let step: Arc<dyn Fn(String) + Send + Sync> = Arc::new(|message| eprintln!("  {message}"));
        let task = omp::run_task(config, kind, cli.repetition, &folder, cancel, Some(step)).await?;
        let status = task.status;
        println!("{}", serde_json::to_string_pretty(&task)?);
        if status != Status::Passed {
            std::process::exit(1);
        }
        return Ok(());
    }
    bail!("完整检测流程正在实现中；当前版本仅供开发验收，尚未达到交付标准")
}
