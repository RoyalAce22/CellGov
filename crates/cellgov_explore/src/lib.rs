//! Schedule exploration engine for CellGov.
//!
//! Sits above `cellgov_core` and orchestrates bounded exploration of
//! alternate legal schedules. The explorer detects decision points
//! (where more than one unit is runnable), replays from snapshots
//! with alternate choices, and classifies outcomes.
//!
//! The underlying deterministic runtime is unmodified; this crate
//! drives it externally through `Runtime::step` and
//! `Runtime::registry().runnable_ids()`.

pub mod classify;
pub mod config;
pub mod decision;
pub mod dependency;
pub mod explore;
pub mod explorer;
pub mod observer;
pub mod oracle;
pub mod prescribed;
pub mod report;
pub mod util;

pub use classify::{ExplorationResult, OutcomeClass, ScheduleRecord};
pub use config::ExplorationConfig;
pub use decision::{DecisionLog, DecisionPoint};
pub use dependency::StepFootprint;
pub use explore::{explore_pair, PairResult};
pub use explorer::explore;
pub use observer::observe_decisions;
pub use oracle::{explore_with_regions, MemoryRegionSpec, OracleExplorationResult};
pub use prescribed::PrescribedScheduler;
