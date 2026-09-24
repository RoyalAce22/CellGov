//! Shared test scaffolding: world builders, scenario fixtures, the canonical
//! runner, assertion helpers, and golden-trace comparisons.
//!
//! Tests build a [`ScenarioFixture`], hand it to [`run`], and assert on the
//! returned [`ScenarioResult`]. No test drives a `Runtime` directly.
//!
//! # Property tests
//!
//! A property is a `proptest` test over generated input. The workspace
//! keeps one convention for them:
//!
//! - A property lives beside the unit tests of the module it covers, in
//!   `src/tests/<subject>_proptests.rs`. The module declares it as
//!   `#[cfg(test)] #[path = "tests/<subject>_proptests.rs"] mod proptests;`.
//! - The crate takes `proptest.workspace = true` as a dev-dependency.
//! - The default case count (256) runs in the per-commit gate. A soak
//!   run sets `PROPTEST_CASES=<n>` in the environment.
//! - A failed property writes its minimised counterexample under the
//!   crate's `proptest-regressions/` directory. Commit that file: it
//!   replays the counterexample first on every later run.
//! - Before the fix lands, pin the minimised counterexample as a named
//!   unit test in the sibling `<subject>_tests.rs` file. The property
//!   finds the defect; the unit test records it.

#![allow(
    clippy::unwrap_used,
    reason = "test scaffolding: every consumer is a test, so .unwrap() panics are the correct failure mode"
)]

pub mod assertions;
pub mod fixtures;
pub mod golden;
pub mod param_sfo;
pub mod runner;
#[cfg(feature = "scratch")]
pub mod scratch;
#[cfg(feature = "scratch")]
pub mod store;
pub mod world;

pub use fixtures::ScenarioFixture;
pub use golden::{assert_golden_trace, assert_golden_trace_prefix};
pub use runner::{run, ScenarioOutcome, ScenarioResult};
