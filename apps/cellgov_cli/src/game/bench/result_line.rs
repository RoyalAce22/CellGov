//! The writer and the reader of the `BENCH_RESULT` line, which must
//! agree on its format.

use std::str::FromStr;
use std::time::Duration;

use cellgov_compare::{BootOutcome, BootOutcomeParseError};
use cellgov_time::Budget;

use super::types::BenchBootResult;

/// The `BENCH_RESULT` line, the child's only channel to the parent.
///
/// Wall time travels in nanoseconds, `Duration`'s own resolution, so
/// the parent reconstructs exactly what the clock returned and
/// recomputes `steps_per_sec` from the same inputs this line was
/// printed from; `steps_per_sec` itself is carried for readers.
/// `run_index` names the measurement a captured line came from, once
/// several runs share one log.
pub(super) fn format_bench_result(r: &BenchBootResult) -> String {
    format!(
        "BENCH_RESULT run_index={} steps={} wall_ns={} steps_per_sec={:.0} budget={} outcome={}",
        r.run_index,
        r.steps,
        r.wall.as_nanos(),
        r.steps_per_sec(),
        r.budget.raw(),
        r.outcome,
    )
}

/// Failure mode while parsing a `BENCH_RESULT` line out of subprocess
/// stdout.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseBenchError {
    #[error("no BENCH_RESULT line")]
    NoResultLine,
    #[error("more than one BENCH_RESULT line")]
    DuplicateResultLine,
    #[error("BENCH_RESULT: missing run_index= field")]
    MissingRunIndex,
    #[error("BENCH_RESULT: malformed run_index={0:?}")]
    MalformedRunIndex(String),
    #[error("BENCH_RESULT: missing steps= field")]
    MissingSteps,
    #[error("BENCH_RESULT: malformed steps={0:?}")]
    MalformedSteps(String),
    #[error("BENCH_RESULT: missing wall_ns= field")]
    MissingWallNs,
    #[error("BENCH_RESULT: malformed wall_ns={0:?}")]
    MalformedWallNs(String),
    #[error("BENCH_RESULT: missing budget= field")]
    MissingBudget,
    #[error("BENCH_RESULT: malformed budget={0:?}")]
    MalformedBudget(String),
    #[error("BENCH_RESULT: missing outcome= field")]
    MissingOutcome,
    #[error("BENCH_RESULT: malformed outcome={token:?}: {source}")]
    UnparseableOutcome {
        token: String,
        #[source]
        source: BootOutcomeParseError,
    },
}

/// Parse the `BENCH_RESULT run_index=I steps=N wall_ns=M
/// steps_per_sec=X budget=B outcome=O` line out of captured stdout.
///
/// `wall_ns` must fit a `u64` (about 584 years); the child's `u128`
/// print never exceeds that for a real run, and a larger value is
/// reported as malformed rather than clamped.
pub(super) fn parse_bench_result(stdout: &str) -> Result<BenchBootResult, ParseBenchError> {
    let mut iter = stdout.lines().filter(|l| l.starts_with("BENCH_RESULT "));
    let line = iter.next().ok_or(ParseBenchError::NoResultLine)?;
    if iter.next().is_some() {
        return Err(ParseBenchError::DuplicateResultLine);
    }
    let mut run_index: Option<usize> = None;
    let mut steps: Option<usize> = None;
    let mut wall_ns: Option<u64> = None;
    let mut budget: Option<u64> = None;
    let mut outcome_token: Option<String> = None;
    let mut reported_sps: Option<f64> = None;
    for tok in line.split_whitespace().skip(1) {
        if let Some(v) = tok.strip_prefix("run_index=") {
            run_index = Some(
                v.parse()
                    .map_err(|_| ParseBenchError::MalformedRunIndex(v.to_string()))?,
            );
        } else if let Some(v) = tok.strip_prefix("steps=") {
            steps = Some(
                v.parse()
                    .map_err(|_| ParseBenchError::MalformedSteps(v.to_string()))?,
            );
        } else if let Some(v) = tok.strip_prefix("wall_ns=") {
            wall_ns = Some(
                v.parse()
                    .map_err(|_| ParseBenchError::MalformedWallNs(v.to_string()))?,
            );
        } else if let Some(v) = tok.strip_prefix("budget=") {
            budget = Some(
                v.parse()
                    .map_err(|_| ParseBenchError::MalformedBudget(v.to_string()))?,
            );
        } else if let Some(v) = tok.strip_prefix("steps_per_sec=") {
            match v.parse() {
                Ok(x) => reported_sps = Some(x),
                // The field is redundant, so a bad value does not
                // reject the line.
                Err(e) => eprintln!(
                    "parse_bench_result: warning: malformed steps_per_sec={v:?} ({e}); the \
                     writer/reader round-trip check is skipped for this line"
                ),
            }
        } else if let Some(v) = tok.strip_prefix("outcome=") {
            outcome_token = Some(v.to_string());
        } else {
            eprintln!(
                "parse_bench_result: warning: unknown token {tok:?} in BENCH_RESULT line; parser may be stale"
            );
        }
    }
    let run_index = run_index.ok_or(ParseBenchError::MissingRunIndex)?;
    let steps = steps.ok_or(ParseBenchError::MissingSteps)?;
    let wall_ns = wall_ns.ok_or(ParseBenchError::MissingWallNs)?;
    let budget = budget.ok_or(ParseBenchError::MissingBudget)?;
    let outcome_token = outcome_token.ok_or(ParseBenchError::MissingOutcome)?;
    let outcome = BootOutcome::from_str(&outcome_token).map_err(|source| {
        ParseBenchError::UnparseableOutcome {
            token: outcome_token.clone(),
            source,
        }
    })?;
    let wall = Duration::from_nanos(wall_ns);
    let result = BenchBootResult {
        run_index,
        steps,
        wall,
        budget: Budget::new(budget),
        outcome,
    };
    // The transport is lossless, so the parent recomputes
    // steps_per_sec from exactly the child's inputs and the only
    // admissible difference is the child's `{:.0}` print rounding.
    // Anything wider means the writer and this reader disagree on
    // the line's units.
    if let Some(reported) = reported_sps {
        let computed = result.steps_per_sec();
        let tolerance = 0.5 + computed.abs() * f64::EPSILON;
        debug_assert!(
            (reported - computed).abs() <= tolerance,
            "BENCH_RESULT steps_per_sec drift: reported={reported} computed={computed} (tolerance={tolerance})"
        );
    }
    Ok(result)
}

#[cfg(test)]
#[path = "tests/result_line_tests.rs"]
mod tests;
