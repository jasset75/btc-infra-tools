use anyhow::Result;
use clap::{Args, Parser, Subcommand, ValueEnum};
use infractl_core::output::OutputEnvelope;
use infractl_core::time::now_utc_rfc3339;

#[derive(Debug, Parser)]
#[command(name = "belter", version, about = "Infrastructure control CLI/TUI")]
struct Cli {
    #[command(subcommand)]
    command: Command,

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
    Init,
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
    #[arg(long, value_enum, default_value = "auto")]
    ui: UiMode,
    #[arg(long, help = "Alias for --ui tui")]
    tui: bool,
}

impl UiArgs {
    fn effective(self) -> UiMode {
        if self.tui { UiMode::Tui } else { self.ui }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Config { command } => match command {
            ConfigCommand::Init => emit(cli.json, "config.init", "initialized config template"),
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
                &format!("restart requested for {name}"),
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
