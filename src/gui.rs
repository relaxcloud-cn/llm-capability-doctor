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
            || std::env::var_os("CI").is_some(),
    )
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
    fn desktop_policy_keeps_linux_cli_and_ssh_headless() {
        assert!(desktop_available(Platform::Macos, false));
        assert!(desktop_available(Platform::Windows, false));
        assert!(!desktop_available(Platform::Linux, false));
        assert!(!desktop_available(Platform::Windows, true));
    }
}
