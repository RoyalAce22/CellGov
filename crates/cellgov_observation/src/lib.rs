//! The observation schema and the records around it, with no runtime
//! behind them.
//!
//! [`observation::Observation`] is the normalized record every runner
//! produces. Beside it sit the run identity it carries, the microtest
//! manifest that says what to observe, the CGOV frame a test emits and
//! its parser, the console profiles, and the committed console captures.
//! `cellgov_compare` re-exports every module and adds the comparison,
//! the runners and the classifiers; a tool that only reads and writes
//! these records, such as the PS3 runner, depends on this crate alone.

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod console_profile;
pub mod frame;
pub mod hardware_capture;
pub mod identity;
pub mod manifest;
pub mod observation;

#[cfg(test)]
#[path = "tests/test_support.rs"]
mod test_support;
