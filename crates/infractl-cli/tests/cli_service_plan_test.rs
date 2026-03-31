mod common;
#[path = "common/unique_fixture_dir.rs"]
mod unique_fixture_dir;

use std::fs;
use std::process::Command;

use common::belter_bin::belter_bin;
use unique_fixture_dir::unique_fixture_dir;

#[test]
fn test_cli_mempool_dry_run_json_plan() {
    let fixture_dir = unique_fixture_dir();
    fs::create_dir_all(&fixture_dir).expect("fixture dir should be created");

    let config_path = fixture_dir.join("belter.toml");
    let env_file_path = fixture_dir.join("mempool.env");
    fs::write(&env_file_path, "").expect("env file should be written");
    fs::write(
        &config_path,
        r#"
[service.mempool]
manager = "podman_compose"
compose_file = "${MEMPOOL_COMPOSE_FILE}"
compose_override = "${MEMPOOL_COMPOSE_OVERRIDE}"
project = "${MEMPOOL_PROJECT}"
env_file = "${MEMPOOL_ENV_FILE}"
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
        .current_dir(&fixture_dir)
        .env("MEMPOOL_COMPOSE_FILE", "/tmp/base.yml")
        .env("MEMPOOL_COMPOSE_OVERRIDE", "/tmp/override.yml")
        .env("MEMPOOL_PROJECT", "docker")
        .env(
            "MEMPOOL_ENV_FILE",
            env_file_path.to_str().expect("utf8 path"),
        )
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

#[test]
fn test_cli_mempool_dry_run_json_plan_loads_service_env_file() {
    let fixture_dir = unique_fixture_dir();
    fs::create_dir_all(&fixture_dir).expect("fixture dir should be created");

    let config_path = fixture_dir.join("belter.toml");
    let env_file_path = fixture_dir.join("mempool.env");
    fs::write(
        &config_path,
        r#"
[service.mempool]
manager = "podman_compose"
compose_file = "${MEMPOOL_COMPOSE_FILE}"
compose_override = "${MEMPOOL_COMPOSE_OVERRIDE}"
project = "${MEMPOOL_PROJECT}"
env_file = "${MEMPOOL_ENV_FILE}"
"#,
    )
    .expect("config should be written");
    fs::write(
        &env_file_path,
        "MEMPOOL_COMPOSE_FILE=/tmp/from-env-file.yml\nMEMPOOL_COMPOSE_OVERRIDE=/tmp/from-env-file.override.yml\nMEMPOOL_PROJECT=envfile-project\n",
    )
    .expect("env file should be written");

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
        .current_dir(&fixture_dir)
        .env(
            "MEMPOOL_ENV_FILE",
            env_file_path.to_str().expect("utf8 path"),
        )
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("\"compose_file\": \"/tmp/from-env-file.yml\""));
    assert!(stdout.contains("\"compose_override\": \"/tmp/from-env-file.override.yml\""));
    assert!(stdout.contains("\"project\": \"envfile-project\""));

    fs::remove_dir_all(&fixture_dir).expect("fixture dir should be removed");
}

#[test]
fn test_cli_stratum_restart_dry_run_json_plan() {
    let fixture_dir = unique_fixture_dir();
    fs::create_dir_all(&fixture_dir).expect("fixture dir should be created");

    let config_path = fixture_dir.join("belter.toml");
    fs::write(
        &config_path,
        r#"
[service.stratum]
manager = "launchd"
unit = "${STRATUM_LAUNCHD_UNIT}"
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
            "restart",
            "stratum",
        ])
        .current_dir(&fixture_dir)
        .env("STRATUM_LAUNCHD_UNIT", "system/io.btc.public-pool")
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("\"command\": \"service.restart\""));
    assert!(stdout.contains("\"dry_run\": true"));
    assert!(stdout.contains("\"RestartLaunchdService\""));
    assert!(stdout.contains("\"unit\": \"system/io.btc.public-pool\""));

    fs::remove_dir_all(&fixture_dir).expect("fixture dir should be removed");
}

#[test]
fn test_cli_mempool_bring_up_dry_run_json_plan_orders_dependencies() {
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
    fs::write(
        &env_file_path,
        "MEMPOOL_COMPOSE_FILE=/tmp/from-env-file.yml\nMEMPOOL_COMPOSE_OVERRIDE=/tmp/from-env-file.override.yml\nMEMPOOL_PROJECT=envfile-project\n",
    )
    .expect("env file should be written");

    let output = Command::new(belter_bin())
        .args([
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--dry-run",
            "--json",
            "service",
            "bring-up",
            "mempool",
        ])
        .current_dir(&fixture_dir)
        .env(
            "MEMPOOL_ENV_FILE",
            env_file_path.to_str().expect("utf8 path"),
        )
        .env("BITCOIND_LAUNCHD_UNIT", "system/com.bitcoind.node")
        .env("PODMAN_MACHINE_NAME", "podman-machine-default")
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("\"command\": \"service.bring-up\""));
    assert!(stdout.contains("\"StartLaunchdService\""));
    assert!(stdout.contains("\"StartPodmanMachine\""));
    assert!(stdout.contains("\"StartPodmanComposeService\""));
    assert!(stdout.contains("\"unit\": \"system/com.bitcoind.node\""));
    assert!(stdout.contains("\"machine\": \"podman-machine-default\""));
    assert!(stdout.contains("\"compose_file\": \"/tmp/from-env-file.yml\""));

    fs::remove_dir_all(&fixture_dir).expect("fixture dir should be removed");
}
