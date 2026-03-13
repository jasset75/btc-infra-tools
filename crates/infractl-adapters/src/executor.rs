use crate::LaunchdAdapter;
use infractl_core::plan::{Executor, Operation, Plan};
use std::io::Write;

pub struct RealExecutor {
    launchd: LaunchdAdapter,
}

impl RealExecutor {
    pub fn new() -> Self {
        Self {
            launchd: LaunchdAdapter,
        }
    }
}

impl Default for RealExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl Executor for RealExecutor {
    fn execute(&mut self, plan: &Plan) -> anyhow::Result<()> {
        for op in &plan.operations {
            match op {
                Operation::StartLaunchdService { unit } => self.launchd.start_unit(unit)?,
                Operation::StopLaunchdService { unit } => self.launchd.stop_unit(unit)?,
                Operation::RestartLaunchdService { unit } => self.launchd.restart_unit(unit)?,
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

impl DryRunExecutor<std::io::Sink> {
    pub fn sink() -> Self {
        Self::new(std::io::sink())
    }
}

impl<W: Write> Executor for DryRunExecutor<W> {
    fn execute(&mut self, plan: &Plan) -> anyhow::Result<()> {
        for (i, op) in plan.operations.iter().enumerate() {
            match op {
                Operation::StartLaunchdService { unit } => writeln!(
                    self.writer,
                    "  [DRY-RUN] {}. Would start `launchd` unit `{}`",
                    i + 1,
                    unit
                )?,
                Operation::StopLaunchdService { unit } => writeln!(
                    self.writer,
                    "  [DRY-RUN] {}. Would stop `launchd` unit `{}`",
                    i + 1,
                    unit
                )?,
                Operation::RestartLaunchdService { unit } => writeln!(
                    self.writer,
                    "  [DRY-RUN] {}. Would restart `launchd` unit `{}`",
                    i + 1,
                    unit
                )?,
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
            operations: vec![Operation::RestartLaunchdService {
                unit: "system/com.bitcoind.node".to_string(),
            }],
        };

        executor
            .execute(&plan)
            .expect("dry-run execution should succeed");

        let rendered = String::from_utf8(output).expect("writer should contain utf8");
        assert!(rendered.contains("Would restart `launchd` unit `system/com.bitcoind.node`"));
    }
}
