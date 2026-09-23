//! Per-instruction differential harness.
//!
//! Each [`InstructionCase`] is a `(initial_state, initial_memory,
//! raw_instruction, expected_state, expected_memory)` tuple plus an
//! [`OracleSource`] tag. The runner loads the initial state, decodes
//! through [`crate::decode::decode`], executes through
//! [`crate::exec::execute`], applies any
//! [`Effect::SharedWriteIntent`](cellgov_effects::Effect::SharedWriteIntent)
//! the executor staged to a memory copy, and diffs the post-state and
//! memory against the expected values.
//!
//! [Martignoni2009 p:127 s:2.3] Both machines start from one synthetic
//! state, execute the instruction at pc, and their resulting states
//! are compared; any difference proves the emulation unfaithful for
//! that state.
//!
//! Every case is single-shot, single-unit, and reads no host state.

mod case;
mod context;
mod runner;

pub mod rpcs3_capture;
pub mod vectors;

pub use case::{InstructionCase, MemorySnapshot, OracleSource, PpuStateSnapshot};
pub use context::is_context_dependent;
pub use runner::{
    assert_case, execute_into_memory, run_case, run_vectors, CaseOutcome, MemoryByteMismatch,
    StateDiff, VectorReport,
};
