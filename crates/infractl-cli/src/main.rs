use anyhow::Result;
use clap::Parser;
use infractl_core::config::DEFAULT_CONFIG_FILE;
use infractl_core::env::{EnvResolver, ProcessEnvResolver};
use infractl_core::time::{Clock, SystemClock};
use infractl_core::usecase::ServiceAction;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

mod cli;
mod commands;
mod output;
mod runtime;

use crate::cli::{Cli, Command, ConfigCommand, HealthCommand, RunCommand, ServiceCommand, TuiCommand};
use crate::commands::config::init_config_file;
use crate::commands::service::{
    StatusEmitCtx, emit_plan, emit_status, execute_service_command_from_config,
};
use crate::output::{emit, error_envelope};
#[cfg(test)]
use crate::output::output_envelope;
use crate::runtime::{DotenvLoader, ProcessDotenvLoader, RuntimeDeps};
#[cfg(test)]
use crate::runtime::NoopDotenvLoader;

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
                    cli.command.label(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use infractl_core::env::FixedEnvResolver;
    use infractl_core::time::FixedClock;
    use serde_json::Value;
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
                    cli.command.label(),
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
        assert!(rendered.contains("\"message\":"));
        assert!(rendered.contains("missing `unit`"));

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
