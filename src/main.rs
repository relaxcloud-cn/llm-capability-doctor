use clap::Parser;
use llm_capability_doctor::cli::{
    CliRunRequest, JsonlProgressSink, LiveExecutor, OutputFormat, generated_run_id,
    generated_timestamp, render_report, run_with_executor_reporting, write_report,
};
use llm_capability_doctor::evaluation::{
    AnalyzerConfig, analyze_modules, render_html, write_bundled_module_inputs,
};
use llm_capability_doctor::gui::{
    NativeGuiLauncher, NativeGuiRequest, SystemNativeGuiLauncher, current_platform,
};
use llm_capability_doctor::ingress::redact_endpoint;
use llm_capability_doctor::preflight::{ConnectivityState, preflight_token, run_startup_preflight};
use llm_capability_doctor::terminal_progress::TerminalProgressSink;
use std::collections::BTreeMap;
use std::env;
use std::fs::File;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(
    name = "agentcheck",
    bin_name = "agentcheck",
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

    /// 请求打开桌面端；不可用时说明原因并继续纯命令行检测。
    #[arg(long, conflicts_with = "no_gui")]
    gui: bool,

    /// 检查内置分析程序是否可运行，无需连接模型。
    #[arg(long)]
    runtime_check: bool,

    /// 查看内置分析程序及其依赖的许可声明。
    #[arg(long)]
    licenses: bool,

    /// 单次服务请求超时时间，单位为秒。
    #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..))]
    timeout_seconds: u64,

    /// 将模块级进度以 JSON Lines 写入文件，供桌面端消费。
    #[arg(long, hide = true, value_name = "PATH")]
    progress_file: Option<String>,

    /// 保存模块检测证据、OhMyPi 模块报告和最终 HTML 的目录。
    #[arg(long, value_name = "DIR")]
    report_dir: Option<String>,

    /// 最终 HTML 报告路径；提供后自动执行 OhMyPi 模块分析。
    #[arg(long, value_name = "PATH")]
    html: Option<String>,
}

fn main() {
    let cli = Cli::parse();
    if cli.licenses {
        match llm_capability_doctor::runtime::licenses() {
            Ok(content) => println!("{content}"),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if cli.runtime_check {
        match llm_capability_doctor::runtime::check() {
            Ok(value) => println!("{}", serde_json::to_string_pretty(&value).unwrap()),
            Err(error) => {
                eprintln!("[运行检查] {error}");
                std::process::exit(1);
            }
        }
        return;
    }
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
    if !cli.no_gui && (cli.gui || preflight.desktop.supported) {
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
                report_dir: cli.report_dir.clone(),
                html: cli.html.clone(),
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
        endpoint: endpoint.clone(),
        model: model.clone(),
        api_key: Some(api_key.clone()),
        selected_modules,
        stop_after,
        run_id: generated_run_id(),
        started_at: generated_timestamp(),
    };
    let mut executor = match LiveExecutor::new_full(
        request.endpoint.clone(),
        request.model.clone(),
        api_key.clone(),
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
        None => {
            let interactive = std::io::IsTerminal::is_terminal(&std::io::stderr())
                && env::var_os("TERM").is_none_or(|term| term != "dumb");
            let color = interactive && env::var_os("NO_COLOR").is_none();
            let mut progress = TerminalProgressSink::new(
                std::io::stderr(),
                interactive,
                color,
                model.clone(),
                redact_endpoint(&endpoint),
            );
            run_with_executor_reporting(request, &mut executor, &mut progress)
        }
    };
    let report = match report_result {
        Ok(report) => report,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };

    {
        let report_dir = cli.report_dir.clone().unwrap_or_else(|| {
            cli.html
                .as_deref()
                .map(std::path::Path::new)
                .and_then(std::path::Path::parent)
                .filter(|path| !path.as_os_str().is_empty())
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_else(|| "agentcheck-report".into())
        });
        let root = std::path::Path::new(&report_dir);
        std::fs::create_dir_all(root).unwrap_or_else(|error| {
            eprintln!("创建报告目录失败：{report_dir}：{error}");
            std::process::exit(1);
        });
        let run_json = root.join("run.json");
        let run_content = serde_json::to_vec_pretty(&report).unwrap_or_else(|error| {
            eprintln!("序列化运行报告失败：{error}");
            std::process::exit(1);
        });
        std::fs::write(&run_json, run_content).unwrap_or_else(|error| {
            eprintln!("写入运行报告失败：{}：{error}", run_json.display());
            std::process::exit(1);
        });
        let input_dir = root.join("module-input");
        let analysis_dir = root.join("module-report");
        let input_paths =
            write_bundled_module_inputs(&report, &input_dir).unwrap_or_else(|error| {
                eprintln!("写入模块检测证据失败：{error}");
                std::process::exit(1);
            });
        let module_report_paths = analyze_modules(
            &input_paths,
            &analysis_dir,
            &AnalyzerConfig {
                endpoint: endpoint.clone(),
                model: model.clone(),
                api_key: api_key.clone(),
            },
        )
        .unwrap_or_else(|error| {
            eprintln!("OhMyPi 模块分析失败：{error}");
            std::process::exit(1);
        });
        let html = render_html(&report, &module_report_paths).unwrap_or_else(|error| {
            eprintln!("生成 HTML 报告失败：{error}");
            std::process::exit(1);
        });
        let html_path = cli
            .html
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| root.join("report.html"));
        if let Some(parent) = html_path.parent() {
            std::fs::create_dir_all(parent).unwrap_or_else(|error| {
                eprintln!("创建 HTML 报告目录失败：{error}");
                std::process::exit(1);
            });
        }
        std::fs::write(&html_path, html).unwrap_or_else(|error| {
            eprintln!("写入 HTML 报告失败：{}：{error}", html_path.display());
            std::process::exit(1);
        });
        eprintln!("[报告] HTML：{}", html_path.display());
    }
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
    let (marker, connectivity_state) = match connectivity.state {
        ConnectivityState::Passed => ("✓", "模型连通性通过"),
        ConnectivityState::Failed => ("✗", "模型连通性失败"),
        ConnectivityState::NotRun => ("○", "模型连通性未执行"),
    };
    let timing = if connectivity.elapsed_ms > 0 {
        format!("，耗时 {}ms", connectivity.elapsed_ms)
    } else {
        String::new()
    };
    let desktop_state = if preflight.desktop.supported {
        "桌面环境可用"
    } else {
        "桌面环境不可用"
    };
    eprintln!(
        "{marker} 启动检查  {connectivity_state}（{}{timing}）· {desktop_state}",
        connectivity.reason
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gui_flags_are_mutually_exclusive() {
        assert!(Cli::try_parse_from(["agentcheck", "--gui", "--no-gui"]).is_err());
        assert!(Cli::try_parse_from(["agentcheck", "--gui"]).unwrap().gui);
        assert!(
            Cli::try_parse_from(["agentcheck", "--no-gui"])
                .unwrap()
                .no_gui
        );
        assert!(
            Cli::try_parse_from(["agentcheck", "--runtime-check"])
                .unwrap()
                .runtime_check
        );
    }
}
