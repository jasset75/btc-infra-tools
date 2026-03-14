mod common;
#[path = "common/repo_root.rs"]
mod repo_root;

use std::process::Command;

use common::belter_bin::belter_bin;
use repo_root::repo_root;

#[test]
fn test_cli_status_mempool_text_output() {
    let output = Command::new(belter_bin())
        .args(["service", "status", "mempool"])
        .current_dir(repo_root())
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
        .current_dir(repo_root())
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
        .current_dir(repo_root())
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("\"command\": \"service.status\""));
    assert!(stdout.contains("\"dry_run\": true"));
    assert!(stdout.contains("\"simulated\": true"));
}
