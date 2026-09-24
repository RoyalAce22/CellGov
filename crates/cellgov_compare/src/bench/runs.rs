//! The verdicts a run set reaches over its own runs: what the runs
//! must reproduce of each other, and the order the gate ranks its
//! failures in.

use crate::witness_parse::parse_witness_lines;

use super::anchor::AnchorVerdict;
use super::result_line::BenchBootResult;

/// Gate verdict of one run set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchGate {
    /// Every run reproduced the same steps, outcome and witness map,
    /// and the anchor comparison found nothing.
    Pass,
    /// Runs disagreed on retired step count, boot outcome, or a
    /// witness.
    DeterminismBreak,
    /// The run disagreed with the cell's committed anchor.
    AnchorDrift,
    /// The set reached no throughput claim under a strict throughput
    /// policy.
    SpreadExceeded,
}

/// Every way the runs of a set failed to reproduce each other.
///
/// `streams[i]` is the stderr run `i` printed. Run 1 is the reference
/// for every comparison, so one counter that moves yields one finding
/// per run that moved. A run with no stream is itself a disagreement.
pub fn determinism_disagreements(runs: &[BenchBootResult], streams: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let Some(first) = runs.first() else {
        return out;
    };
    for (index, run) in runs.iter().enumerate().skip(1) {
        if run.steps != first.steps || run.outcome != first.outcome {
            out.push(format!(
                "run 1 retired {} steps ending {}, run {} retired {} steps ending {}",
                first.steps,
                first.outcome,
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
        if run.budget != first.budget {
            out.push(format!(
                "run 1 ran at budget {}, run {} ran at budget {}",
                first.budget,
                index + 1,
                run.budget,
            ));
        }
        match (streams.first(), streams.get(index)) {
            (Some(a), Some(b)) => out.extend(witness_disagreements(
                "run 1",
                a,
                &format!("run {}", index + 1),
                b,
            )),
            _ => out.push(format!(
                "run {} carries no stream to compare witnesses against",
                index + 1
            )),
        }
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
    for (name, y) in &b.values {
        if !a.values.contains_key(name) {
            out.push(format!(
                "witness {name}: {b_label} {y}, absent from {a_label}"
            ));
        }
    }
    out
}

/// The gate a run set reaches.
///
/// Order matters. A determinism break makes the witness stream
/// meaningless. An anchor disagreement outranks the throughput
/// verdict, so a contended host cannot mask a real regression behind a
/// timing failure.
///
/// Throughput reaches the gate only when `strict_throughput` is set: a
/// busy host inflates the spread of a run that regressed nothing.
pub fn classify_runs(
    determinism_failures: &[String],
    anchor: &AnchorVerdict,
    throughput_measured: bool,
    strict_throughput: bool,
) -> BenchGate {
    if !determinism_failures.is_empty() {
        return BenchGate::DeterminismBreak;
    }
    if matches!(anchor, AnchorVerdict::Drift(_)) {
        return BenchGate::AnchorDrift;
    }
    if strict_throughput && !throughput_measured {
        return BenchGate::SpreadExceeded;
    }
    BenchGate::Pass
}

#[cfg(test)]
#[path = "tests/runs_tests.rs"]
mod tests;
