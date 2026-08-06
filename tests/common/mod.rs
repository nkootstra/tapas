use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[allow(dead_code)]
static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

#[allow(dead_code)]
pub fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/regression/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[allow(dead_code)]
pub fn unique_temp_dir(parent: &Path, prefix: &str) -> PathBuf {
    let sequence = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
    let path = parent.join(format!("{prefix}-{}-{sequence}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    path
}
