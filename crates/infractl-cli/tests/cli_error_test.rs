mod common;
#[path = "common/repo_root.rs"]
mod repo_root;
#[path = "common/unique_fixture_dir.rs"]
mod unique_fixture_dir;

use std::fs;
use std::process::Command;

use common::belter_bin::belter_bin;
use repo_root::repo_root;
use unique_fixture_dir::unique_fixture_dir;

#[test]
fn test_cli_json_error_is_returned_as_envelope() {
    let fixture_dir = unique_fixture_dir();
    fs::create_dir_all(&fixture_dir).expect("fixture dir should be created");

    let config_path = fixture_dir.join("belter.toml");
    fs::write(
        &config_path,
        r#"
[service.bitcoind]
manager = "launchd"
"#,
    )
    .expect("config should be written");

    let output = Command::new(belter_bin())
        .args([
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--json",
            "service",
            "restart",
            "bitcoind",
        ])
        .current_dir(repo_root())
        .output()
        .expect("failed to execute process");

    assert!(!output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("\"command\": \"service.restart\""));
    assert!(stdout.contains("\"status\": \"error\""));
    assert!(stdout.contains("service `bitcoind` is missing `unit`"));

    fs::remove_dir_all(&fixture_dir).expect("fixture dir should be removed");
}
