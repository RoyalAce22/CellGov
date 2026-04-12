//! cellgov_testkit -- scenario builders, fake workloads, replay assertions,
//! invariant checks, determinism harness.
//!
//! Layered as:
//!
//! - `world`      -- declarative builders for runtime, units, memory, mailbox/DMA state, budgets
//! - `fixtures`   -- first-class scenario fixtures (initial state, scheduler knobs, expected hashes)
//! - `runner`     -- the single canonical execution path; captures binary trace, computes hashes
//! - `assertions` -- invariant, golden-trace, and state-equivalence checks
//! - `golden`     -- stored binary traces and normalized snapshots
//!
//! Tests must not invent their own execution loops. They build a fixture, hand
//! it to the runner, and assert on the structured `ScenarioResult`.

pub mod assertions;
pub mod fixtures;
pub mod golden;
pub mod runner;
pub mod world;

// Stable entry points re-exported for convenience.
pub use fixtures::ScenarioFixture;
pub use golden::{assert_golden_trace, assert_golden_trace_prefix};
pub use runner::{run, ScenarioOutcome, ScenarioResult};
