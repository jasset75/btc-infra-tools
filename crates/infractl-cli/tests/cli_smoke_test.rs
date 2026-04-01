mod common;
#[path = "common/unique_fixture_dir.rs"]
mod unique_fixture_dir;

use std::fs;
use std::process::Command;

use common::belter_bin::belter_bin;
use unique_fixture_dir::unique_fixture_dir;

#[test]
fn test_cli_dry_run_parse() {
    let fixture_dir = unique_fixture_dir();
    fs::create_dir_all(&fixture_dir).expect("fixture dir should be created");
    let config_path = fixture_dir.join("belter.toml");
    fs::write(
        &config_path,
        r#"
[service.bitcoind]
manager = "launchd"
unit = "system/com.bitcoind.node"
"#,
    )
    .expect("config should be written");

    let output = Command::new(belter_bin())
        .args([
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--dry-run",
            "service",
            "list",
        ])
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());

    fs::remove_dir_all(&fixture_dir).expect("fixture dir should be removed");
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
        .env(
            "MEMPOOL_ENV_FILE",
            env_file_path.to_str().expect("utf8 path"),
        )
        .env("MEMPOOL_COMPOSE_FILE", "/tmp/base.yml")
        .env("MEMPOOL_COMPOSE_OVERRIDE", "/tmp/override.yml")
        .env("MEMPOOL_PROJECT", "docker")
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());

    fs::remove_dir_all(&fixture_dir).expect("fixture dir should be removed");
}

#[test]
fn test_cli_resolves_config_from_belter_config_env() {
    let fixture_dir = unique_fixture_dir();
    fs::create_dir_all(&fixture_dir).expect("fixture dir should be created");
    let config_path = fixture_dir.join("belter.toml");
    fs::write(
        &config_path,
        r#"
[service.bitcoind]
manager = "launchd"
unit = "system/com.bitcoind.node"
"#,
    )
    .expect("config should be written");

    let outside_dir = unique_fixture_dir();
    fs::create_dir_all(&outside_dir).expect("outside dir should be created");

    let output = Command::new(belter_bin())
        .args(["--dry-run", "service", "list"])
        .current_dir(&outside_dir)
        .env(
            "BELTER_CONFIG",
            config_path.to_str().expect("utf8 config path"),
        )
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("listed 1 configured service(s)"));

    fs::remove_dir_all(&fixture_dir).expect("fixture dir should be removed");
    fs::remove_dir_all(&outside_dir).expect("outside dir should be removed");
}

#[test]
fn test_cli_resolves_config_and_dotenv_from_xdg_location() {
    let fixture_dir = unique_fixture_dir();
    let xdg_config_home = fixture_dir.join("xdg");
    let config_dir = xdg_config_home.join("belter");
    fs::create_dir_all(&config_dir).expect("config dir should be created");
    let config_path = config_dir.join("belter.toml");
    fs::write(
        &config_path,
        r#"
[service.bitcoind]
manager = "launchd"
unit = "${BITCOIND_LAUNCHD_UNIT}"

[service.stratum]
manager = "launchd"
unit = "${STRATUM_LAUNCHD_UNIT}"

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
        config_dir.join(".env"),
        "BITCOIND_LAUNCHD_UNIT=system/com.bitcoind.node\nSTRATUM_LAUNCHD_UNIT=system/io.btc.public-pool\nMEMPOOL_COMPOSE_FILE=/tmp/base.yml\nMEMPOOL_COMPOSE_OVERRIDE=/tmp/override.yml\nMEMPOOL_PROJECT=docker\nMEMPOOL_ENV_FILE=/tmp/mempool.env\n",
    )
    .expect("dotenv should be written");

    let outside_dir = unique_fixture_dir();
    fs::create_dir_all(&outside_dir).expect("outside dir should be created");

    let output = Command::new(belter_bin())
        .args(["config", "validate"])
        .current_dir(&outside_dir)
        .env(
            "XDG_CONFIG_HOME",
            xdg_config_home.to_str().expect("utf8 xdg path"),
        )
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("configuration is valid"));

    fs::remove_dir_all(&fixture_dir).expect("fixture dir should be removed");
    fs::remove_dir_all(&outside_dir).expect("outside dir should be removed");
}
