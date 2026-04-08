mod common;
#[path = "common/unique_fixture_dir.rs"]
mod unique_fixture_dir;

use std::fs;
use std::process::Command;

use common::belter_bin::belter_bin;
use unique_fixture_dir::unique_fixture_dir;

fn write_fixture_config(contents: &str) -> std::path::PathBuf {
    let fixture_dir = unique_fixture_dir();
    fs::create_dir_all(&fixture_dir).expect("fixture dir should be created");
    let config_path = fixture_dir.join("belter.toml");
    fs::write(&config_path, contents).expect("config should be written");
    config_path
}

fn mempool_config() -> &'static str {
    r#"
[service.mempool]
manager = "podman_compose"
compose_file = "${MEMPOOL_COMPOSE_FILE}"
compose_override = "${MEMPOOL_COMPOSE_OVERRIDE}"
project = "${MEMPOOL_PROJECT}"
"#
}

#[test]
fn test_cli_status_mempool_text_output() {
    let config_path = write_fixture_config(mempool_config());
    let fixture_dir = config_path.parent().expect("config should have parent");

    let output = Command::new(belter_bin())
        .args([
            "--config",
            config_path.to_str().expect("utf8 path"),
            "service",
            "status",
            "mempool",
        ])
        .current_dir(fixture_dir)
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("service.status"));
    assert!(stdout.contains("status target=mempool ui=Auto state="));

    fs::remove_dir_all(fixture_dir).expect("fixture dir should be removed");
}

#[test]
fn test_cli_status_bitcoind_text_output() {
    let config_path = write_fixture_config(
        r#"
[service.bitcoind]
manager = "launchd"
unit = "${BITCOIND_LAUNCHD_UNIT}"
"#,
    );
    let fixture_dir = config_path.parent().expect("config should have parent");

    let output = Command::new(belter_bin())
        .args([
            "--config",
            config_path.to_str().expect("utf8 path"),
            "service",
            "status",
            "bitcoind",
        ])
        .current_dir(fixture_dir)
        .env("BITCOIND_LAUNCHD_UNIT", "system/com.bitcoind.node")
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("service.status"));
    assert!(stdout.contains("status target=bitcoind ui=Auto state="));

    fs::remove_dir_all(fixture_dir).expect("fixture dir should be removed");
}

#[test]
fn test_cli_status_mempool_dry_run_json() {
    let config_path = write_fixture_config(mempool_config());
    let fixture_dir = config_path.parent().expect("config should have parent");

    let output = Command::new(belter_bin())
        .args([
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--dry-run",
            "--json",
            "service",
            "status",
            "mempool",
        ])
        .current_dir(fixture_dir)
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("\"command\": \"service.status\""));
    assert!(stdout.contains("\"dry_run\": true"));
    assert!(stdout.contains("\"simulated\": true"));

    fs::remove_dir_all(fixture_dir).expect("fixture dir should be removed");
}

#[test]
fn test_cli_status_mempool_json_unknown_when_env_missing() {
    let config_path = write_fixture_config(mempool_config());
    let fixture_dir = config_path.parent().expect("config should have parent");

    let output = Command::new(belter_bin())
        .args([
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--json",
            "service",
            "status",
            "mempool",
        ])
        .current_dir(fixture_dir)
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("\"command\": \"service.status\""));
    assert!(stdout.contains("\"manager\": \"podman_compose\""));
    assert!(stdout.contains("\"state\": \"unknown\""));
    assert!(stdout.contains("\"query_error\":"));

    fs::remove_dir_all(fixture_dir).expect("fixture dir should be removed");
}

#[test]
fn test_cli_status_mempool_json_contains_podman_fields_when_env_present() {
    let config_path = write_fixture_config(mempool_config());
    let fixture_dir = config_path.parent().expect("config should have parent");

    let output = Command::new(belter_bin())
        .args([
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--json",
            "service",
            "status",
            "mempool",
        ])
        .current_dir(fixture_dir)
        .env("MEMPOOL_COMPOSE_FILE", "/tmp/base.yml")
        .env("MEMPOOL_COMPOSE_OVERRIDE", "/tmp/override.yml")
        .env("MEMPOOL_PROJECT", "docker")
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("\"command\": \"service.status\""));
    assert!(stdout.contains("\"manager\": \"podman_compose\""));
    assert!(stdout.contains("\"compose_file\": \"/tmp/base.yml\""));
    assert!(stdout.contains("\"running_containers\":"));

    fs::remove_dir_all(fixture_dir).expect("fixture dir should be removed");
}

#[test]
fn test_cli_status_podman_runtime_json_contains_machine_field_when_env_present() {
    let config_path = write_fixture_config(
        r#"
[service.podman_runtime]
manager = "podman_machine"
machine = "${PODMAN_MACHINE_NAME}"
"#,
    );
    let fixture_dir = config_path.parent().expect("config should have parent");

    let output = Command::new(belter_bin())
        .args([
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--json",
            "service",
            "status",
            "podman_runtime",
        ])
        .current_dir(fixture_dir)
        .env("PODMAN_MACHINE_NAME", "podman-machine-default")
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("\"command\": \"service.status\""));
    assert!(stdout.contains("\"manager\": \"podman_machine\""));
    assert!(stdout.contains("\"machine\": \"podman-machine-default\""));

    fs::remove_dir_all(fixture_dir).expect("fixture dir should be removed");
}

#[test]
fn test_cli_status_all_text_output_lists_configured_services() {
    let config_path = write_fixture_config(
        r#"
[service.mempool]
manager = "podman_compose"
compose_file = "${MEMPOOL_COMPOSE_FILE}"
compose_override = "${MEMPOOL_COMPOSE_OVERRIDE}"
project = "${MEMPOOL_PROJECT}"

[service.bitcoind]
manager = "launchd"
unit = "${BITCOIND_LAUNCHD_UNIT}"

[service.podman_runtime]
manager = "podman_machine"
machine = "${PODMAN_MACHINE_NAME}"
"#,
    );
    let fixture_dir = config_path.parent().expect("config should have parent");

    let output = Command::new(belter_bin())
        .args([
            "--config",
            config_path.to_str().expect("utf8 path"),
            "service",
            "status",
        ])
        .current_dir(fixture_dir)
        .env("BITCOIND_LAUNCHD_UNIT", "system/com.bitcoind.node")
        .env("MEMPOOL_COMPOSE_FILE", "/tmp/base.yml")
        .env("MEMPOOL_COMPOSE_OVERRIDE", "/tmp/override.yml")
        .env("MEMPOOL_PROJECT", "docker")
        .env("PODMAN_MACHINE_NAME", "podman-machine-default")
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("service.status"));
    assert!(stdout.contains("status target=all ui=Auto services=3"));
    assert!(stdout.contains("- bitcoind (launchd) state="));
    assert!(stdout.contains("- mempool (podman_compose) state="));
    assert!(stdout.contains("- podman_runtime (podman_machine) state="));

    fs::remove_dir_all(fixture_dir).expect("fixture dir should be removed");
}

#[test]
fn test_cli_status_all_json_returns_services_array() {
    let config_path = write_fixture_config(
        r#"
[service.bitcoind]
manager = "launchd"
unit = "${BITCOIND_LAUNCHD_UNIT}"

[service.mempool]
manager = "podman_compose"
compose_file = "${MEMPOOL_COMPOSE_FILE}"
compose_override = "${MEMPOOL_COMPOSE_OVERRIDE}"
project = "${MEMPOOL_PROJECT}"
"#,
    );
    let fixture_dir = config_path.parent().expect("config should have parent");

    let output = Command::new(belter_bin())
        .args([
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--json",
            "service",
            "status",
        ])
        .current_dir(fixture_dir)
        .env("BITCOIND_LAUNCHD_UNIT", "system/com.bitcoind.node")
        .env("MEMPOOL_COMPOSE_FILE", "/tmp/base.yml")
        .env("MEMPOOL_COMPOSE_OVERRIDE", "/tmp/override.yml")
        .env("MEMPOOL_PROJECT", "docker")
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("\"command\": \"service.status\""));
    assert!(stdout.contains("\"services\": ["));
    assert!(stdout.contains("\"service\": \"bitcoind\""));
    assert!(stdout.contains("\"service\": \"mempool\""));

    fs::remove_dir_all(fixture_dir).expect("fixture dir should be removed");
}

#[test]
fn test_cli_status_all_dry_run_json_marks_services_as_simulated() {
    let config_path = write_fixture_config(
        r#"
[service.bitcoind]
manager = "launchd"
unit = "${BITCOIND_LAUNCHD_UNIT}"

[service.mempool]
manager = "podman_compose"
compose_file = "${MEMPOOL_COMPOSE_FILE}"
"#,
    );
    let fixture_dir = config_path.parent().expect("config should have parent");

    let output = Command::new(belter_bin())
        .args([
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--dry-run",
            "--json",
            "service",
            "status",
        ])
        .current_dir(fixture_dir)
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("\"command\": \"service.status\""));
    assert!(stdout.contains("\"dry_run\": true"));
    assert!(stdout.contains("\"service\": \"bitcoind\""));
    assert!(stdout.contains("\"service\": \"mempool\""));
    assert!(stdout.contains("\"simulated\": true"));

    fs::remove_dir_all(fixture_dir).expect("fixture dir should be removed");
}
