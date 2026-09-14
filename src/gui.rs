use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::CliRunReport;

pub const GUI_VERSION: &str = "gui/v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DesktopPlatform {
    Macos,
    Windows,
    Linux,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DesktopEnvironment {
    pub platform: DesktopPlatform,
    pub supported: bool,
    pub display_available: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum GuiLaunchState {
    Launched,
    Unavailable,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GuiLaunchResult {
    pub state: GuiLaunchState,
    pub path: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeGuiRequest {
    pub endpoint: String,
    pub model: String,
    pub modules: Option<Vec<String>>,
    pub stop_after: Option<String>,
    pub timeout_seconds: u64,
    pub output: Option<String>,
    pub api_key: String,
    pub cli_path: PathBuf,
}

pub trait NativeGuiLauncher {
    fn launch(&mut self, request: &NativeGuiRequest) -> Result<PathBuf, String>;
}

#[derive(Debug, Default)]
pub struct SystemNativeGuiLauncher;

impl NativeGuiLauncher for SystemNativeGuiLauncher {
    fn launch(&mut self, request: &NativeGuiRequest) -> Result<PathBuf, String> {
        let executable = native_gui_executable()?;
        let mut command = Command::new(&executable);
        command
            .args(["--agentcheck-endpoint", &request.endpoint])
            .args(["--agentcheck-model", &request.model])
            .args([
                "--agentcheck-cli-path",
                &request.cli_path.display().to_string(),
            ])
            .args([
                "--agentcheck-timeout-seconds",
                &request.timeout_seconds.to_string(),
            ])
            .env("MODEL_API_KEY", &request.api_key);
        if let Some(output) = &request.output {
            command.args(["--agentcheck-output", output]);
        }
        if let Some(modules) = &request.modules {
            command.args(["--agentcheck-modules", &modules.join(",")]);
        }
        if let Some(stop_after) = &request.stop_after {
            command.args(["--agentcheck-stop-after", stop_after]);
        }
        command
            .spawn()
            .map_err(|error| format!("启动 AgentCheck 桌面端失败：{error}"))?;
        Ok(executable)
    }
}

pub fn native_gui_executable() -> Result<PathBuf, String> {
    let candidates = if let Some(path) = env::var_os("AGENTCHECK_GUI_PATH") {
        vec![PathBuf::from(path)]
    } else {
        let current = env::current_exe().map_err(|error| format!("读取 CLI 路径失败：{error}"))?;
        let current_dir = current
            .parent()
            .ok_or_else(|| "CLI 没有可用的安装目录".to_string())?;
        vec![
            current_dir.join("AgentCheck.app"),
            current_dir.join("AgentCheck"),
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("prototypes/agent-check-desktop/build/AgentCheck.app"),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("desktop/AgentCheck.app"),
        ]
    };
    for candidate in candidates {
        let executable = if candidate
            .extension()
            .is_some_and(|extension| extension == "app")
        {
            candidate.join("Contents/MacOS/AgentCheck")
        } else {
            candidate.clone()
        };
        if executable.is_file() {
            return Ok(executable);
        }
    }
    Err("未找到 AgentCheck 桌面端；请先安装发行包中的 GUI，或设置 AGENTCHECK_GUI_PATH".into())
}

pub trait GuiLauncher {
    fn launch(&mut self, path: &Path) -> Result<(), String>;
}

#[derive(Debug, Default)]
pub struct SystemGuiLauncher;

impl GuiLauncher for SystemGuiLauncher {
    fn launch(&mut self, path: &Path) -> Result<(), String> {
        let mut command = if cfg!(target_os = "macos") {
            let mut command = Command::new("open");
            command.arg(path);
            command
        } else if cfg!(target_os = "windows") {
            let mut command = Command::new("cmd");
            command.args(["/C", "start", ""]);
            command.arg(path);
            command
        } else {
            let mut command = Command::new("xdg-open");
            command.arg(path);
            command
        };
        command
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("启动本地工作台失败：{error}"))
    }
}

pub fn current_platform() -> DesktopPlatform {
    if cfg!(target_os = "macos") {
        DesktopPlatform::Macos
    } else if cfg!(target_os = "windows") {
        DesktopPlatform::Windows
    } else if cfg!(target_os = "linux") {
        DesktopPlatform::Linux
    } else {
        DesktopPlatform::Unknown
    }
}

pub fn detect_desktop(
    platform: DesktopPlatform,
    environment: &BTreeMap<String, String>,
) -> DesktopEnvironment {
    let display_available = match platform {
        DesktopPlatform::Macos | DesktopPlatform::Windows => {
            environment.get("SSH_CONNECTION").is_none() && environment.get("CI").is_none()
        }
        DesktopPlatform::Linux => {
            environment.get("SSH_CONNECTION").is_none()
                && environment.get("CI").is_none()
                && (environment.contains_key("DISPLAY")
                    || environment.contains_key("WAYLAND_DISPLAY"))
        }
        DesktopPlatform::Unknown => false,
    };
    let supported = !matches!(platform, DesktopPlatform::Unknown) && display_available;
    let reason = if supported {
        "检测到支持的桌面环境".into()
    } else if matches!(platform, DesktopPlatform::Linux)
        && !environment.contains_key("DISPLAY")
        && !environment.contains_key("WAYLAND_DISPLAY")
    {
        "Linux 未检测到 DISPLAY 或 WAYLAND_DISPLAY，保留 CLI 回退".into()
    } else if environment.contains_key("CI") {
        "CI 环境不自动启动 GUI，保留 CLI 回退".into()
    } else {
        "当前环境不支持自动启动本地工作台，保留 CLI 回退".into()
    };
    DesktopEnvironment {
        platform,
        supported,
        display_available,
        reason,
    }
}

pub fn render_workbench_html(report: &CliRunReport) -> Result<String, serde_json::Error> {
    let report_json = serde_json::to_string(report)?
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e");
    let module_labels = [
        ("ingress", "服务接入"),
        ("specification", "模型规格"),
        ("capability", "能力跑分"),
        ("performance", "性能实测"),
        ("agent", "智能体实测"),
        ("baseline", "模型基线"),
    ];
    let module_cards = module_labels
        .iter()
        .map(|(id, label)| {
            format!(
                r#"<label class="module"><input type="checkbox" value="{id}"><span>{label}</span><strong data-state="{id}">未选</strong></label>"#
            )
        })
        .collect::<String>();
    Ok(format!(
        r##"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>AgentCheck 工作台</title>
  <style>
    :root {{ color-scheme: light; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; background: #f4f6f8; color: #18212b; }}
    * {{ box-sizing: border-box; }}
    body {{ margin: 0; min-width: 320px; }}
    header {{ background: #14202b; color: #fff; padding: 24px clamp(18px, 5vw, 64px); }}
    header h1 {{ margin: 0 0 8px; font-size: clamp(24px, 4vw, 38px); letter-spacing: 0; }}
    header p {{ margin: 0; color: #c5d2de; overflow-wrap: anywhere; }}
    main {{ width: min(1180px, calc(100% - 32px)); margin: 24px auto 56px; display: grid; gap: 18px; }}
    section {{ background: #fff; border: 1px solid #d8e0e7; border-radius: 8px; padding: 20px; }}
    section h2 {{ margin: 0 0 14px; font-size: 20px; }}
    .toolbar {{ display: flex; gap: 12px; align-items: center; flex-wrap: wrap; }}
    button {{ border: 0; border-radius: 6px; padding: 10px 16px; background: #087f5b; color: #fff; font: inherit; cursor: pointer; }}
    button.secondary {{ background: #e9eef2; color: #18212b; }}
    button:focus-visible, input:focus-visible {{ outline: 3px solid #f59f00; outline-offset: 2px; }}
    #run-state {{ color: #596775; }}
    .meta {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 12px; }}
    .meta div {{ min-width: 0; }}
    .meta dt {{ color: #687684; font-size: 13px; margin-bottom: 4px; }}
    .meta dd {{ margin: 0; overflow-wrap: anywhere; font-weight: 600; }}
    .modules {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(210px, 1fr)); gap: 10px; }}
    .module {{ border: 1px solid #d8e0e7; border-radius: 6px; padding: 12px; display: grid; grid-template-columns: auto 1fr auto; gap: 8px; align-items: center; min-height: 52px; }}
    .module strong {{ color: #687684; font-size: 13px; font-weight: 600; }}
    .module strong[data-status="pass"] {{ color: #087f5b; }}
    .module strong[data-status="fail"] {{ color: #c92a2a; }}
    .module strong[data-status="inconclusive"] {{ color: #a15c00; }}
    .progress {{ display: grid; gap: 8px; }}
    .row {{ display: grid; grid-template-columns: minmax(130px, 1fr) 120px 1.6fr; gap: 10px; align-items: center; border-bottom: 1px solid #edf0f2; padding: 9px 0; }}
    .row:last-child {{ border-bottom: 0; }}
    .state {{ font-weight: 700; }}
    .state.pass {{ color: #087f5b; }} .state.fail {{ color: #c92a2a; }} .state.inconclusive {{ color: #a15c00; }} .state.unverified, .state.not_selected {{ color: #687684; }}
    .source {{ color: #687684; font-size: 13px; }}
    .empty {{ color: #687684; margin: 0; }}
    @media (max-width: 640px) {{ main {{ width: min(100% - 20px, 1180px); margin-top: 12px; }} section {{ padding: 15px; }} .row {{ grid-template-columns: 1fr; gap: 4px; }} header {{ padding: 20px; }} }}
  </style>
</head>
<body>
  <header>
    <h1>AgentCheck 工作台</h1>
    <p>Rust CLI 共享记录 · 真实执行、演示数据和历史结果分开标记</p>
  </header>
  <main>
    <section>
      <h2>当前服务</h2>
      <dl class="meta">
        <div><dt>模型</dt><dd id="model"></dd></div>
        <div><dt>地址</dt><dd id="endpoint"></dd></div>
        <div><dt>记录</dt><dd id="record-id"></dd></div>
        <div><dt>来源</dt><dd id="origin"></dd></div>
      </dl>
    </section>
    <section>
      <h2>立即测试</h2>
      <div class="toolbar">
        <button id="run" type="button">立即测试</button>
        <button id="all" class="secondary" type="button">选择全部</button>
        <button id="none" class="secondary" type="button">清空选择</button>
        <span id="run-state" role="status"></span>
      </div>
      <p class="source">选择只影响下一次 CLI 编排；本工作台不重新定义模块结论。</p>
    </section>
    <section>
      <h2>检测项目</h2>
      <div class="modules">{module_cards}</div>
    </section>
    <section>
      <h2>进度与结果</h2>
      <div id="progress" class="progress"></div>
    </section>
    <section>
      <h2>结论</h2>
      <p id="conclusion" class="empty"></p>
      <p class="source">GUI、CLI 和记录使用同一份选择清单、模块状态、证据入口及结论；未测量内容不会被包装成通过。</p>
    </section>
  </main>
  <script>
    const report = {report_json};
    const labels = {{ ingress: "服务接入", specification: "模型规格", capability: "能力跑分", performance: "性能实测", agent: "智能体实测", baseline: "模型基线" }};
    const statusLabel = state => ({{ pass: "通过", fail: "失败", unsupported: "不支持", inconclusive: "待确认", invalid_execution: "执行无效", not_applicable: "不适用", not_selected: "未选择", unverified: "未验证" }}[state] || state);
    document.querySelector("#model").textContent = report.configuration.model;
    document.querySelector("#endpoint").textContent = report.configuration.redacted_endpoint;
    document.querySelector("#record-id").textContent = report.record.id;
    document.querySelector("#origin").textContent = report.execution_origin;
    document.querySelector("#conclusion").textContent = `${{report.customer_conclusion.text}} · 运行状态：${{report.record.lifecycle}}`;
    const selected = new Set(report.selected_modules);
    document.querySelectorAll(".module input").forEach(input => {{
      input.checked = selected.has(input.value);
      input.addEventListener("change", () => updateSelection());
    }});
    function updateSelection() {{
      document.querySelectorAll(".module input").forEach(input => {{
        const state = document.querySelector(`[data-state="${{input.value}}"]`);
        state.textContent = input.checked ? "待执行" : "未选";
        state.dataset.status = input.checked ? "unverified" : "not_selected";
      }});
    }}
    document.querySelector("#all").addEventListener("click", () => {{ document.querySelectorAll(".module input").forEach(input => input.checked = true); updateSelection(); }});
    document.querySelector("#none").addEventListener("click", () => {{ document.querySelectorAll(".module input").forEach(input => input.checked = false); updateSelection(); }});
    document.querySelector("#run").addEventListener("click", () => {{
      const chosen = [...document.querySelectorAll(".module input:checked")].map(input => input.value);
      document.querySelector("#run-state").textContent = chosen.length ? `已记录选择：${{chosen.map(id => labels[id]).join("、")}}；请用同一 CLI 进程执行检测` : "至少选择一个检测项目";
    }});
    const progress = document.querySelector("#progress");
    report.record.module_results.forEach(result => {{
      const row = document.createElement("div"); row.className = "row";
      const name = document.createElement("strong"); name.textContent = labels[result.module_id] || result.module_id;
      const state = document.createElement("span"); state.className = `state ${{result.state}}`; state.textContent = statusLabel(result.state);
      const reason = document.createElement("span"); reason.className = "source"; reason.textContent = result.reason || "已记录当前状态和证据入口";
      row.append(name, state, reason); progress.append(row);
    }});
    updateSelection();
  </script>
</body>
</html>"##,
        report_json = report_json,
        module_cards = module_cards,
    ))
}

pub fn write_workbench(
    report: &CliRunReport,
    directory: impl AsRef<Path>,
) -> Result<PathBuf, String> {
    let directory = directory.as_ref();
    fs::create_dir_all(directory).map_err(|error| format!("创建工作台目录失败：{error}"))?;
    let path = directory.join("agentcheck-workbench.html");
    let html = render_workbench_html(report).map_err(|error| format!("渲染工作台失败：{error}"))?;
    fs::write(&path, html).map_err(|error| format!("写入工作台失败：{error}"))?;
    Ok(path)
}

pub fn open_workbench<E: GuiLauncher>(
    report: &CliRunReport,
    directory: impl AsRef<Path>,
    environment: DesktopEnvironment,
    launcher: &mut E,
) -> GuiLaunchResult {
    if !environment.supported {
        return GuiLaunchResult {
            state: GuiLaunchState::Unavailable,
            path: None,
            reason: environment.reason,
        };
    }
    let path = match write_workbench(report, directory) {
        Ok(path) => path,
        Err(reason) => {
            return GuiLaunchResult {
                state: GuiLaunchState::Failed,
                path: None,
                reason,
            };
        }
    };
    match launcher.launch(&path) {
        Ok(()) => GuiLaunchResult {
            state: GuiLaunchState::Launched,
            path: Some(path.display().to_string()),
            reason: "已打开本地工作台；CLI 仍保留为独立回退入口".into(),
        },
        Err(reason) => GuiLaunchResult {
            state: GuiLaunchState::Failed,
            path: Some(path.display().to_string()),
            reason: format!("{reason}；CLI 仍可继续读取结果"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{CliRunRequest, UnavailableExecutor, run_with_executor};

    #[derive(Debug, Default)]
    struct RecordingLauncher {
        paths: Vec<PathBuf>,
        fail: bool,
    }

    impl GuiLauncher for RecordingLauncher {
        fn launch(&mut self, path: &Path) -> Result<(), String> {
            self.paths.push(path.to_path_buf());
            if self.fail {
                Err("测试启动失败".into())
            } else {
                Ok(())
            }
        }
    }

    fn report() -> CliRunReport {
        let request = CliRunRequest {
            endpoint: "https://api.example.test/v1/chat".into(),
            model: "model-a".into(),
            api_key: None,
            selected_modules: Some(vec!["capability".into()]),
            stop_after: None,
            run_id: "run-gui".into(),
            started_at: "2026-09-11T00:00:00Z".into(),
        };
        run_with_executor(request, &mut UnavailableExecutor).unwrap()
    }

    #[test]
    fn desktop_detection_keeps_linux_headless_and_ci_on_cli() {
        let mut env = BTreeMap::new();
        assert!(!detect_desktop(DesktopPlatform::Linux, &env).supported);
        env.insert("DISPLAY".into(), ":0".into());
        assert!(detect_desktop(DesktopPlatform::Linux, &env).supported);
        env.insert("CI".into(), "true".into());
        assert!(!detect_desktop(DesktopPlatform::Linux, &env).supported);
    }

    #[test]
    fn workbench_contains_shared_record_selection_and_six_modules() {
        let html = render_workbench_html(&report()).unwrap();
        assert!(html.contains("run-gui"));
        assert!(html.contains("capability"));
        assert!(html.contains("服务接入"));
        assert!(html.contains("模型规格"));
        assert!(html.contains("能力跑分"));
        assert!(html.contains("性能实测"));
        assert!(html.contains("智能体实测"));
        assert!(html.contains("模型基线"));
        assert!(!html.contains("secret"));
    }

    #[test]
    fn gui_failure_returns_a_cli_fallback_result() {
        let directory = std::env::temp_dir().join("agentcheck-gui-test");
        let mut launcher = RecordingLauncher {
            fail: true,
            ..Default::default()
        };
        let result = open_workbench(
            &report(),
            &directory,
            DesktopEnvironment {
                platform: DesktopPlatform::Macos,
                supported: true,
                display_available: true,
                reason: "测试桌面".into(),
            },
            &mut launcher,
        );
        assert_eq!(result.state, GuiLaunchState::Failed);
        assert!(result.reason.contains("CLI"));
        assert_eq!(launcher.paths.len(), 1);
    }
}
