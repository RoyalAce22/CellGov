//! Host policy for `cellgov dev fuzz`. The parser hands typed arguments to
//! one engine call. Every result reaches the terminal and the exit status
//! through the typed records in [`outcome`].

mod artifact;
mod campaign;
mod entry;
mod error;
mod outcome;
mod scan;

pub(crate) use entry::run_with_quiet;
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
