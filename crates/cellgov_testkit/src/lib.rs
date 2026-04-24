//! Shared test scaffolding: world builders, scenario fixtures, the canonical
//! runner, assertion helpers, and golden-trace comparisons.
//!
//! Tests build a [`ScenarioFixture`], hand it to [`run`], and assert on the
//! returned [`ScenarioResult`]. No test drives a `Runtime` directly.

pub mod assertions;
pub mod fixtures;
pub mod golden;
pub mod runner;
pub mod world;

pub use fixtures::ScenarioFixture;
pub use golden::{assert_golden_trace, assert_golden_trace_prefix};
pub use runner::{run, ScenarioOutcome, ScenarioResult};
