use anyhow::Result;
use infractl_core::output::{OutputEnvelope, OutputEvent};
use infractl_core::time::Clock;
use serde::Serialize;
use serde_json::Value;
use std::io::Write;

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

pub(crate) fn emit<W: Write>(
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

pub(crate) fn emit_dry_run_report<W: Write>(stdout: &mut W, out: &OutputEnvelope) -> Result<()> {
    writeln!(stdout, "[DRY-RUN] Report:")?;
    writeln!(
        stdout,
        "{}",
        serde_json::to_string_pretty(&dry_run_text_report(out))?
    )?;
    Ok(())
}

pub(crate) fn output_envelope(
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

pub(crate) fn error_envelope(
    clock: &dyn Clock,
    command: &str,
    message: &str,
    dry_run: bool,
) -> OutputEnvelope {
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
