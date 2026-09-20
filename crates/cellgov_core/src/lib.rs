#![deny(unused_must_use)]
#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(
    not(test),
    forbid(
        clippy::disallowed_methods,
        clippy::disallowed_macros,
        clippy::print_stdout,
        clippy::print_stderr,
        clippy::dbg_macro
    )
)]
//! Top-level runtime orchestration: the `Runtime` struct, scheduler loop,
//! unit registry, commit coordination, and stable-ordering rules. No
//! architecture-specific logic; only traits and immutable data packets
//! cross crate boundaries.

pub mod commit;
pub mod registry;
pub mod rsx;
pub mod runtime;
pub mod scheduler;
pub mod syscall_table;
pub mod timer_queue;

pub use commit::{BlockReason, CommitContext, CommitError, CommitOutcome, CommitPipeline};
pub use registry::{RegisteredUnit, UnitRegistry};
pub use runtime::{
    default_budget_for_mode, AddressSpaceId, PendingChildInit, ProcessSpawnLoadError,
    ProcessSpawnLoader, Runtime, RuntimeMode, RuntimeSnapshot, RuntimeStep, RuntimeTap, SpaceError,
    SpawnedProcessImage, SpuFactory, SpuFactoryError, StepError, DEFAULT_DMA_LATENCY_TICKS,
};
pub use scheduler::{RoundRobinScheduler, Scheduler};
pub use syscall_table::SyscallResponseTable;
pub use timer_queue::{TimerWake, TimerWakeKind, TimerWakeQueue};
