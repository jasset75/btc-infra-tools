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
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::io::{Read};
use std::net::TcpStream;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

const LAUNCHD_MANAGER: &str = "launchd";
const PODMAN_COMPOSE_MANAGER: &str = "podman_compose";
const PODMAN_MACHINE_MANAGER: &str = "podman_machine";

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
    machine: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    health_url: Option<String>,
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
            machine: None,
            health_url: None,
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
            machine: None,
            health_url: None,
            running_containers: Some(running_containers),
            query_error,
        }
    }

    fn podman_machine(
        service: &str,
        machine: Option<String>,
        state: &str,
        query_error: Option<String>,
    ) -> Self {
        Self {
            service: service.to_string(),
            manager: PODMAN_MACHINE_MANAGER.to_string(),
            state: state.to_string(),
            unit: None,
            pid: None,
            compose_file: None,
            compose_override: None,
            project: None,
            machine,
            health_url: None,
            running_containers: None,
            query_error,
        }
    }

    fn with_health(mut self, health_url: Option<String>, query_error: Option<String>) -> Self {
        self.health_url = health_url;
        self.query_error = query_error;
        self
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

        let (state, running_containers, query_error, health_url) =
            match infractl_adapters::PodmanComposeAdapter.running_container_ids(
                &compose_file,
                compose_override.as_deref(),
                project.as_deref(),
            ) {
                Ok(ids) => {
                    if ids.is_empty() {
                        ("stopped", ids, None, None)
                    } else if ctx.service_name == "mempool" {
                        let url = mempool_health_url(ctx.env_resolver);
                        match probe_http_200(&url) {
                            Ok(()) => ("running", ids, None, Some(url)),
                            Err(err) => ("degraded", ids, Some(err.to_string()), Some(url)),
                        }
                    } else {
                        ("running", ids, None, None)
                    }
                }
                Err(err) => ("unknown", Vec::new(), Some(err.to_string()), None),
            };

        let message = format!(
            "status target={} ui={:?} state={state}",
            ctx.service_name, ctx.ui_mode
        );
        let status_data = if ctx.service_name == "mempool" {
            ServiceStatusData::podman(
                ctx.service_name,
                Some(compose_file),
                compose_override,
                project,
                state,
                running_containers,
                None,
            )
            .with_health(health_url, query_error)
        } else {
            ServiceStatusData::podman(
                ctx.service_name,
                Some(compose_file),
                compose_override,
                project,
                state,
                running_containers,
                query_error,
            )
        };
        return Ok(StatusComputation {
            message,
            data: status_data,
        });
    }

    if service.manager == PODMAN_MACHINE_MANAGER {
        let service_name = ctx.service_name.to_string();
        let machine_tmpl = service
            .machine
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("service `{}` is missing `machine`", ctx.service_name))?;
        let machine = match expand_placeholders(machine_tmpl, ctx.env_resolver) {
            Ok(value) => value,
            Err(err) => {
                return Ok(unknown_status(
                    ctx,
                    ServiceStatusData::podman_machine(
                        &service_name,
                        Some(machine_tmpl.to_string()),
                        "unknown",
                        Some(err.to_string()),
                    ),
                ));
            }
        };

        let (state, query_error) =
            match infractl_adapters::PodmanMachineAdapter.is_running(&machine) {
                Ok(true) => ("running", None),
                Ok(false) => ("stopped", None),
                Err(err) => ("unknown", Some(err.to_string())),
            };

        let message = format!(
            "status target={} ui={:?} state={state}",
            ctx.service_name, ctx.ui_mode
        );
        let status_data = ServiceStatusData::podman_machine(
            ctx.service_name,
            Some(machine),
            state,
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
            machine: None,
            health_url: None,
            running_containers: None,
            query_error: None,
        },
    })
}

fn mempool_health_url(env_resolver: &dyn EnvResolver) -> String {
    let host = env_resolver
        .resolve("MEMPOOL_HOST")
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let port = env_resolver
        .resolve("MEMPOOL_PORT")
        .unwrap_or_else(|| "8080".to_string());
    format!("http://{host}:{port}/api/v1/backend-info")
}

fn probe_http_200(url: &str) -> Result<()> {
    let endpoint = parse_http_url(url)?;
    let mut stream = TcpStream::connect((&endpoint.host[..], endpoint.port))
        .with_context(|| format!("failed to connect to {}", endpoint.authority()))?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .context("failed to set read timeout")?;
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(2)))
        .context("failed to set write timeout")?;

    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: application/json\r\n\r\n",
        endpoint.path,
        endpoint.authority()
    );
    stream
        .write_all(request.as_bytes())
        .context("failed to write HTTP request")?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .context("failed to read HTTP response")?;

    let status_line = response.lines().next().unwrap_or_default();
    if status_line.contains(" 200 ") {
        return Ok(());
    }

    bail!("unexpected HTTP status from {}: {}", endpoint.authority(), status_line)
}

struct HttpEndpoint {
    host: String,
    port: u16,
    path: String,
}

impl HttpEndpoint {
    fn authority(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

fn parse_http_url(url: &str) -> Result<HttpEndpoint> {
    let without_scheme = url
        .strip_prefix("http://")
        .ok_or_else(|| anyhow::anyhow!("unsupported URL scheme for `{url}`"))?;
    let (authority, path) = match without_scheme.split_once('/') {
        Some((authority, rest)) => (authority, format!("/{}", rest)),
        None => (without_scheme, "/".to_string()),
    };

    let (host, port) = match authority.split_once(':') {
        Some((host, port)) => (
            host.to_string(),
            port.parse::<u16>()
                .with_context(|| format!("invalid port in `{url}`"))?,
        ),
        None => (authority.to_string(), 80),
    };

    Ok(HttpEndpoint { host, port, path })
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

    maybe_load_service_env_file(env_resolver, &config, service_name)?;

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

pub(crate) fn execute_service_bring_up_from_config(
    env_resolver: &dyn EnvResolver,
    config_path: &PathBuf,
    service_name: &str,
    dry_run: bool,
) -> Result<PlanExecutionResult> {
    let raw = fs::read_to_string(config_path)
        .with_context(|| format!("failed to read config file {}", config_path.display()))?;
    let config: BelterConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse TOML from {}", config_path.display()))?;

    let ordered_services = resolve_bring_up_order(&config, service_name)?;
    for name in &ordered_services {
        maybe_load_service_env_file(env_resolver, &config, name)?;
    }

    let plan = build_bring_up_plan(&config, env_resolver, &ordered_services, dry_run)?;

    use infractl_core::plan::Executor;

    if dry_run {
        let mut executor = infractl_adapters::executor::DryRunExecutor::sink();
        executor.execute(&plan)?;
        return Ok(PlanExecutionResult {
            plan,
            message: format!(
                "would bring up service `{service_name}` (dependencies: {})",
                ordered_services.join(", ")
            ),
            execution_report: Vec::new(),
            events: Vec::new(),
        });
    }

    let mut executor = infractl_adapters::executor::RealExecutor::new();
    let execution_report = executor.execute(&plan)?;

    if service_name == "mempool" {
        wait_for_mempool_readiness(env_resolver)?;
    }

    Ok(PlanExecutionResult {
        plan,
        message: format!(
            "bring-up completed for service `{service_name}` (dependencies: {})",
            ordered_services.join(", ")
        ),
        execution_report,
        events: Vec::new(),
    })
}

fn maybe_load_service_env_file(
    env_resolver: &dyn EnvResolver,
    config: &BelterConfig,
    service_name: &str,
) -> Result<()> {
    let Some(service) = config.service_by_name(service_name) else {
        return Ok(());
    };

    if service.manager != PODMAN_COMPOSE_MANAGER {
        return Ok(());
    }

    let Some(env_file_tmpl) = service.env_file.as_deref() else {
        return Ok(());
    };

    let env_file = expand_placeholders(env_file_tmpl, env_resolver)?;
    dotenvy::from_filename_override(&env_file)
        .with_context(|| format!("failed to load service env file {env_file}"))?;
    Ok(())
}

fn resolve_bring_up_order(config: &BelterConfig, service_name: &str) -> Result<Vec<String>> {
    let mut visited = HashSet::new();
    let mut stack = HashSet::new();
    let mut ordered = Vec::new();
    visit_dependency(config, service_name, &mut visited, &mut stack, &mut ordered)?;
    Ok(ordered)
}

fn visit_dependency(
    config: &BelterConfig,
    service_name: &str,
    visited: &mut HashSet<String>,
    stack: &mut HashSet<String>,
    ordered: &mut Vec<String>,
) -> Result<()> {
    if visited.contains(service_name) {
        return Ok(());
    }
    if !stack.insert(service_name.to_string()) {
        bail!("cyclic service dependency detected at `{service_name}`");
    }

    let service = config
        .service_by_name(service_name)
        .ok_or_else(|| anyhow::anyhow!("service `{service_name}` not found in config"))?;

    for dependency in service.depends_on.as_deref().unwrap_or(&[]) {
        visit_dependency(config, dependency, visited, stack, ordered)?;
    }

    stack.remove(service_name);
    visited.insert(service_name.to_string());
    ordered.push(service_name.to_string());
    Ok(())
}

fn build_bring_up_plan(
    config: &BelterConfig,
    env_resolver: &dyn EnvResolver,
    ordered_services: &[String],
    dry_run: bool,
) -> Result<Plan> {
    let mut operations = Vec::new();

    for service_name in ordered_services {
        if dry_run {
            let req = ServiceCommandRequest {
                config,
                service_name,
                action: ServiceAction::Start,
            };
            let mut plan = req.plan(env_resolver)?;
            operations.append(&mut plan.operations);
        } else {
            let service = config
                .service_by_name(service_name)
                .ok_or_else(|| anyhow::anyhow!("service `{service_name}` not found in config"))?;
            let state = compute_runtime_state(service_name, service, env_resolver)?;

            if should_start_for_bring_up(service_name, state) {
                let req = ServiceCommandRequest {
                    config,
                    service_name,
                    action: ServiceAction::Start,
                };
                let mut plan = req.plan(env_resolver)?;
                operations.append(&mut plan.operations);
            }
        }
    }

    Ok(Plan { operations })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RuntimeState {
    Running,
    Stopped,
    Degraded,
    Unknown,
}

fn compute_runtime_state(
    service_name: &str,
    service: &infractl_core::config::ServiceConfig,
    env_resolver: &dyn EnvResolver,
) -> Result<RuntimeState> {
    match service.manager.as_str() {
        LAUNCHD_MANAGER => {
            let unit_tmpl = service
                .unit
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("service `{service_name}` is missing `unit`"))?;
            let unit = expand_placeholders(unit_tmpl, env_resolver)?;
            match infractl_adapters::LaunchdAdapter.unit_pid_for_status(&unit) {
                Ok(Some(_)) => Ok(RuntimeState::Running),
                Ok(None) => Ok(RuntimeState::Stopped),
                Err(_) => Ok(RuntimeState::Unknown),
            }
        }
        PODMAN_MACHINE_MANAGER => {
            let machine_tmpl = service
                .machine
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("service `{service_name}` is missing `machine`"))?;
            let machine = expand_placeholders(machine_tmpl, env_resolver)?;
            match infractl_adapters::PodmanMachineAdapter.is_running(&machine) {
                Ok(true) => Ok(RuntimeState::Running),
                Ok(false) => Ok(RuntimeState::Stopped),
                Err(_) => Ok(RuntimeState::Unknown),
            }
        }
        PODMAN_COMPOSE_MANAGER => {
            let compose_file_tmpl = service.compose_file.as_deref().ok_or_else(|| {
                anyhow::anyhow!("service `{service_name}` is missing `compose_file`")
            })?;
            let compose_file = expand_placeholders(compose_file_tmpl, env_resolver)?;
            let compose_override = service
                .compose_override
                .as_deref()
                .map(|value| expand_placeholders(value, env_resolver))
                .transpose()?;
            let project = service
                .project
                .as_deref()
                .map(|value| expand_placeholders(value, env_resolver))
                .transpose()?;

            match infractl_adapters::PodmanComposeAdapter.running_container_ids(
                &compose_file,
                compose_override.as_deref(),
                project.as_deref(),
            ) {
                Ok(ids) if ids.is_empty() => Ok(RuntimeState::Stopped),
                Ok(_) if service_name == "mempool" => {
                    let url = mempool_health_url(env_resolver);
                    match probe_http_200(&url) {
                        Ok(()) => Ok(RuntimeState::Running),
                        Err(_) => Ok(RuntimeState::Degraded),
                    }
                }
                Ok(_) => Ok(RuntimeState::Running),
                Err(_) => Ok(RuntimeState::Unknown),
            }
        }
        other => bail!("service `{service_name}` uses unsupported manager `{other}`"),
    }
}

fn should_start_for_bring_up(service_name: &str, state: RuntimeState) -> bool {
    match state {
        RuntimeState::Stopped | RuntimeState::Unknown => true,
        RuntimeState::Degraded if service_name == "mempool" => true,
        RuntimeState::Running | RuntimeState::Degraded => false,
    }
}

fn wait_for_mempool_readiness(env_resolver: &dyn EnvResolver) -> Result<()> {
    let url = mempool_health_url(env_resolver);
    let mut delay = Duration::from_secs(1);

    for attempt in 1..=5 {
        match probe_http_200(&url) {
            Ok(()) => return Ok(()),
            Err(err) if attempt == 5 => {
                bail!("mempool did not become ready at {url}: {err}");
            }
            Err(_) => {
                thread::sleep(delay);
                delay = std::cmp::min(delay * 2, Duration::from_secs(8));
            }
        }
    }

    bail!("mempool did not become ready at {url}")
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
