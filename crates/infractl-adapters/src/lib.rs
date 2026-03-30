use anyhow::{Result, bail};
use infractl_core::plan::ExecutionDetails;
use serde_json::Value;
use std::process::Command;

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
        let output = Command::new("launchctl")
            .args(args)
            .output()?;

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
        self.run_compose(compose_file, compose_override, project, &["up", "-d"], "start")
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
        self.run_compose(compose_file, compose_override, project, &["down"], "restart")?;
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
        for id in output.lines().map(str::trim).filter(|line| !line.is_empty()) {
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
            .ok_or_else(|| anyhow::anyhow!("podman inspect output missing `State.Running` field"))?;

        Ok(running)
    }
}

pub struct PodmanMachineAdapter;

impl PodmanMachineAdapter {
    pub fn is_running(&self, machine: &str) -> Result<bool> {
        let output = Command::new("podman")
            .args(["machine", "inspect", machine])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let code = output.status.code();
            bail!(
                "podman machine status failed (machine={machine}, status={code:?}, stdout={stdout}, stderr={stderr})"
            );
        }

        let parsed: Value = serde_json::from_slice(&output.stdout)?;
        let state = parsed
            .as_array()
            .and_then(|items| items.first())
            .and_then(|item| item.get("State"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("podman machine inspect output missing `State` field"))?;

        Ok(state.eq_ignore_ascii_case("running"))
    }

    pub fn start(&self, machine: &str) -> Result<()> {
        self.run_machine(&["machine", "start", machine], machine, "start")
    }

    pub fn stop(&self, machine: &str) -> Result<()> {
        self.run_machine(&["machine", "stop", machine], machine, "stop")
    }

    pub fn restart(&self, machine: &str) -> Result<()> {
        self.run_machine(&["machine", "stop", machine], machine, "restart")?;
        self.run_machine(&["machine", "start", machine], machine, "restart")
    }

    fn run_machine(&self, args: &[&str], machine: &str, action: &str) -> Result<()> {
        let output = Command::new("podman").args(args).output()?;
        if output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let code = output.status.code();
        bail!(
            "podman machine {action} failed (machine={machine}, status={code:?}, stdout={stdout}, stderr={stderr})"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::parse_launchctl_pid;

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
}
