//! The comparisons a run set makes between its own runs. When a break
//! reproduces, this module narrows it to the first divergent step.

use std::path::PathBuf;

use cellgov_compare::witness_parse::parse_witness_lines;

use super::options::BenchOptions;
use super::types::BenchBootResult;

/// Every way the runs of a set failed to reproduce each other.
///
/// Run 1 is the reference for every comparison, so one counter that
/// moves yields one finding per run that moved.
pub(super) fn determinism_disagreements(
    runs: &[BenchBootResult],
    streams: &[String],
) -> Vec<String> {
    // The one production caller fills both vectors from the same loop,
    // so a shorter `streams` cannot reach this point.
    debug_assert_eq!(
        runs.len(),
        streams.len(),
        "every run of a set carries the stream it printed"
    );
    let mut out = Vec::new();
    for (index, run) in runs.iter().enumerate().skip(1) {
        if run.steps != runs[0].steps || run.outcome != runs[0].outcome {
            out.push(format!(
                "run 1 retired {} steps ending {}, run {} retired {} steps ending {}",
                runs[0].steps,
                runs[0].outcome,
                index + 1,
                run.steps,
                run.outcome,
            ));
        }
        // Each child re-resolves its own composition, so the budget is
        // a per-run result. Only run 1's budget reaches the anchor
        // check. A budget that moves between runs is otherwise
        // invisible: it retires a different trajectory under an
        // unmoved step count.
        if run.budget != runs[0].budget {
            out.push(format!(
                "run 1 ran at budget {}, run {} ran at budget {}",
                runs[0].budget,
                index + 1,
                run.budget,
            ));
        }
        out.extend(witness_disagreements(
            "run 1",
            &streams[0],
            &format!("run {}", index + 1),
            &streams[index],
        ));
    }
    out
}

/// Witness-level disagreements between two runs of a set.
///
/// The steps/outcome comparison cannot see a counter that moved
/// without changing either, and the anchor check reads one run's
/// stream; without this, a witness that is nondeterministic across
/// runs passes the gate whenever it happens to match the anchor.
fn witness_disagreements(
    a_label: &str,
    a_stderr: &str,
    b_label: &str,
    b_stderr: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut parsed = Vec::new();
    for (label, stderr) in [(a_label, a_stderr), (b_label, b_stderr)] {
        match parse_witness_lines(stderr) {
            Ok(w) => parsed.push(w),
            Err(errs) => out.extend(
                errs.iter()
                    .map(|e| format!("{label} witness line did not parse: {e}")),
            ),
        }
    }
    let [a, b] = parsed.as_slice() else {
        // This arm runs only after at least one stream failed to parse,
        // and every parse failure pushes its reason into `out`.
        debug_assert!(
            !out.is_empty(),
            "a stream that did not parse must leave its reason in the report"
        );
        return out;
    };
    for line in a.seen_lines.symmetric_difference(&b.seen_lines) {
        let present = if a.seen_lines.contains(line) {
            a_label
        } else {
            b_label
        };
        out.push(format!("witness line {line} appeared in {present} only"));
    }
    for (name, x) in &a.values {
        let Some(y) = b.values.get(name) else {
            out.push(format!(
                "witness {name}: {a_label} {x}, absent from {b_label}"
            ));
            continue;
        };
        if x != y {
            out.push(format!("witness {name}: {a_label} {x} != {b_label} {y}"));
        }
    }
    for name in b.values.keys() {
        if !a.values.contains_key(name) {
            out.push(format!(
                "witness {name}: {b_label} {}, absent from {a_label}",
                b.values[name]
            ));
        }
    }
    out
}

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
