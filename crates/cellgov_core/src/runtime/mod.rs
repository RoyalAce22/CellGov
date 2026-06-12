//! Step driver and commit pipeline over registered units and guest memory.
//!
//! Determinism contract: every observable output is a pure function of
//! runtime state + registry contents + scheduler decisions. No host time,
//! no `HashMap` iteration. The `max_steps` cap trips
//! [`StepError::MaxStepsExceeded`] rather than looping on a stalled system.
//!
//! The main trace stream is fixed-size-per-record; full PPU register
//! snapshots route to `zoom_trace` to keep the main stream homogeneous.

mod accessors;
mod commit_step;
mod commit_trace;
mod construction;
mod dma;
mod lv2_dispatch;
mod mem_helpers;
mod ppu_create;
mod snapshot;
mod state;
mod state_hash;
mod step;
mod sync_wakes;
mod trace_bridge;
mod types;

pub use snapshot::RuntimeSnapshot;
pub use state::Runtime;
pub use types::{
    default_budget_for_mode, PpuFactory, RuntimeMode, RuntimeStep, SpuFactory, StepError,
};

#[cfg(test)]
#[path = "tests/runtime_tests.rs"]
mod tests;
