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
fn test_cli_mempool_dry_run_json_plan() {
    let fixture_dir = unique_fixture_dir();
    fs::create_dir_all(&fixture_dir).expect("fixture dir should be created");

    let config_path = fixture_dir.join("belter.toml");
    fs::write(
        &config_path,
        r#"
[service.mempool]
manager = "podman_compose"
compose_file = "${MEMPOOL_COMPOSE_FILE}"
compose_override = "${MEMPOOL_COMPOSE_OVERRIDE}"
project = "${MEMPOOL_PROJECT}"
"#,
    )
    .expect("config should be written");

    let output = Command::new(belter_bin())
        .args([
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--dry-run",
            "--json",
            "service",
            "start",
            "mempool",
        ])
        .current_dir(repo_root())
        .env("MEMPOOL_COMPOSE_FILE", "/tmp/base.yml")
        .env("MEMPOOL_COMPOSE_OVERRIDE", "/tmp/override.yml")
        .env("MEMPOOL_PROJECT", "docker")
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("\"command\": \"service.start\""));
    assert!(stdout.contains("\"dry_run\": true"));
    assert!(stdout.contains("\"events\": []"));
    assert!(stdout.contains("\"compose_file\": \"/tmp/base.yml\""));
    assert!(!stdout.contains("service.start.preview"));

    fs::remove_dir_all(&fixture_dir).expect("fixture dir should be removed");
}
