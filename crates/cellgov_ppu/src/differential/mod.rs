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
//! Every case is single-shot, single-unit, and reads no host state.

mod case;
mod context;
mod runner;

pub mod corpora;
pub mod rpcs3_capture;

pub use case::{InstructionCase, MemorySnapshot, OracleSource, PpuStateSnapshot};
pub use context::is_context_dependent;
pub use runner::{
    assert_case, run_case, run_corpus, CaseOutcome, CorpusReport, MemoryByteMismatch, StateDiff,
};
