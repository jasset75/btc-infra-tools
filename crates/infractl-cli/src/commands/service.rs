use crate::cli::UiMode;
use crate::output::{emit_dry_run_report, output_envelope};
use anyhow::{Context, Result, bail};
use infractl_core::config::BelterConfig;
use infractl_core::env::{EnvResolver, expand_placeholders};
use infractl_core::output::{OutputEvent, SeverityLevel};
use infractl_core::plan::{ExecutionDetails, ExecutionReport, Executor, Plan};
use infractl_core::time::{Clock, SystemClock};
use infractl_core::usecase::{ServiceAction, ServiceCommandRequest};
use serde::Serialize;
use serde_json::json;
use std::collections::HashSet;
use std::fs;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::net::TcpStream;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

const LAUNCHD_MANAGER: &str = "launchd";
const PODMAN_COMPOSE_MANAGER: &str = "podman_compose";
const PODMAN_MACHINE_MANAGER: &str = "podman_machine";

#[derive(Serialize)]
struct ServiceListItem {
    service: String,
    manager: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    depends_on: Vec<String>,
}

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
    fn launchd(
        service: &str,
        unit: String,
        state: &str,
        pid: Option<i32>,
        query_error: Option<String>,
    ) -> Self {
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

#[derive(Serialize)]
struct AggregatedServiceStatusData {
    services: Vec<ServiceStatusData>,
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

struct PodmanComposeStatusSnapshot {
    compose_file: String,
    compose_override: Option<String>,
    project: Option<String>,
    state: RuntimeState,
    running_containers: Vec<String>,
    query_error: Option<String>,
    health_url: Option<String>,
}

struct DependentComposeIssue {
    service: String,
    state: RuntimeState,
    running_containers: Vec<String>,
    query_error: Option<String>,
}

pub(crate) fn emit_list<W: Write>(
    clock: &dyn Clock,
    stdout: &mut W,
    json: bool,
    dry_run: bool,
    config_path: &PathBuf,
) -> Result<()> {
    let raw = fs::read_to_string(config_path)
        .with_context(|| format!("failed to read config file {}", config_path.display()))?;
    let config: BelterConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse TOML from {}", config_path.display()))?;

    let mut services: Vec<ServiceListItem> = config
        .service
        .unwrap_or_default()
        .into_iter()
        .map(|(service, cfg)| ServiceListItem {
            service,
            manager: cfg.manager,
            depends_on: cfg.depends_on.unwrap_or_default(),
        })
        .collect();
    services.sort_by(|a, b| a.service.cmp(&b.service));

    let out = output_envelope(
        clock,
        "service.list",
        "ok",
        &format!("listed {} configured service(s)", services.len()),
        dry_run,
        json!({ "services": services }),
        Vec::new(),
    );

    if json {
        writeln!(stdout, "{}", serde_json::to_string_pretty(&out)?)?;
    } else {
        writeln!(stdout, "[{}] {}: {}", out.ts, out.command, out.message)?;
        let services = out
            .data
            .get("services")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();

        for service in services {
            let name = service
                .get("service")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let manager = service
                .get("manager")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let depends_on = service
                .get("depends_on")
                .and_then(|value| value.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            if depends_on.is_empty() {
                writeln!(stdout, "- {name} ({manager})")?;
            } else {
                writeln!(
                    stdout,
                    "- {name} ({manager}) depends_on={}",
                    depends_on.join(", ")
                )?;
            }
        }
    }

    Ok(())
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

    let computed = compute_status(&ctx, &config, service)?;
    emit_status_out(
        ctx.clock,
        ctx.stdout,
        ctx.json,
        &computed.message,
        computed.data,
    )
}

pub(crate) fn emit_status_all<W: Write>(
    clock: &dyn Clock,
    stdout: &mut W,
    json: bool,
    dry_run: bool,
    config_path: &PathBuf,
    env_resolver: &dyn EnvResolver,
    ui_mode: UiMode,
) -> Result<()> {
    let raw = fs::read_to_string(config_path)
        .with_context(|| format!("failed to read config file {}", config_path.display()))?;
    let config: BelterConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse TOML from {}", config_path.display()))?;

    let mut service_names: Vec<String> = config
        .service
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing [service] section"))?
        .keys()
        .cloned()
        .collect();
    service_names.sort();

    if dry_run {
        let out = output_envelope(
            clock,
            "service.status",
            "ok",
            &format!(
                "would query status target=all services={} ui={ui_mode:?}",
                service_names.len()
            ),
            true,
            json!({
                "services": service_names
                    .iter()
                    .map(|service| json!({
                        "service": service,
                        "simulated": true,
                    }))
                    .collect::<Vec<_>>(),
            }),
            Vec::new(),
        );
        if json {
            writeln!(stdout, "{}", serde_json::to_string_pretty(&out)?)?;
        } else {
            writeln!(stdout, "[{}] {}: {}", out.ts, out.command, out.message)?;
            emit_dry_run_report(stdout, &out)?;
        }
        return Ok(());
    }

    let services = config
        .service
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing [service] section"))?;
    let mut computed = Vec::with_capacity(service_names.len());
    for service_name in &service_names {
        let service = services
            .get(service_name)
            .ok_or_else(|| anyhow::anyhow!("service `{service_name}` not found in config"))?;
        let ctx = StatusEmitCtx {
            clock,
            stdout,
            json,
            dry_run,
            config_path,
            env_resolver,
            service_name,
            ui_mode,
        };
        computed.push(compute_status(&ctx, &config, service)?);
    }

    let unknown = computed
        .iter()
        .filter(|item| item.data.state == "unknown")
        .count();
    let degraded = computed
        .iter()
        .filter(|item| item.data.state == "degraded")
        .count();
    let syncing = computed
        .iter()
        .filter(|item| item.data.state == "syncing")
        .count();
    let running = computed
        .iter()
        .filter(|item| item.data.state == "running")
        .count();
    let stopped = computed
        .iter()
        .filter(|item| item.data.state == "stopped")
        .count();
    let message = format!(
        "status target=all ui={ui_mode:?} services={} running={} stopped={} syncing={} degraded={} unknown={}",
        computed.len(),
        running,
        stopped,
        syncing,
        degraded,
        unknown
    );
    let data = AggregatedServiceStatusData {
        services: computed.into_iter().map(|item| item.data).collect(),
    };
    emit_status_all_out(clock, stdout, json, &message, data)
}

fn compute_status(
    ctx: &StatusEmitCtx<'_, impl Write>,
    config: &BelterConfig,
    service: &infractl_core::config::ServiceConfig,
) -> Result<StatusComputation> {
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
        let status_data =
            ServiceStatusData::launchd(ctx.service_name, unit, state, pid, query_error);
        return Ok(StatusComputation {
            message,
            data: status_data,
        });
    }

    if service.manager == PODMAN_COMPOSE_MANAGER {
        let snapshot =
            compute_podman_compose_status_snapshot(ctx.service_name, service, ctx.env_resolver)?;

        let message = format!(
            "status target={} ui={:?} state={}",
            ctx.service_name,
            ctx.ui_mode,
            runtime_state_label(snapshot.state)
        );
        let status_data = if ctx.service_name == "mempool" {
            ServiceStatusData::podman(
                ctx.service_name,
                Some(snapshot.compose_file),
                snapshot.compose_override,
                snapshot.project,
                runtime_state_label(snapshot.state),
                snapshot.running_containers,
                None,
            )
            .with_health(snapshot.health_url, snapshot.query_error)
        } else {
            ServiceStatusData::podman(
                ctx.service_name,
                Some(snapshot.compose_file),
                snapshot.compose_override,
                snapshot.project,
                runtime_state_label(snapshot.state),
                snapshot.running_containers,
                snapshot.query_error,
            )
        };
        return Ok(StatusComputation {
            message,
            data: status_data,
        });
    }

    if service.manager == PODMAN_MACHINE_MANAGER {
        let service_name = ctx.service_name.to_string();
        let machine_tmpl = service.machine.as_deref().ok_or_else(|| {
            anyhow::anyhow!("service `{}` is missing `machine`", ctx.service_name)
        })?;
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
                Ok(true) => derive_podman_runtime_state(dependent_compose_issues(
                    config,
                    ctx.service_name,
                    ctx.env_resolver,
                )),
                Ok(false) => ("stopped", None),
                Err(err) => ("unknown", Some(err.to_string())),
            };

        let message = format!(
            "status target={} ui={:?} state={state}",
            ctx.service_name, ctx.ui_mode
        );
        let status_data =
            ServiceStatusData::podman_machine(ctx.service_name, Some(machine), state, query_error);
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

fn compute_podman_compose_status_snapshot(
    service_name: &str,
    service: &infractl_core::config::ServiceConfig,
    env_resolver: &dyn EnvResolver,
) -> Result<PodmanComposeStatusSnapshot> {
    let compose_file_tmpl = service
        .compose_file
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("service `{service_name}` is missing `compose_file`"))?;
    let compose_file = match expand_placeholders(compose_file_tmpl, env_resolver) {
        Ok(value) => value,
        Err(err) => {
            return Ok(PodmanComposeStatusSnapshot {
                compose_file: compose_file_tmpl.to_string(),
                compose_override: None,
                project: None,
                state: RuntimeState::Unknown,
                running_containers: Vec::new(),
                query_error: Some(err.to_string()),
                health_url: None,
            });
        }
    };
    let compose_override = match service
        .compose_override
        .as_deref()
        .map(|value| expand_placeholders(value, env_resolver))
        .transpose()
    {
        Ok(value) => value,
        Err(err) => {
            return Ok(PodmanComposeStatusSnapshot {
                compose_file,
                compose_override: None,
                project: None,
                state: RuntimeState::Unknown,
                running_containers: Vec::new(),
                query_error: Some(err.to_string()),
                health_url: None,
            });
        }
    };
    let project = match service
        .project
        .as_deref()
        .map(|value| expand_placeholders(value, env_resolver))
        .transpose()
    {
        Ok(value) => value,
        Err(err) => {
            return Ok(PodmanComposeStatusSnapshot {
                compose_file,
                compose_override,
                project: None,
                state: RuntimeState::Unknown,
                running_containers: Vec::new(),
                query_error: Some(err.to_string()),
                health_url: None,
            });
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
                    (RuntimeState::Stopped, ids, None, None)
                } else if service_name == "mempool" {
                    let url = mempool_health_url(env_resolver);
                    match probe_mempool_status(env_resolver) {
                        MempoolProbeStatus::Ready => (RuntimeState::Running, ids, None, Some(url)),
                        MempoolProbeStatus::Syncing(detail) => {
                            (RuntimeState::Syncing, ids, Some(detail), Some(url))
                        }
                        MempoolProbeStatus::Degraded(err) => (
                            RuntimeState::Degraded,
                            ids,
                            Some(err.to_string()),
                            Some(url),
                        ),
                    }
                } else {
                    (RuntimeState::Running, ids, None, None)
                }
            }
            Err(err) => (
                RuntimeState::Unknown,
                Vec::new(),
                Some(err.to_string()),
                None,
            ),
        };

    Ok(PodmanComposeStatusSnapshot {
        compose_file,
        compose_override,
        project,
        state,
        running_containers,
        query_error,
        health_url,
    })
}

fn dependent_compose_issues(
    config: &BelterConfig,
    runtime_service_name: &str,
    env_resolver: &dyn EnvResolver,
) -> Vec<DependentComposeIssue> {
    config
        .service
        .as_ref()
        .into_iter()
        .flat_map(|services| services.iter())
        .filter(|(_, service)| service.manager == PODMAN_COMPOSE_MANAGER)
        .filter(|(_, service)| {
            service
                .depends_on
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .any(|dependency| dependency == runtime_service_name)
        })
        .filter_map(|(name, service)| {
            let snapshot =
                compute_podman_compose_status_snapshot(name, service, env_resolver).ok()?;
            if snapshot.state == RuntimeState::Degraded && !snapshot.running_containers.is_empty() {
                Some(DependentComposeIssue {
                    service: name.clone(),
                    state: snapshot.state,
                    running_containers: snapshot.running_containers,
                    query_error: snapshot.query_error,
                })
            } else {
                None
            }
        })
        .collect()
}

fn derive_podman_runtime_state(
    dependent_issues: Vec<DependentComposeIssue>,
) -> (&'static str, Option<String>) {
    if dependent_issues.is_empty() {
        return ("running", None);
    }

    let issue = &dependent_issues[0];
    let detail = issue
        .query_error
        .as_deref()
        .unwrap_or("dependent compose service reported degraded state");
    (
        "degraded",
        Some(format!(
            "dependent service `{}` is {} with {} running container(s); podman port forwarding may be unhealthy: {}",
            issue.service,
            runtime_state_label(issue.state),
            issue.running_containers.len(),
            detail
        )),
    )
}

fn mempool_health_url(env_resolver: &dyn EnvResolver) -> String {
    mempool_probe_urls(env_resolver)
        .into_iter()
        .next()
        .expect("mempool probe list should not be empty")
}

fn mempool_probe_urls(env_resolver: &dyn EnvResolver) -> Vec<String> {
    [
        "/api/v1/backend-info",
        "/api/v1/fees/recommended",
        "/api/mempool",
        "/api/blocks/tip/height",
    ]
    .into_iter()
    .map(|path| mempool_url(env_resolver, path))
    .collect()
}

fn mempool_url(env_resolver: &dyn EnvResolver, path: &str) -> String {
    let host = env_resolver
        .resolve("MEMPOOL_HOST")
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let port = env_resolver
        .resolve("MEMPOOL_PORT")
        .unwrap_or_else(|| "8080".to_string());
    format!("http://{host}:{port}{path}")
}

enum MempoolProbeStatus {
    Ready,
    Syncing(String),
    Degraded(anyhow::Error),
}

fn probe_mempool_status(env_resolver: &dyn EnvResolver) -> MempoolProbeStatus {
    let backend_info_url = mempool_url(env_resolver, "/api/v1/backend-info");
    if let Err(err) = probe_http_200(&backend_info_url) {
        return MempoolProbeStatus::Degraded(
            err.context(format!("mempool probe failed for {backend_info_url}")),
        );
    }

    let fees_url = mempool_url(env_resolver, "/api/v1/fees/recommended");
    match probe_http_status(&fees_url) {
        Ok(status) => {
            if let Some(status) = classify_mempool_fees_status(&fees_url, &status) {
                return status;
            }
        }
        Err(err) => {
            return MempoolProbeStatus::Degraded(
                err.context(format!("mempool probe failed for {fees_url}")),
            );
        }
    }

    for path in ["/api/mempool", "/api/blocks/tip/height"] {
        let url = mempool_url(env_resolver, path);
        if let Err(err) = probe_http_200(&url) {
            return MempoolProbeStatus::Degraded(
                err.context(format!("mempool probe failed for {url}")),
            );
        }
    }

    MempoolProbeStatus::Ready
}

fn classify_mempool_fees_status(fees_url: &str, status: &HttpStatus) -> Option<MempoolProbeStatus> {
    match status.status_code {
        200 => None,
        503 => Some(MempoolProbeStatus::Syncing(format!(
            "mempool backend is syncing: {fees_url} returned {}",
            status.status_line
        ))),
        _ => Some(MempoolProbeStatus::Degraded(anyhow::anyhow!(
            "mempool probe failed for {fees_url}: unexpected HTTP status from {}: {}",
            status.authority,
            status.status_line
        ))),
    }
}

fn probe_http_200(url: &str) -> Result<()> {
    let status = probe_http_status(url)?;
    validate_http_status(&status.authority, &status.status_line, status.status_code)
}

struct HttpStatus {
    authority: String,
    status_line: String,
    status_code: u16,
}

fn probe_http_status(url: &str) -> Result<HttpStatus> {
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

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .context("failed to read HTTP response")?;
    let authority = endpoint.authority();
    let status_line = status_line.trim_end_matches(['\r', '\n']).to_string();
    let status_code = parse_http_status_code(&authority, &status_line)?;
    Ok(HttpStatus {
        authority,
        status_line,
        status_code,
    })
}

#[cfg(test)]
fn validate_http_status_line(authority: &str, status_line: &str) -> Result<()> {
    let status_line = status_line.trim_end_matches(['\r', '\n']);
    let status_code = parse_http_status_code(authority, status_line)?;
    validate_http_status(authority, status_line, status_code)
}

fn validate_http_status(authority: &str, status_line: &str, status_code: u16) -> Result<()> {
    if status_code == 200 {
        return Ok(());
    }

    bail!("unexpected HTTP status from {authority}: {status_line}")
}

fn parse_http_status_code(authority: &str, status_line: &str) -> Result<u16> {
    status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("invalid HTTP status from {authority}: {status_line}"))?
        .parse::<u16>()
        .with_context(|| format!("invalid HTTP status code from {authority}: {status_line}"))
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

fn unknown_status(
    ctx: &StatusEmitCtx<'_, impl Write>,
    data: ServiceStatusData,
) -> StatusComputation {
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

fn emit_status_all_out<W: Write>(
    clock: &dyn Clock,
    stdout: &mut W,
    json: bool,
    message: &str,
    status_data: AggregatedServiceStatusData,
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
        let services = out
            .data
            .get("services")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        for service in services {
            let name = service
                .get("service")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let manager = service
                .get("manager")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let state = service
                .get("state")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            writeln!(stdout, "- {name} ({manager}) state={state}")?;
        }
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

    let initial_runtime_state = if dry_run {
        None
    } else {
        let service = config
            .service_by_name(service_name)
            .ok_or_else(|| anyhow::anyhow!("service `{service_name}` not found in config"))?;
        Some(compute_runtime_state(service_name, service, env_resolver)?)
    };

    let bring_up = build_bring_up_plan(&config, env_resolver, &ordered_services, dry_run)?;

    use infractl_core::plan::Executor;

    if dry_run {
        let mut executor = infractl_adapters::executor::DryRunExecutor::sink();
        executor.execute(&bring_up.plan)?;
        return Ok(PlanExecutionResult {
            plan: bring_up.plan,
            message: format!(
                "would bring up service `{service_name}` (dependencies: {})",
                ordered_services.join(", ")
            ),
            execution_report: Vec::new(),
            events: bring_up.events,
        });
    }

    let mut executor = infractl_adapters::executor::RealExecutor::new();
    let mut plan = bring_up.plan;
    let mut execution_report = executor.execute(&plan)?;
    let mut events = bring_up.events;

    if service_name == "mempool" {
        events.push(output_event(
            SeverityLevel::Info,
            "bring_up.waiting_readiness",
            "waiting for mempool HTTP readiness",
            json!({ "health_url": mempool_health_url(env_resolver) }),
        ));
        let initial_runtime_state = initial_runtime_state.unwrap_or(RuntimeState::Unknown);
        let mut recovery = MempoolBringUpRecoveryCtx {
            config: &config,
            env_resolver,
            executor: &mut executor,
            accumulated_plan: &mut plan,
            execution_report: &mut execution_report,
            events: &mut events,
            service_name,
        };
        recover_mempool_readiness_if_needed(&mut recovery, initial_runtime_state)?;
        events.push(output_event(
            SeverityLevel::Info,
            "bring_up.ready",
            "mempool HTTP readiness reached",
            json!({ "health_url": mempool_health_url(env_resolver) }),
        ));
    }

    Ok(PlanExecutionResult {
        plan,
        message: format!(
            "bring-up completed for service `{service_name}` (dependencies: {})",
            ordered_services.join(", ")
        ),
        execution_report,
        events,
    })
}

fn recover_mempool_readiness_if_needed(
    recovery: &mut MempoolBringUpRecoveryCtx<'_>,
    initial_runtime_state: RuntimeState,
) -> Result<()> {
    match wait_for_mempool_readiness(recovery.env_resolver) {
        Ok(()) => Ok(()),
        Err(err) => {
            let maybe_err =
                retry_mempool_service_after_failed_readiness(recovery, initial_runtime_state, err)?;
            let Some(err) = maybe_err else {
                return Ok(());
            };
            retry_mempool_runtime_after_failed_readiness(recovery, err)
        }
    }
}

fn retry_mempool_service_after_failed_readiness(
    recovery: &mut MempoolBringUpRecoveryCtx<'_>,
    initial_runtime_state: RuntimeState,
    previous_error: anyhow::Error,
) -> Result<Option<anyhow::Error>> {
    let Some(action) =
        bring_up_retry_action_after_failed_readiness(recovery.service_name, initial_runtime_state)
    else {
        return Ok(Some(previous_error));
    };

    recovery.events.push(output_event(
        SeverityLevel::Warning,
        "bring_up.readiness_retry",
        &format!(
            "mempool readiness failed after {}; retrying with {}",
            action_label(ServiceAction::Start),
            action_label(action)
        ),
        json!({
            "service": recovery.service_name,
            "initial_state": runtime_state_label(initial_runtime_state),
            "health_url": mempool_health_url(recovery.env_resolver),
            "previous_error": previous_error.to_string(),
            "retry_action": action_label(action),
        }),
    ));
    execute_service_action(recovery, recovery.service_name, action)?;
    recovery.events.push(output_event(
        SeverityLevel::Info,
        "bring_up.waiting_readiness",
        "waiting for mempool HTTP readiness",
        json!({ "health_url": mempool_health_url(recovery.env_resolver) }),
    ));

    match wait_for_mempool_readiness(recovery.env_resolver) {
        Ok(()) => Ok(None),
        Err(err) => Ok(Some(err)),
    }
}

fn retry_mempool_runtime_after_failed_readiness(
    recovery: &mut MempoolBringUpRecoveryCtx<'_>,
    previous_error: anyhow::Error,
) -> Result<()> {
    let Some(runtime_service_name) =
        bring_up_runtime_recovery_service(recovery.config, recovery.service_name)
    else {
        return Err(previous_error);
    };

    let runtime_service = recovery
        .config
        .service_by_name(&runtime_service_name)
        .ok_or_else(|| anyhow::anyhow!("service `{runtime_service_name}` not found in config"))?;
    let runtime_state = compute_runtime_state(
        &runtime_service_name,
        runtime_service,
        recovery.env_resolver,
    )?;
    let Some(runtime_action) = bring_up_runtime_retry_action_after_failed_readiness(runtime_state)
    else {
        return Err(previous_error);
    };

    recovery.events.push(output_event(
        SeverityLevel::Warning,
        "bring_up.runtime_recovery",
        &format!(
            "mempool readiness still failing; retrying by {} `{}`",
            action_label(runtime_action),
            runtime_service_name
        ),
        json!({
            "service": recovery.service_name,
            "runtime_service": runtime_service_name,
            "runtime_state": runtime_state_label(runtime_state),
            "health_url": mempool_health_url(recovery.env_resolver),
            "previous_error": previous_error.to_string(),
            "retry_action": action_label(runtime_action),
        }),
    ));
    execute_service_action(recovery, &runtime_service_name, runtime_action)?;
    recovery.events.push(output_event(
        SeverityLevel::Info,
        "bring_up.service_reapply",
        "reapplying mempool after runtime recovery",
        json!({
            "service": recovery.service_name,
            "runtime_service": runtime_service_name,
        }),
    ));
    execute_service_action(recovery, recovery.service_name, ServiceAction::Start)?;
    recovery.events.push(output_event(
        SeverityLevel::Info,
        "bring_up.waiting_readiness",
        "waiting for mempool HTTP readiness",
        json!({ "health_url": mempool_health_url(recovery.env_resolver) }),
    ));
    wait_for_mempool_readiness(recovery.env_resolver)
}

fn execute_service_action(
    recovery: &mut MempoolBringUpRecoveryCtx<'_>,
    service_name: &str,
    action: ServiceAction,
) -> Result<()> {
    let action_plan = ServiceCommandRequest {
        config: recovery.config,
        service_name,
        action,
    }
    .plan(recovery.env_resolver)?;
    recovery
        .execution_report
        .extend(recovery.executor.execute(&action_plan)?);
    recovery
        .accumulated_plan
        .operations
        .extend(action_plan.operations);
    Ok(())
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
) -> Result<BringUpPlan> {
    let mut operations = Vec::new();
    let mut events = Vec::new();

    for service_name in ordered_services {
        if dry_run {
            events.push(output_event(
                SeverityLevel::Info,
                "bring_up.plan_start",
                &format!("would start `{service_name}`"),
                json!({ "service": service_name }),
            ));
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

            if let Some(action) = bring_up_action(service_name, state) {
                events.push(output_event(
                    SeverityLevel::Info,
                    "bring_up.applying",
                    &format!("{} `{service_name}`", action_label(action)),
                    json!({
                        "service": service_name,
                        "state": runtime_state_label(state),
                        "action": action_label(action),
                    }),
                ));
                let req = ServiceCommandRequest {
                    config,
                    service_name,
                    action,
                };
                let mut plan = req.plan(env_resolver)?;
                operations.append(&mut plan.operations);
            } else {
                events.push(output_event(
                    SeverityLevel::Info,
                    "bring_up.skipped",
                    &format!("`{service_name}` already healthy; skipping"),
                    json!({ "service": service_name, "state": runtime_state_label(state) }),
                ));
            }
        }
    }

    Ok(BringUpPlan {
        plan: Plan { operations },
        events,
    })
}

struct BringUpPlan {
    plan: Plan,
    events: Vec<OutputEvent>,
}

struct MempoolBringUpRecoveryCtx<'a> {
    config: &'a BelterConfig,
    env_resolver: &'a dyn EnvResolver,
    executor: &'a mut infractl_adapters::executor::RealExecutor,
    accumulated_plan: &'a mut Plan,
    execution_report: &'a mut Vec<ExecutionReport>,
    events: &'a mut Vec<OutputEvent>,
    service_name: &'a str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RuntimeState {
    Running,
    Stopped,
    Syncing,
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
                    Ok(match probe_mempool_status(env_resolver) {
                        MempoolProbeStatus::Ready => RuntimeState::Running,
                        MempoolProbeStatus::Syncing(_) => RuntimeState::Syncing,
                        MempoolProbeStatus::Degraded(_) => RuntimeState::Degraded,
                    })
                }
                Ok(_) => Ok(RuntimeState::Running),
                Err(_) => Ok(RuntimeState::Unknown),
            }
        }
        other => bail!("service `{service_name}` uses unsupported manager `{other}`"),
    }
}

fn bring_up_action(service_name: &str, state: RuntimeState) -> Option<ServiceAction> {
    match state {
        RuntimeState::Stopped | RuntimeState::Unknown => Some(ServiceAction::Start),
        RuntimeState::Degraded if service_name == "mempool" => Some(ServiceAction::Restart),
        RuntimeState::Running | RuntimeState::Syncing | RuntimeState::Degraded => None,
    }
}

fn bring_up_retry_action_after_failed_readiness(
    service_name: &str,
    initial_state: RuntimeState,
) -> Option<ServiceAction> {
    match (service_name, initial_state) {
        ("mempool", RuntimeState::Unknown) => Some(ServiceAction::Restart),
        _ => None,
    }
}

fn bring_up_runtime_recovery_service(config: &BelterConfig, service_name: &str) -> Option<String> {
    config
        .service_by_name(service_name)?
        .depends_on
        .as_deref()?
        .iter()
        .find(|dependency| {
            config
                .service_by_name(dependency)
                .map(|service| service.manager == PODMAN_MACHINE_MANAGER)
                .unwrap_or(false)
        })
        .cloned()
}

fn bring_up_runtime_retry_action_after_failed_readiness(
    runtime_state: RuntimeState,
) -> Option<ServiceAction> {
    match runtime_state {
        RuntimeState::Degraded => Some(ServiceAction::Restart),
        RuntimeState::Running
        | RuntimeState::Stopped
        | RuntimeState::Syncing
        | RuntimeState::Unknown => None,
    }
}

fn runtime_state_label(state: RuntimeState) -> &'static str {
    match state {
        RuntimeState::Running => "running",
        RuntimeState::Stopped => "stopped",
        RuntimeState::Syncing => "syncing",
        RuntimeState::Degraded => "degraded",
        RuntimeState::Unknown => "unknown",
    }
}

fn wait_for_mempool_readiness(env_resolver: &dyn EnvResolver) -> Result<()> {
    let url = mempool_health_url(env_resolver);
    let mut delay = Duration::from_secs(1);

    for attempt in 1..=5 {
        match probe_mempool_status(env_resolver) {
            MempoolProbeStatus::Ready | MempoolProbeStatus::Syncing(_) => return Ok(()),
            MempoolProbeStatus::Degraded(err) if attempt == 5 => {
                bail!("mempool did not become ready at {url}: {err}");
            }
            MempoolProbeStatus::Degraded(_) => {
                thread::sleep(delay);
                delay = std::cmp::min(delay * 2, Duration::from_secs(8));
            }
        }
    }

    bail!("mempool did not become ready at {url}")
}

fn output_event(
    level: SeverityLevel,
    code: &str,
    message: &str,
    details: serde_json::Value,
) -> OutputEvent {
    let clock = SystemClock;
    OutputEvent {
        ts: clock.now_utc_rfc3339(),
        level,
        code: code.to_string(),
        message: message.to_string(),
        details,
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
                for event in &out.events {
                    let level = match event.level {
                        SeverityLevel::Debug => "debug",
                        SeverityLevel::Info => "info",
                        SeverityLevel::Warning => "warning",
                        SeverityLevel::Error => "error",
                        SeverityLevel::Fatal => "fatal",
                    };
                    writeln!(stdout, "  [{level}] {}: {}", event.code, event.message)?;
                }
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

#[cfg(test)]
mod tests {
    use super::{
        DependentComposeIssue, HttpStatus, MempoolProbeStatus, RuntimeState, bring_up_action,
        bring_up_retry_action_after_failed_readiness, bring_up_runtime_recovery_service,
        bring_up_runtime_retry_action_after_failed_readiness, classify_mempool_fees_status,
        derive_podman_runtime_state, mempool_health_url, mempool_probe_urls, runtime_state_label,
        validate_http_status_line,
    };
    use infractl_core::config::{BelterConfig, ServiceConfig};
    use infractl_core::env::FixedEnvResolver;
    use infractl_core::usecase::ServiceAction;
    use std::collections::HashMap;

    fn mempool_with_runtime_config() -> BelterConfig {
        let mut services = HashMap::new();
        services.insert(
            "podman_runtime".to_string(),
            ServiceConfig {
                manager: "podman_machine".to_string(),
                unit: None,
                compose_file: None,
                compose_override: None,
                project: None,
                env_file: None,
                machine: Some("${PODMAN_MACHINE_NAME}".to_string()),
                depends_on: None,
            },
        );
        services.insert(
            "mempool".to_string(),
            ServiceConfig {
                manager: "podman_compose".to_string(),
                unit: None,
                compose_file: Some("${MEMPOOL_COMPOSE_FILE}".to_string()),
                compose_override: Some("${MEMPOOL_COMPOSE_OVERRIDE}".to_string()),
                project: Some("${MEMPOOL_PROJECT}".to_string()),
                env_file: Some("${MEMPOOL_ENV_FILE}".to_string()),
                machine: None,
                depends_on: Some(vec!["podman_runtime".to_string()]),
            },
        );
        BelterConfig {
            service: Some(services),
        }
    }

    #[test]
    fn bring_up_action_restarts_degraded_mempool() {
        assert!(matches!(
            bring_up_action("mempool", RuntimeState::Degraded),
            Some(ServiceAction::Restart)
        ));
    }

    #[test]
    fn bring_up_action_starts_stopped_services() {
        assert!(matches!(
            bring_up_action("mempool", RuntimeState::Stopped),
            Some(ServiceAction::Start)
        ));
        assert!(matches!(
            bring_up_action("bitcoind", RuntimeState::Unknown),
            Some(ServiceAction::Start)
        ));
    }

    #[test]
    fn bring_up_action_skips_syncing_mempool() {
        assert!(bring_up_action("mempool", RuntimeState::Syncing).is_none());
        assert_eq!(runtime_state_label(RuntimeState::Syncing), "syncing");
    }

    #[test]
    fn bring_up_retry_action_restarts_unknown_mempool_after_failed_readiness() {
        assert!(matches!(
            bring_up_retry_action_after_failed_readiness("mempool", RuntimeState::Unknown),
            Some(ServiceAction::Restart)
        ));
        assert!(
            bring_up_retry_action_after_failed_readiness("mempool", RuntimeState::Stopped)
                .is_none()
        );
        assert!(
            bring_up_retry_action_after_failed_readiness("bitcoind", RuntimeState::Unknown)
                .is_none()
        );
    }

    #[test]
    fn bring_up_runtime_recovery_service_finds_podman_machine_dependency() {
        let config = mempool_with_runtime_config();
        assert_eq!(
            bring_up_runtime_recovery_service(&config, "mempool").as_deref(),
            Some("podman_runtime")
        );
        assert!(bring_up_runtime_recovery_service(&config, "podman_runtime").is_none());
    }

    #[test]
    fn bring_up_runtime_retry_action_restarts_only_degraded_runtime() {
        assert!(matches!(
            bring_up_runtime_retry_action_after_failed_readiness(RuntimeState::Degraded),
            Some(ServiceAction::Restart)
        ));
        assert!(
            bring_up_runtime_retry_action_after_failed_readiness(RuntimeState::Running).is_none()
        );
    }

    #[test]
    fn derive_podman_runtime_state_degrades_when_dependent_compose_service_is_degraded() {
        let (state, query_error) = derive_podman_runtime_state(vec![DependentComposeIssue {
            service: "mempool".to_string(),
            state: RuntimeState::Degraded,
            running_containers: vec!["abc".to_string(), "def".to_string()],
            query_error: Some(
                "unexpected HTTP status from 127.0.0.1:8080: HTTP/1.1 502 Bad Gateway".to_string(),
            ),
        }]);

        assert_eq!(state, "degraded");
        let query_error = query_error.expect("query_error should be present");
        assert!(query_error.contains("dependent service `mempool`"));
        assert!(query_error.contains("podman port forwarding may be unhealthy"));
        assert!(query_error.contains("HTTP/1.1 502 Bad Gateway"));
    }

    #[test]
    fn validate_http_status_line_reports_non_200_status() {
        let err = validate_http_status_line("127.0.0.1:8080", "HTTP/1.1 502 Bad Gateway\r\n")
            .expect_err("validation should fail for non-200 response");
        assert!(
            err.to_string()
                .contains("unexpected HTTP status from 127.0.0.1"),
            "unexpected error: {err:#}"
        );
        assert!(
            err.to_string().contains("HTTP/1.1 502 Bad Gateway"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn validate_http_status_line_accepts_200_status() {
        validate_http_status_line("127.0.0.1:8080", "HTTP/1.1 200 OK\r\n")
            .expect("validation should succeed for 200 response");
    }

    #[test]
    fn classify_mempool_fees_503_as_syncing() {
        let status = HttpStatus {
            authority: "127.0.0.1:8080".to_string(),
            status_line: "HTTP/1.1 503 Service Unavailable".to_string(),
            status_code: 503,
        };

        match classify_mempool_fees_status("http://127.0.0.1:8080/api/v1/fees/recommended", &status)
        {
            Some(MempoolProbeStatus::Syncing(detail)) => {
                assert!(detail.contains("mempool backend is syncing"));
                assert!(detail.contains("503 Service Unavailable"));
            }
            _ => panic!("503 fees response should be classified as syncing"),
        }
    }

    #[test]
    fn classify_mempool_fees_502_as_degraded() {
        let status = HttpStatus {
            authority: "127.0.0.1:8080".to_string(),
            status_line: "HTTP/1.1 502 Bad Gateway".to_string(),
            status_code: 502,
        };

        assert!(matches!(
            classify_mempool_fees_status("http://127.0.0.1:8080/api/v1/fees/recommended", &status,),
            Some(MempoolProbeStatus::Degraded(_))
        ));
    }

    #[test]
    fn mempool_probe_urls_cover_frontend_data_dependencies() {
        let mut values = HashMap::new();
        values.insert("MEMPOOL_HOST".to_string(), "mempool.local".to_string());
        values.insert("MEMPOOL_PORT".to_string(), "8081".to_string());
        let resolver = FixedEnvResolver::new(values);

        assert_eq!(
            mempool_probe_urls(&resolver),
            vec![
                "http://mempool.local:8081/api/v1/backend-info",
                "http://mempool.local:8081/api/v1/fees/recommended",
                "http://mempool.local:8081/api/mempool",
                "http://mempool.local:8081/api/blocks/tip/height",
            ]
        );
    }

    #[test]
    fn mempool_health_url_remains_primary_backend_info_probe() {
        let resolver = FixedEnvResolver::new(HashMap::new());

        assert_eq!(
            mempool_health_url(&resolver),
            "http://127.0.0.1:8080/api/v1/backend-info"
        );
    }
}
