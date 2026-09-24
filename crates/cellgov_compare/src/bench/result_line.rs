//! The `BENCH_RESULT` line: the one channel a measuring child has to
//! its parent. The writer and the reader live here together, so they
//! cannot disagree on the format.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use cellgov_time::Budget;

use crate::runner_cellgov::{BootOutcome, BootOutcomeParseError};

/// The first token of every result line.
pub const BENCH_RESULT_PREFIX: &str = "BENCH_RESULT";

/// One completed bench run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BenchBootResult {
    /// The measurement of the set this run is.
    pub run_index: usize,
    /// Steps the run retired.
    pub steps: usize,
    /// Wall time of the step loop.
    pub wall: Duration,
    /// Instructions each step was granted; `steps * budget` is the
    /// count the run retired.
    pub budget: Budget,
    /// How the run ended.
    pub outcome: BootOutcome,
}

impl BenchBootResult {
    /// Retired steps per second of wall time, rounded to the nearest
    /// integer. A zero wall gives zero.
    pub fn steps_per_sec(&self) -> u64 {
        let wall = self.wall.as_nanos();
        if wall == 0 {
            return 0;
        }
        // `steps` is at most 2^64, so the product stays below 2^94 and
        // the sum cannot overflow a u128.
        let scaled = self.steps as u128 * 1_000_000_000;
        u64::try_from((scaled + wall / 2) / wall).unwrap_or(u64::MAX)
    }
}

/// Render `r` as its `BENCH_RESULT` line.
///
/// Wall time travels in nanoseconds, `Duration`'s own resolution, so
/// the parent reconstructs exactly what the clock returned.
/// `steps_per_sec` is redundant and carried for readers. `run_index`
/// names the measurement a captured line came from, once several runs
/// share one log.
pub fn format_bench_result(r: &BenchBootResult) -> String {
    format!(
        "{BENCH_RESULT_PREFIX} run_index={} steps={} wall_ns={} steps_per_sec={} budget={} outcome={}",
        r.run_index,
        r.steps,
        r.wall.as_nanos(),
        r.steps_per_sec(),
        r.budget.raw(),
        r.outcome,
    )
}

/// Why no result came out of a child's stdout.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseBenchError {
    /// No line starts with the prefix.
    #[error("no BENCH_RESULT line")]
    NoResultLine,
    /// Two lines start with the prefix, so neither belongs to one run.
    #[error("more than one BENCH_RESULT line")]
    DuplicateResultLine,
    /// The line has no `run_index=` token.
    #[error("BENCH_RESULT: missing run_index= field")]
    MissingRunIndex,
    /// `run_index=` holds no `usize`.
    #[error("BENCH_RESULT: malformed run_index={0:?}")]
    MalformedRunIndex(String),
    /// The line has no `steps=` token.
    #[error("BENCH_RESULT: missing steps= field")]
    MissingSteps,
    /// `steps=` holds no `usize`.
    #[error("BENCH_RESULT: malformed steps={0:?}")]
    MalformedSteps(String),
    /// The line has no `wall_ns=` token.
    #[error("BENCH_RESULT: missing wall_ns= field")]
    MissingWallNs,
    /// `wall_ns=` holds no `u64`.
    #[error("BENCH_RESULT: malformed wall_ns={0:?}")]
    MalformedWallNs(String),
    /// The line has no `budget=` token.
    #[error("BENCH_RESULT: missing budget= field")]
    MissingBudget,
    /// `budget=` holds no `u64`.
    #[error("BENCH_RESULT: malformed budget={0:?}")]
    MalformedBudget(String),
    /// The line has no `outcome=` token.
    #[error("BENCH_RESULT: missing outcome= field")]
    MissingOutcome,
    /// `outcome=` names no boot outcome.
    #[error("BENCH_RESULT: malformed outcome={token:?}: {source}")]
    UnparseableOutcome {
        /// The token as printed.
        token: String,
        /// Why it names no outcome.
        #[source]
        source: BootOutcomeParseError,
    },
}

/// Something the reader skipped on a line it still accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BenchLineWarning {
    /// `steps_per_sec=` did not parse. The field is redundant, so the
    /// line stands, but the reader skips the writer/reader agreement
    /// check.
    MalformedStepsPerSec(String),
    /// A token this reader does not know, so the writer is newer than
    /// the reader.
    UnknownToken(String),
}

impl fmt::Display for BenchLineWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedStepsPerSec(v) => write!(
                f,
                "BENCH_RESULT: malformed steps_per_sec={v:?}; the writer/reader agreement \
                 check is skipped for this line"
            ),
            Self::UnknownToken(t) => write!(
                f,
                "BENCH_RESULT: unknown token {t:?}; the reader may be older than the writer"
            ),
        }
    }
}

/// A result line the reader accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedBenchLine {
    /// The run the line describes.
    pub result: BenchBootResult,
    /// What the reader skipped, in token order.
    pub warnings: Vec<BenchLineWarning>,
}

/// Parse the one `BENCH_RESULT run_index=I steps=N wall_ns=M
/// steps_per_sec=X budget=B outcome=O` line out of captured stdout.
///
/// The reader refuses a second result line: one boot prints the line once,
/// so two lines mean two runs' output reached one pipe. `wall_ns` must
/// fit a `u64` (about 584 years), and a larger value is malformed
/// rather than clamped.
pub fn parse_bench_result(stdout: &str) -> Result<ParsedBenchLine, ParseBenchError> {
    let mut lines = stdout.lines().filter(|l| {
        l.strip_prefix(BENCH_RESULT_PREFIX)
            .is_some_and(|rest| rest.starts_with(' '))
    });
    let line = lines.next().ok_or(ParseBenchError::NoResultLine)?;
    if lines.next().is_some() {
        return Err(ParseBenchError::DuplicateResultLine);
    }
    let mut warnings = Vec::new();
    let mut run_index: Option<usize> = None;
    let mut steps: Option<usize> = None;
    let mut wall_ns: Option<u64> = None;
    let mut budget: Option<u64> = None;
    let mut outcome_token: Option<String> = None;
    let mut reported_sps: Option<u64> = None;
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
                Err(_) => warnings.push(BenchLineWarning::MalformedStepsPerSec(v.to_string())),
            }
        } else if let Some(v) = tok.strip_prefix("outcome=") {
            outcome_token = Some(v.to_string());
        } else {
            warnings.push(BenchLineWarning::UnknownToken(tok.to_string()));
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
    let result = BenchBootResult {
        run_index,
        steps,
        wall: Duration::from_nanos(wall_ns),
        budget: Budget::new(budget),
        outcome,
    };
    // The transport is lossless, so the reader recomputes the rate from
    // exactly the writer's inputs. One unit of slack admits a writer
    // that rounded a float rather than the integer quotient.
    if let Some(reported) = reported_sps {
        let computed = result.steps_per_sec();
        debug_assert!(
            reported.abs_diff(computed) <= 1,
            "BENCH_RESULT steps_per_sec drift: reported={reported} computed={computed}"
        );
    }
    Ok(ParsedBenchLine { result, warnings })
}

#[cfg(test)]
#[path = "tests/result_line_tests.rs"]
mod tests;
