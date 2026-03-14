use std::path::PathBuf;

pub fn repo_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("crate should live under <workspace>/crates/infractl-cli")
        .to_path_buf()
}
