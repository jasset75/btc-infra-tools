use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use infractl_core::config::{BelterConfig, DEFAULT_CONFIG_FILE, default_config_template};
use infractl_core::env::{EnvResolver, ProcessEnvResolver};
use infractl_core::output::{OutputEnvelope, OutputEvent, SeverityLevel};
use infractl_core::plan::{Operation, Plan};
use infractl_core::time::{Clock, SystemClock};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "belter", version, about = "Infrastructure control CLI/TUI")]
struct Cli {
    #[command(subcommand)]
    command: Command,

    #[arg(
        long,
        global = true,
        default_value = DEFAULT_CONFIG_FILE,
        help = "Path to belter config file"
    )]
    config: PathBuf,

    #[arg(long, global = true, help = "Emit machine-readable JSON output")]
    json: bool,

    #[arg(
        long,
        global = true,
        help = "Simulate command without making actual changes"
    )]
    dry_run: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
    Health {
        #[command(subcommand)]
        command: HealthCommand,
    },
    Run {
        #[command(subcommand)]
        command: RunCommand,
    },
    Tui {
        #[command(subcommand)]
        command: TuiCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    Init {
        #[arg(long, help = "Write config file to a custom path")]
        path: Option<PathBuf>,
        #[arg(long, help = "Overwrite target file if it already exists")]
        force: bool,
    },
    Validate,
    Show,
}

#[derive(Debug, Subcommand)]
enum ServiceCommand {
    List,
    Status {
        name: Option<String>,
        #[command(flatten)]
        ui: UiArgs,
    },
    Start {
        name: String,
    },
    Stop {
        name: String,
    },
    Restart {
        name: String,
    },
    Logs {
        name: String,
        #[arg(long)]
        follow: bool,
    },
}

#[derive(Debug, Subcommand)]
enum HealthCommand {
    Check {
        #[arg(long, conflicts_with = "id")]
        all: bool,
        #[arg(long)]
        id: Option<String>,
        #[command(flatten)]
        ui: UiArgs,
    },
    Snapshot,
}

#[derive(Debug, Subcommand)]
enum RunCommand {
    Action { id: String },
}

#[derive(Debug, Subcommand)]
enum TuiCommand {
    Dashboard,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum UiMode {
    Auto,
    Cli,
    Tui,
}

#[derive(Debug, Args)]
struct UiArgs {
    #[arg(long, value_enum)]
    ui: Option<UiMode>,
    #[arg(long, conflicts_with = "ui", help = "Shortcut for --ui tui")]
    tui: bool,
}

impl UiArgs {
    fn effective(self) -> UiMode {
        if self.tui {
            UiMode::Tui
        } else {
            self.ui.unwrap_or(UiMode::Auto)
        }
    }
}

struct RuntimeDeps<C, E> {
    clock: C,
    env_resolver: E,
}

fn main() -> Result<()> {
    load_dotenv_if_present()?;
    let cli = Cli::parse();
    let deps = RuntimeDeps {
        clock: SystemClock,
        env_resolver: ProcessEnvResolver,
    };
    match cli.command {
        Command::Config { command } => match command {
            ConfigCommand::Init { path, force } => {
                let target = path.unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_FILE));
                init_config_file(&target, force)?;
                emit(
                    &deps.clock,
                    cli.json,
                    "config.init",
                    &format!("created configuration file at {}", target.display()),
                )
            }
            ConfigCommand::Validate => emit(
                &deps.clock,
                cli.json,
                "config.validate",
                "configuration is valid",
            ),
            ConfigCommand::Show => emit(
                &deps.clock,
                cli.json,
                "config.show",
                "showing effective configuration",
            ),
        },
        Command::Service { command } => match command {
            ServiceCommand::List => emit(
                &deps.clock,
                cli.json,
                "service.list",
                "configured services: bitcoind, stratum, mempool",
            ),
            ServiceCommand::Status { name, ui } => {
                let service = name.unwrap_or_else(|| "all".to_string());
                emit(
                    &deps.clock,
                    cli.json,
                    "service.status",
                    &format!("status target={service} ui={:?}", ui.effective()),
                )
            }
            ServiceCommand::Start { name } => emit(
                &deps.clock,
                cli.json,
                "service.start",
                &format!("start requested for {name}"),
            ),
            ServiceCommand::Stop { name } => emit(
                &deps.clock,
                cli.json,
                "service.stop",
                &format!("stop requested for {name}"),
            ),
            ServiceCommand::Restart { name } => emit_plan(
                &deps.clock,
                cli.json,
                cli.dry_run,
                "service.restart",
                restart_service_from_config(
                    &deps.clock,
                    &deps.env_resolver,
                    &cli.config,
                    &name,
                    cli.dry_run,
                ),
            ),
            ServiceCommand::Logs { name, follow } => emit(
                &deps.clock,
                cli.json,
                "service.logs",
                &format!("logs target={name} follow={follow}"),
            ),
        },
        Command::Health { command } => match command {
            HealthCommand::Check { all, id, ui } => emit(
                &deps.clock,
                cli.json,
                "health.check",
                &format!("check all={all} id={id:?} ui={:?}", ui.effective()),
            ),
            HealthCommand::Snapshot => emit(
                &deps.clock,
                cli.json,
                "health.snapshot",
                "snapshot generated",
            ),
        },
        Command::Run { command } => match command {
            RunCommand::Action { id } => {
                emit(&deps.clock, cli.json, "run.action", &format!("action={id}"))
            }
        },
        Command::Tui { command } => match command {
            TuiCommand::Dashboard => {
                emit(&deps.clock, cli.json, "tui.dashboard", "starting dashboard")
            }
        },
    }
}

fn load_dotenv_if_present() -> Result<()> {
    let path = PathBuf::from(".env");
    if !path.exists() {
        return Ok(());
    }

    dotenvy::from_filename(&path)
        .with_context(|| format!("failed to load environment from {}", path.display()))?;
    Ok(())
}

fn emit(clock: &dyn Clock, json: bool, command: &str, message: &str) -> Result<()> {
    let out = output_envelope(clock, command, message, false, Value::Null, Vec::new());

    if json {
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("[{}] {}: {}", out.ts, out.command, out.message);
    }
    Ok(())
}

fn output_envelope(
    clock: &dyn Clock,
    command: &str,
    message: &str,
    dry_run: bool,
    data: Value,
    events: Vec<OutputEvent>,
) -> OutputEnvelope {
    OutputEnvelope {
        ts: clock.now_utc_rfc3339(),
        command: command.to_string(),
        status: "ok".to_string(),
        message: message.to_string(),
        dry_run,
        data,
        events,
    }
}

fn emit_plan(
    clock: &dyn Clock,
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
                &plan_result.message,
                dry_run,
                json!({ "plan": plan_result.plan }),
                plan_result.events,
            );
            if json {
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                println!("[{}] {}: {}", out.ts, out.command, out.message);
                if dry_run {
                    for event in &out.events {
                        println!("[DRY-RUN] {}", event.message);
                    }
                    println!("[DRY-RUN] Plan payload structure:");
                    println!("{}", serde_json::to_string_pretty(&out.data)?);
                }
            }
            Ok(())
        }
        Err(e) => {
            bail!(e);
        }
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

fn restart_service_from_config(
    clock: &dyn Clock,
    env_resolver: &dyn EnvResolver,
    config_path: &PathBuf,
    service_name: &str,
    dry_run: bool,
) -> Result<PlanExecutionResult> {
    let raw = fs::read_to_string(config_path)
        .with_context(|| format!("failed to read config file {}", config_path.display()))?;
    let config: BelterConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse TOML from {}", config_path.display()))?;

    let req = infractl_core::usecase::RestartServiceRequest {
        config: &config,
        service_name,
    };

    let plan = req.plan(env_resolver)?;

    use infractl_core::plan::Executor;

    if dry_run {
        let mut executor = infractl_adapters::executor::DryRunExecutor::sink();
        executor.execute(&plan)?;
        let events = dry_run_events(clock, "service.restart", &plan);
        Ok(PlanExecutionResult {
            plan,
            message: format!("would restart service `{service_name}`"),
            events,
        })
    } else {
        let mut executor = infractl_adapters::executor::RealExecutor::new();
        executor.execute(&plan)?;
        Ok(PlanExecutionResult {
            plan,
            message: format!("restarted service `{service_name}`"),
            events: Vec::new(),
        })
    }
}

struct PlanExecutionResult {
    plan: Plan,
    message: String,
    events: Vec<OutputEvent>,
}

fn dry_run_events(clock: &dyn Clock, command: &str, plan: &Plan) -> Vec<OutputEvent> {
    plan.operations
        .iter()
        .enumerate()
        .map(|(index, operation)| dry_run_event(clock, command, index, operation))
        .collect()
}

fn dry_run_event(
    clock: &dyn Clock,
    command: &str,
    index: usize,
    operation: &Operation,
) -> OutputEvent {
    match operation {
        Operation::RestartService { manager, unit } => OutputEvent {
            ts: clock.now_utc_rfc3339(),
            level: SeverityLevel::Info,
            code: format!("{command}.preview"),
            message: format!("{}. Would restart `{}` unit `{}`", index + 1, manager, unit),
            details: json!({
                "operation_index": index + 1,
                "manager": manager,
                "unit": unit,
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use infractl_core::time::FixedClock;

    #[test]
    fn output_envelope_uses_injected_fixed_clock() {
        let clock = FixedClock::new("2026-03-12T10:00:00Z");
        let out = output_envelope(&clock, "service.list", "ok", false, Value::Null, Vec::new());
        assert_eq!(out.ts, "2026-03-12T10:00:00Z");
        assert_eq!(out.command, "service.list");
        assert_eq!(out.message, "ok");
        assert_eq!(out.status, "ok");
        assert!(!out.dry_run);
        assert_eq!(out.data, Value::Null);
        assert!(out.events.is_empty());
    }

    #[test]
    fn dry_run_event_uses_stable_shape() {
        let clock = FixedClock::new("2026-03-12T10:00:00Z");
        let event = dry_run_event(
            &clock,
            "service.restart",
            0,
            &Operation::RestartService {
                manager: "launchd".to_string(),
                unit: "system/com.bitcoind.node".to_string(),
            },
        );
        assert_eq!(event.ts, "2026-03-12T10:00:00Z");
        assert_eq!(event.level, SeverityLevel::Info);
        assert_eq!(event.code, "service.restart.preview");
        assert_eq!(
            event.message,
            "1. Would restart `launchd` unit `system/com.bitcoind.node`"
        );
        assert_eq!(
            event.details,
            json!({
                "operation_index": 1,
                "manager": "launchd",
                "unit": "system/com.bitcoind.node",
            })
        );
    }
}
