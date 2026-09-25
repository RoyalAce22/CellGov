//! `diff diverge` and `diff zoom` over per-step state traces.

use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::exit_codes;
use crate::cli::self_load::load_file;

/// Exit code: a state or zoom trace failed to decode, so no verdict
/// covers the records past the cut.
const EXIT_CORRUPT_TRACE: i32 = exit_codes::command_specific(31);

/// Exit code: `diff zoom` found the requested step in neither window.
const EXIT_MISSING_STEP: i32 = exit_codes::command_specific(30);

/// Streaming scan of two per-step state-trace files.
///
/// # Exit status
///
/// - Status 0 means every `PpuStateHash` record matches.
/// - Status 1 means the step or trace length differs.
/// - [`EXIT_CORRUPT_TRACE`] means a trace failed to decode before the
///   scan finished. The scan prints no verdict for that file.
/// - `EXIT_SCHEME_MISMATCH` means the two traces hold state hashes of
///   two schemes, so the scan compares no record.
///
/// # Errors
///
/// Returns an error if a trace file cannot be read.
pub(crate) fn run_diverge(a_path: &str, b_path: &str) -> Result<CommandExitCode, CommandError> {
    use cellgov_compare::{diverge, DivergeField, DivergeReport, TraceDecodeError};
    let a_bytes = load_file(a_path)?;
    let b_bytes = load_file(b_path)?;
    report_trace_identity(&a_bytes, a_path, &b_bytes, b_path);
    match diverge(&a_bytes, &b_bytes) {
        DivergeReport::SchemeMismatch { a, b } => {
            println!(
                "SCHEME_MISMATCH  a_scheme=0x{a:016x} b_scheme=0x{b:016x}  (the two captures hold state hashes of two schemes; no record was compared)"
            );
            Ok(CommandExitCode::new(super::scenario::EXIT_SCHEME_MISMATCH))
        }
        DivergeReport::Identical { count } => {
            println!("IDENTICAL  {count} PpuStateHash records matched");
            if count == 0 {
                eprintln!(
                    "WARN: zero PpuStateHash records matched; trace files may be empty or truncated"
                );
            }
            Ok(CommandExitCode::SUCCESS)
        }
        DivergeReport::Differs {
            step,
            a_pc,
            b_pc,
            a_hash,
            b_hash,
            field,
        } => {
            let field_str = match field {
                DivergeField::Pc => "pc",
                DivergeField::Hash => "hash",
            };
            println!(
                "DIVERGE step={step} field={field_str}  a_pc=0x{a_pc:x} b_pc=0x{b_pc:x}  a_hash=0x{a_hash:x} b_hash=0x{b_hash:x}"
            );
            Ok(CommandExitCode::new(exit_codes::FAILED))
        }
        DivergeReport::LengthDiffers {
            common_count,
            a_count,
            b_count,
        } => {
            println!(
                "LENGTH_DIFFERS  common={common_count}  a={a_count}  b={b_count}  ({a_path} vs {b_path})"
            );
            Ok(CommandExitCode::new(exit_codes::FAILED))
        }
        DivergeReport::CorruptTrace {
            common_count,
            a_error,
            b_error,
        } => {
            let describe =
                |e: Option<TraceDecodeError>| e.map_or_else(|| "ok".into(), |e| e.to_string());
            println!(
                "CORRUPT_TRACE  common={common_count}  a: {}  b: {}  (a state file failed to decode; the {common_count} records before the cut matched and nothing past it was compared)",
                describe(a_error),
                describe(b_error)
            );
            Ok(CommandExitCode::new(EXIT_CORRUPT_TRACE))
        }
    }
}

fn report_trace_identity(a: &[u8], a_path: &str, b: &[u8], b_path: &str) {
    for line in cellgov_compare::cross_trace_identity_warning(
        cellgov_compare::trace_identity(a),
        a_path,
        cellgov_compare::trace_identity(b),
        b_path,
    ) {
        eprintln!("{line}");
    }
}

/// Per-field register diff at the named step.
///
/// # Exit status
///
/// - Status 0 means every fingerprint field and the PC agree.
/// - Status 1 means a register field or the PC differs.
/// - [`EXIT_MISSING_STEP`] means one or both windows omit the step.
/// - [`EXIT_CORRUPT_TRACE`] means a zoom trace failed to decode.
///
/// # Errors
///
/// Returns an error if a trace file cannot be read.
pub(crate) fn run_zoom(
    a_path: &str,
    b_path: &str,
    step: u64,
) -> Result<CommandExitCode, CommandError> {
    use cellgov_compare::{zoom_lookup, ZoomLookup};
    let a_bytes = load_file(a_path)?;
    let b_bytes = load_file(b_path)?;
    match zoom_lookup(&a_bytes, &b_bytes, step) {
        ZoomLookup::Found {
            step,
            a_pc,
            b_pc,
            diffs,
        } => {
            if diffs.is_empty() {
                // PC is outside the fingerprint, so `diff diverge` can name
                // a Pc divergence whose zoom diff is empty -- that is
                // a real control-flow divergence, not harness skew.
                if a_pc != b_pc {
                    println!(
                        "PC_DIFF step={step} a_pc=0x{a_pc:x} b_pc=0x{b_pc:x}  registers agree but control flow diverged; the PC split is the divergence"
                    );
                    return Ok(CommandExitCode::new(exit_codes::FAILED));
                }
                println!("NO_FIELD_DIFF step={step} pc=0x{a_pc:x}  snapshots agree on every fingerprint field and PC; if the hash stream diverged at this step, the harness is skewing snapshots against hashes -- investigate, do not resume the scan");
                Ok(CommandExitCode::SUCCESS)
            } else {
                println!(
                    "ZOOM step={step} a_pc=0x{a_pc:x} b_pc=0x{b_pc:x}  {} field(s) differ:",
                    diffs.len()
                );
                for d in &diffs {
                    println!("  {:<5}  a=0x{:016x}  b=0x{:016x}", d.field, d.a, d.b);
                }
                Ok(CommandExitCode::new(exit_codes::FAILED))
            }
        }
        ZoomLookup::MissingStep {
            step,
            a_missing,
            b_missing,
        } => {
            let a_has_step = !a_missing;
            let b_has_step = !b_missing;
            println!(
                "MISSING_STEP step={step}  a_has_step={a_has_step}  b_has_step={b_has_step}  (zoom window did not cover this step on at least one side)"
            );
            Ok(CommandExitCode::new(EXIT_MISSING_STEP))
        }
        ZoomLookup::CorruptTrace { a_error, b_error } => {
            let describe = |e: Option<String>| e.unwrap_or_else(|| "ok".into());
            println!(
                "CORRUPT_TRACE  a: {}  b: {}  (zoom file damaged; widening the window will not help)",
                describe(a_error),
                describe(b_error)
            );
            Ok(CommandExitCode::new(EXIT_CORRUPT_TRACE))
        }
    }
}
