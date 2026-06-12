//! Spec-derived corpora for the differential harness.
//!
//! Each generator returns a `Vec<InstructionCase>` keyed off the
//! PowerPC / Cell spec definition of its target class. The expected
//! post-state is computed from the spec, not from CellGov, so a
//! corpus run that passes confirms the executor matches the spec
//! transcription. The [`super::OracleSource::Spec`] tag carries the
//! per-instruction citation.

use super::{InstructionCase, MemorySnapshot, OracleSource, PpuStateSnapshot};
use cellgov_ps3_abi::hardware::GPR_COUNT;

pub mod altivec_memory_loads;
pub mod altivec_memory_stores;
pub mod byte_reverse;
pub mod cell_unaligned_vxu_stores;

/// Zeroed state with one GPR pre-set to `value`.
pub(super) fn state_with_gpr(index: usize, value: u64) -> PpuStateSnapshot {
    debug_assert!(index < GPR_COUNT);
    let mut s = PpuStateSnapshot::zero();
    s.gpr[index] = value;
    s
}

/// Zeroed state with two GPRs pre-set.
pub(super) fn state_with_two_gprs(
    ra_index: usize,
    ra_value: u64,
    rb_index: usize,
    rb_value: u64,
) -> PpuStateSnapshot {
    debug_assert!(ra_index < GPR_COUNT && rb_index < GPR_COUNT);
    let mut s = PpuStateSnapshot::zero();
    s.gpr[ra_index] = ra_value;
    s.gpr[rb_index] = rb_value;
    s
}

/// Zeroed state with three GPRs pre-set.
pub(super) fn state_with_three_gprs(
    a: (usize, u64),
    b: (usize, u64),
    c: (usize, u64),
) -> PpuStateSnapshot {
    debug_assert!(a.0 < GPR_COUNT && b.0 < GPR_COUNT && c.0 < GPR_COUNT);
    let mut s = PpuStateSnapshot::zero();
    s.gpr[a.0] = a.1;
    s.gpr[b.0] = b.1;
    s.gpr[c.0] = c.1;
    s
}

/// Build an [`InstructionCase`] tagged [`OracleSource::Spec`].
pub(super) fn case_keep_memory(
    label: impl Into<String>,
    raw: u32,
    initial_state: PpuStateSnapshot,
    initial_memory: MemorySnapshot,
    expected_state: PpuStateSnapshot,
    expected_memory: MemorySnapshot,
    rationale: &'static str,
) -> InstructionCase {
    InstructionCase {
        label: label.into(),
        initial_state,
        initial_memory,
        raw_instruction: raw,
        expected_state,
        expected_memory,
        source: OracleSource::Spec { rationale },
    }
}
