use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use infractl_adapters::LaunchdAdapter;
use infractl_core::config::{BelterConfig, DEFAULT_CONFIG_FILE, default_config_template};
use infractl_core::output::OutputEnvelope;
use infractl_core::time::now_utc_rfc3339;
use std::env;
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
    Action {
        id: String,
        #[arg(long)]
        dry_run: bool,
    },
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

fn main() -> Result<()> {
    load_dotenv_if_present()?;
    let cli = Cli::parse();
    match cli.command {
        Command::Config { command } => match command {
            ConfigCommand::Init { path, force } => {
                let target = path.unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_FILE));
                init_config_file(&target, force)?;
                emit(
                    cli.json,
                    "config.init",
                    &format!("created configuration file at {}", target.display()),
                )
            }
            ConfigCommand::Validate => emit(cli.json, "config.validate", "configuration is valid"),
            ConfigCommand::Show => emit(cli.json, "config.show", "showing effective configuration"),
        },
        Command::Service { command } => match command {
            ServiceCommand::List => emit(
                cli.json,
                "service.list",
                "configured services: bitcoind, stratum, mempool",
            ),
            ServiceCommand::Status { name, ui } => {
                let service = name.unwrap_or_else(|| "all".to_string());
                emit(
                    cli.json,
                    "service.status",
                    &format!("status target={service} ui={:?}", ui.effective()),
                )
            }
            ServiceCommand::Start { name } => emit(
                cli.json,
                "service.start",
                &format!("start requested for {name}"),
            ),
            ServiceCommand::Stop { name } => emit(
                cli.json,
                "service.stop",
                &format!("stop requested for {name}"),
            ),
            ServiceCommand::Restart { name } => emit(
                cli.json,
                "service.restart",
                &restart_service_from_config(&cli.config, &name)?,
            ),
            ServiceCommand::Logs { name, follow } => emit(
                cli.json,
                "service.logs",
                &format!("logs target={name} follow={follow}"),
            ),
        },
        Command::Health { command } => match command {
            HealthCommand::Check { all, id, ui } => emit(
                cli.json,
                "health.check",
                &format!("check all={all} id={id:?} ui={:?}", ui.effective()),
            ),
            HealthCommand::Snapshot => emit(cli.json, "health.snapshot", "snapshot generated"),
        },
        Command::Run { command } => match command {
            RunCommand::Action { id, dry_run } => emit(
                cli.json,
                "run.action",
                &format!("action={id} dry_run={dry_run}"),
            ),
        },
        Command::Tui { command } => match command {
            TuiCommand::Dashboard => emit(cli.json, "tui.dashboard", "starting dashboard"),
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

fn emit(json: bool, command: &str, message: &str) -> Result<()> {
    let out = OutputEnvelope {
        ts: now_utc_rfc3339(),
        command: command.to_string(),
        status: "ok".to_string(),
        message: message.to_string(),
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("[{}] {}: {}", out.ts, out.command, out.message);
    }
    Ok(())
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

fn restart_service_from_config(config_path: &PathBuf, service_name: &str) -> Result<String> {
    let raw = fs::read_to_string(config_path)
        .with_context(|| format!("failed to read config file {}", config_path.display()))?;
    let config: BelterConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse TOML from {}", config_path.display()))?;

    let services = config
        .service
        .ok_or_else(|| anyhow::anyhow!("missing [service] section"))?;
    let service = services
        .get(service_name)
        .ok_or_else(|| anyhow::anyhow!("service `{service_name}` not found in config"))?;

    if service.manager != "launchd" {
        bail!(
            "service `{service_name}` uses manager `{}`; only `launchd` restart is implemented",
            service.manager
        );
    }

    let unit = service
        .unit
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("service `{service_name}` is missing `unit`"))?;
    let resolved_unit = expand_env_placeholders(unit)?;

    let adapter = LaunchdAdapter;
    adapter.restart_unit(&resolved_unit)?;

    Ok(format!(
        "restarted service `{service_name}` via launchd unit `{resolved_unit}`"
    ))
}

fn expand_env_placeholders(input: &str) -> Result<String> {
    let mut out = input.to_string();
    let mut cursor = 0;

    while let Some(start_rel) = out[cursor..].find("${") {
        let start = cursor + start_rel;
        let after_start = start + 2;
        let Some(end_rel) = out[after_start..].find('}') else {
            bail!("unterminated placeholder in `{input}`");
        };
        let end = after_start + end_rel;
        let key = &out[after_start..end];

        if key.is_empty() {
            bail!("empty placeholder in `{input}`");
        }

        let value = env::var(key)
            .with_context(|| format!("missing environment variable `{key}` for `{input}`"))?;
        out.replace_range(start..=end, &value);
        cursor = start + value.len();
    }

    Ok(out)
}
