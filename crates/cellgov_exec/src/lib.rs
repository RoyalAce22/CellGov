//! Seam between architecture interpreters and the runtime.
//!
//! Owns the `ExecutionUnit` trait, its input `ExecutionContext`, its
//! return-shape `ExecutionStepResult`, the `YieldReason` vocabulary,
//! and a `FakeIsaUnit` for pressure-testing the runtime contract
//! without a real arch unit.
//!
//! No dependency on a concrete scheduler. Architecture-specific
//! decoding lives in `cellgov_ppu` / `cellgov_spu`, not here.

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
