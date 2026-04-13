//! cellgov_exec -- generic execution unit trait and resumable unit interface.
//!
//! Owns:
//!
//! - the `ExecutionUnit` trait
//! - `ExecutionContext` (readonly view exposed to a running unit)
//! - `YieldReason` enum
//! - `ExecutionStepResult` (yield reason, consumed budget, emitted effects in
//!   stable emission order, local diagnostics, optional fault data)
//!
//! Must not depend on a concrete scheduler implementation. Fake PPU/SPU units
//! for early testing live here too. Architecture-specific decoding does not.
//!
//! `Self::Snapshot` must be pure deterministic data: no raw pointers, no host
//! handles, no allocator-dependent internals. Snapshots must be reconstructible
//! into an equivalent unit state on a different host.

pub mod context;
pub mod fake_isa;
pub mod step_result;
pub mod unit;
pub mod yield_reason;

pub use context::ExecutionContext;
pub use fake_isa::{FakeIsaUnit, FakeOp};
pub use step_result::{ExecutionStepResult, FaultRegisterDump, LocalDiagnostics};
pub use unit::{ExecutionUnit, UnitStatus};
pub use yield_reason::YieldReason;
