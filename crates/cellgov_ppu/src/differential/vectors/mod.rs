//! Spec-derived vectors for the differential harness.
//!
//! Each generator returns a `Vec<InstructionCase>` keyed off the
//! PowerPC / Cell spec definition of its target class. The expected
//! post-state is computed from the spec, not from CellGov, so a
//! vector run that passes confirms the executor matches the spec
//! transcription. The [`super::OracleSource::Spec`] tag carries the
//! per-instruction citation.
//! [McKeeman1998 p:101 s:Seeking an Oracle] A check that a result has
//! not changed proves nothing unless the result is known to be
//! correct, so the expected state comes from outside the executor.

pub mod altivec_memory_loads;
pub mod altivec_memory_stores;
mod builders;
pub mod byte_reverse;
pub mod cell_unaligned_vxu_stores;

use builders::{case_keep_memory, state_with_gpr, state_with_three_gprs, state_with_two_gprs};
