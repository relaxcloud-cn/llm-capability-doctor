use clap::Parser;
use llm_capability_doctor::cli::{
    CliRunRequest, JsonlProgressSink, LiveExecutor, OutputFormat, generated_run_id,
    generated_timestamp, render_report, run_with_executor, run_with_executor_reporting,
    write_report,
};
use llm_capability_doctor::gui::{
    NativeGuiLauncher, NativeGuiRequest, SystemNativeGuiLauncher, current_platform, detect_desktop,
};
use std::collections::BTreeMap;
use std::env;
use std::fs::File;

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
    let Some(endpoint) = cli.url else {
        eprintln!("缺少服务 URL；请使用 --url 提供目标地址");
        std::process::exit(2);
    };
    let Some(model) = cli.model else {
        eprintln!("缺少模型名称；请使用 --model 提供配置值");
        std::process::exit(2);
    };
    let api_key = cli
        .api_key
        .or_else(|| std::env::var("MODEL_API_KEY").ok())
        .filter(|key| !key.trim().is_empty());
    let Some(api_key) = api_key else {
        eprintln!("缺少 API 密钥；请使用 MODEL_API_KEY 提供，命令行参数仅用于兼容隐藏输入");
        std::process::exit(2);
    };
    let selected_modules = cli.modules.clone();
    let stop_after = cli.stop_after.clone();
    if !cli.no_gui && cfg!(target_os = "macos") {
        let environment = env::vars().collect::<BTreeMap<_, _>>();
        let desktop = detect_desktop(current_platform(), &environment);
        if desktop.supported {
            let cli_path = match env::current_exe() {
                Ok(path) => path,
                Err(error) => {
                    eprintln!("读取 CLI 路径失败，回退到纯 CLI：{error}");
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
                };
                match launcher.launch(&request) {
                    Ok(path) => {
                        println!("AgentCheck 桌面端已启动：{}", path.display());
                        return;
                    }
                    Err(error) => eprintln!("{error}；回退到纯 CLI 执行"),
                }
            }
        }
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
