use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use infractl_core::time::{Clock, SystemClock};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_INTERVAL_SECONDS: u64 = 600;
const DEFAULT_CONFIRM_AFTER_SECONDS: u64 = 30;
const DEFAULT_COOLDOWN_SECONDS: u64 = 600;
const DEFAULT_TIMEOUT_SECONDS: u64 = 120;

#[derive(Parser)]
#[command(name = "belter-watchdog")]
#[command(version)]
#[command(about = "Small command-based watchdog for belter-managed services")]
struct Cli {
    #[command(subcommand)]
    command: WatchdogCommand,
}

#[derive(Subcommand)]
enum WatchdogCommand {
    Run {
        #[arg(short, long, default_value = "watchdog.toml")]
        config: PathBuf,
        #[arg(long)]
        once: bool,
    },
    Init {
        #[arg(short, long, default_value = "watchdog.toml")]
        path: PathBuf,
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Deserialize)]
struct WatchdogConfig {
    version: u32,
    #[serde(default)]
    logging: LoggingConfig,
    #[serde(default)]
    defaults: WatchDefaults,
    #[serde(default)]
    watch: Vec<WatchConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct LoggingConfig {
    stdout_path: Option<String>,
    stderr_path: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct WatchDefaults {
    interval_seconds: Option<u64>,
    confirm_after_seconds: Option<u64>,
    cooldown_seconds: Option<u64>,
    timeout_seconds: Option<u64>,
    shell: Option<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize)]
struct WatchConfig {
    name: String,
    diagnose: String,
    recovery: String,
    #[serde(default = "default_enabled")]
    enabled: bool,
    interval_seconds: Option<u64>,
    confirm_after_seconds: Option<u64>,
    cooldown_seconds: Option<u64>,
    timeout_seconds: Option<u64>,
    healthy_json_path: Option<String>,
    healthy_equals: Option<String>,
    healthy_exit_code: Option<i32>,
    shell: Option<Vec<String>>,
}

#[derive(Debug)]
struct EffectiveWatch<'a> {
    config: &'a WatchConfig,
    interval: Duration,
    confirm_after: Duration,
    cooldown: Duration,
    timeout: Duration,
    shell: Vec<String>,
}

#[derive(Debug)]
struct CommandOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HealthState {
    Healthy,
    Unhealthy,
}

#[derive(Debug, Default)]
struct WatchState {
    next_check: Option<Instant>,
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();

    match run(&cli, &SystemClock, &mut stdout) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(stderr, "Error: {error:#}");
            std::process::ExitCode::from(1)
        }
    }
}

fn run<W: Write>(cli: &Cli, clock: &dyn Clock, stdout: &mut W) -> Result<()> {
    match &cli.command {
        WatchdogCommand::Run { config, once } => {
            let config = load_config(config)?;
            validate_config(&config)?;
            let mut log_writer = open_log_writer(&config.logging, stdout)?;
            run_watchdog(&config, *once, clock, &mut log_writer)
        }
        WatchdogCommand::Init { path, force } => init_config(path, *force, stdout),
    }
}

fn load_config(path: &Path) -> Result<WatchdogConfig> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read watchdog config {}", path.display()))?;
    toml::from_str(&raw)
        .with_context(|| format!("failed to parse watchdog TOML from {}", path.display()))
}

fn validate_config(config: &WatchdogConfig) -> Result<()> {
    if config.version != 1 {
        bail!(
            "unsupported watchdog config version {}; expected 1",
            config.version
        );
    }
    if config.watch.is_empty() {
        bail!("watchdog config must define at least one [[watch]] entry");
    }
    if let Some(path) = &config.logging.stdout_path
        && path.trim().is_empty()
    {
        bail!("logging.stdout_path cannot be empty when set");
    }
    if let Some(path) = &config.logging.stderr_path
        && path.trim().is_empty()
    {
        bail!("logging.stderr_path cannot be empty when set");
    }

    for watch in &config.watch {
        if watch.name.trim().is_empty() {
            bail!("watch entry has empty name");
        }
        if watch.diagnose.trim().is_empty() {
            bail!("watch `{}` has empty diagnose command", watch.name);
        }
        if watch.recovery.trim().is_empty() {
            bail!("watch `{}` has empty recovery command", watch.name);
        }
        if watch.healthy_json_path.is_none() && watch.healthy_exit_code.is_none() {
            bail!(
                "watch `{}` must set healthy_json_path or healthy_exit_code",
                watch.name
            );
        }
        if watch.healthy_json_path.is_some() && watch.healthy_equals.is_none() {
            bail!(
                "watch `{}` must set healthy_equals when healthy_json_path is used",
                watch.name
            );
        }
    }

    Ok(())
}

fn open_log_writer<'a, W: Write>(
    logging: &LoggingConfig,
    stdout: &'a mut W,
) -> Result<Box<dyn Write + 'a>> {
    match (
        logging.stdout_path.as_deref(),
        logging.stderr_path.as_deref(),
    ) {
        (None, None) => Ok(Box::new(stdout)),
        (Some(path), None) | (None, Some(path)) => Ok(Box::new(open_append_log(path)?)),
        (Some(stdout_path), Some(stderr_path)) if stdout_path == stderr_path => {
            Ok(Box::new(open_append_log(stdout_path)?))
        }
        (Some(stdout_path), Some(stderr_path)) => Ok(Box::new(MultiWriter {
            stdout: open_append_log(stdout_path)?,
            stderr: open_append_log(stderr_path)?,
        })),
    }
}

fn open_append_log(path: &str) -> Result<fs::File> {
    let expanded = expand_home(path)?;
    if let Some(parent) = expanded.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create log directory {}", parent.display()))?;
    }

    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&expanded)
        .with_context(|| format!("failed to open log file {}", expanded.display()))
}

fn expand_home(path: &str) -> Result<PathBuf> {
    if path == "$HOME" || path == "~" {
        return home_dir();
    }
    if let Some(rest) = path.strip_prefix("$HOME/") {
        return Ok(home_dir()?.join(rest));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return Ok(home_dir()?.join(rest));
    }
    Ok(PathBuf::from(path))
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("HOME is not set; cannot expand watchdog log path"))
}

struct MultiWriter {
    stdout: fs::File,
    stderr: fs::File,
}

impl Write for MultiWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stdout.write_all(buf)?;
        self.stderr.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stdout.flush()?;
        self.stderr.flush()
    }
}

fn run_watchdog<W: Write>(
    config: &WatchdogConfig,
    once: bool,
    clock: &dyn Clock,
    stdout: &mut W,
) -> Result<()> {
    let enabled: Vec<_> = config.watch.iter().filter(|watch| watch.enabled).collect();
    if enabled.is_empty() {
        bail!("watchdog config has no enabled watches");
    }

    let mut states: HashMap<String, WatchState> = HashMap::new();

    loop {
        let now = Instant::now();
        let mut next_due = now + Duration::from_secs(DEFAULT_INTERVAL_SECONDS);

        for watch in &enabled {
            let effective = effective_watch(config, watch)?;
            let state = states.entry(watch.name.clone()).or_default();
            if state.next_check.is_some_and(|due| due > now) {
                next_due = next_due.min(state.next_check.expect("checked above"));
                continue;
            }

            let next_delay = match run_watch_cycle(&effective, clock, stdout) {
                Ok(next_delay) => next_delay,
                Err(err) if once => return Err(err),
                Err(err) => {
                    log_line(
                        clock,
                        stdout,
                        &watch.name,
                        "watch.error",
                        &format!("error={}", sanitize_log_detail(&err.to_string())),
                    )?;
                    effective.interval.max(effective.cooldown)
                }
            };
            state.next_check = Some(Instant::now() + next_delay);
            next_due = next_due.min(state.next_check.expect("set above"));
        }

        if once {
            return Ok(());
        }

        let sleep_for = next_due.saturating_duration_since(Instant::now());
        thread::sleep(sleep_for.min(Duration::from_secs(1)));
    }
}

fn run_watch_cycle<W: Write>(
    watch: &EffectiveWatch<'_>,
    clock: &dyn Clock,
    stdout: &mut W,
) -> Result<Duration> {
    log_line(clock, stdout, &watch.config.name, "check.start", "")?;
    let initial = run_diagnose(watch)?;
    log_line(
        clock,
        stdout,
        &watch.config.name,
        "check.result",
        health_label(initial),
    )?;

    if initial == HealthState::Healthy {
        return Ok(watch.interval);
    }

    if !watch.confirm_after.is_zero() {
        log_line(
            clock,
            stdout,
            &watch.config.name,
            "check.confirm_wait",
            &format!("seconds={}", watch.confirm_after.as_secs()),
        )?;
        thread::sleep(watch.confirm_after);

        let confirmed = run_diagnose(watch)?;
        log_line(
            clock,
            stdout,
            &watch.config.name,
            "check.confirm_result",
            health_label(confirmed),
        )?;

        if confirmed == HealthState::Healthy {
            return Ok(watch.interval);
        }
    }

    log_line(clock, stdout, &watch.config.name, "recovery.start", "")?;
    let recovery = run_shell_command(&watch.shell, &watch.config.recovery, watch.timeout)
        .with_context(|| format!("failed to run recovery for `{}`", watch.config.name))?;
    log_line(
        clock,
        stdout,
        &watch.config.name,
        "recovery.done",
        &format!(
            "exit_code={} timed_out={}",
            display_code(recovery.code),
            recovery.timed_out
        ),
    )?;

    if recovery.timed_out || recovery.code != Some(0) {
        bail!(
            "recovery for `{}` failed: exit_code={} timed_out={} stderr={}",
            watch.config.name,
            display_code(recovery.code),
            recovery.timed_out,
            recovery.stderr.trim()
        );
    }

    let post = run_diagnose(watch)?;
    log_line(
        clock,
        stdout,
        &watch.config.name,
        "recovery.post_check",
        health_label(post),
    )?;
    if post != HealthState::Healthy {
        bail!(
            "watch `{}` is still unhealthy after recovery",
            watch.config.name
        );
    }

    if !watch.cooldown.is_zero() {
        log_line(
            clock,
            stdout,
            &watch.config.name,
            "cooldown",
            &format!("seconds={}", watch.cooldown.as_secs()),
        )?;
    }

    Ok(watch.interval.max(watch.cooldown))
}

fn run_diagnose(watch: &EffectiveWatch<'_>) -> Result<HealthState> {
    let output = run_shell_command(&watch.shell, &watch.config.diagnose, watch.timeout)
        .with_context(|| format!("failed to run diagnose for `{}`", watch.config.name))?;
    Ok(evaluate_health(watch.config, &output))
}

fn evaluate_health(watch: &WatchConfig, output: &CommandOutput) -> HealthState {
    if output.timed_out {
        return HealthState::Unhealthy;
    }

    if let Some(expected_code) = watch.healthy_exit_code
        && output.code != Some(expected_code)
    {
        return HealthState::Unhealthy;
    }

    if let Some(path) = &watch.healthy_json_path {
        let Ok(parsed) = serde_json::from_str::<Value>(&output.stdout) else {
            return HealthState::Unhealthy;
        };
        let Some(actual) = json_path_value(&parsed, path) else {
            return HealthState::Unhealthy;
        };
        let expected = watch.healthy_equals.as_deref().unwrap_or_default();

        if !json_value_equals(actual, expected) {
            return HealthState::Unhealthy;
        }
    }

    HealthState::Healthy
}

fn run_shell_command(shell: &[String], command: &str, timeout: Duration) -> Result<CommandOutput> {
    if shell.is_empty() {
        bail!("shell command prefix cannot be empty");
    }

    let mut child = Command::new(&shell[0])
        .args(&shell[1..])
        .arg(command)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn command `{command}`"))?;

    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait()?.is_some() {
            let output = child.wait_with_output()?;
            return Ok(CommandOutput {
                code: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                timed_out: false,
            });
        }

        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output()?;
            return Ok(CommandOutput {
                code: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                timed_out: true,
            });
        }

        thread::sleep(Duration::from_millis(100));
    }
}

fn effective_watch<'a>(
    config: &'a WatchdogConfig,
    watch: &'a WatchConfig,
) -> Result<EffectiveWatch<'a>> {
    let shell = watch
        .shell
        .clone()
        .or_else(|| config.defaults.shell.clone())
        .unwrap_or_else(default_shell);
    if shell.is_empty() {
        return Err(anyhow!("watch `{}` has empty shell", watch.name));
    }

    Ok(EffectiveWatch {
        config: watch,
        interval: Duration::from_secs(
            watch
                .interval_seconds
                .or(config.defaults.interval_seconds)
                .unwrap_or(DEFAULT_INTERVAL_SECONDS),
        ),
        confirm_after: Duration::from_secs(
            watch
                .confirm_after_seconds
                .or(config.defaults.confirm_after_seconds)
                .unwrap_or(DEFAULT_CONFIRM_AFTER_SECONDS),
        ),
        cooldown: Duration::from_secs(
            watch
                .cooldown_seconds
                .or(config.defaults.cooldown_seconds)
                .unwrap_or(DEFAULT_COOLDOWN_SECONDS),
        ),
        timeout: Duration::from_secs(
            watch
                .timeout_seconds
                .or(config.defaults.timeout_seconds)
                .unwrap_or(DEFAULT_TIMEOUT_SECONDS),
        ),
        shell,
    })
}

fn json_path_value<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let trimmed = path.trim().trim_start_matches('.');
    if trimmed.is_empty() {
        return Some(value);
    }

    let mut current = value;
    for segment in trimmed.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

fn json_value_equals(value: &Value, expected: &str) -> bool {
    match value {
        Value::String(actual) => actual == expected,
        Value::Bool(actual) => expected.parse::<bool>() == Ok(*actual),
        Value::Number(actual) => actual.to_string() == expected,
        Value::Null => expected == "null",
        Value::Array(_) | Value::Object(_) => {
            serde_json::from_str::<Value>(expected).is_ok_and(|expected| expected == *value)
        }
    }
}

fn init_config<W: Write>(path: &Path, force: bool, stdout: &mut W) -> Result<()> {
    if path.exists() && !force {
        bail!(
            "{} already exists; pass --force to overwrite",
            path.display()
        );
    }

    fs::write(path, default_config_template())
        .with_context(|| format!("failed to write {}", path.display()))?;
    writeln!(stdout, "created watchdog config at {}", path.display())?;
    Ok(())
}

fn default_config_template() -> &'static str {
    r#"version = 1

[logging]
stdout_path = "$HOME/.local/state/belter-watchdog/logs/watchdog.out.log"
stderr_path = "$HOME/.local/state/belter-watchdog/logs/watchdog.err.log"

[defaults]
interval_seconds = 600
confirm_after_seconds = 30
cooldown_seconds = 600
timeout_seconds = 120
shell = ["zsh", "-lc"]

[[watch]]
name = "mempool"
diagnose = "belter --json service status mempool"
recovery = "belter service bring-up mempool"
healthy_json_path = ".data.state"
healthy_equals = "running"
healthy_exit_code = 0
"#
}

fn default_shell() -> Vec<String> {
    vec!["zsh".to_string(), "-lc".to_string()]
}

fn default_enabled() -> bool {
    true
}

fn health_label(state: HealthState) -> &'static str {
    match state {
        HealthState::Healthy => "state=healthy",
        HealthState::Unhealthy => "state=unhealthy",
    }
}

fn display_code(code: Option<i32>) -> String {
    code.map_or_else(|| "signal".to_string(), |code| code.to_string())
}

fn sanitize_log_detail(value: &str) -> String {
    value.replace(['\r', '\n'], " ")
}

fn log_line<W: Write>(
    clock: &dyn Clock,
    stdout: &mut W,
    watch: &str,
    event: &str,
    detail: &str,
) -> Result<()> {
    if detail.is_empty() {
        writeln!(
            stdout,
            "[{}] {event}: watch={watch}",
            clock.now_utc_rfc3339()
        )?;
    } else {
        writeln!(
            stdout,
            "[{}] {event}: watch={watch} {detail}",
            clock.now_utc_rfc3339()
        )?;
    }
    stdout.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use infractl_core::time::FixedClock;

    fn sample_watch() -> WatchConfig {
        WatchConfig {
            name: "mempool".to_string(),
            diagnose: "diagnose".to_string(),
            recovery: "recovery".to_string(),
            enabled: true,
            interval_seconds: None,
            confirm_after_seconds: None,
            cooldown_seconds: None,
            timeout_seconds: None,
            healthy_json_path: Some(".data.state".to_string()),
            healthy_equals: Some("running".to_string()),
            healthy_exit_code: Some(0),
            shell: None,
        }
    }

    #[test]
    fn json_path_resolves_simple_dot_path() {
        let value: Value = serde_json::json!({
            "data": {
                "state": "running"
            }
        });

        assert_eq!(
            json_path_value(&value, ".data.state").and_then(Value::as_str),
            Some("running")
        );
        assert!(json_path_value(&value, ".data.missing").is_none());
    }

    #[test]
    fn evaluate_health_accepts_expected_json_and_exit_code() {
        let watch = sample_watch();
        let output = CommandOutput {
            code: Some(0),
            stdout: r#"{"data":{"state":"running"}}"#.to_string(),
            stderr: String::new(),
            timed_out: false,
        };

        assert_eq!(evaluate_health(&watch, &output), HealthState::Healthy);
    }

    #[test]
    fn evaluate_health_rejects_degraded_json() {
        let watch = sample_watch();
        let output = CommandOutput {
            code: Some(0),
            stdout: r#"{"data":{"state":"degraded"}}"#.to_string(),
            stderr: String::new(),
            timed_out: false,
        };

        assert_eq!(evaluate_health(&watch, &output), HealthState::Unhealthy);
    }

    #[test]
    fn effective_watch_uses_defaults() {
        let config = WatchdogConfig {
            version: 1,
            logging: LoggingConfig::default(),
            defaults: WatchDefaults {
                interval_seconds: Some(600),
                confirm_after_seconds: Some(30),
                cooldown_seconds: Some(900),
                timeout_seconds: Some(120),
                shell: Some(vec!["/bin/sh".to_string(), "-c".to_string()]),
            },
            watch: vec![sample_watch()],
        };

        let effective = effective_watch(&config, &config.watch[0]).expect("effective watch");
        assert_eq!(effective.interval, Duration::from_secs(600));
        assert_eq!(effective.confirm_after, Duration::from_secs(30));
        assert_eq!(effective.cooldown, Duration::from_secs(900));
        assert_eq!(effective.timeout, Duration::from_secs(120));
        assert_eq!(effective.shell, vec!["/bin/sh", "-c"]);
    }

    #[test]
    fn log_line_writes_stable_event_shape() {
        let clock = FixedClock::new("2026-05-08T15:00:00Z");
        let mut out = Vec::new();

        log_line(&clock, &mut out, "mempool", "check.result", "state=healthy").expect("log line");

        let rendered = String::from_utf8(out).expect("utf8");
        assert_eq!(
            rendered,
            "[2026-05-08T15:00:00Z] check.result: watch=mempool state=healthy\n"
        );
    }

    #[test]
    fn expand_home_rewrites_home_prefix() {
        let home = std::env::var("HOME").expect("HOME should be set in tests");
        let expanded = expand_home("$HOME/.local/state/example.log").expect("expanded path");

        assert_eq!(
            expanded,
            PathBuf::from(home).join(".local/state/example.log")
        );
    }
}
