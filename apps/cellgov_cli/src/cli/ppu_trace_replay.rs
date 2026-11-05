//! `dev ppu-trace-replay`: replay the PPU instruction trace the patched
//! RPCS3 writes (`bridges/rpcs3-patch/0003-cellgov-ppu-trace.patch`) and
//! report each decoded kind and the class of each failing record.
//!
//! `cellgov_fuzz::ppu_trace` replays and classifies; this command renders
//! what it finds.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;

use cellgov_fuzz::ppu_trace::{
    console_vectors, differing_fields, replay_trace, PpuTraceClass, PpuTraceFailure,
    PpuTraceOptions, PpuTraceReport,
};
use cellgov_ppu::differential::rpcs3_capture::read_trace_bytes;
use cellgov_ppu::differential::ControlRegisters;

use super::exit::{CommandError, CommandExitCode};
use super::exit_codes;
use super::parse::{OutputFormat, PpuTraceReplayArgs};

/// Exit code: the command replays no record, so it compares nothing.
///
/// The command replays no record when:
/// - the trace holds no record, or
/// - `--skip-context-dependent` skips every record.
pub(crate) const EXIT_NOTHING_REPLAYED: i32 = exit_codes::command_specific(10);

/// Replays the trace and prints the report.
///
/// # Errors
///
/// Returns an error when:
/// - the command cannot read or parse the trace,
/// - the command cannot read or parse the reference directory, or
/// - a write to stdout fails for a reason other than a closed pipe.
pub(crate) fn run(
    args: &PpuTraceReplayArgs,
    format: OutputFormat,
) -> Result<CommandExitCode, CommandError> {
    let report = replay(args)?;
    let text = match format {
        OutputFormat::Human => render_human(&report, args.max_failures),
        OutputFormat::Json => render_json(&report, args.max_failures),
    };
    match std::io::stdout().lock().write_all(text.as_bytes()) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {
            return Ok(CommandExitCode::new(exit_codes::BROKEN_PIPE));
        }
        Err(error) => {
            return Err(CommandError::failed(format!(
                "ppu-trace-replay: stdout write: {error}"
            )));
        }
    }
    let replayed = report.kinds.iter().any(|row| row.passed + row.failed > 0);
    Ok(if report.triage_count() > 0 {
        CommandExitCode::new(exit_codes::FAILED)
    } else if !replayed {
        CommandExitCode::new(EXIT_NOTHING_REPLAYED)
    } else {
        CommandExitCode::SUCCESS
    })
}

/// Reads the trace and the references the arguments name and replays
/// the trace.
fn replay(args: &PpuTraceReplayArgs) -> Result<PpuTraceReport, CommandError> {
    let bytes = std::fs::read(&args.trace).map_err(|error| {
        CommandError::failed(format!(
            "ppu-trace-replay: {}: {error}",
            args.trace.display()
        ))
    })?;
    let records = read_trace_bytes(&bytes).map_err(|error| {
        CommandError::failed(format!(
            "ppu-trace-replay: {}: {error}",
            args.trace.display()
        ))
    })?;
    let console = match &args.references {
        Some(dir) => console_vectors(dir)
            .map_err(|error| CommandError::failed(format!("ppu-trace-replay: {error}")))?,
        None => Vec::new(),
    };
    let options = PpuTraceOptions {
        skip_context_dependent: args.skip_context_dependent,
    };
    Ok(replay_trace(&records, &console, options))
}

/// What the trace says about VRSAVE, the FPSCR and the VSCR.
fn control_note(report: &PpuTraceReport) -> &'static str {
    match report.control_registers {
        Some(ControlRegisters::Recorded) => "vrsave, fpscr and vscr compared",
        Some(ControlRegisters::NotRecorded) => {
            "vrsave, fpscr and vscr not recorded by this trace version, so not compared"
        }
        None => "no record",
    }
}

/// The JSON token for what the trace says about VRSAVE, the FPSCR and
/// the VSCR.
fn control_token(report: &PpuTraceReport) -> &'static str {
    match report.control_registers {
        Some(ControlRegisters::Recorded) => "recorded",
        Some(ControlRegisters::NotRecorded) => "not_recorded",
        None => "none",
    }
}

/// The class's detail: the sets or the vector that decided it.
fn class_detail(class: &PpuTraceClass) -> String {
    match class {
        PpuTraceClass::Undefined { sets } => {
            let ids: Vec<&str> = sets.iter().map(|set| set.id()).collect();
            ids.join(",")
        }
        PpuTraceClass::CellGovDefect { vector } | PpuTraceClass::Rpcs3Divergence { vector } => {
            vector.clone()
        }
        PpuTraceClass::ContextDependent | PpuTraceClass::Unclassified => String::new(),
    }
}

/// Failures counted by class name, in name order.
fn class_counts(report: &PpuTraceReport) -> BTreeMap<&'static str, usize> {
    let mut counts = BTreeMap::new();
    for failure in &report.failures {
        *counts.entry(failure.class.name()).or_default() += 1;
    }
    counts
}

/// The report as text lines, in this order:
/// - the header,
/// - one line per kind,
/// - the unreached kinds,
/// - up to `max_failures` failures and a count of the rest,
/// - the summary.
pub(crate) fn render_human(report: &PpuTraceReport, max_failures: usize) -> String {
    let mut text = String::new();
    let _ = writeln!(
        text,
        "ppu-trace-replay: {} record(s); {}",
        report.records,
        control_note(report)
    );
    for row in &report.kinds {
        let _ = writeln!(
            text,
            "kind {} seen={} passed={} failed={} skipped={}",
            row.kind, row.seen, row.passed, row.failed, row.skipped
        );
    }
    let _ = writeln!(
        text,
        "unreached {} kind(s): {}",
        report.unreached.len(),
        report.unreached.join(" ")
    );
    for failure in report.failures.iter().take(max_failures) {
        let _ = writeln!(text, "{}", failure_line(failure));
    }
    if report.failures.len() > max_failures {
        let _ = writeln!(
            text,
            "... {} more failure(s) not listed",
            report.failures.len() - max_failures
        );
    }
    let classes: Vec<String> = class_counts(report)
        .into_iter()
        .map(|(name, count)| format!("{name}={count}"))
        .collect();
    let triage = report.triage_count();
    let _ = writeln!(
        text,
        "summary: {} failure(s) [{}], {triage} to triage",
        report.failures.len(),
        classes.join(" ")
    );
    text
}

fn failure_line(failure: &PpuTraceFailure) -> String {
    let detail = class_detail(&failure.class);
    let detail = if detail.is_empty() {
        String::new()
    } else {
        format!(" ({detail})")
    };
    format!(
        "failure record={} pc=0x{:08x} raw=0x{:08x} kind={} class={}{detail} fields=[{}]",
        failure.record,
        failure.pc,
        failure.raw,
        failure.kind,
        failure.class.name(),
        differing_fields(&failure.outcome).join(",")
    )
}

/// The report as one JSON document, listing up to `max_failures`
/// failures.
pub(crate) fn render_json(report: &PpuTraceReport, max_failures: usize) -> String {
    let kinds: Vec<serde_json::Value> = report
        .kinds
        .iter()
        .map(|row| {
            serde_json::json!({
                "kind": row.kind,
                "seen": row.seen,
                "passed": row.passed,
                "failed": row.failed,
                "skipped": row.skipped,
            })
        })
        .collect();
    let failures: Vec<serde_json::Value> = report
        .failures
        .iter()
        .take(max_failures)
        .map(|failure| {
            serde_json::json!({
                "record": failure.record,
                "pc": failure.pc,
                "raw": failure.raw,
                "kind": failure.kind,
                "class": failure.class.name(),
                "detail": class_detail(&failure.class),
                "fields": differing_fields(&failure.outcome),
            })
        })
        .collect();
    let document = serde_json::json!({
        "records": report.records,
        "control_registers": control_token(report),
        "kinds": kinds,
        "unreached": report.unreached,
        "failure_count": report.failures.len(),
        "triage_count": report.triage_count(),
        "failure_classes": class_counts(report),
        "failures": failures,
    });
    format!("{document:#}\n")
}

#[cfg(test)]
#[path = "tests/ppu_trace_replay_tests.rs"]
mod tests;
