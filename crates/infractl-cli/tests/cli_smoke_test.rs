mod common;

use std::process::Command;

use common::belter_bin::belter_bin;

#[test]
fn test_cli_dry_run_parse() {
    let output = Command::new(belter_bin())
        .args(["--dry-run", "service", "list"])
        .output()
        .expect("failed to execute process");

    assert!(output.status.success());
}
