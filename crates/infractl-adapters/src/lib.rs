use anyhow::{Result, bail};
use std::process::Command;

pub trait ServiceAdapter {
    fn status(&self, _service: &str) -> Result<String>;
    fn start(&self, _service: &str) -> Result<()>;
    fn stop(&self, _service: &str) -> Result<()>;
    fn restart(&self, _service: &str) -> Result<()>;
}

pub struct LaunchdAdapter;

impl LaunchdAdapter {
    pub fn restart_unit(&self, unit: &str) -> Result<()> {
        let output = Command::new("launchctl")
            .args(["kickstart", "-k", unit])
            .output()?;

        if output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        bail!(
            "launchctl restart failed for unit {unit} (status={:?}, stdout={stdout}, stderr={stderr})",
            output.status.code()
        )
    }
}
