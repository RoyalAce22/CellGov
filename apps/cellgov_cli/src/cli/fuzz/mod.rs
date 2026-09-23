//! Host policy for `cellgov dev fuzz`. The parser hands typed arguments to
//! one engine call. Every result reaches the terminal and the exit status
//! through the typed records in [`outcome`].

mod artifact;
mod campaign;
mod census;
mod entry;
mod error;
mod evaluate;
mod outcome;
mod scan;
mod smoke;

pub(crate) use entry::run_with_render;
pub(crate) use error::FuzzCliError;

#[cfg(test)]
#[path = "tests/fuzz_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/outcome_tests.rs"]
mod outcome_tests;

#[cfg(test)]
#[path = "tests/guard_tests.rs"]
mod guard_tests;

#[cfg(test)]
#[path = "tests/evaluate_tests.rs"]
mod evaluate_tests;

#[cfg(test)]
#[path = "tests/smoke_tests.rs"]
mod smoke_tests;

#[cfg(test)]
#[path = "tests/census_tests.rs"]
mod census_tests;
