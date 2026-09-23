//! The seeded hooks in test builds: one thread-local defect drives them.

use std::cell::Cell;

use cellgov_effects::EffectKind;
use cellgov_ppu::instruction::fuzz::PpuOutcomeClass;
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::observation::PpuObservation;
use cellgov_spu::fuzz::SpuOutcomeClass;
use cellgov_spu::instruction::SpuInstruction;
use cellgov_spu::observation::SpuAllowedFootprint;
use cellgov_spu::state::{SpuObservableSnapshot, SPU_REG_COUNT};

use super::SeededDefect;

thread_local! {
    static ACTIVE: Cell<Option<SeededDefect>> = const { Cell::new(None) };
}

/// Restores the previous seeding when dropped.
#[must_use = "the seeding ends when the guard drops, so bind it for the run"]
pub(crate) struct SeededGuard(Option<SeededDefect>);

impl Drop for SeededGuard {
    fn drop(&mut self) {
        ACTIVE.with(|active| active.set(self.0));
    }
}

/// Seeds `defect` for the current thread until the guard drops.
pub(crate) fn seed(defect: SeededDefect) -> SeededGuard {
    SeededGuard(ACTIVE.with(|active| active.replace(Some(defect))))
}

/// The defect seeded for the current thread.
pub(crate) fn active() -> Option<SeededDefect> {
    ACTIVE.with(Cell::get)
}

const PPU_OUTCOMES: [PpuOutcomeClass; 6] = [
    PpuOutcomeClass::Syscall,
    PpuOutcomeClass::Branch,
    PpuOutcomeClass::Continue,
    PpuOutcomeClass::Fault,
    PpuOutcomeClass::MemoryFault,
    PpuOutcomeClass::BufferFull,
];

const SPU_OUTCOMES: [SpuOutcomeClass; 5] = [
    SpuOutcomeClass::Yield,
    SpuOutcomeClass::Branch,
    SpuOutcomeClass::Continue,
    SpuOutcomeClass::MemoryRead,
    SpuOutcomeClass::Fault,
];

const EFFECTS: [EffectKind; 3] = [
    EffectKind::ClockRead,
    EffectKind::MailboxSend,
    EffectKind::TraceMarker,
];

/// Register value corruption every seeded state defect applies.
const CORRUPTION: u8 = 0x5a;

/// Decodes a PPU word at the decoder boundary.
pub(crate) fn ppu_decode(
    raw: u32,
) -> Result<PpuInstruction, cellgov_ppu::instruction::PpuDecodeError> {
    assert!(
        active() != Some(SeededDefect::DecoderPanic),
        "seeded decoder panic"
    );
    cellgov_ppu::decode::decode(raw)
}

/// Decodes an SPU word at the decoder boundary.
pub(crate) fn spu_decode(
    raw: u32,
) -> Result<SpuInstruction, cellgov_spu::instruction::SpuDecodeError> {
    assert!(
        active() != Some(SeededDefect::DecoderPanic),
        "seeded decoder panic"
    );
    cellgov_spu::decode::decode(raw)
}

/// Marks entry to an executor boundary.
pub(crate) fn executor_boundary() {
    assert!(
        active() != Some(SeededDefect::ExecutorPanic),
        "seeded executor panic"
    );
}

/// The outcome class the first PPU run reports.
pub(crate) fn ppu_outcome(observed: PpuOutcomeClass, legal: &[PpuOutcomeClass]) -> PpuOutcomeClass {
    if active() != Some(SeededDefect::IllegalOutcome) {
        return observed;
    }
    PPU_OUTCOMES
        .into_iter()
        .find(|class| !legal.contains(class))
        .unwrap_or(observed)
}

/// The outcome class the first SPU run reports.
pub(crate) fn spu_outcome(observed: SpuOutcomeClass, legal: &[SpuOutcomeClass]) -> SpuOutcomeClass {
    if active() != Some(SeededDefect::IllegalOutcome) {
        return observed;
    }
    SPU_OUTCOMES
        .into_iter()
        .find(|class| !legal.contains(class))
        .unwrap_or(observed)
}

/// An effect class the first run emits beyond the executor's own.
pub(crate) fn extra_effect(legal: &[EffectKind]) -> Option<EffectKind> {
    if active() != Some(SeededDefect::IllegalEffect) {
        return None;
    }
    EFFECTS.into_iter().find(|kind| !legal.contains(kind))
}

/// Corrupts every PPU observation the same way.
pub(crate) fn ppu_observed(observation: &mut PpuObservation) {
    if active() == Some(SeededDefect::CommonMode) {
        observation.state.gpr[0] ^= u64::from(CORRUPTION);
    }
}

/// Corrupts SPU registers after every execution of `instruction`.
///
/// The common-mode defect touches a register the footprint allows and
/// only when the outcome publishes register values; the footprint
/// defect touches one it forbids.
pub(crate) fn spu_observed(
    instruction: &SpuInstruction,
    outcome: SpuOutcomeClass,
    regs: &mut [[u8; 16]; SPU_REG_COUNT],
) {
    let footprint = SpuAllowedFootprint::for_instruction(instruction);
    let register = match active() {
        Some(SeededDefect::CommonMode)
            if !matches!(outcome, SpuOutcomeClass::Yield | SpuOutcomeClass::Fault) =>
        {
            footprint.registers.iter().next().copied()
        }
        Some(SeededDefect::IllegalFootprint) => (0..SPU_REG_COUNT as u8)
            .rev()
            .find(|register| !footprint.registers.contains(register)),
        _ => None,
    };
    if let Some(register) = register {
        regs[usize::from(register)][15] ^= CORRUPTION;
    }
}

/// The program counter an SPU sequence ends at.
pub(crate) fn spu_program_counter(pc: &mut u32) {
    if active() == Some(SeededDefect::InvalidProgramCounter) {
        *pc |= 2;
    }
}

/// Corrupts the PPU replay run only.
pub(crate) fn ppu_replayed(observation: &mut PpuObservation) {
    if active() == Some(SeededDefect::Nondeterministic) {
        observation.state.gpr[0] ^= u64::from(CORRUPTION);
    }
}

/// Corrupts the SPU replay run only.
pub(crate) fn spu_replayed(state: &mut SpuObservableSnapshot) {
    if active() == Some(SeededDefect::Nondeterministic) {
        state.regs[0][15] ^= CORRUPTION;
    }
}

/// Corrupts the PPU metamorphic partner only.
pub(crate) fn ppu_partner(observation: &mut PpuObservation) {
    if active() == Some(SeededDefect::MetamorphicMismatch) {
        observation.state.gpr[0] ^= u64::from(CORRUPTION);
    }
}

/// Corrupts the SPU metamorphic partner only.
pub(crate) fn spu_partner(state: &mut SpuObservableSnapshot) {
    if active() == Some(SeededDefect::MetamorphicMismatch) {
        state.regs[0][15] ^= CORRUPTION;
    }
}
