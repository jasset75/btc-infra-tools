use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn test_cli_dry_run_parse() {
    let output = Command::new(belter_bin())
        .args(["--dry-run", "service", "list"])
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());
}

#[test]
fn test_cli_status_mempool_text_output() {
    let output = Command::new(belter_bin())
        .args(["service", "status", "mempool"])
        .current_dir("/Users/juan/work/btc-infra-upstream/btc-infra-tools")
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("service.status"));
    assert!(stdout.contains("status target=mempool ui=Auto manager=podman_compose"));
}

#[test]
fn test_cli_status_bitcoind_text_output() {
    let output = Command::new(belter_bin())
        .args(["service", "status", "bitcoind"])
        .current_dir("/Users/juan/work/btc-infra-upstream/btc-infra-tools")
        .env("BITCOIND_LAUNCHD_UNIT", "system/com.bitcoind.node")
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("service.status"));
    assert!(stdout.contains("status target=bitcoind ui=Auto state="));
}

#[test]
fn test_cli_status_mempool_dry_run_json() {
    let output = Command::new(belter_bin())
        .args(["--dry-run", "--json", "service", "status", "mempool"])
        .current_dir("/Users/juan/work/btc-infra-upstream/btc-infra-tools")
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("\"command\": \"service.status\""));
    assert!(stdout.contains("\"dry_run\": true"));
    assert!(stdout.contains("\"simulated\": true"));
}

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
        .current_dir("/Users/juan/work/btc-infra-upstream/btc-infra-tools")
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
        .current_dir("/Users/juan/work/btc-infra-upstream/btc-infra-tools")
        .output()
        .expect("failed to execute process");

    assert!(!output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("\"command\": \"service.restart\""));
    assert!(stdout.contains("\"status\": \"error\""));
    assert!(stdout.contains("service `bitcoind` is missing `unit`"));

    fs::remove_dir_all(&fixture_dir).expect("fixture dir should be removed");
}

fn unique_fixture_dir() -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    std::env::temp_dir().join(format!("belter-cli-test-{ts}"))
}

fn belter_bin() -> &'static str {
    env!("CARGO_BIN_EXE_belter")
}
