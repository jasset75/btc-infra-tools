use crate::{LaunchdAdapter, PodmanComposeAdapter};
use infractl_core::plan::{Executor, Operation, Plan};
use std::io::Write;

pub struct RealExecutor {
    launchd: LaunchdAdapter,
    podman_compose: PodmanComposeAdapter,
}

impl RealExecutor {
    pub fn new() -> Self {
        Self {
            launchd: LaunchdAdapter,
            podman_compose: PodmanComposeAdapter,
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
                Operation::StartPodmanComposeService {
                    compose_file,
                    compose_override,
                    project,
                } => self.podman_compose.start(
                    compose_file,
                    compose_override.as_deref(),
                    project.as_deref(),
                )?,
                Operation::StopPodmanComposeService {
                    compose_file,
                    compose_override,
                    project,
                } => self.podman_compose.stop(
                    compose_file,
                    compose_override.as_deref(),
                    project.as_deref(),
                )?,
                Operation::RestartPodmanComposeService {
                    compose_file,
                    compose_override,
                    project,
                } => self.podman_compose.restart(
                    compose_file,
                    compose_override.as_deref(),
                    project.as_deref(),
                )?,
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
                Operation::StartPodmanComposeService {
                    compose_file,
                    compose_override,
                    project,
                } => writeln!(
                    self.writer,
                    "  [DRY-RUN] {}. Would start `podman_compose` project {:?} with base `{}` override {:?}",
                    i + 1,
                    project,
                    compose_file,
                    compose_override
                )?,
                Operation::StopPodmanComposeService {
                    compose_file,
                    compose_override,
                    project,
                } => writeln!(
                    self.writer,
                    "  [DRY-RUN] {}. Would stop `podman_compose` project {:?} with base `{}` override {:?}",
                    i + 1,
                    project,
                    compose_file,
                    compose_override
                )?,
                Operation::RestartPodmanComposeService {
                    compose_file,
                    compose_override,
                    project,
                } => writeln!(
                    self.writer,
                    "  [DRY-RUN] {}. Would restart `podman_compose` project {:?} with base `{}` override {:?}",
                    i + 1,
                    project,
                    compose_file,
                    compose_override
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

    #[test]
    fn dry_run_executor_renders_podman_compose_operations() {
        let mut output = Vec::new();
        let mut executor = DryRunExecutor::new(&mut output);
        let plan = Plan {
            operations: vec![Operation::RestartPodmanComposeService {
                compose_file: "/tmp/base.yml".to_string(),
                compose_override: Some("/tmp/override.yml".to_string()),
                project: Some("docker".to_string()),
            }],
        };

        executor
            .execute(&plan)
            .expect("dry-run execution should succeed");

        let rendered = String::from_utf8(output).expect("writer should contain utf8");
        assert!(rendered.contains("Would restart `podman_compose` project Some(\"docker\")"));
    }
}
