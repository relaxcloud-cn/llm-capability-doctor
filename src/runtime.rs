#[cfg(any(feature = "bundled-runtime", test))]
use anyhow::Context;
use anyhow::Result;
#[cfg(any(feature = "bundled-runtime", test))]
use anyhow::bail;
#[cfg(any(feature = "bundled-runtime", test))]
use serde::Deserialize;
#[cfg(any(feature = "bundled-runtime", test))]
use sha2::{Digest, Sha256};
use std::path::PathBuf;
#[cfg(any(feature = "bundled-runtime", test))]
use std::path::{Component, Path};
#[cfg(any(feature = "bundled-runtime", test))]
use std::{
    collections::BTreeMap,
    io::{Read, Write},
};
#[cfg(feature = "bundled-runtime")]
mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded.rs"));
}
#[cfg(feature = "bundled-runtime")]
#[derive(Deserialize)]
struct Manifest {
    target: String,
    omp_version: String,
    components: BTreeMap<String, Bundle>,
}
#[cfg(any(feature = "bundled-runtime", test))]
#[derive(Deserialize)]
struct Bundle {
    archive_sha256: String,
    entrypoint: String,
    entries: Vec<Entry>,
}
#[cfg(any(feature = "bundled-runtime", test))]
#[derive(Deserialize)]
struct Entry {
    path: String,
    kind: String,
    size: u64,
    sha256: Option<String>,
    target: Option<String>,
    executable: bool,
}
#[cfg(any(feature = "bundled-runtime", test))]
fn relative(path: &str) -> Result<PathBuf> {
    let path = Path::new(path);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!("运行环境路径不安全");
    }
    Ok(path.to_owned())
}
#[cfg(any(feature = "bundled-runtime", test))]
fn link_safe(path: &str, target: &str) -> Result<()> {
    let mut depth = relative(path)?.components().count().saturating_sub(1);
    for component in Path::new(target).components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir => {
                if depth == 0 {
                    bail!("运行环境链接指向组件外部");
                }
                depth -= 1;
            }
            _ => bail!("运行环境链接不安全"),
        }
    }
    Ok(())
}
#[cfg(any(feature = "bundled-runtime", test))]
fn digest(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let len = file.read(&mut buffer)?;
        if len == 0 {
            break;
        }
        hash.update(&buffer[..len]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
#[cfg(any(feature = "bundled-runtime", test))]
fn valid(root: &Path, bundle: &Bundle) -> bool {
    if !std::fs::symlink_metadata(root)
        .is_ok_and(|meta| meta.is_dir() && !meta.file_type().is_symlink())
        || bundle.entries.is_empty()
        || relative(&bundle.entrypoint).is_err()
        || !bundle
            .entries
            .iter()
            .any(|entry| entry.path == bundle.entrypoint && entry.kind == "file")
    {
        return false;
    }
    bundle.entries.iter().all(|entry| {
        let Ok(path) = relative(&entry.path).map(|p| root.join(p)) else {
            return false;
        };
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            return false;
        };
        match entry.kind.as_str() {
            "directory" => meta.is_dir() && !meta.file_type().is_symlink(),
            "symlink" => {
                meta.file_type().is_symlink()
                    && std::fs::read_link(&path).is_ok_and(|p| {
                        Some(p.to_string_lossy().as_ref()) == entry.target.as_deref()
                    })
            }
            "file" => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if entry.executable && meta.permissions().mode() & 0o100 == 0 {
                        return false;
                    }
                }
                meta.is_file()
                    && !meta.file_type().is_symlink()
                    && meta.len() == entry.size
                    && digest(&path).ok().as_deref() == entry.sha256.as_deref()
            }
            _ => false,
        }
    })
}
#[cfg(feature = "bundled-runtime")]
fn cache_root() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("AGENTCHECK_CACHE_DIR") {
        return Ok(PathBuf::from(path));
    }
    #[cfg(windows)]
    {
        return std::env::var_os("LOCALAPPDATA")
            .map(|v| PathBuf::from(v).join("AgentCheck/Cache"))
            .context("没有可用的本机缓存目录");
    }
    #[cfg(not(windows))]
    {
        let home = std::env::var_os("HOME").context("没有可用的用户目录")?;
        if cfg!(target_os = "macos") {
            Ok(PathBuf::from(home).join("Library/Caches/AgentCheck"))
        } else {
            Ok(std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(home).join(".cache"))
                .join("AgentCheck"))
        }
    }
}
#[cfg(any(feature = "bundled-runtime", test))]
fn unpack(root: &Path, bundle: &Bundle, bytes: &[u8]) -> Result<()> {
    if format!("{:x}", Sha256::digest(bytes)) != bundle.archive_sha256 {
        bail!("内置运行环境压缩包校验失败");
    }
    let entries: BTreeMap<_, _> = bundle
        .entries
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    let reader = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(reader);
    let mut links = vec![];
    for item in archive.entries()? {
        let mut item = item?;
        let name = item
            .path()?
            .to_string_lossy()
            .trim_end_matches('/')
            .to_owned();
        let path = relative(&name)?;
        let entry = entries
            .get(name.as_str())
            .context("压缩包包含未登记的文件")?;
        if !seen.insert(name.clone()) {
            bail!("压缩包文件路径重复");
        }
        let target = root.join(path);
        match entry.kind.as_str() {
            "directory" => {
                if !item.header().entry_type().is_dir() {
                    bail!("运行环境目录类型不一致");
                }
                std::fs::create_dir_all(&target)?;
            }
            "file" => {
                if !item.header().entry_type().is_file() || item.size() != entry.size {
                    bail!("运行环境文件类型或大小不一致");
                }
                std::fs::create_dir_all(target.parent().context("运行环境文件没有父目录")?)?;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)?;
                std::io::copy(&mut item, &mut file)?;
                file.flush()?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(
                        &target,
                        std::fs::Permissions::from_mode(if entry.executable {
                            0o700
                        } else {
                            0o600
                        }),
                    )?;
                }
            }
            "symlink" => {
                if !item.header().entry_type().is_symlink() {
                    bail!("运行环境链接类型不一致");
                }
                let link = item
                    .link_name()?
                    .context("运行环境链接缺少目标")?
                    .to_string_lossy()
                    .into_owned();
                if Some(link.as_str()) != entry.target.as_deref() {
                    bail!("运行环境链接目标与清单不一致");
                }
                link_safe(&name, &link)?;
                links.push((target, link));
            }
            _ => bail!("运行环境清单包含不支持的类型"),
        }
    }
    if seen.len() != entries.len() {
        bail!("运行环境文件缺失");
    }
    for (path, target) in links {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, path)?;
        }
        #[cfg(windows)]
        {
            let _ = (path, target);
            bail!("Windows 运行环境不能包含未配置权限的链接");
        }
    }
    if !valid(root, bundle) {
        bail!("释放后的运行环境校验失败");
    }
    Ok(())
}
pub fn is_bundled() -> bool {
    cfg!(feature = "bundled-runtime")
}
pub fn component(name: &str) -> Result<Option<PathBuf>> {
    #[cfg(not(feature = "bundled-runtime"))]
    {
        let _ = name;
        Ok(None)
    }
    #[cfg(feature = "bundled-runtime")]
    {
        let manifest: Manifest = serde_json::from_str(embedded::MANIFEST)?;
        if manifest.target != env!("AGENTCHECK_TARGET")
            || manifest.omp_version != crate::model::OMP_VERSION
        {
            bail!("内置运行环境版本或平台不匹配");
        }
        let Some(bundle) = manifest.components.get(name) else {
            return Ok(None);
        };
        let bytes = match name {
            "omp" => embedded::OMP,
            "gui" => embedded::GUI,
            _ => bail!("未知运行环境组件"),
        };
        let cache = cache_root()?.join(&bundle.archive_sha256);
        std::fs::create_dir_all(&cache)?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(cache.join(format!("{name}.lock")))?;
        fs2::FileExt::lock_exclusive(&lock)?;
        let destination = cache.join(name);
        if !valid(&destination, bundle) {
            let staging = tempfile::Builder::new()
                .prefix("extract-")
                .tempdir_in(&cache)?;
            unpack(staging.path(), bundle, bytes)?;
            let old = cache.join(format!("replaced-{}", uuid::Uuid::new_v4()));
            if std::fs::symlink_metadata(&destination).is_ok() {
                std::fs::rename(&destination, &old)?;
            }
            let extracted = staging.keep();
            if let Err(error) = std::fs::rename(&extracted, &destination) {
                if old.exists() {
                    let _ = std::fs::rename(old, &destination);
                }
                return Err(error.into());
            }
            if old.exists() {
                let _ = std::fs::remove_dir_all(old);
            }
        }
        Ok(Some(destination.join(relative(&bundle.entrypoint)?)))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_components_verify_every_file_and_detect_cache_damage() {
        let content = b"runtime payload";
        let mut encoded = Vec::new();
        {
            let gzip = flate2::write::GzEncoder::new(&mut encoded, flate2::Compression::default());
            let mut archive = tar::Builder::new(gzip);
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o700);
            header.set_cksum();
            archive
                .append_data(&mut header, "bin/runtime", &content[..])
                .unwrap();
            archive.into_inner().unwrap().finish().unwrap();
        }
        let bundle = Bundle {
            archive_sha256: format!("{:x}", Sha256::digest(&encoded)),
            entrypoint: "bin/runtime".into(),
            entries: vec![Entry {
                path: "bin/runtime".into(),
                kind: "file".into(),
                size: content.len() as u64,
                sha256: Some(format!("{:x}", Sha256::digest(content))),
                target: None,
                executable: true,
            }],
        };
        let root = tempfile::tempdir().unwrap();
        assert!(!valid(root.path(), &bundle));
        unpack(root.path(), &bundle, &encoded).unwrap();
        assert!(valid(root.path(), &bundle));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let executable = root.path().join("bin/runtime");
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert!(
                !valid(root.path(), &bundle),
                "missing execute permission must trigger repair"
            );
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
            let links = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(root.path(), links.path().join("component")).unwrap();
            assert!(!valid(&links.path().join("component"), &bundle));
        }
        std::fs::write(root.path().join("bin/runtime"), b"broken").unwrap();
        assert!(!valid(root.path(), &bundle));
        let mut corrupted = encoded.clone();
        corrupted[0] ^= 1;
        let clean = tempfile::tempdir().unwrap();
        assert!(unpack(clean.path(), &bundle, &corrupted).is_err());
        assert!(!clean.path().join("bin/runtime").exists());
    }
    #[test]
    fn paths_and_framework_links_are_confined_to_the_component() {
        assert!(relative("../secret").is_err());
        assert!(relative("/absolute").is_err());
        assert!(link_safe("X.framework/Versions/Current", "A").is_ok());
        assert!(link_safe("X.framework/Contents", "Versions/Current/Contents").is_ok());
        assert!(link_safe("a/link", "../../outside").is_err());
    }
}
