use crate::LaunchdAdapter;
use anyhow::bail;
use infractl_core::plan::{Executor, Operation, Plan};
use std::io::Write;

pub struct RealExecutor {
    launchd: LaunchdAdapter,
}

impl RealExecutor {
    pub fn new() -> Self {
        Self { launchd: LaunchdAdapter }
    }
}

impl Executor for RealExecutor {
    fn execute(&mut self, plan: &Plan) -> anyhow::Result<()> {
        for op in &plan.operations {
            match op {
                Operation::RestartService { manager, unit } => match manager.as_str() {
                    "launchd" => self.launchd.restart_unit(unit)?,
                    _ => bail!(
                        "restart manager `{manager}` is not implemented by this executor"
                    ),
                }
            }
        }
        Ok(())
    }
}

pub struct DryRunExecutor<W: Write> {
    writer: W,
}

impl<W: Write> DryRunExecutor<W> {
    pub fn new(writer: W) -> Self {
        Self { writer }
    }
}

impl DryRunExecutor<std::io::Stdout> {
    pub fn stdout() -> Self {
        Self::new(std::io::stdout())
    }
}

impl<W: Write> Executor for DryRunExecutor<W> {
    fn execute(&mut self, plan: &Plan) -> anyhow::Result<()> {
        for (i, op) in plan.operations.iter().enumerate() {
            match op {
                Operation::RestartService { manager, unit } => {
                    writeln!(
                        self.writer,
                        "  [DRY-RUN] {}. Would restart `{}` unit `{}`",
                        i + 1,
                        manager,
                        unit
                    )?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_executor_writes_operations_to_writer() {
        let mut output = Vec::new();
        let mut executor = DryRunExecutor::new(&mut output);
        let plan = Plan {
            operations: vec![Operation::RestartService {
                manager: "launchd".to_string(),
                unit: "system/com.bitcoind.node".to_string(),
            }],
        };

        executor.execute(&plan).expect("dry-run execution should succeed");

        let rendered = String::from_utf8(output).expect("writer should contain utf8");
        assert!(rendered.contains("Would restart `launchd` unit `system/com.bitcoind.node`"));
    }

    #[test]
    fn real_executor_rejects_unknown_manager() {
        let mut executor = RealExecutor::new();
        let plan = Plan {
            operations: vec![Operation::RestartService {
                manager: "systemd".to_string(),
                unit: "bitcoind.service".to_string(),
            }],
        };

        let err = executor.execute(&plan).unwrap_err();
        assert!(err
            .to_string()
            .contains("restart manager `systemd` is not implemented by this executor"));
    }
}
