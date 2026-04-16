//! cellgov_core -- top-level runtime orchestration.
//!
//! Owns:
//!
//! - the `Runtime` struct
//! - the scheduler loop
//! - the unit registry seam (assigns `UnitId`, records constructor params,
//!   handles dynamic spawning through the same path used by static units)
//! - commit cycle coordination
//! - stable ordering rules
//!
//! No device-specific logic beyond orchestration. Concrete scheduler types
//! must not be exposed across crate boundaries -- only traits and small
//! immutable data packets cross.
//!
//! Runtime pipeline (one pass per unit yield):
//!
//! 1. select runnable unit deterministically
//! 2. grant budget
//! 3. run unit until yield
//! 4. collect emitted effects
//! 5. validate effects
//! 6. stage commit batch
//! 7. apply commit batch to shared visible state
//! 8. inject resulting events/wakeups via status overrides, the DMA
//!    completion queue, and the syscall response table
//! 9. advance guest time / epoch deterministically
//! 10. trace every decision

pub mod commit;
mod hle;
pub mod hle_context;
mod hle_gcm;
mod hle_sys;
pub mod registry;
pub mod runtime;
pub mod scheduler;
pub mod syscall_table;

pub use commit::{BlockReason, CommitContext, CommitError, CommitOutcome, CommitPipeline};
pub use registry::{RegisteredUnit, UnitRegistry};
pub use runtime::{Runtime, RuntimeMode, RuntimeStep, SpuFactory, StepError};
pub use scheduler::{RoundRobinScheduler, Scheduler};
pub use syscall_table::SyscallResponseTable;
