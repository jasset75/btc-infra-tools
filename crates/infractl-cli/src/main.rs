use anyhow::{Context, Result, bail};
use clap::Parser;
use infractl_core::config::{BelterConfig, DEFAULT_CONFIG_FILE, default_config_template};
use infractl_core::env::{EnvResolver, ProcessEnvResolver, expand_placeholders};
use infractl_core::output::{OutputEnvelope, OutputEvent};
use infractl_core::plan::{ExecutionDetails, ExecutionReport, Plan};
use infractl_core::time::{Clock, SystemClock};
use infractl_core::usecase::{ServiceAction, ServiceCommandRequest};
use serde::Serialize;
use serde_json::{Value, json};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

mod cli;
mod runtime;

use crate::cli::{Cli, Command, ConfigCommand, HealthCommand, RunCommand, ServiceCommand, TuiCommand, UiMode};
use crate::runtime::{DotenvLoader, ProcessDotenvLoader, RuntimeDeps};
#[cfg(test)]
use crate::runtime::NoopDotenvLoader;

const LAUNCHD_MANAGER: &str = "launchd";

fn main() -> ExitCode {
    let cli = Cli::parse();
    let deps = RuntimeDeps {
        clock: SystemClock,
        env_resolver: ProcessEnvResolver,
        dotenv_loader: ProcessDotenvLoader,
    };
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();

    run_cli(&deps, &cli, &mut stdout, &mut stderr)
}

fn run_cli<C: Clock, E: EnvResolver, D: DotenvLoader, O: Write, Er: Write>(
    deps: &RuntimeDeps<C, E, D>,
    cli: &Cli,
    stdout: &mut O,
    stderr: &mut Er,
) -> ExitCode {
    match run(deps, cli, stdout) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if cli.json {
                let out = error_envelope(
                    &deps.clock,
                    command_label(&cli.command),
                    &error.to_string(),
                    cli.dry_run,
                );
                match serde_json::to_string_pretty(&out) {
                    Ok(serialized) => {
                        let _ = writeln!(stdout, "{serialized}");
                    }
                    Err(json_error) => {
                        let _ = writeln!(
                            stderr,
                            "Error: failed to serialize JSON error output: {json_error}"
                        );
                    }
                }
            } else {
                let _ = writeln!(stderr, "Error: {error}");
            }

            ExitCode::from(1)
        }
    }
}

fn run<C: Clock, E: EnvResolver, D: DotenvLoader, O: Write>(
    deps: &RuntimeDeps<C, E, D>,
    cli: &Cli,
    stdout: &mut O,
) -> Result<()> {
    deps.dotenv_loader.load_if_present()?;

    match &cli.command {
        Command::Config { command } => match command {
            ConfigCommand::Init { path, force } => {
                let target = path
                    .clone()
                    .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_FILE));
                init_config_file(&target, *force)?;
                emit(
                    &deps.clock,
                    stdout,
                    cli.json,
                    cli.dry_run,
                    "config.init",
                    &format!("created configuration file at {}", target.display()),
                )
            }
            ConfigCommand::Validate => emit(
                &deps.clock,
                stdout,
                cli.json,
                cli.dry_run,
                "config.validate",
                "configuration is valid",
            ),
            ConfigCommand::Show => emit(
                &deps.clock,
                stdout,
                cli.json,
                cli.dry_run,
                "config.show",
                "showing effective configuration",
            ),
        },
        Command::Service { command } => match command {
            ServiceCommand::List => emit(
                &deps.clock,
                stdout,
                cli.json,
                cli.dry_run,
                "service.list",
                "configured services: bitcoind, stratum, mempool",
            ),
            ServiceCommand::Status { name, ui } => {
                match name {
                    Some(service_name) => emit_status(StatusEmitCtx {
                        clock: &deps.clock,
                        stdout,
                        json: cli.json,
                        dry_run: cli.dry_run,
                        config_path: &cli.config,
                        env_resolver: &deps.env_resolver,
                        service_name,
                        ui_mode: ui.effective(),
                    }),
                    None => emit(
                        &deps.clock,
                        stdout,
                        cli.json,
                        cli.dry_run,
                        "service.status",
                        &format!("status target=all ui={:?}", ui.effective()),
                    ),
                }
            }
            ServiceCommand::Start { name } => emit_plan(
                &deps.clock,
                stdout,
                cli.json,
                cli.dry_run,
                "service.start",
                execute_service_command_from_config(
                    &deps.env_resolver,
                    &cli.config,
                    name,
                    ServiceAction::Start,
                    cli.dry_run,
                ),
            ),
            ServiceCommand::Stop { name } => emit_plan(
                &deps.clock,
                stdout,
                cli.json,
                cli.dry_run,
                "service.stop",
                execute_service_command_from_config(
                    &deps.env_resolver,
                    &cli.config,
                    name,
                    ServiceAction::Stop,
                    cli.dry_run,
                ),
            ),
            ServiceCommand::Restart { name } => emit_plan(
                &deps.clock,
                stdout,
                cli.json,
                cli.dry_run,
                "service.restart",
                execute_service_command_from_config(
                    &deps.env_resolver,
                    &cli.config,
                    name,
                    ServiceAction::Restart,
                    cli.dry_run,
                ),
            ),
            ServiceCommand::Logs { name, follow } => emit(
                &deps.clock,
                stdout,
                cli.json,
                cli.dry_run,
                "service.logs",
                &format!("logs target={name} follow={follow}"),
            ),
        },
        Command::Health { command } => match command {
            HealthCommand::Check { all, id, ui } => emit(
                &deps.clock,
                stdout,
                cli.json,
                cli.dry_run,
                "health.check",
                &format!("check all={all} id={id:?} ui={:?}", ui.effective()),
            ),
            HealthCommand::Snapshot => emit(
                &deps.clock,
                stdout,
                cli.json,
                cli.dry_run,
                "health.snapshot",
                "snapshot generated",
            ),
        },
        Command::Run { command } => match command {
            RunCommand::Action { id } => {
                emit(
                    &deps.clock,
                    stdout,
                    cli.json,
                    cli.dry_run,
                    "run.action",
                    &format!("action={id}"),
                )
            }
        },
        Command::Tui { command } => match command {
            TuiCommand::Dashboard => {
                emit(
                    &deps.clock,
                    stdout,
                    cli.json,
                    cli.dry_run,
                    "tui.dashboard",
                    "starting dashboard",
                )
            }
        },
    }
}

struct StatusEmitCtx<'a, W: Write> {
    clock: &'a dyn Clock,
    stdout: &'a mut W,
    json: bool,
    dry_run: bool,
    config_path: &'a PathBuf,
    env_resolver: &'a dyn EnvResolver,
    service_name: &'a str,
    ui_mode: UiMode,
}

#[derive(Serialize)]
struct DryRunTextReport<'a> {
    command: &'a str,
    status: &'a str,
    message: &'a str,
    dry_run: bool,
    data: &'a Value,
}

fn dry_run_text_report(out: &OutputEnvelope) -> DryRunTextReport<'_> {
    DryRunTextReport {
        command: &out.command,
        status: &out.status,
        message: &out.message,
        dry_run: out.dry_run,
        data: &out.data,
    }
}

fn emit_status<W: Write>(ctx: StatusEmitCtx<'_, W>) -> Result<()> {
    let raw = fs::read_to_string(ctx.config_path)
        .with_context(|| format!("failed to read config file {}", ctx.config_path.display()))?;
    let config: BelterConfig = toml::from_str(&raw).with_context(|| {
        format!(
            "failed to parse TOML from {}",
            ctx.config_path.display()
        )
    })?;

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
            writeln!(ctx.stdout, "[DRY-RUN] Report:")?;
            writeln!(
                ctx.stdout,
                "{}",
                serde_json::to_string_pretty(&dry_run_text_report(&out))?
            )?;
        }
        return Ok(());
    }

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
        let data = json!({
            "service": ctx.service_name,
            "manager": LAUNCHD_MANAGER,
            "unit": unit,
            "state": state,
            "pid": pid,
            "query_error": query_error,
        });
        let out = output_envelope(
            ctx.clock,
            "service.status",
            "ok",
            &message,
            false,
            data,
            Vec::new(),
        );
        if ctx.json {
            writeln!(ctx.stdout, "{}", serde_json::to_string_pretty(&out)?)?;
        } else {
            writeln!(ctx.stdout, "[{}] {}: {}", out.ts, out.command, out.message)?;
        }
        return Ok(());
    }

    emit(
        ctx.clock,
        ctx.stdout,
        ctx.json,
        false,
        "service.status",
        &format!(
            "status target={} ui={:?} manager={} (real status not implemented)",
            ctx.service_name, ctx.ui_mode, service.manager
        ),
    )
}

fn emit<W: Write>(
    clock: &dyn Clock,
    stdout: &mut W,
    json: bool,
    dry_run: bool,
    command: &str,
    message: &str,
) -> Result<()> {
    let out = output_envelope(clock, command, "ok", message, dry_run, Value::Null, Vec::new());

    if json {
        writeln!(stdout, "{}", serde_json::to_string_pretty(&out)?)?;
    } else {
        writeln!(stdout, "[{}] {}: {}", out.ts, out.command, out.message)?;
    }
    Ok(())
}

fn output_envelope(
    clock: &dyn Clock,
    command: &str,
    status: &str,
    message: &str,
    dry_run: bool,
    data: Value,
    events: Vec<OutputEvent>,
) -> OutputEnvelope {
    OutputEnvelope {
        ts: clock.now_utc_rfc3339(),
        command: command.to_string(),
        status: status.to_string(),
        message: message.to_string(),
        dry_run,
        data,
        events,
    }
}

fn error_envelope(clock: &dyn Clock, command: &str, message: &str, dry_run: bool) -> OutputEnvelope {
    output_envelope(
        clock,
        command,
        "error",
        message,
        dry_run,
        Value::Null,
        Vec::new(),
    )
}

fn emit_plan<W: Write>(
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
                    let report = json!({
                        "command": out.command,
                        "status": out.status,
                        "message": out.message,
                        "dry_run": out.dry_run,
                        "data": out.data,
                    });
                    writeln!(stdout, "[DRY-RUN] Report:")?;
                    writeln!(stdout, "{}", serde_json::to_string_pretty(&report)?)?;
                }
            }
            Ok(())
        }
        Err(e) => {
            bail!(e);
        }
    }
}

fn command_label(command: &Command) -> &'static str {
    match command {
        Command::Config { command } => match command {
            ConfigCommand::Init { .. } => "config.init",
            ConfigCommand::Validate => "config.validate",
            ConfigCommand::Show => "config.show",
        },
        Command::Service { command } => match command {
            ServiceCommand::List => "service.list",
            ServiceCommand::Status { .. } => "service.status",
            ServiceCommand::Start { .. } => "service.start",
            ServiceCommand::Stop { .. } => "service.stop",
            ServiceCommand::Restart { .. } => "service.restart",
            ServiceCommand::Logs { .. } => "service.logs",
        },
        Command::Health { command } => match command {
            HealthCommand::Check { .. } => "health.check",
            HealthCommand::Snapshot => "health.snapshot",
        },
        Command::Run { command } => match command {
            RunCommand::Action { .. } => "run.action",
        },
        Command::Tui { command } => match command {
            TuiCommand::Dashboard => "tui.dashboard",
        },
    }
}

fn init_config_file(path: &PathBuf, force: bool) -> Result<()> {
    if path.exists() && !force {
        bail!(
            "config file already exists at {} (use --force to overwrite)",
            path.display()
        );
    }

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }

    fs::write(path, default_config_template())
        .with_context(|| format!("failed to write config file {}", path.display()))?;
    Ok(())
}

fn execute_service_command_from_config(
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

struct PlanExecutionResult {
    plan: Plan,
    message: String,
    execution_report: Vec<ExecutionReport>,
    events: Vec<OutputEvent>,
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

    if let Some((pid_before, pid_after)) = execution_report.iter().map(|report| {
        match &report.details {
            ExecutionDetails::LaunchdRestartPidChange {
                pid_before,
                pid_after,
                ..
            } => (*pid_before, *pid_after),
        }
    }).next() {
        let restart_observed = matches!((pid_before, pid_after), (Some(before), Some(after)) if before != after);
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
    use super::*;
    use infractl_core::env::FixedEnvResolver;
    use infractl_core::time::FixedClock;
    use std::collections::HashMap;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn output_envelope_uses_injected_fixed_clock() {
        let clock = FixedClock::new("2026-03-12T10:00:00Z");
        let out = output_envelope(
            &clock,
            "service.list",
            "ok",
            "ok",
            false,
            Value::Null,
            Vec::new(),
        );
        assert_eq!(out.ts, "2026-03-12T10:00:00Z");
        assert_eq!(out.command, "service.list");
        assert_eq!(out.message, "ok");
        assert_eq!(out.status, "ok");
        assert!(!out.dry_run);
        assert_eq!(out.data, Value::Null);
        assert!(out.events.is_empty());
    }

    #[test]
    fn run_renders_dry_run_service_list() {
        let clock = FixedClock::new("2026-03-12T10:00:00Z");
        let deps = RuntimeDeps {
            clock,
            env_resolver: FixedEnvResolver::new(HashMap::new()),
            dotenv_loader: NoopDotenvLoader,
        };
        let cli = Cli::parse_from(["belter", "--dry-run", "service", "list"]);
        let mut stdout = Vec::new();

        run(&deps, &cli, &mut stdout).expect("run should succeed");

        let rendered = String::from_utf8(stdout).expect("stdout should be utf8");
        assert!(rendered.contains("[2026-03-12T10:00:00Z] service.list"));
        assert!(rendered.contains("configured services: bitcoind, stratum, mempool"));
    }

    #[test]
    fn run_renders_json_dry_run_plan() {
        let fixture_dir = unique_fixture_dir();
        fs::create_dir_all(&fixture_dir).expect("fixture dir should be created");

        let config_path = fixture_dir.join("belter.toml");
        fs::write(
            &config_path,
            r#"
[service.mempool]
manager = "podman_compose"
compose_file = "${MEMPOOL_COMPOSE_FILE}"
compose_override = "${MEMPOOL_COMPOSE_OVERRIDE}"
project = "${MEMPOOL_PROJECT}"
"#,
        )
        .expect("config should be written");

        let clock = FixedClock::new("2026-03-12T10:00:00Z");
        let deps = RuntimeDeps {
            clock,
            env_resolver: FixedEnvResolver::new(HashMap::from([
                (
                    "MEMPOOL_COMPOSE_FILE".to_string(),
                    "/tmp/base.yml".to_string(),
                ),
                (
                    "MEMPOOL_COMPOSE_OVERRIDE".to_string(),
                    "/tmp/override.yml".to_string(),
                ),
                ("MEMPOOL_PROJECT".to_string(), "docker".to_string()),
            ])),
            dotenv_loader: NoopDotenvLoader,
        };
        let cli = Cli::parse_from([
            "belter",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--dry-run",
            "--json",
            "service",
            "start",
            "mempool",
        ]);
        let mut stdout = Vec::new();

        run(&deps, &cli, &mut stdout).expect("run should succeed");

        let rendered = String::from_utf8(stdout).expect("stdout should be utf8");
        assert!(rendered.contains("\"command\": \"service.start\""));
        assert!(rendered.contains("\"dry_run\": true"));
        assert!(rendered.contains("\"events\": []"));
        assert!(rendered.contains("\"compose_file\": \"/tmp/base.yml\""));
        assert!(!rendered.contains("service.start.preview"));

        fs::remove_dir_all(&fixture_dir).expect("fixture dir should be removed");
    }

    #[test]
    fn run_cli_renders_json_errors_to_stdout() {
        let fixture_dir = unique_fixture_dir();
        fs::create_dir_all(&fixture_dir).expect("fixture dir should be created");

        let config_path = fixture_dir.join("belter.toml");
        fs::write(
            &config_path,
            r#"
[service.bitcoind]
manager = "launchd"
"#,
        )
        .expect("config should be written");

        let clock = FixedClock::new("2026-03-12T10:00:00Z");
        let deps = RuntimeDeps {
            clock,
            env_resolver: FixedEnvResolver::new(HashMap::new()),
            dotenv_loader: NoopDotenvLoader,
        };
        let cli = Cli::parse_from([
            "belter",
            "--config",
            config_path.to_str().expect("utf8 path"),
            "--json",
            "service",
            "restart",
            "bitcoind",
        ]);
        let mut stdout = Vec::new();
        let exit = match run(&deps, &cli, &mut stdout) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                let out = error_envelope(
                    &deps.clock,
                    command_label(&cli.command),
                    &error.to_string(),
                    cli.dry_run,
                );
                writeln!(&mut stdout, "{}", serde_json::to_string_pretty(&out).expect("serialize error envelope"))
                    .expect("stdout write should succeed");
                ExitCode::from(1)
            }
        };

        assert_eq!(exit, ExitCode::from(1));
        let rendered = String::from_utf8(stdout).expect("stdout should be utf8");
        assert!(rendered.contains("\"command\": \"service.restart\""));
        assert!(rendered.contains("\"status\": \"error\""));
        assert!(rendered.contains("service `bitcoind` is missing `unit`"));

        fs::remove_dir_all(&fixture_dir).expect("fixture dir should be removed");
    }

    fn unique_fixture_dir() -> PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be monotonic")
            .as_nanos();
        std::env::temp_dir().join(format!("belter-cli-test-{ts}"))
    }
}
