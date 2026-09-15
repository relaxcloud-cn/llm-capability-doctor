#[path = "src/bundle_format.rs"]
mod bundle_format;

use bundle_format::{BundleFile, Manifest, safe_path};
use flate2::{Compression, write::GzEncoder};
use sha2::{Digest, Sha256};
use std::{env, fs, io, path::Path};

fn append_files(
    root: &Path,
    directory: &Path,
    archive: &mut tar::Builder<GzEncoder<fs::File>>,
    manifest: &mut Manifest,
) -> io::Result<()> {
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        println!("cargo:rerun-if-changed={}", path.display());
        if metadata.is_dir() {
            append_files(root, &path, archive, manifest)?;
            continue;
        }
        assert!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "内置组件不能包含链接或特殊文件：{}",
            path.display()
        );
        let relative = path
            .strip_prefix(root)
            .unwrap()
            .to_str()
            .expect("组件路径必须为 UTF-8")
            .replace('\\', "/");
        assert!(safe_path(&relative), "无效的组件路径：{relative}");
        let data = fs::read(&path)?;
        let executable =
            relative == "omp" || relative == "omp.exe" || relative.ends_with("/MacOS/AgentCheck");
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(if executable { 0o700 } else { 0o600 });
        header.set_mtime(0);
        header.set_cksum();
        archive.append_data(&mut header, &relative, &data[..])?;
        manifest.files.push(BundleFile {
            path: relative,
            sha256: format!("{:x}", Sha256::digest(&data)),
            size: data.len() as u64,
            executable,
        });
    }
    Ok(())
}

fn main() -> io::Result<()> {
    println!("cargo:rerun-if-env-changed=AGENTCHECK_BUNDLE_DIR");
    println!("cargo:rerun-if-changed=src/bundle_format.rs");
    let output = env::var_os("OUT_DIR").unwrap();
    let output = Path::new(&output);
    let target = env::var("TARGET").unwrap();
    let mut generated = format!(
        "pub const BUILD_TARGET: &str = {target:?};\nconst COMPONENTS: &[EmbeddedComponent] = &[\n"
    );
    if env::var_os("CARGO_FEATURE_BUNDLED_RUNTIME").is_some() {
        let root = env::var_os("AGENTCHECK_BUNDLE_DIR")
            .expect("单文件构建必须设置 AGENTCHECK_BUNDLE_DIR；请运行 scripts/package-release.sh");
        let root = Path::new(&root);
        println!("cargo:rerun-if-changed={}", root.join("TARGET").display());
        let staged_target = fs::read_to_string(root.join("TARGET"))?;
        assert_eq!(
            staged_target.trim(),
            target,
            "内置组件与 CLI 的目标平台不一致"
        );
        let omp_name = if target.contains("windows") {
            "omp.exe"
        } else {
            "omp"
        };
        assert!(
            root.join("runtime").join(omp_name).is_file(),
            "缺少目标平台的 OhMyPi"
        );
        for component in ["runtime", "gui"] {
            let source = root.join(component);
            println!("cargo:rerun-if-changed={}", source.display());
            if !source.exists() {
                continue;
            }
            let archive_path = output.join(format!("{component}.tar.gz"));
            let encoder = GzEncoder::new(fs::File::create(&archive_path)?, Compression::default());
            let mut archive = tar::Builder::new(encoder);
            let mut manifest = Manifest {
                target: target.clone(),
                files: Vec::new(),
            };
            append_files(&source, &source, &mut archive, &mut manifest)?;
            assert!(!manifest.files.is_empty(), "内置组件不能为空：{component}");
            archive.into_inner()?.finish()?;
            let manifest_path = output.join(format!("{component}.json"));
            fs::write(&manifest_path, serde_json::to_vec(&manifest)?)?;
            let digest = format!("{:x}", Sha256::digest(fs::read(&manifest_path)?));
            generated.push_str(&format!("EmbeddedComponent {{ name: {component:?}, digest: {digest:?}, manifest: include_str!({manifest_path:?}), archive: include_bytes!({archive_path:?}) }},\n"));
        }
    }
    generated.push_str("];\n");
    fs::write(output.join("bundle.rs"), generated)
}
