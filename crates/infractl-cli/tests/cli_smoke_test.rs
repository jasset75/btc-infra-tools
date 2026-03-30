mod common;
#[path = "common/unique_fixture_dir.rs"]
mod unique_fixture_dir;

use std::fs;
use std::process::Command;

use common::belter_bin::belter_bin;
use unique_fixture_dir::unique_fixture_dir;

#[test]
fn test_cli_dry_run_parse() {
    let output = Command::new(belter_bin())
        .args(["--dry-run", "service", "list"])
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());
}

#[test]
fn test_cli_info_pool_parse() {
    let output = Command::new(belter_bin())
        .args(["--dry-run", "info", "pool", "192.0.2.10"])
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());
}

#[test]
fn test_cli_service_bring_up_parse() {
    let fixture_dir = unique_fixture_dir();
    fs::create_dir_all(&fixture_dir).expect("fixture dir should be created");
    let config_path = fixture_dir.join("belter.toml");
    let env_file_path = fixture_dir.join("mempool.env");
    fs::write(
        &config_path,
        r#"
[service.bitcoind]
manager = "launchd"
unit = "${BITCOIND_LAUNCHD_UNIT}"

[service.podman_runtime]
manager = "podman_machine"
machine = "${PODMAN_MACHINE_NAME}"

[service.mempool]
manager = "podman_compose"
compose_file = "${MEMPOOL_COMPOSE_FILE}"
compose_override = "${MEMPOOL_COMPOSE_OVERRIDE}"
project = "${MEMPOOL_PROJECT}"
env_file = "${MEMPOOL_ENV_FILE}"
depends_on = ["bitcoind", "podman_runtime"]
"#,
    )
    .expect("config should be written");
    fs::write(&env_file_path, "").expect("env file should be written");

    let output = Command::new(belter_bin())
        .args([
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--dry-run",
            "service",
            "bring-up",
            "mempool",
        ])
        .current_dir(&fixture_dir)
        .env("BITCOIND_LAUNCHD_UNIT", "system/com.bitcoind.node")
        .env("PODMAN_MACHINE_NAME", "podman-machine-default")
        .env("MEMPOOL_ENV_FILE", env_file_path.to_str().expect("utf8 path"))
        .env("MEMPOOL_COMPOSE_FILE", "/tmp/base.yml")
        .env("MEMPOOL_COMPOSE_OVERRIDE", "/tmp/override.yml")
        .env("MEMPOOL_PROJECT", "docker")
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());

    fs::remove_dir_all(&fixture_dir).expect("fixture dir should be removed");
}
