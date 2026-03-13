use anyhow::{Result, bail};
use infractl_core::plan::ExecutionDetails;
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
