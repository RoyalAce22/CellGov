//! What a bench run set decides without knowing which title it ran:
//! the result line a measuring child prints, the comparison against a
//! cell's committed anchor, and the gate over the set's own runs.
//!
//! The throughput verdict stays with the caller: it reasons in
//! fractions of a second, and this crate carries no floating point.

mod anchor;
mod result_line;
mod runs;

#[cfg(test)]
#[path = "tests/test_fixtures.rs"]
mod test_fixtures;

pub use anchor::{
    anchor_from_measurement, hold_against_anchor, load_anchor, AnchorLoadError, AnchorMeasurement,
    AnchorVerdict, MeasuredRun,
};
pub use result_line::{
    format_bench_result, parse_bench_result, BenchBootResult, BenchLineWarning, ParseBenchError,
    ParsedBenchLine, BENCH_RESULT_PREFIX,
};
pub use runs::{classify_runs, determinism_disagreements, BenchGate};
