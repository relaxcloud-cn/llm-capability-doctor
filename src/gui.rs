use crate::{config::ModelConfig, preflight::ConnectionReceipt};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Macos,
    Windows,
    Linux,
    Other,
}
pub fn platform() -> Platform {
    if cfg!(target_os = "macos") {
        Platform::Macos
    } else if cfg!(target_os = "windows") {
        Platform::Windows
    } else if cfg!(target_os = "linux") {
        Platform::Linux
    } else {
        Platform::Other
    }
}
pub fn desktop_available(platform: Platform, headless: bool) -> bool {
    matches!(platform, Platform::Macos | Platform::Windows) && !headless
}
pub fn current_desktop() -> bool {
    desktop_available(
        platform(),
        std::env::var_os("SSH_CONNECTION").is_some()
            || std::env::var_os("SSH_TTY").is_some()
            || std::env::var_os("SSH_CLIENT").is_some()
            || std::env::var_os("CI").is_some(),
    )
}
pub fn auto_open_report_allowed(
    desktop: bool,
    interactive: bool,
    json_output: bool,
    disabled: bool,
    cancelled: bool,
) -> bool {
    desktop && interactive && !json_output && !disabled && !cancelled
}
fn report_open_command(platform: Platform, file: &Path) -> Result<Command> {
    let mut command = match platform {
        Platform::Macos => {
            let mut command = Command::new("/usr/bin/open");
            command.arg(file);
            command
        }
        Platform::Windows => {
            let system_root =
                std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
            let mut command = Command::new(
                PathBuf::from(system_root).join("System32/WindowsPowerShell/v1.0/powershell.exe"),
            );
            command.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference='Stop'; Start-Process -FilePath $env:AGENTCHECK_REPORT_FILE"]);
            command.env("AGENTCHECK_REPORT_FILE", file);
            command
        }
        _ => bail!("本平台保留报告路径，不自动打开浏览器"),
    };
    command
        .env_remove("MODEL_API_KEY")
        .env_remove("AGENTCHECK_SSH_PASSWORD")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    Ok(command)
}
async fn dispatch_report(mut command: Command, timeout: Duration) -> Result<()> {
    let status = tokio::time::timeout(timeout, command.status())
        .await
        .context("打开报告等待超时")??;
    if !status.success() {
        bail!("系统未能打开报告");
    }
    Ok(())
}
pub async fn open_report_file(path: &Path) -> Result<()> {
    let file = path.canonicalize().context("报告文件不存在")?;
    if !file.is_file() || file.extension().is_none_or(|value| value != "html") {
        bail!("报告必须是已保存的 HTML 文件");
    }
    dispatch_report(
        report_open_command(platform(), &file)?,
        Duration::from_secs(5),
    )
    .await
}
pub fn executable() -> Result<PathBuf> {
    if let Some(path) = crate::runtime::component("gui")? {
        return Ok(path);
    }
    if let Some(path) = std::env::var_os("AGENTCHECK_GUI_PATH") {
        return Ok(PathBuf::from(path));
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let relative = match platform() {
        Platform::Macos if cfg!(target_arch = "aarch64") => {
            "desktop/out/AgentCheck-darwin-arm64/AgentCheck.app/Contents/MacOS/AgentCheck"
        }
        Platform::Macos => {
            "desktop/out/AgentCheck-darwin-x64/AgentCheck.app/Contents/MacOS/AgentCheck"
        }
        Platform::Windows if cfg!(target_arch = "aarch64") => {
            "desktop/out/AgentCheck-win32-arm64/AgentCheck.exe"
        }
        Platform::Windows => "desktop/out/AgentCheck-win32-x64/AgentCheck.exe",
        _ => bail!("本平台使用纯 CLI"),
    };
    let path = root.join(relative);
    if !path.is_file() {
        bail!("桌面程序尚未准备");
    }
    Ok(path)
}
async fn terminate(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
        #[cfg(windows)]
        {
            let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
            let _ = Command::new(PathBuf::from(root).join("System32/taskkill.exe"))
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await;
        }
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
}
/// Open local history without credentials or model traffic for delivery verification.
pub async fn open_offline(report_root: &Path) -> Result<()> {
    let mut command = Command::new(executable()?);
    command
        .arg("--agentcheck-cli")
        .arg(std::env::current_exe()?)
        .arg("--agentcheck-report-dir")
        .arg(std::path::absolute(report_root)?)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    let mut child = command.spawn().context("离线桌面自检启动失败")?;
    let mut lines = BufReader::new(child.stdout.take().context("GUI 启动确认管道不可用")?).lines();
    let ready = async {
        while let Some(line) = lines.next_line().await? {
            let value: Value = serde_json::from_str(&line)?;
            if value["type"] == "ready" {
                return Ok::<(), anyhow::Error>(());
            }
        }
        bail!("GUI 在就绪前退出")
    };
    if !matches!(
        tokio::time::timeout(Duration::from_secs(10), ready).await,
        Ok(Ok(()))
    ) {
        terminate(&mut child).await;
        bail!("离线桌面自检未在 10 秒内完成启动");
    }
    Ok(())
}
pub async fn launch(
    config: &ModelConfig,
    receipt: &ConnectionReceipt,
    report_root: &Path,
    no_open_report: bool,
) -> Result<()> {
    let gui = executable()?;
    let mut command = Command::new(gui);
    command
        .arg("--agentcheck-cli")
        .arg(std::env::current_exe()?)
        .arg("--agentcheck-report-dir")
        .arg(std::path::absolute(report_root)?)
        .arg("--agentcheck-handoff")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    if no_open_report {
        command.arg("--agentcheck-no-open-report");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    let mut child = command.spawn().context("桌面程序启动失败")?;
    let mut lines = BufReader::new(child.stdout.take().context("GUI 启动确认管道不可用")?).lines();
    let mut input = child.stdin.take().context("GUI 配置管道不可用")?;
    let prepared = async {
        loop {
            let line = lines.next_line().await?.context("GUI 提前退出")?;
            let value: Value = serde_json::from_str(&line).context("GUI 启动确认无效")?;
            if value["type"] == "ready" {
                break;
            }
        }
        let mut data = serde_json::to_vec(
            &json!({"type":"prepare","config":{"base_url":config.endpoint.as_str(),"model":config.model,"api_key":config.api_key(),"protocol":config.protocol},"receipt":receipt}),
        )?;
        data.push(b'\n');
        input.write_all(&data).await?;
        input.flush().await?;
        loop {
            let line = lines.next_line().await?.context("GUI 准备配置时退出")?;
            let value: Value = serde_json::from_str(&line).context("GUI 配置确认无效")?;
            if value["type"] == "prepared" {
                return Ok::<(), anyhow::Error>(());
            }
        }
    };
    match tokio::time::timeout(Duration::from_secs(10), prepared).await {
        Ok(Ok(())) => {}
        _ => {
            terminate(&mut child).await;
            bail!("GUI 未在 10 秒内确认启动，回退到纯 CLI");
        }
    }
    // No model task starts before this short commit message. Once committed, the GUI owns execution.
    if input.write_all(b"{\"type\":\"begin\"}\n").await.is_err() {
        terminate(&mut child).await;
        bail!("GUI 交接提交失败，回退到纯 CLI");
    }
    let _ = input.flush().await;
    drop(input);
    drop(lines);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn report_auto_open_requires_interactive_desktop_and_respects_suppression() {
        assert!(auto_open_report_allowed(true, true, false, false, false));
        for flags in [
            (false, true, false, false, false),
            (true, false, false, false, false),
            (true, true, true, false, false),
            (true, true, false, true, false),
            (true, true, false, false, true),
        ] {
            assert!(!auto_open_report_allowed(
                flags.0, flags.1, flags.2, flags.3, flags.4
            ));
        }
    }
    #[test]
    fn report_paths_are_arguments_or_environment_values_not_shell_code() {
        let path = Path::new("/tmp/report ' $(command) & space/report.html");
        let mac = report_open_command(Platform::Macos, path).unwrap();
        assert_eq!(mac.as_std().get_program(), "/usr/bin/open");
        assert_eq!(
            mac.as_std().get_args().collect::<Vec<_>>(),
            vec![path.as_os_str()]
        );
        let windows = report_open_command(Platform::Windows, path).unwrap();
        assert!(
            windows
                .as_std()
                .get_envs()
                .any(|(key, value)| key == "AGENTCHECK_REPORT_FILE"
                    && value == Some(path.as_os_str()))
        );
        assert!(
            !windows
                .as_std()
                .get_args()
                .any(|arg| arg == path.as_os_str())
        );
        assert!(
            windows
                .as_std()
                .get_envs()
                .any(|(key, value)| key == "MODEL_API_KEY" && value.is_none())
        );
        assert!(report_open_command(Platform::Linux, path).is_err());
    }
    #[tokio::test]
    async fn missing_or_non_html_files_are_rejected_without_launching() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            open_report_file(&dir.path().join("missing.html"))
                .await
                .is_err()
        );
        let path = dir.path().join("not-a-report.txt");
        std::fs::write(&path, "text").unwrap();
        assert!(open_report_file(&path).await.is_err());
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn opener_failure_and_timeout_do_not_wait_indefinitely() {
        assert!(
            dispatch_report(Command::new("/usr/bin/false"), Duration::from_secs(1))
                .await
                .is_err()
        );
        let mut command = Command::new("/bin/sleep");
        command.arg("10").kill_on_drop(true);
        let started = std::time::Instant::now();
        assert!(
            dispatch_report(command, Duration::from_millis(20))
                .await
                .is_err()
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn desktop_policy_keeps_linux_cli_and_ssh_headless() {
        assert!(desktop_available(Platform::Macos, false));
        assert!(desktop_available(Platform::Windows, false));
        assert!(!desktop_available(Platform::Linux, false));
        assert!(!desktop_available(Platform::Windows, true));
    }
}
