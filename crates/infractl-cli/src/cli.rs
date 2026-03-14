use clap::{Args, Parser, Subcommand, ValueEnum};
use infractl_core::config::DEFAULT_CONFIG_FILE;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "belter", version, about = "Infrastructure control CLI/TUI")]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,

    #[arg(
        long,
        global = true,
        default_value = DEFAULT_CONFIG_FILE,
        help = "Path to belter config file"
    )]
    pub(crate) config: PathBuf,

    #[arg(long, global = true, help = "Emit machine-readable JSON output")]
    pub(crate) json: bool,

    #[arg(
        long,
        global = true,
        help = "Simulate command without making actual changes"
    )]
    pub(crate) dry_run: bool,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
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
pub(crate) enum ConfigCommand {
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
pub(crate) enum ServiceCommand {
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
pub(crate) enum HealthCommand {
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
pub(crate) enum RunCommand {
    Action { id: String },
}

#[derive(Debug, Subcommand)]
pub(crate) enum TuiCommand {
    Dashboard,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum UiMode {
    Auto,
    Cli,
    Tui,
}

#[derive(Debug, Clone, Copy, Args)]
pub(crate) struct UiArgs {
    #[arg(long, value_enum)]
    ui: Option<UiMode>,
    #[arg(long, conflicts_with = "ui", help = "Shortcut for --ui tui")]
    tui: bool,
}

impl UiArgs {
    pub(crate) fn effective(self) -> UiMode {
        if self.tui {
            UiMode::Tui
        } else {
            self.ui.unwrap_or(UiMode::Auto)
        }
    }
}
