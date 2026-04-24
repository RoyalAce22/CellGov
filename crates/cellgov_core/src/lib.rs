#![deny(unused_must_use)]
//! Top-level runtime orchestration: the `Runtime` struct, scheduler loop,
//! unit registry, commit coordination, and stable-ordering rules. No
//! architecture-specific logic; only traits and immutable data packets
//! cross crate boundaries.

pub mod commit;
pub mod hle;
pub mod registry;
pub mod rsx;
pub mod runtime;
pub mod scheduler;
pub mod syscall_table;

pub use commit::{BlockReason, CommitContext, CommitError, CommitOutcome, CommitPipeline};
pub use registry::{RegisteredUnit, UnitRegistry};
pub use runtime::{
    default_budget_for_mode, Runtime, RuntimeMode, RuntimeStep, SpuFactory, StepError,
};
pub use scheduler::{RoundRobinScheduler, Scheduler};
pub use syscall_table::SyscallResponseTable;
