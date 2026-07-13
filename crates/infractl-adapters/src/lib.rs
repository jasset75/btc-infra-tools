use anyhow::{Result, bail};
use infractl_core::plan::ExecutionDetails;
use serde_json::Value;
use std::process::Command;
use std::thread;
use std::time::Duration;

pub mod executor;

pub trait ServiceAdapter {
    fn status(&self, _service: &str) -> Result<String>;
    fn start(&self, _service: &str) -> Result<()>;
    fn stop(&self, _service: &str) -> Result<()>;
    fn restart(&self, _service: &str) -> Result<()>;
}

pub struct LaunchdAdapter;

impl LaunchdAdapter {
    pub fn unit_pid_for_status(&self, unit: &str) -> Result<Option<i32>> {
        self.unit_pid(unit)
    }

    pub fn start_unit(&self, unit: &str) -> Result<()> {
        self.run_launchctl(&["bootstrap", unit], unit, "start")
    }

    pub fn stop_unit(&self, unit: &str) -> Result<()> {
        self.run_launchctl(&["bootout", unit], unit, "stop")
    }

    pub fn restart_unit(&self, unit: &str) -> Result<ExecutionDetails> {
        let pid_before = self.unit_pid(unit)?;
        self.run_launchctl(&["kickstart", "-k", unit], unit, "restart")?;
        let pid_after = self.unit_pid(unit)?;
        Ok(ExecutionDetails::LaunchdRestartPidChange {
            unit: unit.to_string(),
            pid_before,
            pid_after,
        })
    }

    fn run_launchctl(&self, args: &[&str], unit: &str, action: &str) -> Result<()> {
        let output = Command::new("launchctl").args(args).output()?;

        if output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let code = output.status.code();

        if stderr.contains("Unrecognized target specifier") {
            bail!(
                "launchctl {action} failed for unit {unit}: invalid target specifier. Use full launchctl target format '<domain>/<label>' (example: 'system/com.bitcoind.node'). Raw error: {stderr}"
            );
        }

        if stderr.contains("Operation not permitted") {
            bail!(
                "launchctl {action} failed for unit {unit}: insufficient privileges. For system domain units, run belter with elevated permissions (example: 'sudo -E belter service restart ...'). Raw error: {stderr}"
            );
        }

        bail!(
            "launchctl {action} failed for unit {unit} (status={:?}, stdout={stdout}, stderr={stderr})",
            code,
        )
    }

    fn unit_pid(&self, unit: &str) -> Result<Option<i32>> {
        let output = Command::new("launchctl").args(["print", unit]).output()?;
        if !output.status.success() {
            return Ok(None);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(parse_launchctl_pid(&stdout))
    }
}

fn parse_launchctl_pid(launchctl_print_output: &str) -> Option<i32> {
    launchctl_print_output.lines().find_map(|line| {
        let trimmed = line.trim();
        let value = trimmed.strip_prefix("pid = ")?;
        value.trim().parse::<i32>().ok()
    })
}

pub struct PodmanComposeAdapter;

impl PodmanComposeAdapter {
    pub fn start(
        &self,
        compose_file: &str,
        compose_override: Option<&str>,
        project: Option<&str>,
    ) -> Result<()> {
        self.run_compose(
            compose_file,
            compose_override,
            project,
            &["up", "-d"],
            "start",
        )
    }

    pub fn stop(
        &self,
        compose_file: &str,
        compose_override: Option<&str>,
        project: Option<&str>,
    ) -> Result<()> {
        self.run_compose(compose_file, compose_override, project, &["down"], "stop")
    }

    pub fn restart(
        &self,
        compose_file: &str,
        compose_override: Option<&str>,
        project: Option<&str>,
    ) -> Result<()> {
        self.run_compose(
            compose_file,
            compose_override,
            project,
            &["down"],
            "restart",
        )?;
        self.run_compose(
            compose_file,
            compose_override,
            project,
            &["up", "-d"],
            "restart",
        )
    }

    pub fn running_container_ids(
        &self,
        compose_file: &str,
        compose_override: Option<&str>,
        project: Option<&str>,
    ) -> Result<Vec<String>> {
        let output = self.run_compose_capture(
            compose_file,
            compose_override,
            project,
            &["ps", "-q"],
            "status",
        )?;

        let mut running = Vec::new();
        for id in output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            if self.container_is_running(id)? {
                running.push(id.to_owned());
            }
        }

        Ok(running)
    }

    fn run_compose(
        &self,
        compose_file: &str,
        compose_override: Option<&str>,
        project: Option<&str>,
        action_args: &[&str],
        action: &str,
    ) -> Result<()> {
        let mut args = vec!["compose"];
        if let Some(project) = project {
            args.extend(["-p", project]);
        }
        args.extend(["-f", compose_file]);
        if let Some(compose_override) = compose_override {
            args.extend(["-f", compose_override]);
        }
        args.extend(action_args);

        let output = Command::new("podman").args(&args).output()?;
        if output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let code = output.status.code();
        bail!(
            "podman compose {action} failed (compose_file={compose_file}, override={compose_override:?}, project={project:?}, status={code:?}, stdout={stdout}, stderr={stderr})"
        )
    }

    fn run_compose_capture(
        &self,
        compose_file: &str,
        compose_override: Option<&str>,
        project: Option<&str>,
        action_args: &[&str],
        action: &str,
    ) -> Result<String> {
        let mut args = vec!["compose"];
        if let Some(project) = project {
            args.extend(["-p", project]);
        }
        args.extend(["-f", compose_file]);
        if let Some(compose_override) = compose_override {
            args.extend(["-f", compose_override]);
        }
        args.extend(action_args);

        let output = Command::new("podman").args(&args).output()?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).to_string());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let code = output.status.code();
        bail!(
            "podman compose {action} failed (compose_file={compose_file}, override={compose_override:?}, project={project:?}, status={code:?}, stdout={stdout}, stderr={stderr})"
        )
    }

    fn container_is_running(&self, container_id: &str) -> Result<bool> {
        let output = Command::new("podman")
            .args(["inspect", container_id])
            .output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let code = output.status.code();
            bail!(
                "podman inspect failed (container_id={container_id}, status={code:?}, stdout={stdout}, stderr={stderr})"
            );
        }

        let parsed: Value = serde_json::from_slice(&output.stdout)?;
        let running = parsed
            .as_array()
            .and_then(|items| items.first())
            .and_then(|item| item.get("State"))
            .and_then(|state| state.get("Running"))
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                anyhow::anyhow!("podman inspect output missing `State.Running` field")
            })?;

        Ok(running)
    }
}

pub struct PodmanMachineAdapter;

const PODMAN_READY_CHECKS: usize = 15;
const PODMAN_READY_POLL_INTERVAL: Duration = Duration::from_secs(2);
const PODMAN_START_ATTEMPTS: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PodmanMachineStatus {
    Running,
    Stopped,
    Degraded(String),
}

struct PodmanCommandOutput {
    success: bool,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

trait PodmanCommandRunner {
    fn run(&mut self, args: &[&str]) -> Result<PodmanCommandOutput>;
    fn sleep(&mut self, duration: Duration);
}

struct ProcessPodmanCommandRunner;

impl PodmanCommandRunner for ProcessPodmanCommandRunner {
    fn run(&mut self, args: &[&str]) -> Result<PodmanCommandOutput> {
        let output = Command::new("podman").args(args).output()?;
        Ok(PodmanCommandOutput {
            success: output.status.success(),
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }

    fn sleep(&mut self, duration: Duration) {
        thread::sleep(duration);
    }
}

struct PodmanMachineInspection {
    running: bool,
    rootful: bool,
}

impl PodmanMachineAdapter {
    pub fn status(&self, machine: &str) -> Result<PodmanMachineStatus> {
        self.status_with(machine, &mut ProcessPodmanCommandRunner)
    }

    fn status_with(
        &self,
        machine: &str,
        runner: &mut impl PodmanCommandRunner,
    ) -> Result<PodmanMachineStatus> {
        let output = runner.run(&["machine", "inspect", machine])?;
        if !output.success {
            bail!(
                "podman machine status failed (machine={machine}, status={:?}, stdout={}, stderr={})",
                output.code,
                output.stdout,
                output.stderr,
            );
        }

        let inspection = parse_podman_machine_inspection(&output.stdout)?;
        if !inspection.running {
            return Ok(PodmanMachineStatus::Stopped);
        }

        let connection = podman_machine_connection(machine, inspection.rootful);
        let api = runner.run(&["--connection", &connection, "info"])?;
        if api.success {
            Ok(PodmanMachineStatus::Running)
        } else {
            Ok(PodmanMachineStatus::Degraded(format!(
                "Podman machine is running but its API is unavailable (machine={machine}, connection={connection}, status={:?}, stdout={}, stderr={})",
                api.code, api.stdout, api.stderr
            )))
        }
    }

    pub fn start(&self, machine: &str) -> Result<()> {
        self.start_with(machine, "start", &mut ProcessPodmanCommandRunner)
    }

    pub fn stop(&self, machine: &str) -> Result<()> {
        self.run_machine(&["machine", "stop", machine], machine, "stop")
    }

    pub fn restart(&self, machine: &str) -> Result<()> {
        self.run_machine(&["machine", "stop", machine], machine, "restart")?;
        self.start_with(machine, "restart", &mut ProcessPodmanCommandRunner)
    }

    fn start_with(
        &self,
        machine: &str,
        action: &str,
        runner: &mut impl PodmanCommandRunner,
    ) -> Result<()> {
        self.run_machine_with(&["machine", "start", machine], machine, action, runner)?;

        let mut start_attempts = 1;
        let mut last_status = "Podman readiness was not checked".to_string();
        for check in 1..=PODMAN_READY_CHECKS {
            match self.status_with(machine, runner) {
                Ok(PodmanMachineStatus::Running) => return Ok(()),
                Ok(PodmanMachineStatus::Stopped)
                    if start_attempts < PODMAN_START_ATTEMPTS && check < PODMAN_READY_CHECKS =>
                {
                    start_attempts += 1;
                    last_status = format!(
                        "machine returned to stopped state; start attempt {start_attempts} requested"
                    );
                    self.run_machine_with(&["machine", "start", machine], machine, action, runner)?;
                }
                Ok(PodmanMachineStatus::Stopped) => {
                    last_status = "machine is stopped".to_string();
                }
                Ok(PodmanMachineStatus::Degraded(detail)) => {
                    last_status = detail;
                }
                Err(err) => {
                    last_status = err.to_string();
                }
            }

            if check < PODMAN_READY_CHECKS {
                runner.sleep(PODMAN_READY_POLL_INTERVAL);
            }
        }

        bail!(
            "podman machine {action} did not produce a ready API after {} checks (machine={machine}, start_attempts={start_attempts}, last_status={last_status})",
            PODMAN_READY_CHECKS
        )
    }

    fn run_machine(&self, args: &[&str], machine: &str, action: &str) -> Result<()> {
        self.run_machine_with(args, machine, action, &mut ProcessPodmanCommandRunner)
    }

    fn run_machine_with(
        &self,
        args: &[&str],
        machine: &str,
        action: &str,
        runner: &mut impl PodmanCommandRunner,
    ) -> Result<()> {
        let output = runner.run(args)?;
        if output.success {
            return Ok(());
        }

        bail!(
            "podman machine {action} failed (machine={machine}, status={:?}, stdout={}, stderr={})",
            output.code,
            output.stdout,
            output.stderr,
        )
    }
}

fn parse_podman_machine_inspection(output: &str) -> Result<PodmanMachineInspection> {
    let parsed: Value = serde_json::from_str(output)?;
    let machine = parsed
        .as_array()
        .and_then(|items| items.first())
        .ok_or_else(|| anyhow::anyhow!("podman machine inspect output is empty"))?;
    let state = machine
        .get("State")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("podman machine inspect output missing `State` field"))?;
    let rootful = machine
        .get("Rootful")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Ok(PodmanMachineInspection {
        running: state.eq_ignore_ascii_case("running"),
        rootful,
    })
}

fn podman_machine_connection(machine: &str, rootful: bool) -> String {
    if rootful {
        format!("{machine}-root")
    } else {
        machine.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PodmanCommandOutput, PodmanCommandRunner, PodmanMachineAdapter, PodmanMachineStatus,
        parse_launchctl_pid, parse_podman_machine_inspection,
    };
    use anyhow::Result;
    use std::collections::VecDeque;
    use std::time::Duration;

    struct MockPodmanRunner {
        outputs: VecDeque<PodmanCommandOutput>,
        calls: Vec<Vec<String>>,
        sleeps: Vec<Duration>,
    }

    impl MockPodmanRunner {
        fn new(outputs: Vec<PodmanCommandOutput>) -> Self {
            Self {
                outputs: outputs.into(),
                calls: Vec::new(),
                sleeps: Vec::new(),
            }
        }
    }

    impl PodmanCommandRunner for MockPodmanRunner {
        fn run(&mut self, args: &[&str]) -> Result<PodmanCommandOutput> {
            self.calls
                .push(args.iter().map(|arg| (*arg).to_string()).collect());
            Ok(self.outputs.pop_front().expect("mock output should exist"))
        }

        fn sleep(&mut self, duration: Duration) {
            self.sleeps.push(duration);
        }
    }

    fn successful_output(stdout: &str) -> PodmanCommandOutput {
        PodmanCommandOutput {
            success: true,
            code: Some(0),
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }

    fn failed_output(stderr: &str) -> PodmanCommandOutput {
        PodmanCommandOutput {
            success: false,
            code: Some(125),
            stdout: String::new(),
            stderr: stderr.to_string(),
        }
    }

    fn machine_inspect_output(state: &str, rootful: bool) -> String {
        format!(r#"[{{"State":"{state}","Rootful":{rootful}}}]"#)
    }

    #[test]
    fn parse_launchctl_pid_reads_pid_line() {
        let output = r#"
system/com.bitcoind.node = {
    active count = 1
    pid = 12345
}
"#;

        assert_eq!(parse_launchctl_pid(output), Some(12345));
    }

    #[test]
    fn parse_launchctl_pid_returns_none_without_pid_line() {
        let output = "system/com.bitcoind.node = {\n    active count = 0\n}";
        assert_eq!(parse_launchctl_pid(output), None);
    }

    #[test]
    fn parse_machine_inspection_reads_state_and_rootful_mode() {
        let inspection = parse_podman_machine_inspection(&machine_inspect_output("running", true))
            .expect("inspection should parse");

        assert!(inspection.running);
        assert!(inspection.rootful);
    }

    #[test]
    fn podman_machine_status_is_stopped_without_api_probe() {
        let mut runner = MockPodmanRunner::new(vec![successful_output(&machine_inspect_output(
            "stopped", false,
        ))]);

        let status = PodmanMachineAdapter
            .status_with("podman-machine-default", &mut runner)
            .expect("status should succeed");

        assert_eq!(status, PodmanMachineStatus::Stopped);
        assert_eq!(runner.calls.len(), 1);
    }

    #[test]
    fn podman_machine_status_is_degraded_when_api_is_unavailable() {
        let mut runner = MockPodmanRunner::new(vec![
            successful_output(&machine_inspect_output("running", true)),
            failed_output("connection refused"),
        ]);

        let status = PodmanMachineAdapter
            .status_with("podman-machine-default", &mut runner)
            .expect("status should succeed");

        let PodmanMachineStatus::Degraded(detail) = status else {
            panic!("expected degraded status");
        };
        assert!(detail.contains("connection refused"));
        assert_eq!(
            runner.calls[1],
            vec!["--connection", "podman-machine-default-root", "info"]
        );
    }

    #[test]
    fn podman_machine_start_retries_when_machine_returns_to_stopped() {
        let mut runner = MockPodmanRunner::new(vec![
            successful_output("started"),
            successful_output(&machine_inspect_output("stopped", false)),
            successful_output("started again"),
            successful_output(&machine_inspect_output("running", false)),
            successful_output("api ready"),
        ]);

        PodmanMachineAdapter
            .start_with("podman-machine-default", "start", &mut runner)
            .expect("second start should reach readiness");

        let start_calls = runner
            .calls
            .iter()
            .filter(|args| args.as_slice() == ["machine", "start", "podman-machine-default"])
            .count();
        assert_eq!(start_calls, 2);
        assert_eq!(runner.sleeps.len(), 1);
    }

    #[test]
    fn podman_machine_start_waits_for_api_readiness() {
        let mut runner = MockPodmanRunner::new(vec![
            successful_output("started"),
            successful_output(&machine_inspect_output("running", false)),
            failed_output("connection refused"),
            successful_output(&machine_inspect_output("running", false)),
            successful_output("api ready"),
        ]);

        PodmanMachineAdapter
            .start_with("podman-machine-default", "start", &mut runner)
            .expect("API should become ready");

        assert_eq!(runner.sleeps.len(), 1);
    }
}
