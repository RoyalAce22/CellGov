//! The seeded hooks outside test builds: every one is an identity.

use cellgov_effects::EffectKind;
use cellgov_ppu::instruction::fuzz::PpuOutcomeClass;
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::observation::PpuObservation;
use cellgov_spu::fuzz::SpuOutcomeClass;
use cellgov_spu::instruction::SpuInstruction;
use cellgov_spu::state::{SpuObservableSnapshot, SPU_REG_COUNT};

/// Decodes a PPU word at the decoder boundary.
#[inline(always)]
pub(crate) fn ppu_decode(
    raw: u32,
) -> Result<PpuInstruction, cellgov_ppu::instruction::PpuDecodeError> {
    cellgov_ppu::decode::decode(raw)
}

/// Decodes an SPU word at the decoder boundary.
#[inline(always)]
pub(crate) fn spu_decode(
    raw: u32,
) -> Result<SpuInstruction, cellgov_spu::instruction::SpuDecodeError> {
    cellgov_spu::decode::decode(raw)
}

/// Marks entry to an executor boundary.
#[inline(always)]
pub(crate) fn executor_boundary() {}

/// The outcome class the first PPU run reports.
#[inline(always)]
pub(crate) fn ppu_outcome(
    observed: PpuOutcomeClass,
    _legal: &[PpuOutcomeClass],
) -> PpuOutcomeClass {
    observed
}

/// The outcome class the first SPU run reports.
#[inline(always)]
pub(crate) fn spu_outcome(
    observed: SpuOutcomeClass,
    _legal: &[SpuOutcomeClass],
) -> SpuOutcomeClass {
    observed
}

/// An effect class the first run emits beyond the executor's own.
#[inline(always)]
pub(crate) fn extra_effect(_legal: &[EffectKind]) -> Option<EffectKind> {
    None
}

/// Corrupts every PPU observation the same way.
#[inline(always)]
pub(crate) fn ppu_observed(_observation: &mut PpuObservation) {}

/// Corrupts SPU registers after every execution of `instruction`.
#[inline(always)]
pub(crate) fn spu_observed(
    _instruction: &SpuInstruction,
    _outcome: SpuOutcomeClass,
    _regs: &mut [[u8; 16]; SPU_REG_COUNT],
) {
}

/// The program counter an SPU sequence ends at.
#[inline(always)]
pub(crate) fn spu_program_counter(_pc: &mut u32) {}

/// Corrupts the PPU replay run only.
#[inline(always)]
pub(crate) fn ppu_replayed(_observation: &mut PpuObservation) {}

/// Corrupts the SPU replay run only.
#[inline(always)]
pub(crate) fn spu_replayed(_state: &mut SpuObservableSnapshot) {}

/// Corrupts the PPU metamorphic partner only.
#[inline(always)]
pub(crate) fn ppu_partner(_observation: &mut PpuObservation) {}

/// Corrupts the SPU metamorphic partner only.
#[inline(always)]
pub(crate) fn spu_partner(_state: &mut SpuObservableSnapshot) {}
