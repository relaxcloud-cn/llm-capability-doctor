use clap::Parser;
use llm_capability_doctor::cli::{
    CliRunRequest, JsonlProgressSink, LiveExecutor, OutputFormat, generated_run_id,
    generated_timestamp, render_report, run_with_executor, run_with_executor_reporting,
    write_report,
};
use llm_capability_doctor::gui::{
    NativeGuiLauncher, NativeGuiRequest, SystemNativeGuiLauncher, current_platform,
};
use llm_capability_doctor::preflight::{ConnectivityState, preflight_token, run_startup_preflight};
use std::collections::BTreeMap;
use std::env;
use std::fs::File;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(
    name = "llm-capability-doctor",
    version,
    about = "检测模型服务能力并输出有证据的使用结论"
)]
struct Cli {
    /// 当前模型服务地址；输出时会自动脱敏。
    #[arg(long, value_name = "URL")]
    url: Option<String>,

    /// 客户配置的模型名称。
    #[arg(long, value_name = "MODEL")]
    model: Option<String>,

    /// API 密钥，仅用于后续请求，不会输出。
    #[arg(long, value_name = "KEY", hide = true)]
    api_key: Option<String>,

    /// 要执行的正式项目，逗号分隔；省略时执行当前版本全部项目。
    #[arg(long, value_delimiter = ',', value_name = "MODULE")]
    modules: Option<Vec<String>>,

    /// 执行到指定项目后停止并保留剩余项目的未验证状态。
    #[arg(long, value_name = "MODULE")]
    stop_after: Option<String>,

    /// 输出格式。
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    format: OutputFormat,

    /// 将报告写入文件；不提供时直接输出到终端。
    #[arg(long, value_name = "PATH")]
    output: Option<String>,

    /// 禁止桌面环境自动打开本地工作台，保留纯 CLI 流程。
    #[arg(long)]
    no_gui: bool,

    /// 单次服务请求超时时间，单位为秒。
    #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..))]
    timeout_seconds: u64,

    /// 将模块级进度以 JSON Lines 写入文件，供桌面端消费。
    #[arg(long, hide = true, value_name = "PATH")]
    progress_file: Option<String>,
}

fn main() {
    let cli = Cli::parse();
    let api_key = cli
        .api_key
        .or_else(|| std::env::var("MODEL_API_KEY").ok())
        .filter(|key| !key.trim().is_empty());
    let environment = env::vars().collect::<BTreeMap<_, _>>();
    let preflight = run_startup_preflight(
        cli.url.as_deref(),
        cli.model.as_deref(),
        api_key.as_deref(),
        env::var("MODEL_API_PREFLIGHT_TOKEN").ok().as_deref(),
        Duration::from_secs(cli.timeout_seconds.min(30)),
        current_platform(),
        &environment,
    );
    print_preflight_results(&preflight);

    if preflight.connectivity.state != ConnectivityState::Passed {
        eprintln!("[启动方式] 未开始正式检测：请先修正模型配置或连接问题");
        std::process::exit(2);
    }

    let Some(endpoint) = cli.url else {
        unreachable!("通过模型连通性检查后 URL 必然存在");
    };
    let Some(model) = cli.model else {
        unreachable!("通过模型连通性检查后模型名称必然存在");
    };
    let Some(api_key) = api_key else {
        unreachable!("通过模型连通性检查后 API 密钥必然存在");
    };
    let selected_modules = cli.modules.clone();
    let stop_after = cli.stop_after.clone();
    if !cli.no_gui && cfg!(target_os = "macos") && preflight.desktop.supported {
        let cli_path = match env::current_exe() {
            Ok(path) => path,
            Err(error) => {
                eprintln!("[启动方式] 读取 CLI 路径失败，回退到纯 CLI：{error}");
                std::path::PathBuf::new()
            }
        };
        if !cli_path.as_os_str().is_empty() {
            let mut launcher = SystemNativeGuiLauncher;
            let request = NativeGuiRequest {
                endpoint: endpoint.clone(),
                model: model.clone(),
                modules: selected_modules.clone(),
                stop_after: stop_after.clone(),
                timeout_seconds: cli.timeout_seconds,
                output: cli.output.clone(),
                api_key: api_key.clone(),
                cli_path,
                preflight_token: preflight_token(&endpoint, &model, &api_key),
            };
            match launcher.launch(&request) {
                Ok(path) => {
                    eprintln!("[启动方式] GUI：启动成功（{}）", path.display());
                    return;
                }
                Err(error) => eprintln!("[启动方式] GUI：启动失败（{error}），回退到纯 CLI"),
            }
        }
    } else if cli.no_gui {
        eprintln!("[启动方式] CLI：已指定 --no-gui");
    } else {
        eprintln!("[启动方式] CLI：当前环境不自动启动原生 GUI");
    }
    let request = CliRunRequest {
        endpoint,
        model,
        api_key: Some(api_key.clone()),
        selected_modules,
        stop_after,
        run_id: generated_run_id(),
        started_at: generated_timestamp(),
    };
    let mut executor = match LiveExecutor::new_full(
        request.endpoint.clone(),
        request.model.clone(),
        api_key,
        std::time::Duration::from_secs(cli.timeout_seconds),
    ) {
        Ok(executor) => executor,
        Err(error) => {
            eprintln!("创建真实服务执行器失败：{error}");
            std::process::exit(1);
        }
    };
    let report_result = match cli.progress_file {
        Some(path) => {
            let file = match File::create(&path) {
                Ok(file) => file,
                Err(error) => {
                    eprintln!("创建进度文件失败：{path}：{error}");
                    std::process::exit(1);
                }
            };
            let mut progress = JsonlProgressSink::new(file);
            run_with_executor_reporting(request, &mut executor, &mut progress)
        }
        None => run_with_executor(request, &mut executor),
    };
    let report = match report_result {
        Ok(report) => report,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    let content = match render_report(&report, cli.format) {
        Ok(content) => content,
        Err(error) => {
            eprintln!("渲染 CLI 报告失败：{error}");
            std::process::exit(1);
        }
    };
    if let Some(path) = cli.output {
        if let Err(error) = write_report(&path, &content) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        println!("报告已写入：{path}");
    } else {
        println!("{content}");
    }
}

fn print_preflight_results(preflight: &llm_capability_doctor::preflight::StartupPreflight) {
    let connectivity = &preflight.connectivity;
    let connectivity_state = match connectivity.state {
        ConnectivityState::Passed => "通过",
        ConnectivityState::Failed => "失败",
        ConnectivityState::NotRun => "未执行",
    };
    eprintln!(
        "[启动检查] 模型连通性：{connectivity_state}（{}，耗时 {} ms）",
        connectivity.reason, connectivity.elapsed_ms
    );

    let desktop_state = if preflight.desktop.supported {
        "可用"
    } else {
        "不可用"
    };
    eprintln!(
        "[启动检查] 桌面环境：{desktop_state}（{}）",
        preflight.desktop.reason
    );
}
