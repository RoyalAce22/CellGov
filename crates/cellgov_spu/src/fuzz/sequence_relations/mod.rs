//! Sequence relations: an SPU instruction sequence and a partner that must
//! leave the same observed state from the same start state.
//!
//! A partner is another guest sequence, or a fused reference: a typed
//! operation on the start state that writes a named set of registers, the
//! form a recompiler emits for a fused sequence. Rows name registers
//! symbolically, numbered in order of first appearance, so one row covers
//! every register assignment. The rows come from the fusions a recompiler
//! applies to SPU code: compare and select, the 32-bit multiply, element
//! insertion, negated shift counts, branch tests, split local-store
//! addresses, pass-through moves, and the float estimate, Newton, square
//! root, division and minimum or maximum fusions.

mod catalog;
mod describe;
mod lanes;
mod rows_branch;
mod rows_compare;
mod rows_float;
mod rows_integer;
mod rows_memory;
mod rows_shuffle;
mod types;

pub use catalog::sequence_relations;
pub use describe::{isa_citation, SpuSequenceRelationText};
pub use lanes::ulp_distance;
pub use types::*;

#[cfg(test)]
#[path = "tests/sequence_relations_tests.rs"]
mod tests;
