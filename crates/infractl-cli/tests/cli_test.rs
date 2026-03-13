use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn test_cli_dry_run_parse() {
    // A simple sanity check that --dry-run is recognized
    // We would use the actual Cli but we just test via assert_cmd or Command if we wanted
    // For now we just check the bin runs with --dry-run
    let output = Command::new("cargo")
        .args(["run", "-p", "belter", "--", "--dry-run", "service", "list"])
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());
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

    let output = Command::new("cargo")
        .args([
            "run",
            "-p",
            "belter",
            "--",
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
    assert!(stdout.contains("\"manager\": \"podman_compose\""));
    assert!(stdout.contains("\"compose_file\": \"/tmp/base.yml\""));

    fs::remove_dir_all(&fixture_dir).expect("fixture dir should be removed");
}

fn unique_fixture_dir() -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be monotonic")
        .as_nanos();
    std::env::temp_dir().join(format!("belter-cli-test-{ts}"))
}
