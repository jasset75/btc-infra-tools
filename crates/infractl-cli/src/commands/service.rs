use crate::cli::UiMode;
use crate::output::{emit_dry_run_report, output_envelope};
use anyhow::{Context, Result, bail};
use infractl_core::config::BelterConfig;
use infractl_core::env::{EnvResolver, expand_placeholders};
use infractl_core::output::OutputEvent;
use infractl_core::plan::{ExecutionDetails, ExecutionReport, Plan};
use infractl_core::time::Clock;
use infractl_core::usecase::{ServiceAction, ServiceCommandRequest};
use serde::Serialize;
use serde_json::json;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

const LAUNCHD_MANAGER: &str = "launchd";
const PODMAN_COMPOSE_MANAGER: &str = "podman_compose";

#[derive(Serialize)]
struct ServiceStatusData {
    service: String,
    manager: String,
    state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pid: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    compose_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    compose_override: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    running_containers: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    query_error: Option<String>,
}

impl ServiceStatusData {
    fn launchd(service: &str, unit: String, state: &str, pid: Option<i32>, query_error: Option<String>) -> Self {
        Self {
            service: service.to_string(),
            manager: LAUNCHD_MANAGER.to_string(),
            state: state.to_string(),
            unit: Some(unit),
            pid,
            compose_file: None,
            compose_override: None,
            project: None,
            running_containers: None,
            query_error,
        }
    }

    fn podman(
        service: &str,
        compose_file: Option<String>,
        compose_override: Option<String>,
        project: Option<String>,
        state: &str,
        running_containers: Vec<String>,
        query_error: Option<String>,
    ) -> Self {
        Self {
            service: service.to_string(),
            manager: PODMAN_COMPOSE_MANAGER.to_string(),
            state: state.to_string(),
            unit: None,
            pid: None,
            compose_file,
            compose_override,
            project,
            running_containers: Some(running_containers),
            query_error,
        }
    }
}

pub(crate) struct StatusEmitCtx<'a, W: Write> {
    pub(crate) clock: &'a dyn Clock,
    pub(crate) stdout: &'a mut W,
    pub(crate) json: bool,
    pub(crate) dry_run: bool,
    pub(crate) config_path: &'a PathBuf,
    pub(crate) env_resolver: &'a dyn EnvResolver,
    pub(crate) service_name: &'a str,
    pub(crate) ui_mode: UiMode,
}

struct StatusComputation {
    message: String,
    data: ServiceStatusData,
}

pub(crate) fn emit_status<W: Write>(ctx: StatusEmitCtx<'_, W>) -> Result<()> {
    let raw = fs::read_to_string(ctx.config_path)
        .with_context(|| format!("failed to read config file {}", ctx.config_path.display()))?;
    let config: BelterConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse TOML from {}", ctx.config_path.display()))?;

    let services = config
        .service
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing [service] section"))?;
    let service = services
        .get(ctx.service_name)
        .ok_or_else(|| anyhow::anyhow!("service `{}` not found in config", ctx.service_name))?;

    if ctx.dry_run {
        let out = output_envelope(
            ctx.clock,
            "service.status",
            "ok",
            &format!(
                "would query status target={} ui={:?}",
                ctx.service_name, ctx.ui_mode
            ),
            true,
            json!({
                "service": ctx.service_name,
                "manager": service.manager,
                "simulated": true,
            }),
            Vec::new(),
        );
        if ctx.json {
            writeln!(ctx.stdout, "{}", serde_json::to_string_pretty(&out)?)?;
        } else {
            writeln!(ctx.stdout, "[{}] {}: {}", out.ts, out.command, out.message)?;
            emit_dry_run_report(ctx.stdout, &out)?;
        }
        return Ok(());
    }

    let computed = compute_status(&ctx, service)?;
    emit_status_out(
        ctx.clock,
        ctx.stdout,
        ctx.json,
        &computed.message,
        computed.data,
    )
}

fn compute_status(ctx: &StatusEmitCtx<'_, impl Write>, service: &infractl_core::config::ServiceConfig) -> Result<StatusComputation> {
    if service.manager == LAUNCHD_MANAGER {
        let unit_tmpl = service
            .unit
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("service `{}` is missing `unit`", ctx.service_name))?;
        let unit = expand_placeholders(unit_tmpl, ctx.env_resolver)?;
        let (state, pid, query_error) =
            match infractl_adapters::LaunchdAdapter.unit_pid_for_status(&unit) {
                Ok(pid) => {
                    let state = if pid.is_some() { "running" } else { "stopped" };
                    (state, pid, None)
                }
                Err(err) => ("unknown", None, Some(err.to_string())),
            };
        let message = format!(
            "status target={} ui={:?} state={state}",
            ctx.service_name, ctx.ui_mode
        );
        let status_data = ServiceStatusData::launchd(ctx.service_name, unit, state, pid, query_error);
        return Ok(StatusComputation {
            message,
            data: status_data,
        });
    }

    if service.manager == PODMAN_COMPOSE_MANAGER {
        let service_name = ctx.service_name.to_string();
        let compose_file_tmpl = service.compose_file.as_deref().ok_or_else(|| {
            anyhow::anyhow!("service `{}` is missing `compose_file`", ctx.service_name)
        })?;
        let compose_file = match expand_placeholders(compose_file_tmpl, ctx.env_resolver) {
            Ok(value) => value,
            Err(err) => {
                return Ok(unknown_status(
                    ctx,
                    ServiceStatusData::podman(
                        &service_name,
                        Some(compose_file_tmpl.to_string()),
                        None,
                        None,
                        "unknown",
                        Vec::new(),
                        Some(err.to_string()),
                    ),
                ));
            }
        };
        let compose_override = match service
            .compose_override
            .as_deref()
            .map(|value| expand_placeholders(value, ctx.env_resolver))
            .transpose()
        {
            Ok(value) => value,
            Err(err) => {
                return Ok(unknown_status(
                    ctx,
                    ServiceStatusData::podman(
                        &service_name,
                        Some(compose_file.clone()),
                        None,
                        None,
                        "unknown",
                        Vec::new(),
                        Some(err.to_string()),
                    ),
                ));
            }
        };
        let project = match service
            .project
            .as_deref()
            .map(|value| expand_placeholders(value, ctx.env_resolver))
            .transpose()
        {
            Ok(value) => value,
            Err(err) => {
                return Ok(unknown_status(
                    ctx,
                    ServiceStatusData::podman(
                        &service_name,
                        Some(compose_file.clone()),
                        compose_override.clone(),
                        None,
                        "unknown",
                        Vec::new(),
                        Some(err.to_string()),
                    ),
                ));
            }
        };

        let (state, running_containers, query_error) =
            match infractl_adapters::PodmanComposeAdapter.running_container_ids(
                &compose_file,
                compose_override.as_deref(),
                project.as_deref(),
            ) {
                Ok(ids) => {
                    let state = if ids.is_empty() { "stopped" } else { "running" };
                    (state, ids, None)
                }
                Err(err) => ("unknown", Vec::new(), Some(err.to_string())),
            };

        let message = format!(
            "status target={} ui={:?} state={state}",
            ctx.service_name, ctx.ui_mode
        );
        let status_data = ServiceStatusData::podman(
            ctx.service_name,
            Some(compose_file),
            compose_override,
            project,
            state,
            running_containers,
            query_error,
        );
        return Ok(StatusComputation {
            message,
            data: status_data,
        });
    }

    Ok(StatusComputation {
        message: format!(
            "status target={} ui={:?} manager={} (real status not implemented)",
            ctx.service_name, ctx.ui_mode, service.manager
        ),
        data: ServiceStatusData {
            service: ctx.service_name.to_string(),
            manager: service.manager.clone(),
            state: "unknown".to_string(),
            unit: None,
            pid: None,
            compose_file: None,
            compose_override: None,
            project: None,
            running_containers: None,
            query_error: None,
        },
    })
}

fn unknown_status(ctx: &StatusEmitCtx<'_, impl Write>, data: ServiceStatusData) -> StatusComputation {
    StatusComputation {
        message: format!(
            "status target={} ui={:?} state=unknown",
            ctx.service_name, ctx.ui_mode
        ),
        data,
    }
}

fn emit_status_out<W: Write>(
    clock: &dyn Clock,
    stdout: &mut W,
    json: bool,
    message: &str,
    status_data: ServiceStatusData,
) -> Result<()> {
    let data = serde_json::to_value(status_data).context("failed to serialize status data")?;
    let out = output_envelope(
        clock,
        "service.status",
        "ok",
        message,
        false,
        data,
        Vec::new(),
    );
    if json {
        writeln!(stdout, "{}", serde_json::to_string_pretty(&out)?)?;
    } else {
        writeln!(stdout, "[{}] {}: {}", out.ts, out.command, out.message)?;
    }
    Ok(())
}

pub(crate) fn execute_service_command_from_config(
    env_resolver: &dyn EnvResolver,
    config_path: &PathBuf,
    service_name: &str,
    action: ServiceAction,
    dry_run: bool,
) -> Result<PlanExecutionResult> {
    let raw = fs::read_to_string(config_path)
        .with_context(|| format!("failed to read config file {}", config_path.display()))?;
    let config: BelterConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse TOML from {}", config_path.display()))?;

    let req = ServiceCommandRequest {
        config: &config,
        service_name,
        action,
    };

    let plan = req.plan(env_resolver)?;

    use infractl_core::plan::Executor;

    if dry_run {
        let mut executor = infractl_adapters::executor::DryRunExecutor::sink();
        executor.execute(&plan)?;
        Ok(PlanExecutionResult {
            plan,
            message: format!("would {} service `{service_name}`", action_label(action)),
            execution_report: Vec::new(),
            events: Vec::new(),
        })
    } else {
        let mut executor = infractl_adapters::executor::RealExecutor::new();
        let execution_report = executor.execute(&plan)?;
        Ok(PlanExecutionResult {
            plan,
            message: execution_message(service_name, action, &execution_report),
            execution_report,
            events: Vec::new(),
        })
    }
}

pub(crate) struct PlanExecutionResult {
    pub(crate) plan: Plan,
    pub(crate) message: String,
    pub(crate) execution_report: Vec<ExecutionReport>,
    pub(crate) events: Vec<OutputEvent>,
}

pub(crate) fn emit_plan<W: Write>(
    clock: &dyn Clock,
    stdout: &mut W,
    json: bool,
    dry_run: bool,
    command: &str,
    result: Result<PlanExecutionResult>,
) -> Result<()> {
    match result {
        Ok(plan_result) => {
            let out = output_envelope(
                clock,
                command,
                "ok",
                &plan_result.message,
                dry_run,
                json!({
                    "plan": plan_result.plan,
                    "execution_report": plan_result.execution_report,
                }),
                plan_result.events,
            );
            if json {
                writeln!(stdout, "{}", serde_json::to_string_pretty(&out)?)?;
            } else {
                writeln!(stdout, "[{}] {}: {}", out.ts, out.command, out.message)?;
                if dry_run {
                    emit_dry_run_report(stdout, &out)?;
                }
            }
            Ok(())
        }
        Err(e) => {
            bail!(e);
        }
    }
}

fn execution_message(
    service_name: &str,
    action: ServiceAction,
    execution_report: &[ExecutionReport],
) -> String {
    let base = match action {
        ServiceAction::Start => format!("started service `{service_name}`"),
        ServiceAction::Stop => format!("stopped service `{service_name}`"),
        ServiceAction::Restart => format!("restart requested for service `{service_name}`"),
    };

    if let Some((pid_before, pid_after)) = execution_report
        .iter()
        .map(|report| match &report.details {
            ExecutionDetails::LaunchdRestartPidChange {
                pid_before,
                pid_after,
                ..
            } => (*pid_before, *pid_after),
        })
        .next()
    {
        let restart_observed =
            matches!((pid_before, pid_after), (Some(before), Some(after)) if before != after);
        return format!(
            "{base} (restart observed: {}, pid before: {}, pid after: {})",
            if restart_observed { "yes" } else { "no" },
            pid_before
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            pid_after
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );
    }

    base
}

fn action_label(action: ServiceAction) -> &'static str {
    match action {
        ServiceAction::Start => "start",
        ServiceAction::Stop => "stop",
        ServiceAction::Restart => "restart",
    }
}
