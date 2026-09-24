//! Narrowing a determinism break to the first divergent step, by two
//! traced re-runs of the boot.

use std::path::PathBuf;

use cellgov_compare::bench::BenchBootResult;

use super::options::BenchOptions;

/// Retired steps above which a break is localized by hand.
///
/// `DeterminismCheck` records a state hash per retired instruction, so
/// a traced boot costs orders of magnitude more time and memory than
/// the measurement it re-runs. Past this cap the report names the two
/// commands and runs neither.
const LOCALIZE_MAX_STEPS: usize = 25_000;

/// Localize a determinism break to its first divergent step.
///
/// Two more boots run under `DeterminismCheck`, and the two traces go
/// through the same comparison `diff diverge` uses. A break that
/// `DeterminismCheck` mode does not reproduce reports as identical.
///
/// The cap reads the longest run: when a break moves the step count,
/// run 1 bounds neither re-run.
pub(super) fn locate_divergence(
    opts: BenchOptions<'_>,
    runs: &[BenchBootResult],
) -> Result<Vec<String>, crate::cli::exit::CommandError> {
    let mut out = Vec::new();
    let steps = runs.iter().map(|r| r.steps).max().unwrap_or(0);
    if steps > LOCALIZE_MAX_STEPS {
        out.push(format!(
            "diverge: not run automatically -- the boot retires {steps} steps, past the \
             {LOCALIZE_MAX_STEPS} a traced re-run is affordable at. Localize it by hand:"
        ));
        for i in 0..2 {
            let argv = localization_command_argv(opts, i, &format!("run{i}.state"));
            out.push(format!("  {}", render_command(&argv)));
        }
        out.push("  cellgov diff diverge run0.state run1.state".to_string());
        return Ok(out);
    }
    // This prints before the two boots below, which run in
    // DeterminismCheck mode with no progress bar and take far longer
    // than the measurements did.
    println!(
        "    diverge: re-running the boot twice under --save-state-trace to localize the \
         break; this is slower than the measurement was"
    );
    let pid = std::process::id();
    let paths: Vec<PathBuf> = (0..2)
        .map(|i| std::env::temp_dir().join(format!("cellgov-bench-diverge-{pid}-{i}.state")))
        .collect();
    let mut traces = Vec::with_capacity(paths.len());
    for (i, path) in paths.iter().enumerate() {
        let Some(text) = path.to_str() else {
            out.push(format!(
                "cannot localize: the temporary trace path {} is not valid UTF-8",
                path.display()
            ));
            cleanup_traces(&paths);
            return Ok(out);
        };
        let exe = match std::env::current_exe() {
            Ok(e) => e,
            Err(e) => {
                out.push(format!("cannot localize: current_exe: {e}"));
                cleanup_traces(&paths);
                return Ok(out);
            }
        };
        let mut cmd = std::process::Command::new(exe);
        let argv = localization_command_argv(opts, i, text);
        cmd.args(&argv[1..]);
        match cmd.output() {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                cleanup_traces(&paths);
                crate::cli::exit::propagate_interrupt(o.status)?;
                out.push(format!(
                    "cannot localize: the traced re-run exited {:?}",
                    o.status.code()
                ));
                // The child's own stderr is the only account of why it
                // refused.
                out.extend(stderr_tail(&o.stderr));
                return Ok(out);
            }
            Err(e) => {
                out.push(format!("cannot localize: spawning the traced re-run: {e}"));
                cleanup_traces(&paths);
                return Ok(out);
            }
        }
        match std::fs::read(path) {
            Ok(bytes) => traces.push(bytes),
            Err(e) => {
                out.push(format!("cannot localize: reading {}: {e}", path.display()));
                cleanup_traces(&paths);
                return Ok(out);
            }
        }
    }
    cleanup_traces(&paths);
    out.push(format_diverge(&cellgov_compare::diverge(
        &traces[0], &traces[1],
    )));
    Ok(out)
}

fn localization_command_argv(
    mut opts: BenchOptions<'_>,
    diagnostic_index: usize,
    trace_path: &str,
) -> Vec<String> {
    // The offset puts these indices outside the measured set's range,
    // so a captured log cannot read a diagnostic boot as a measurement.
    opts.run_index += 1000 + diagnostic_index;
    let mut cmd = std::process::Command::new("cellgov");
    opts.encode_to_command(&mut cmd);
    cmd.arg("--save-state-trace").arg(trace_path);
    std::iter::once("cellgov".to_string())
        .chain(cmd.get_args().map(|arg| arg.to_string_lossy().into_owned()))
        .collect()
}

fn render_command(argv: &[String]) -> String {
    argv.iter()
        .map(|arg| {
            if arg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_-./:=+".contains(c))
            {
                arg.clone()
            } else {
                quote_command_arg(arg)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(windows)]
fn quote_command_arg(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "''"))
}

#[cfg(not(windows))]
fn quote_command_arg(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "'\"'\"'"))
}

/// Lines a failing child left on stderr, indented for the report.
///
/// A boot that refuses can print a whole witness block first, and the
/// refusal is the last thing it says.
fn stderr_tail(stderr: &[u8]) -> Vec<String> {
    const TAIL_LINES: usize = 8;
    let text = String::from_utf8_lossy(stderr);
    let mut tail: Vec<String> = text
        .lines()
        .rev()
        .take(TAIL_LINES)
        .map(|l| format!("  {l}"))
        .collect();
    tail.reverse();
    tail
}

fn cleanup_traces(paths: &[PathBuf]) {
    for path in paths {
        // Best effort: this path already reports a failure, and a
        // leftover file in the OS temp directory adds nothing to it.
        drop(std::fs::remove_file(path));
    }
}

/// One line naming where two traced re-runs first disagree.
fn format_diverge(report: &cellgov_compare::DivergeReport) -> String {
    use cellgov_compare::{DivergeField, DivergeReport};
    match report {
        DivergeReport::Identical { count } => format!(
            "diverge: the two traced re-runs matched over {count} PpuStateHash record(s); \
             the break did not reproduce under DeterminismCheck mode"
        ),
        DivergeReport::Differs {
            step,
            a_pc,
            b_pc,
            a_hash,
            b_hash,
            field,
        } => {
            let field = match field {
                DivergeField::Pc => "pc",
                DivergeField::Hash => "hash",
            };
            format!(
                "diverge: first divergent step={step} field={field} \
                 a_pc=0x{a_pc:x} b_pc=0x{b_pc:x} a_hash=0x{a_hash:x} b_hash=0x{b_hash:x}"
            )
        }
        DivergeReport::LengthDiffers {
            common_count,
            a_count,
            b_count,
        } => format!(
            "diverge: the traced re-runs agreed over {common_count} record(s) then ran to \
             different lengths (a={a_count}, b={b_count})"
        ),
        // Both sides render, as `cellgov diff diverge` renders them.
        DivergeReport::CorruptTrace {
            common_count,
            a_error,
            b_error,
        } => {
            let a = a_error
                .as_ref()
                .map_or_else(|| "ok".to_string(), ToString::to_string);
            let b = b_error
                .as_ref()
                .map_or_else(|| "ok".to_string(), ToString::to_string);
            format!(
                "diverge: a traced re-run failed to decode after {common_count} record(s), so \
                 nothing past the cut was compared (a: {a}, b: {b})"
            )
        }
    }
}

#[cfg(test)]
#[path = "tests/divergence_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/localization_command_tests.rs"]
mod localization_command_tests;
