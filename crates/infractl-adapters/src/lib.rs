use anyhow::{Result, bail};
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

    pub fn restart_unit(&self, unit: &str) -> Result<()> {
        self.run_launchctl(&["kickstart", "-k", unit], unit, "restart")
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
}
