use std::{env, fs, path::Path};
fn main() {
    println!("cargo:rerun-if-changed=runtime/pi-release.json");
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read("runtime/pi-release.json").expect("OMP release manifest missing"),
    )
    .expect("invalid OMP release manifest");
    let version = manifest["version"].as_str().expect("OMP version missing");
    println!("cargo:rustc-env=AGENTCHECK_OMP_VERSION={version}");
    println!(
        "cargo:rustc-env=AGENTCHECK_TARGET={}",
        env::var("TARGET").expect("Cargo target missing")
    );
    if env::var_os("CARGO_FEATURE_BUNDLED_RUNTIME").is_some() {
        // A release must never silently fall back to a developer's .cache directory.
        let bundle = env::var("AGENTCHECK_BUNDLE_DIR")
            .expect("single-file bundling is pending; AGENTCHECK_BUNDLE_DIR is required");
        assert!(
            Path::new(&bundle).is_dir(),
            "runtime bundle directory does not exist"
        );
        panic!(
            "single-file runtime embedding is not implemented yet; refusing an incomplete release build"
        );
    }
}
