use agent_check::{
    config::{ModelConfig, Protocol},
    model::{CheckId, Status, Verdict},
    omp,
    transport::Transport,
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
    #[arg(long,conflicts_with_all=["no_gui","json"])]
    gui: bool,
    #[arg(long)]
    report_dir: Option<PathBuf>,
    #[arg(long, help = "输出最终 JSON，使用 CLI 模式")]
    json: bool,
    #[arg(long)]
    no_color: bool,
    #[arg(long, hide = true)]
    worker: bool,
    #[arg(long, hide = true)]
    offline_self_test: bool,
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
fn cancellation() -> CancellationToken {
    let cancel = CancellationToken::new();
    let signal = cancel.clone();
    tokio::spawn(async move {
        #[cfg(unix)]
        if let Ok(mut term) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! {_=tokio::signal::ctrl_c()=>{},_=term.recv()=>{}}
            signal.cancel();
            return;
        }
        let _ = tokio::signal::ctrl_c().await;
        signal.cancel();
    });
    cancel
}
fn configuration(cli: &Cli) -> Result<ModelConfig> {
    if let Some(path) = &cli.config {
        ModelConfig::load_with_protocol(path, cli.protocol)
    } else {
        ModelConfig::new(
            cli.url.as_deref().context("请提供 --url 或 --config")?,
            cli.model.as_deref().context("请提供 --model 或 --config")?,
            cli.api_key
                .clone()
                .or_else(|| std::env::var("MODEL_API_KEY").ok())
                .context("请设置 MODEL_API_KEY 或通过本机配置提供密钥")?,
            cli.protocol,
        )
    }
}
async fn run() -> Result<()> {
    let cli = Cli::parse();
    let root = cli
        .report_dir
        .clone()
        .map(Ok)
        .unwrap_or_else(agent_check::store::default_root)?;
    if cli.offline_self_test {
        let runtime = agent_check::omp::development_runtime()?;
        let output = tokio::process::Command::new(&runtime)
            .arg("--version")
            .output()
            .await?;
        let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !output.status.success() || version != format!("omp/{}", agent_check::model::OMP_VERSION)
        {
            bail!("OMP 自检失败");
        }
        let gui = if agent_check::gui::platform() == agent_check::gui::Platform::Linux {
            None
        } else {
            Some(agent_check::gui::executable()?)
        };
        let gui_opened = cli.gui && gui.is_some();
        if gui_opened {
            agent_check::gui::open_offline(&root).await?;
        }
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"bundled":agent_check::runtime::is_bundled(),"omp_version":version,"omp":runtime,"gui":gui,"gui_opened":gui_opened,"model_requests":0})
            )?
        );
        return Ok(());
    }
    if cli.worker {
        return agent_check::worker::serve(std::path::absolute(root)?).await;
    }
    if let Some(path) = &cli.render_result {
        let mut run: agent_check::model::RunResult = serde_json::from_slice(&std::fs::read(path)?)?;
        agent_check::report::write(&mut run, path.parent().context("结果文件没有目录")?)?;
        println!("{}", run.report_path.unwrap_or_default());
        return Ok(());
    }
    let developer =
        cli.preflight_only || cli.dev_run || cli.dev_interfaces || cli.dev_task.is_some();
    if !developer {
        agent_check::terminal::startup("正在检查模型配置与连接…", "·", "dim", cli.no_color);
    }
    let config = match configuration(&cli) {
        Ok(config) => config,
        Err(error) => {
            if !developer {
                agent_check::terminal::startup(
                    &format!("配置检查失败：{error}"),
                    "✗",
                    "red",
                    cli.no_color,
                );
                agent_check::terminal::startup(
                    "未开始正式检测，请修正配置后重新运行",
                    "✗",
                    "red",
                    cli.no_color,
                );
            }
            return Err(error);
        }
    };
    let cancel = cancellation();
    if cli.preflight_only {
        let receipt = agent_check::preflight::check(&config, &root, cancel).await?;
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"connection_ok":true,"receipt":receipt,"capability_verdict":"not_evaluated"})
            )?
        );
        return Ok(());
    }
    if cli.dev_interfaces {
        let folder = root.join(format!("interfaces-{}", uuid::Uuid::new_v4().simple()));
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
    if let Some(kind) = &cli.dev_task {
        let kind = match kind.as_str() {
            "continuous" => CheckId::Continuous,
            "recovery" => CheckId::Recovery,
            "multiturn" => CheckId::Multiturn,
            _ => bail!("开发任务必须是 continuous、recovery 或 multiturn"),
        };
        let folder =
            std::path::absolute(root.join(format!("task-{}", uuid::Uuid::new_v4().simple())))?;
        std::fs::create_dir_all(&folder)?;
        eprintln!(
            "正在执行 {} · 第 {} 次；记录目录 {}",
            kind.name(),
            cli.repetition,
            folder.display()
        );
        let step: Arc<dyn Fn(String) + Send + Sync> = Arc::new(|message| eprintln!("  {message}"));
        let task = omp::run_task(config, kind, cli.repetition, &folder, cancel, Some(step)).await?;
        println!("{}", serde_json::to_string_pretty(&task)?);
        if task.status != Status::Passed {
            std::process::exit(1);
        }
        return Ok(());
    }
    if !developer {
        agent_check::terminal::startup("配置完整", "✓", "green", cli.no_color);
        let receipt = match agent_check::preflight::check(&config, &root, cancel.clone()).await {
            Ok(receipt) => receipt,
            Err(error) => {
                agent_check::terminal::startup(&error.to_string(), "✗", "red", cli.no_color);
                agent_check::terminal::startup(
                    "未开始正式检测，请修正配置后重新运行",
                    "✗",
                    "red",
                    cli.no_color,
                );
                return Err(error);
            }
        };
        agent_check::terminal::startup(
            &format!("模型连通性通过（耗时 {}ms）", receipt.elapsed_ms),
            "✓",
            "green",
            cli.no_color,
        );
        if cli.gui && agent_check::gui::platform() == agent_check::gui::Platform::Linux {
            agent_check::terminal::startup(
                "本平台不提供 GUI · 启动方式  CLI",
                "·",
                "cyan",
                cli.no_color,
            );
        } else if !cli.no_gui && !cli.json && agent_check::gui::current_desktop() {
            agent_check::terminal::startup(
                "桌面环境可用，正在启动 GUI…",
                "·",
                "cyan",
                cli.no_color,
            );
            match agent_check::gui::launch(&config, &receipt, &root).await {
                Ok(()) => {
                    agent_check::terminal::startup("GUI 窗口已显示", "✓", "green", cli.no_color);
                    agent_check::terminal::startup(
                        "GUI 已启动，配置已传入，检测将自动继续",
                        "✓",
                        "green",
                        cli.no_color,
                    );
                    return Ok(());
                }
                Err(error) => {
                    agent_check::terminal::startup(&error.to_string(), "!", "yellow", cli.no_color);
                    agent_check::terminal::startup(
                        "启动方式  CLI（继续使用原配置）",
                        "·",
                        "cyan",
                        cli.no_color,
                    );
                }
            }
        } else {
            agent_check::terminal::startup(
                if cli.no_gui || cli.json {
                    "启动方式  CLI"
                } else {
                    "桌面环境不可用 · 启动方式  CLI"
                },
                "·",
                "cyan",
                cli.no_color,
            );
        }
    }
    let (events, receiver) = tokio::sync::mpsc::unbounded_channel();
    let printer = tokio::spawn(agent_check::terminal::display(receiver, cli.no_color));
    let output = agent_check::engine::run(config, &root, cancel, Some(events)).await;
    printer.await??;
    let output = output?;
    if cli.json || cli.dev_run {
        println!("{}", serde_json::to_string_pretty(&output.run)?);
    } else if let Some(path) = &output.run.report_path {
        eprintln!(
            "[报告] HTML：{path}\n[记录] ZIP ：{}",
            output.directory.join("records.zip").display()
        );
    }
    if output.run.cancelled {
        std::process::exit(130);
    }
    if output.run.verdict == Verdict::Failed {
        std::process::exit(1);
    }
    if output.run.verdict == Verdict::Incomplete {
        std::process::exit(2);
    }
    Ok(())
}
