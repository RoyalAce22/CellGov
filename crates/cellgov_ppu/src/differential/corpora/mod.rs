//! Spec-derived corpora for the differential harness.
//!
//! Each generator returns a `Vec<InstructionCase>` keyed off the
//! PowerPC / Cell spec definition of its target class. The expected
//! post-state is computed from the spec, not from CellGov, so a
//! corpus run that passes confirms the executor matches the spec
//! transcription. The [`super::OracleSource::Spec`] tag carries the
//! per-instruction citation.

pub mod altivec_memory_loads;
pub mod altivec_memory_stores;
mod builders;
pub mod byte_reverse;
pub mod cell_unaligned_vxu_stores;

use builders::{case_keep_memory, state_with_gpr, state_with_three_gprs, state_with_two_gprs};
