use sha2::{Digest, Sha256};
use std::{env, fs, path::Path};
fn main() {
    println!("cargo:rerun-if-changed=runtime/pi-release.json");
    println!("cargo:rerun-if-env-changed=AGENTCHECK_BUNDLE_DIR");
    let release: serde_json::Value =
        serde_json::from_slice(&fs::read("runtime/pi-release.json").expect("OMP manifest missing"))
            .expect("invalid OMP manifest");
    let version = release["version"].as_str().expect("OMP version missing");
    let target = env::var("TARGET").expect("target missing");
    println!("cargo:rustc-env=AGENTCHECK_OMP_VERSION={version}");
    println!("cargo:rustc-env=AGENTCHECK_TARGET={target}");
    if env::var_os("CARGO_FEATURE_BUNDLED_RUNTIME").is_none() {
        return;
    }
    let bundle = env::var("AGENTCHECK_BUNDLE_DIR")
        .expect("prepare verified component bundles and set AGENTCHECK_BUNDLE_DIR");
    let bundle = Path::new(&bundle)
        .canonicalize()
        .expect("bundle directory missing");
    let manifest_path = bundle.join("manifest.json");
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).expect("bundle manifest missing"))
            .expect("invalid bundle manifest");
    assert_eq!(
        manifest["target"].as_str(),
        Some(target.as_str()),
        "bundle target mismatch"
    );
    assert_eq!(
        manifest["omp_version"].as_str(),
        Some(version),
        "bundle OMP version mismatch"
    );
    let mut code = format!(
        "pub const MANIFEST:&str=include_str!({:?});\n",
        manifest_path
    );
    for (name, constant) in [("omp", "OMP"), ("gui", "GUI")] {
        let component = &manifest["components"][name];
        if component.is_null() {
            assert!(
                name == "gui" && !target.contains("apple-darwin") && !target.contains("windows"),
                "desktop component missing"
            );
            code.push_str(&format!("pub const {constant}:&[u8]=&[];\n"));
            continue;
        }
        let file = component["archive"].as_str().expect("archive missing");
        assert!(
            !file.contains('/') && !file.contains('\\'),
            "unsafe archive name"
        );
        let archive = bundle.join(file);
        let expected = component["archive_sha256"]
            .as_str()
            .expect("archive hash missing");
        let bytes = fs::read(&archive).expect("component archive missing");
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            expected,
            "component archive hash mismatch"
        );
        assert!(
            component["entries"]
                .as_array()
                .is_some_and(|v| !v.is_empty()),
            "empty component"
        );
        println!("cargo:rerun-if-changed={}", archive.display());
        code.push_str(&format!(
            "pub const {constant}:&[u8]=include_bytes!({:?});\n",
            archive
        ));
    }
    fs::write(
        Path::new(&env::var("OUT_DIR").unwrap()).join("embedded.rs"),
        code,
    )
    .unwrap();
}
