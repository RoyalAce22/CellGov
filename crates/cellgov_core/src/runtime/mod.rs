//! Step driver and commit pipeline over registered units and guest memory.
//!
//! Determinism contract: every observable output is a pure function of
//! runtime state + registry contents + scheduler decisions. No host time,
//! no `HashMap` iteration. The `max_steps` cap trips
//! [`StepError::MaxStepsExceeded`] rather than looping on a stalled system.
//!
//! The main trace stream is fixed-size-per-record; full PPU register
//! snapshots route to `zoom_trace` to keep the main stream homogeneous.
//!
//! Process-exit residue: a process exit sets each unit of the process
//! to `Finished`, and a boot-process exit sets every registered unit.
//! The exit deallocates every thread of the process, so none of these
//! units runs again. The exit sweep cancels each unit's timer deadline
//! and drops its parked response. Other records can still name such a
//! unit:
//!
//! - an LV2 waiter list;
//! - an in-flight DMA transfer.
//!
//! The DMA-completion, timer-wake and sync-wake paths each skip a
//! `Finished` unit. Their `Runnable` override would replace `Finished`
//! and resume the thread.

mod accessors;
mod commit_step;
mod commit_trace;
mod construction;
mod dma;
mod host_write;
mod lv2_dispatch;
mod mem_helpers;
mod ppu_create;
mod process_spawn;
mod snapshot;
mod spaces;
mod state;
mod state_hash;
mod step;
mod sync_wakes;
mod tap;
mod timer;
mod trace_bridge;
mod types;

pub use construction::DEFAULT_DMA_LATENCY_TICKS;
pub use snapshot::RuntimeSnapshot;
pub use spaces::{AddressSpaceId, SpaceError};
pub use state::Runtime;
pub use tap::RuntimeTap;
pub use types::{
    default_budget_for_mode, PendingChildInit, PpuFactory, ProcessSpawnLoadError,
    ProcessSpawnLoader, RuntimeMode, RuntimeStep, SpawnedProcessImage, SpuFactory, SpuFactoryError,
    StepError,
};

#[cfg(test)]
#[path = "tests/runtime_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/sync_state_lanes_tests.rs"]
mod sync_state_lanes_tests;

#[cfg(test)]
#[path = "tests/sync_state_golden_tests.rs"]
mod sync_state_golden_tests;

#[cfg(test)]
#[path = "tests/sync_state_pending_fields_tests.rs"]
mod sync_state_pending_fields_tests;
