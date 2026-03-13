use std::process::Command;

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
