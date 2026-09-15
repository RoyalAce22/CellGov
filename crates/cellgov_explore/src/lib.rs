//! Bounded enumeration of legal alternate schedules over an unmodified
//! runtime, classifying outcomes as schedule-stable, schedule-sensitive,
//! or inconclusive.

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod classify;
pub mod config;
pub mod decision;
pub mod dependency;
pub mod execution;
pub mod explore;
pub mod explorer;
pub mod observer;
pub mod oracle;
pub mod prescribed;
pub mod report;
pub mod util;

pub use classify::{BaselineRun, ExplorationResult, OutcomeClass, ScheduleRecord};
pub use config::ExplorationConfig;
pub use decision::{DecisionLog, DecisionPoint};
pub use dependency::StepFootprint;
pub use execution::{ClockCost, ClockVector, Event, EventId, Execution, HappensBefore, Race};
pub use explore::{explore_pair, PairResult};
pub use explorer::{explore, explore_window};
pub use observer::observe_decisions;
pub use oracle::{explore_with_regions, MemoryRegionSpec, OracleExplorationResult};
pub use prescribed::PrescribedScheduler;
pub use util::{StopClass, StopReason};

#[cfg(test)]
#[path = "tests/read_intent_tests.rs"]
mod read_intent_tests;

#[cfg(test)]
#[path = "tests/truncation_tests.rs"]
mod truncation_tests;
