use crate::bundle_format::{Manifest, safe_path};
use flate2::read::GzDecoder;
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

struct EmbeddedComponent {
    name: &'static str,
    digest: &'static str,
    manifest: &'static str,
    archive: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/bundle.rs"));

static RUNTIME: OnceLock<Result<Option<PathBuf>, String>> = OnceLock::new();
static GUI: OnceLock<Result<Option<PathBuf>, String>> = OnceLock::new();

pub fn is_bundled() -> bool {
    !COMPONENTS.is_empty()
}

pub fn omp_path() -> Result<Option<PathBuf>, String> {
    RUNTIME
        .get_or_init(|| prepare("runtime"))
        .clone()
        .map(|root| root.map(|path| path.join(if cfg!(windows) { "omp.exe" } else { "omp" })))
}

pub fn gui_path() -> Result<Option<PathBuf>, String> {
    GUI.get_or_init(|| prepare("gui"))
        .clone()
        .map(|root| root.map(|path| path.join("AgentCheck.app/Contents/MacOS/AgentCheck")))
}

fn cache_directory(
    platform: &str,
    environment: &BTreeMap<String, String>,
) -> Result<PathBuf, String> {
    let absolute = |name: &str| {
        environment
            .get(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    if let Some(path) = absolute("AGENTCHECK_CACHE_DIR") {
        return Ok(path);
    }
    if environment.contains_key("AGENTCHECK_CACHE_DIR") {
        return Err("AGENTCHECK_CACHE_DIR 必须是绝对路径".into());
    }
    let path = match platform {
        "windows" => absolute("LOCALAPPDATA").map(|path| path.join("AgentCheck/Cache")),
        "macos" => absolute("HOME").map(|path| path.join("Library/Caches/AgentCheck")),
        _ => absolute("XDG_CACHE_HOME")
            .map(|path| path.join("agentcheck"))
            .or_else(|| absolute("HOME").map(|path| path.join(".cache/agentcheck"))),
    };
    path.ok_or_else(|| {
        "找不到用户缓存目录；请设置 AGENTCHECK_CACHE_DIR 为可写且允许执行程序的绝对路径".into()
    })
}

fn prepare(name: &str) -> Result<Option<PathBuf>, String> {
    let Some(component) = COMPONENTS.iter().find(|component| component.name == name) else {
        return Ok(None);
    };
    let cache = cache_directory(env::consts::OS, &env::vars().collect())?;
    install(component, &cache).map(Some).map_err(|error| {
        format!(
            "准备内置组件失败：{error}；请检查缓存目录权限和可用空间，或设置 AGENTCHECK_CACHE_DIR"
        )
    })
}

fn checked_directory(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    // Reject redirected cache paths before writing executable content.
    for ancestor in path.ancestors() {
        if fs::symlink_metadata(ancestor)?.file_type().is_symlink() {
            // macOS /var and /tmp are system-managed links; callers canonicalize the parent.
            return Err(io::Error::other("缓存路径包含符号链接"));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn regular_file(path: &Path) -> io::Result<File> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(io::Error::other("缓存包含非普通文件"));
    }
    File::open(path)
}

fn verify(root: &Path, manifest: &Manifest) -> io::Result<()> {
    if !fs::symlink_metadata(root)?.is_dir() {
        return Err(io::Error::other("缓存目录无效"));
    }
    for expected in &manifest.files {
        if !safe_path(&expected.path) {
            return Err(io::Error::other("内置组件路径无效"));
        }
        let path = root.join(&expected.path);
        let mut parent = path.parent();
        while let Some(directory) = parent.filter(|directory| *directory != root) {
            let metadata = fs::symlink_metadata(directory)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(io::Error::other("缓存子目录无效"));
            }
            parent = directory.parent();
        }
        let mut file = regular_file(&path)?;
        if file.metadata()?.len() != expected.size {
            return Err(io::Error::other("缓存文件大小不匹配"));
        }
        let mut hasher = Sha256::new();
        io::copy(&mut file, &mut hasher)?;
        if format!("{:x}", hasher.finalize()) != expected.sha256 {
            return Err(io::Error::other("缓存文件校验失败"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if expected.executable && file.metadata()?.permissions().mode() & 0o100 == 0 {
                return Err(io::Error::other("缓存程序缺少执行权限"));
            }
        }
    }
    Ok(())
}

fn unpack(
    component: &EmbeddedComponent,
    manifest: &Manifest,
    destination: &Path,
) -> io::Result<()> {
    let files = manifest
        .files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect::<BTreeMap<_, _>>();
    if files.len() != manifest.files.len() {
        return Err(io::Error::other("组件清单包含重复文件"));
    }
    let mut seen = BTreeSet::new();
    let mut archive = tar::Archive::new(GzDecoder::new(component.archive));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry
            .path()?
            .to_str()
            .ok_or_else(|| io::Error::other("组件路径不是 UTF-8"))?
            .to_owned();
        let Some(expected) = files.get(path.as_str()) else {
            return Err(io::Error::other("组件包含未登记文件"));
        };
        if !safe_path(&path)
            || !entry.header().entry_type().is_file()
            || !seen.insert(path.clone())
            || entry.size() != expected.size
        {
            return Err(io::Error::other("组件包含非法路径、链接或不匹配的文件"));
        }
        let output = destination.join(&path);
        fs::create_dir_all(output.parent().unwrap())?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&output)?;
        io::copy(&mut entry, &mut file)?;
        file.flush()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(if expected.executable {
                0o700
            } else {
                0o600
            }))?;
        }
    }
    if seen.len() != files.len() {
        return Err(io::Error::other("组件缺少文件"));
    }
    verify(destination, manifest)
}

fn install(component: &EmbeddedComponent, cache: &Path) -> io::Result<PathBuf> {
    if format!("{:x}", Sha256::digest(component.manifest.as_bytes())) != component.digest {
        return Err(io::Error::other("组件清单校验失败"));
    }
    let manifest: Manifest = serde_json::from_str(component.manifest)?;
    if manifest.target != BUILD_TARGET || manifest.files.is_empty() {
        return Err(io::Error::other("组件目标平台不匹配或清单为空"));
    }
    if fs::symlink_metadata(cache).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(io::Error::other("缓存目录不能是符号链接"));
    }
    fs::create_dir_all(cache)?;
    let cache = fs::canonicalize(cache)?;
    checked_directory(&cache)?;
    let directory = cache.join(BUILD_TARGET);
    checked_directory(&directory)?;
    let lock_path = directory.join("runtime.lock");
    if fs::symlink_metadata(&lock_path)
        .is_ok_and(|metadata| !metadata.is_file() || metadata.file_type().is_symlink())
    {
        return Err(io::Error::other("缓存锁文件无效"));
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.lock_exclusive()?;
    let current = directory.join(format!("{}-{}", component.name, component.digest));
    if verify(&current, &manifest).is_ok() {
        return Ok(current);
    }
    let stage = tempfile::Builder::new()
        .prefix(".extract-")
        .tempdir_in(&directory)?;
    unpack(component, &manifest, stage.path())?;
    if current.exists() || fs::symlink_metadata(&current).is_ok() {
        // Preserve old generations, including executables still open on Windows.
        let stale = tempfile::Builder::new()
            .prefix(".stale-")
            .tempdir_in(&directory)?;
        fs::rename(&current, stale.path().join("old"))?;
        let _ = stale.keep();
    }
    fs::rename(stage.path(), &current)?;
    Ok(current)
}

pub fn check() -> Result<serde_json::Value, String> {
    let omp = omp_path()?
        .ok_or_else(|| "当前为开发构建，未内置分析程序；请使用单文件发行包".to_string())?;
    let result = Command::new(&omp)
        .arg("--version")
        .output()
        .map_err(|error| format!("内置分析程序无法执行：{error}"))?;
    if !result.status.success() {
        let detail = String::from_utf8_lossy(&result.stderr)
            .chars()
            .take(2000)
            .collect::<String>();
        return Err(format!(
            "内置分析程序启动失败：{}；{}",
            result.status,
            detail.trim()
        ));
    }
    Ok(
        serde_json::json!({"target": BUILD_TARGET, "omp": omp, "omp_version": String::from_utf8_lossy(&result.stdout).trim(), "gui": gui_path()?, "status": "pass"}),
    )
}

pub fn licenses() -> Result<String, String> {
    let omp = omp_path()?
        .ok_or_else(|| "开发构建未内置第三方程序；请使用单文件发行包查看第三方声明".to_string())?;
    let root = omp.parent().unwrap();
    ["OMP-LICENSE.txt", "THIRD-PARTY-NOTICES.txt"]
        .into_iter()
        .map(|name| {
            fs::read_to_string(root.join(name))
                .map_err(|error| format!("读取第三方声明失败：{error}"))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle_format::BundleFile;
    use flate2::{Compression, write::GzEncoder};

    fn fixture() -> EmbeddedComponent {
        let data = b"test-runtime";
        let mut archive = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o700);
        header.set_cksum();
        archive.append_data(&mut header, "omp", &data[..]).unwrap();
        let archive = archive.into_inner().unwrap().finish().unwrap();
        let manifest = serde_json::to_string(&Manifest {
            target: BUILD_TARGET.into(),
            files: vec![BundleFile {
                path: "omp".into(),
                sha256: format!("{:x}", Sha256::digest(data)),
                size: data.len() as u64,
                executable: true,
            }],
        })
        .unwrap();
        let digest = format!("{:x}", Sha256::digest(manifest.as_bytes()));
        EmbeddedComponent {
            name: "test",
            archive: Box::leak(archive.into_boxed_slice()),
            manifest: Box::leak(manifest.into_boxed_str()),
            digest: Box::leak(digest.into_boxed_str()),
        }
    }

    #[test]
    fn cache_locations_cover_all_platforms() {
        let home = env::temp_dir().join("home");
        let environment = BTreeMap::from([
            ("HOME".into(), home.display().to_string()),
            (
                "LOCALAPPDATA".into(),
                home.join("Local").display().to_string(),
            ),
        ]);
        assert_eq!(
            cache_directory("macos", &environment).unwrap(),
            home.join("Library/Caches/AgentCheck")
        );
        assert_eq!(
            cache_directory("linux", &environment).unwrap(),
            home.join(".cache/agentcheck")
        );
        assert_eq!(
            cache_directory("windows", &environment).unwrap(),
            home.join("Local/AgentCheck/Cache")
        );
        assert!(
            cache_directory(
                "linux",
                &BTreeMap::from([("AGENTCHECK_CACHE_DIR".into(), "relative".into())])
            )
            .is_err()
        );
    }

    #[test]
    fn extraction_reuses_and_repairs_tampered_files() {
        let cache = tempfile::tempdir().unwrap();
        let component = fixture();
        let root = install(&component, cache.path()).unwrap();
        let modified = fs::metadata(root.join("omp")).unwrap().modified().unwrap();
        assert_eq!(install(&component, cache.path()).unwrap(), root);
        assert_eq!(
            fs::metadata(root.join("omp")).unwrap().modified().unwrap(),
            modified
        );
        fs::write(root.join("omp"), b"bad").unwrap();
        assert_eq!(install(&component, cache.path()).unwrap(), root);
        assert_eq!(fs::read(root.join("omp")).unwrap(), b"test-runtime");
    }

    #[test]
    fn concurrent_extraction_publishes_only_complete_files() {
        let cache = tempfile::tempdir().unwrap();
        std::thread::scope(|scope| {
            let handles = (0..4)
                .map(|_| scope.spawn(|| install(&fixture(), cache.path()).unwrap()))
                .collect::<Vec<_>>();
            let results = handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>();
            assert!(results.windows(2).all(|pair| pair[0] == pair[1]));
        });
    }

    #[test]
    fn broken_archive_never_replaces_current_cache() {
        let cache = tempfile::tempdir().unwrap();
        let mut component = fixture();
        let root = install(&component, cache.path()).unwrap();
        fs::write(root.join("omp"), b"old").unwrap();
        component.archive = b"invalid gzip";
        assert!(install(&component, cache.path()).is_err());
        assert_eq!(fs::read(root.join("omp")).unwrap(), b"old");
    }

    #[test]
    fn rejects_cross_platform_paths() {
        for path in [
            "../omp",
            "/omp",
            "C:/omp.exe",
            "a\\omp",
            "a/../omp",
            "./omp",
            "",
            "a//omp",
        ] {
            assert!(!safe_path(path), "{path}");
        }
        assert!(safe_path("AgentCheck.app/Contents/MacOS/AgentCheck"));
    }

    #[test]
    fn rejects_links_in_archive() {
        let cache = tempfile::tempdir().unwrap();
        let mut component = fixture();
        let mut archive = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o700);
        header.set_link_name("../outside").unwrap();
        header.set_cksum();
        archive
            .append_data(&mut header, "omp", io::empty())
            .unwrap();
        component.archive = Box::leak(
            archive
                .into_inner()
                .unwrap()
                .finish()
                .unwrap()
                .into_boxed_slice(),
        );
        assert!(install(&component, cache.path()).is_err());
        assert!(!cache.path().join("outside").exists());
    }

    #[test]
    fn rejects_different_target_and_manifest_digest() {
        let cache = tempfile::tempdir().unwrap();
        let mut component = fixture();
        component.digest = "incorrect";
        assert!(install(&component, cache.path()).is_err());
        let mut manifest: Manifest = serde_json::from_str(component.manifest).unwrap();
        manifest.target = "other-platform".into();
        component.manifest = Box::leak(serde_json::to_string(&manifest).unwrap().into_boxed_str());
        component.digest = Box::leak(
            format!("{:x}", Sha256::digest(component.manifest.as_bytes())).into_boxed_str(),
        );
        assert!(install(&component, cache.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn linked_cache_is_rejected_and_linked_program_is_repaired() {
        use std::os::unix::fs::symlink;
        let cache = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let linked = cache.path().join("linked");
        symlink(external.path(), &linked).unwrap();
        assert!(install(&fixture(), &linked).is_err());
        let root = install(&fixture(), cache.path()).unwrap();
        fs::remove_file(root.join("omp")).unwrap();
        symlink(external.path().join("untouched"), root.join("omp")).unwrap();
        install(&fixture(), cache.path()).unwrap();
        assert!(!external.path().join("untouched").exists());
        assert!(
            !fs::symlink_metadata(root.join("omp"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}
