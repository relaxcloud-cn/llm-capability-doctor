use crate::model::RunResult;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
pub fn default_root() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os("AGENTCHECK_DATA_DIR") {
        return Ok(PathBuf::from(root).join("results"));
    }
    #[cfg(target_os = "windows")]
    {
        return std::env::var_os("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("AgentCheck/Results"))
            .context("无法找到本机应用数据目录");
    }
    #[cfg(not(target_os = "windows"))]
    {
        let home = std::env::var_os("HOME").context("无法找到本机用户目录")?;
        if cfg!(target_os = "macos") {
            Ok(PathBuf::from(home).join("Library/Application Support/AgentCheck/results"))
        } else {
            Ok(std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(home).join(".local/share"))
                .join("AgentCheck/results"))
        }
    }
}
pub fn directory(root: &Path, id: &str) -> Result<PathBuf> {
    if !id.starts_with("AC-")
        || id.len() > 128
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("无效的检测编号");
    }
    Ok(root.join(id))
}
pub fn result(root: &Path, id: &str) -> Result<RunResult> {
    let path = directory(root, id)?.join("result.json");
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.len() > 16 * 1024 * 1024 {
        bail!("检测结果不是有效的本机普通文件");
    }
    let run: RunResult = serde_json::from_slice(&std::fs::read(&path)?)?;
    if run.id != id {
        bail!("检测编号与结果文件不一致");
    }
    Ok(run)
}
pub fn history(root: &Path, active: Option<&str>) -> Result<Vec<RunResult>> {
    if !root.exists() {
        return Ok(vec![]);
    }
    let mut runs = vec![];
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if active == Some(&id) {
            continue;
        }
        if let Ok(run) = result(root, &id) {
            if run.finished_at.is_some() {
                runs.push(run);
            }
        }
    }
    runs.sort_by_key(|run| std::cmp::Reverse(run.started_at));
    Ok(runs)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{Protocol, Target},
        engine::atomic_json,
    };
    #[test]
    fn history_keeps_each_run_config_and_rejects_traversal_and_wrong_identity() {
        let root = tempfile::tempdir().unwrap();
        assert!(directory(root.path(), "../../secret").is_err());
        let mut run = RunResult::new(Target {
            model: "original model".into(),
            endpoint: "https://a.example/v1/chat/completions".into(),
            protocol: Protocol::ChatCompletions,
            insecure: false,
        });
        run.finished_at = Some(chrono::Utc::now());
        atomic_json(&root.path().join(&run.id).join("result.json"), &run).unwrap();
        assert_eq!(
            history(root.path(), None).unwrap()[0].target.model,
            "original model"
        );
        assert!(history(root.path(), Some(&run.id)).unwrap().is_empty());
        let wrong = root.path().join("AC-wrong");
        atomic_json(&wrong.join("result.json"), &run).unwrap();
        assert!(result(root.path(), "AC-wrong").is_err());
    }
}
