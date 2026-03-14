use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn unique_fixture_dir() -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    std::env::temp_dir().join(format!("belter-cli-test-{ts}"))
}
