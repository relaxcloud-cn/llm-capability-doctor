use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub target: String,
    pub files: Vec<BundleFile>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BundleFile {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub executable: bool,
}

pub fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', ':'])
        && !path.starts_with('/')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}
