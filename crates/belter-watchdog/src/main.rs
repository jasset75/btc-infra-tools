use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use infractl_core::time::{Clock, SystemClock};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const DEFAULT_INTERVAL_SECONDS: u64 = 600;
const DEFAULT_CONFIRM_AFTER_SECONDS: u64 = 30;
const DEFAULT_COOLDOWN_SECONDS: u64 = 600;
const DEFAULT_TIMEOUT_SECONDS: u64 = 120;
const DEFAULT_STDOUT_LOG_PATH: &str = "$HOME/.local/state/belter-watchdog/logs/watchdog.out.log";

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
    Stats {
        #[arg(long, default_value = DEFAULT_STDOUT_LOG_PATH)]
        log: String,
        #[arg(long)]
        watch: Option<String>,
        #[arg(long)]
        json: bool,
    },
    ClearLog {
        #[arg(long, default_value = DEFAULT_STDOUT_LOG_PATH)]
        log: String,
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
            let mut logger = open_logger(&config.logging, stdout)?;
            run_watchdog(&config, *once, clock, &mut logger)
        }
        WatchdogCommand::Init { path, force } => init_config(path, *force, stdout),
        WatchdogCommand::Stats { log, watch, json } => {
            let stats = build_stats_report(log, watch.as_deref())?;
            if *json {
                serde_json::to_writer_pretty(&mut *stdout, &stats)?;
                writeln!(stdout)?;
            } else {
                write_stats_report(stdout, &stats)?;
            }
            Ok(())
        }
        WatchdogCommand::ClearLog { log } => clear_log(log, stdout),
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

fn open_logger<'a, W: Write>(logging: &LoggingConfig, stdout: &'a mut W) -> Result<Logger<'a>> {
    let stdout: Box<dyn Write + 'a> = match logging.stdout_path.as_deref() {
        Some(path) => Box::new(open_append_log(path)?),
        None => Box::new(stdout),
    };
    let stderr: Option<Box<dyn Write + 'a>> = match logging.stderr_path.as_deref() {
        Some(path) if Some(path) != logging.stdout_path.as_deref() => {
            Some(Box::new(open_append_log(path)?))
        }
        _ => None,
    };

    Ok(Logger { stdout, stderr })
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

struct Logger<'a> {
    stdout: Box<dyn Write + 'a>,
    stderr: Option<Box<dyn Write + 'a>>,
}

impl Logger<'_> {
    fn info(&mut self, message: &str) -> Result<()> {
        writeln!(self.stdout, "{message}")?;
        self.stdout.flush()?;
        Ok(())
    }

    fn error(&mut self, message: &str) -> Result<()> {
        match &mut self.stderr {
            Some(stderr) => {
                writeln!(stderr, "{message}")?;
                stderr.flush()?;
            }
            None => {
                writeln!(self.stdout, "{message}")?;
                self.stdout.flush()?;
            }
        }
        Ok(())
    }
}

fn run_watchdog(
    config: &WatchdogConfig,
    once: bool,
    clock: &dyn Clock,
    logger: &mut Logger<'_>,
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

            let next_delay = match run_watch_cycle(&effective, clock, logger) {
                Ok(next_delay) => next_delay,
                Err(err) if once => return Err(err),
                Err(err) => {
                    log_error(
                        clock,
                        logger,
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

fn run_watch_cycle(
    watch: &EffectiveWatch<'_>,
    clock: &dyn Clock,
    logger: &mut Logger<'_>,
) -> Result<Duration> {
    log_line(clock, logger, &watch.config.name, "check.start", "")?;
    let initial = run_diagnose(watch)?;
    log_line(
        clock,
        logger,
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
            logger,
            &watch.config.name,
            "check.confirm_wait",
            &format!("seconds={}", watch.confirm_after.as_secs()),
        )?;
        thread::sleep(watch.confirm_after);

        let confirmed = run_diagnose(watch)?;
        log_line(
            clock,
            logger,
            &watch.config.name,
            "check.confirm_result",
            health_label(confirmed),
        )?;

        if confirmed == HealthState::Healthy {
            return Ok(watch.interval);
        }
    }

    log_line(clock, logger, &watch.config.name, "recovery.start", "")?;
    let recovery = run_shell_command(&watch.shell, &watch.config.recovery, watch.timeout)
        .with_context(|| format!("failed to run recovery for `{}`", watch.config.name))?;
    log_line(
        clock,
        logger,
        &watch.config.name,
        "recovery.done",
        &format!(
            "exit_code={} timed_out={}",
            display_code(recovery.code),
            recovery.timed_out
        ),
    )?;

    let recovery_failed = is_recovery_failure(&recovery);
    if recovery_failed {
        log_recovery_command_failure(clock, logger, watch, &recovery)?;
    }

    let post = run_diagnose(watch)?;
    log_line(
        clock,
        logger,
        &watch.config.name,
        "recovery.post_check",
        health_label(post),
    )?;
    if post != HealthState::Healthy {
        if recovery_failed {
            bail!(
                "recovery for `{}` failed and post-check is still unhealthy: exit_code={} timed_out={} stderr={}",
                watch.config.name,
                display_code(recovery.code),
                recovery.timed_out,
                recovery.stderr.trim()
            );
        }
        bail!(
            "watch `{}` is still unhealthy after recovery",
            watch.config.name
        );
    }

    if !watch.cooldown.is_zero() {
        log_line(
            clock,
            logger,
            &watch.config.name,
            "cooldown",
            &format!("seconds={}", watch.cooldown.as_secs()),
        )?;
    }

    Ok(watch.interval.max(watch.cooldown))
}

fn is_recovery_failure(output: &CommandOutput) -> bool {
    output.timed_out || output.code != Some(0)
}

fn log_recovery_command_failure(
    clock: &dyn Clock,
    logger: &mut Logger<'_>,
    watch: &EffectiveWatch<'_>,
    recovery: &CommandOutput,
) -> Result<()> {
    log_line(
        clock,
        logger,
        &watch.config.name,
        "recovery.command_failure",
        &format!(
            "exit_code={} timed_out={} stderr={}",
            display_code(recovery.code),
            recovery.timed_out,
            sanitize_log_detail(recovery.stderr.trim())
        ),
    )
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

#[derive(Debug)]
struct LogEvent {
    timestamp: OffsetDateTime,
    event: String,
    watch: String,
    fields: HashMap<String, String>,
}

#[derive(Debug)]
struct OutageIncident {
    outage_at: OffsetDateTime,
    recovery_started_at: Option<OffsetDateTime>,
    recovery_done_at: Option<OffsetDateTime>,
    recovery_exit_code: Option<String>,
    recovery_timed_out: Option<bool>,
    recovered_at: Option<OffsetDateTime>,
}

#[derive(Debug, Default)]
struct WatchStatsBuilder {
    incidents: Vec<OutageIncident>,
    open_incident: Option<OutageIncident>,
    last_known_state: Option<String>,
}

#[derive(Debug, Serialize)]
struct StatsReport {
    log_path: String,
    log_started_at: Option<String>,
    log_ended_at: Option<String>,
    watch_filter: Option<String>,
    total: StatsTotals,
    watches: Vec<WatchStatsReport>,
}

#[derive(Debug, Default, Serialize)]
struct StatsTotals {
    confirmed_outages: usize,
    recovered: usize,
    unrecovered: usize,
    recovery_attempts: usize,
    recovery_command_failures: usize,
}

#[derive(Debug, Serialize)]
struct WatchStatsReport {
    watch: String,
    confirmed_outages: usize,
    recovered: usize,
    unrecovered: usize,
    recovery_attempts: usize,
    recovery_command_failures: usize,
    recovery_success_rate: Option<f64>,
    average_recovery_seconds: Option<i64>,
    min_recovery_seconds: Option<i64>,
    max_recovery_seconds: Option<i64>,
    average_seconds_between_outages: Option<i64>,
    min_seconds_between_outages: Option<i64>,
    max_seconds_between_outages: Option<i64>,
    last_outage: Option<String>,
    last_recovery: Option<String>,
    last_known_state: Option<String>,
}

fn build_stats_report(log_path: &str, watch_filter: Option<&str>) -> Result<StatsReport> {
    let expanded = expand_home(log_path)?;
    let file = fs::File::open(&expanded)
        .with_context(|| format!("failed to open watchdog log {}", expanded.display()))?;
    let reader = BufReader::new(file);
    let mut builders: BTreeMap<String, WatchStatsBuilder> = BTreeMap::new();
    let mut log_started_at: Option<OffsetDateTime> = None;
    let mut log_ended_at: Option<OffsetDateTime> = None;

    for line in reader.lines() {
        let line = line?;
        let Some(event) = parse_log_event(&line) else {
            continue;
        };
        log_started_at = Some(log_started_at.map_or(event.timestamp, |started_at| {
            started_at.min(event.timestamp)
        }));
        log_ended_at =
            Some(log_ended_at.map_or(event.timestamp, |ended_at| ended_at.max(event.timestamp)));
        if watch_filter.is_some_and(|watch| event.watch != watch) {
            continue;
        }
        builders
            .entry(event.watch.clone())
            .or_default()
            .apply(event);
    }

    let mut total = StatsTotals::default();
    let mut watches = Vec::new();
    for (watch, mut builder) in builders {
        let report = builder.finish(watch);
        total.confirmed_outages += report.confirmed_outages;
        total.recovered += report.recovered;
        total.unrecovered += report.unrecovered;
        total.recovery_attempts += report.recovery_attempts;
        total.recovery_command_failures += report.recovery_command_failures;
        watches.push(report);
    }

    Ok(StatsReport {
        log_path: expanded.display().to_string(),
        log_started_at: log_started_at.map(format_timestamp),
        log_ended_at: log_ended_at.map(format_timestamp),
        watch_filter: watch_filter.map(str::to_string),
        total,
        watches,
    })
}

impl WatchStatsBuilder {
    fn apply(&mut self, event: LogEvent) {
        if let Some(state) = event.fields.get("state")
            && matches!(
                event.event.as_str(),
                "check.result" | "check.confirm_result" | "recovery.post_check"
            )
        {
            self.last_known_state = Some(state.clone());
        }

        match event.event.as_str() {
            "check.confirm_result"
                if event.fields.get("state").is_some_and(|s| s == "unhealthy") =>
            {
                if let Some(open) = self.open_incident.take() {
                    self.incidents.push(open);
                }
                self.open_incident = Some(OutageIncident {
                    outage_at: event.timestamp,
                    recovery_started_at: None,
                    recovery_done_at: None,
                    recovery_exit_code: None,
                    recovery_timed_out: None,
                    recovered_at: None,
                });
            }
            "recovery.start" => {
                if let Some(incident) = &mut self.open_incident {
                    incident.recovery_started_at = Some(event.timestamp);
                }
            }
            "recovery.done" => {
                if let Some(incident) = &mut self.open_incident {
                    incident.recovery_done_at = Some(event.timestamp);
                    incident.recovery_exit_code = event.fields.get("exit_code").cloned();
                    incident.recovery_timed_out = event
                        .fields
                        .get("timed_out")
                        .and_then(|value| value.parse().ok());
                }
            }
            "recovery.post_check" if event.fields.get("state").is_some_and(|s| s == "healthy") => {
                if let Some(mut incident) = self.open_incident.take() {
                    incident.recovered_at = Some(event.timestamp);
                    self.incidents.push(incident);
                }
            }
            _ => {}
        }
    }

    fn finish(&mut self, watch: String) -> WatchStatsReport {
        if let Some(open) = self.open_incident.take() {
            self.incidents.push(open);
        }

        self.incidents.sort_by_key(|incident| incident.outage_at);
        let confirmed_outages = self.incidents.len();
        let recovered = self
            .incidents
            .iter()
            .filter(|incident| incident.recovered_at.is_some())
            .count();
        let unrecovered = confirmed_outages.saturating_sub(recovered);
        let recovery_attempts = self
            .incidents
            .iter()
            .filter(|incident| incident.recovery_started_at.is_some())
            .count();
        let recovery_command_failures = self
            .incidents
            .iter()
            .filter(|incident| {
                incident
                    .recovery_timed_out
                    .is_some_and(|timed_out| timed_out)
                    || incident
                        .recovery_exit_code
                        .as_deref()
                        .is_some_and(|code| code != "0")
            })
            .count();
        let recovery_seconds: Vec<i64> = self
            .incidents
            .iter()
            .filter_map(|incident| seconds_between(incident.outage_at, incident.recovered_at?))
            .collect();
        let outage_interval_seconds: Vec<i64> = self
            .incidents
            .windows(2)
            .filter_map(|window| seconds_between(window[0].outage_at, window[1].outage_at))
            .collect();

        WatchStatsReport {
            watch,
            confirmed_outages,
            recovered,
            unrecovered,
            recovery_attempts,
            recovery_command_failures,
            recovery_success_rate: if confirmed_outages == 0 {
                None
            } else {
                Some(recovered as f64 / confirmed_outages as f64)
            },
            average_recovery_seconds: average_seconds(&recovery_seconds),
            min_recovery_seconds: recovery_seconds.iter().min().copied(),
            max_recovery_seconds: recovery_seconds.iter().max().copied(),
            average_seconds_between_outages: average_seconds(&outage_interval_seconds),
            min_seconds_between_outages: outage_interval_seconds.iter().min().copied(),
            max_seconds_between_outages: outage_interval_seconds.iter().max().copied(),
            last_outage: self
                .incidents
                .last()
                .map(|incident| format_timestamp(incident.outage_at)),
            last_recovery: self
                .incidents
                .iter()
                .rev()
                .find_map(|incident| incident.recovered_at.map(format_timestamp)),
            last_known_state: self.last_known_state.clone(),
        }
    }
}

fn parse_log_event(line: &str) -> Option<LogEvent> {
    let rest = line.strip_prefix('[')?;
    let (timestamp, rest) = rest.split_once("] ")?;
    let timestamp = OffsetDateTime::parse(timestamp, &Rfc3339).ok()?;
    let (event, rest) = rest.split_once(": ")?;

    let mut fields = HashMap::new();
    for token in rest.split_whitespace() {
        let Some((key, value)) = token.split_once('=') else {
            continue;
        };
        fields.insert(key.to_string(), value.to_string());
    }

    Some(LogEvent {
        timestamp,
        event: event.to_string(),
        watch: fields.get("watch")?.clone(),
        fields,
    })
}

fn seconds_between(start: OffsetDateTime, end: OffsetDateTime) -> Option<i64> {
    let seconds = (end - start).whole_seconds();
    (seconds >= 0).then_some(seconds)
}

fn average_seconds(values: &[i64]) -> Option<i64> {
    if values.is_empty() {
        None
    } else {
        Some(values.iter().sum::<i64>() / values.len() as i64)
    }
}

fn format_timestamp(timestamp: OffsetDateTime) -> String {
    timestamp
        .format(&Rfc3339)
        .expect("RFC3339 formatting should not fail")
}

fn write_stats_report<W: Write>(stdout: &mut W, report: &StatsReport) -> Result<()> {
    writeln!(stdout, "Watchdog stats")?;
    writeln!(stdout)?;
    writeln!(stdout, "Log: {}", report.log_path)?;
    writeln!(
        stdout,
        "Log started at: {}",
        report.log_started_at.as_deref().unwrap_or("n/a")
    )?;
    writeln!(
        stdout,
        "Log ended at: {}",
        report.log_ended_at.as_deref().unwrap_or("n/a")
    )?;
    match &report.watch_filter {
        Some(watch) => writeln!(stdout, "Watch filter: {watch}")?,
        None => writeln!(stdout, "Watch filter: all")?,
    }
    writeln!(stdout)?;
    writeln!(stdout, "All watches:")?;
    writeln!(
        stdout,
        "  confirmed outages: {}",
        report.total.confirmed_outages
    )?;
    writeln!(stdout, "  recovered: {}", report.total.recovered)?;
    writeln!(stdout, "  unrecovered: {}", report.total.unrecovered)?;
    writeln!(
        stdout,
        "  recovery attempts: {}",
        report.total.recovery_attempts
    )?;
    writeln!(
        stdout,
        "  recovery command failures: {}",
        report.total.recovery_command_failures
    )?;

    if report.watches.is_empty() {
        writeln!(stdout)?;
        writeln!(stdout, "No matching watchdog events found.")?;
        return Ok(());
    }

    for watch in &report.watches {
        writeln!(stdout)?;
        writeln!(stdout, "{}:", watch.watch)?;
        writeln!(stdout, "  confirmed outages: {}", watch.confirmed_outages)?;
        writeln!(stdout, "  recovered: {}", watch.recovered)?;
        writeln!(stdout, "  unrecovered: {}", watch.unrecovered)?;
        writeln!(stdout, "  recovery attempts: {}", watch.recovery_attempts)?;
        writeln!(
            stdout,
            "  recovery command failures: {}",
            watch.recovery_command_failures
        )?;
        writeln!(
            stdout,
            "  recovery success rate: {}",
            format_rate(watch.recovery_success_rate)
        )?;
        writeln!(
            stdout,
            "  average recovery time: {}",
            format_optional_duration(watch.average_recovery_seconds)
        )?;
        writeln!(
            stdout,
            "  min recovery time: {}",
            format_optional_duration(watch.min_recovery_seconds)
        )?;
        writeln!(
            stdout,
            "  max recovery time: {}",
            format_optional_duration(watch.max_recovery_seconds)
        )?;
        writeln!(
            stdout,
            "  average time between outages: {}",
            format_optional_duration(watch.average_seconds_between_outages)
        )?;
        writeln!(
            stdout,
            "  min time between outages: {}",
            format_optional_duration(watch.min_seconds_between_outages)
        )?;
        writeln!(
            stdout,
            "  max time between outages: {}",
            format_optional_duration(watch.max_seconds_between_outages)
        )?;
        writeln!(
            stdout,
            "  last outage: {}",
            watch.last_outage.as_deref().unwrap_or("n/a")
        )?;
        writeln!(
            stdout,
            "  last recovery: {}",
            watch.last_recovery.as_deref().unwrap_or("n/a")
        )?;
        writeln!(
            stdout,
            "  last known state: {}",
            watch.last_known_state.as_deref().unwrap_or("n/a")
        )?;
    }

    Ok(())
}

fn format_rate(rate: Option<f64>) -> String {
    rate.map_or_else(|| "n/a".to_string(), |rate| format!("{:.0}%", rate * 100.0))
}

fn format_optional_duration(seconds: Option<i64>) -> String {
    seconds.map_or_else(|| "n/a".to_string(), format_duration)
}

fn format_duration(seconds: i64) -> String {
    let days = seconds / 86_400;
    let hours = seconds % 86_400 / 3_600;
    let minutes = seconds % 3_600 / 60;
    let seconds = seconds % 60;

    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

fn clear_log<W: Write>(log_path: &str, stdout: &mut W) -> Result<()> {
    let expanded = expand_home(log_path)?;
    if let Some(parent) = expanded.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create log directory {}", parent.display()))?;
    }

    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&expanded)
        .with_context(|| format!("failed to clear watchdog log {}", expanded.display()))?;
    writeln!(stdout, "cleared watchdog log at {}", expanded.display())?;
    Ok(())
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

fn format_log_line(clock: &dyn Clock, watch: &str, event: &str, detail: &str) -> String {
    if detail.is_empty() {
        format!("[{}] {event}: watch={watch}", clock.now_utc_rfc3339())
    } else {
        format!(
            "[{}] {event}: watch={watch} {detail}",
            clock.now_utc_rfc3339()
        )
    }
}

fn log_line(
    clock: &dyn Clock,
    logger: &mut Logger<'_>,
    watch: &str,
    event: &str,
    detail: &str,
) -> Result<()> {
    logger.info(&format_log_line(clock, watch, event, detail))
}

fn log_error(
    clock: &dyn Clock,
    logger: &mut Logger<'_>,
    watch: &str,
    event: &str,
    detail: &str,
) -> Result<()> {
    logger.error(&format_log_line(clock, watch, event, detail))
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

        assert_eq!(
            format_log_line(&clock, "mempool", "check.result", "state=healthy"),
            "[2026-05-08T15:00:00Z] check.result: watch=mempool state=healthy"
        );
    }

    #[test]
    fn watch_cycle_closes_outage_when_failed_recovery_leaves_service_healthy() {
        let state_path = unique_temp_path("watchdog-recovery-state");
        let _ = fs::remove_file(&state_path);
        let mut watch = sample_watch();
        watch.diagnose = format!(
            "if test -f '{}'; then printf '%s' '{{\"data\":{{\"state\":\"running\"}}}}'; else printf '%s' '{{\"data\":{{\"state\":\"degraded\"}}}}'; fi",
            state_path.display()
        );
        watch.recovery = format!(
            "touch '{}'; printf '%s\\n' 'simulated recovery command failure' >&2; exit 1",
            state_path.display()
        );
        let effective = EffectiveWatch {
            config: &watch,
            interval: Duration::from_secs(600),
            confirm_after: Duration::ZERO,
            cooldown: Duration::ZERO,
            timeout: Duration::from_secs(5),
            shell: default_shell(),
        };
        let clock = FixedClock::new("2026-05-08T15:00:00Z");
        let mut output = Vec::new();
        let mut logger = Logger {
            stdout: Box::new(&mut output),
            stderr: None,
        };

        let next_delay = run_watch_cycle(&effective, &clock, &mut logger)
            .expect("healthy post-check should close outage");
        drop(logger);
        let logs = String::from_utf8(output).expect("utf8 logs");

        assert_eq!(next_delay, Duration::from_secs(600));
        assert!(logs.contains("recovery.done: watch=mempool exit_code=1 timed_out=false"));
        assert!(logs.contains("recovery.command_failure: watch=mempool"));
        assert!(logs.contains("stderr=simulated recovery command failure"));
        assert!(logs.contains("recovery.post_check: watch=mempool state=healthy"));

        let _ = fs::remove_file(state_path);
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

    #[test]
    fn parse_log_event_reads_watch_event_and_fields() {
        let event = parse_log_event(
            "[2026-05-10T06:37:54.174671Z] check.confirm_result: watch=mempool state=unhealthy",
        )
        .expect("parsed event");

        assert_eq!(event.event, "check.confirm_result");
        assert_eq!(event.watch, "mempool");
        assert_eq!(
            event.fields.get("state").map(String::as_str),
            Some("unhealthy")
        );
    }

    #[test]
    fn stats_count_confirmed_outages_and_recoveries_by_watch() {
        let mut builder = WatchStatsBuilder::default();
        for line in [
            "[2026-05-10T06:37:20.925254Z] check.result: watch=mempool state=unhealthy",
            "[2026-05-10T06:37:54.174671Z] check.confirm_result: watch=mempool state=unhealthy",
            "[2026-05-10T06:37:54.175791Z] recovery.start: watch=mempool",
            "[2026-05-10T06:39:16.915443Z] recovery.done: watch=mempool exit_code=0 timed_out=false",
            "[2026-05-10T06:39:17.826971Z] recovery.post_check: watch=mempool state=healthy",
            "[2026-05-10T07:37:54.174671Z] check.confirm_result: watch=mempool state=unhealthy",
        ] {
            builder.apply(parse_log_event(line).expect("parsed event"));
        }

        let report = builder.finish("mempool".to_string());

        assert_eq!(report.confirmed_outages, 2);
        assert_eq!(report.recovered, 1);
        assert_eq!(report.unrecovered, 1);
        assert_eq!(report.recovery_attempts, 1);
        assert_eq!(report.recovery_command_failures, 0);
        assert_eq!(report.average_recovery_seconds, Some(83));
        assert_eq!(report.average_seconds_between_outages, Some(3600));
        assert_eq!(report.last_known_state.as_deref(), Some("unhealthy"));
    }

    #[test]
    fn stats_marks_failed_recovery_command() {
        let mut builder = WatchStatsBuilder::default();
        for line in [
            "[2026-05-10T06:37:54Z] check.confirm_result: watch=stratum state=unhealthy",
            "[2026-05-10T06:37:55Z] recovery.start: watch=stratum",
            "[2026-05-10T06:38:55Z] recovery.done: watch=stratum exit_code=1 timed_out=false",
        ] {
            builder.apply(parse_log_event(line).expect("parsed event"));
        }

        let report = builder.finish("stratum".to_string());

        assert_eq!(report.confirmed_outages, 1);
        assert_eq!(report.recovered, 0);
        assert_eq!(report.unrecovered, 1);
        assert_eq!(report.recovery_command_failures, 1);
    }

    #[test]
    fn stats_counts_failed_recovery_command_with_healthy_post_check_as_recovered() {
        let mut builder = WatchStatsBuilder::default();
        for line in [
            "[2026-05-10T06:37:54Z] check.confirm_result: watch=mempool state=unhealthy",
            "[2026-05-10T06:37:55Z] recovery.start: watch=mempool",
            "[2026-05-10T06:38:55Z] recovery.done: watch=mempool exit_code=1 timed_out=false",
            "[2026-05-10T06:38:56Z] recovery.command_failure: watch=mempool exit_code=1 timed_out=false stderr=simulated failure",
            "[2026-05-10T06:38:57Z] recovery.post_check: watch=mempool state=healthy",
        ] {
            builder.apply(parse_log_event(line).expect("parsed event"));
        }

        let report = builder.finish("mempool".to_string());

        assert_eq!(report.confirmed_outages, 1);
        assert_eq!(report.recovered, 1);
        assert_eq!(report.unrecovered, 0);
        assert_eq!(report.recovery_command_failures, 1);
        assert_eq!(report.last_known_state.as_deref(), Some("healthy"));
    }

    #[test]
    fn stats_report_includes_log_window() {
        let path = unique_temp_path("watchdog-stats-window.log");
        fs::write(
            &path,
            [
                "[2026-05-10T06:37:54Z] check.confirm_result: watch=mempool state=unhealthy",
                "[2026-05-10T06:39:17Z] recovery.post_check: watch=mempool state=healthy",
            ]
            .join("\n"),
        )
        .expect("write temp log");

        let report = build_stats_report(path.to_str().expect("utf8 path"), None)
            .expect("stats report should build");

        assert_eq!(
            report.log_started_at.as_deref(),
            Some("2026-05-10T06:37:54Z")
        );
        assert_eq!(report.log_ended_at.as_deref(), Some("2026-05-10T06:39:17Z"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn clear_log_truncates_existing_log() {
        let path = unique_temp_path("watchdog-clear.log");
        fs::write(&path, "existing log line\n").expect("write temp log");

        let mut output = Vec::new();
        clear_log(path.to_str().expect("utf8 path"), &mut output).expect("clear log");

        assert_eq!(fs::read_to_string(&path).expect("read temp log"), "");
        assert!(
            String::from_utf8(output)
                .expect("utf8 output")
                .contains("cleared watchdog log at")
        );

        let _ = fs::remove_file(path);
    }

    fn unique_temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{}-{}", std::process::id(), name))
    }
}
