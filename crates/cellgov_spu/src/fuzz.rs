//! Interpreter-owned contracts used by instruction fuzzers.

use cellgov_effects::EffectKind;
use cellgov_ps3_abi::hw::spu;

use crate::instruction::{SpuInstruction, SpuInstructionKind};

/// Encoding form used by an SPU instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuEncodingForm {
    /// Three-field register form.
    Rrr,
    /// Four-field register form.
    Rrrr,
    /// Register plus 7-bit immediate.
    Ri7,
    /// Register plus 10-bit immediate.
    Ri10,
    /// Register plus 16-bit immediate.
    Ri16,
    /// Register plus 18-bit immediate.
    Ri18,
    /// Branch encoding family.
    Branch,
    /// Channel encoding family.
    Channel,
    /// Control and hint encoding family.
    Control,
}

/// Observable SPU state used for comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuObservableState {
    /// Registers, local store, PC, channels, reservation, outcome, and effects.
    Complete,
}

/// Legal result class for one SPU instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuOutcomeClass {
    /// Ordinary completion.
    Continue,
    /// Explicit PC change.
    Branch,
    /// Runtime yield with effects.
    Yield,
    /// Caller-serviced committed-memory read.
    MemoryRead,
    /// Architectural fault.
    Fault,
}

impl SpuOutcomeClass {
    /// Classify an executor outcome exhaustively.
    pub fn from_outcome(outcome: &crate::exec::SpuStepOutcome) -> Self {
        match outcome {
            crate::exec::SpuStepOutcome::Continue => Self::Continue,
            crate::exec::SpuStepOutcome::Branch => Self::Branch,
            crate::exec::SpuStepOutcome::Yield { .. } => Self::Yield,
            crate::exec::SpuStepOutcome::MemoryRead { .. } => Self::MemoryRead,
            crate::exec::SpuStepOutcome::Fault(_) => Self::Fault,
        }
    }
}

/// Interpreter self-relation suitable for fuzz checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuMetamorphicRelation {
    /// Identical inputs give identical outputs.
    Deterministic,
}

/// Complete fuzz contract for one decoded SPU instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpuFuzzDescriptor {
    /// Stable instruction identity.
    pub kind: SpuInstructionKind,
    /// Encoding form.
    pub form: SpuEncodingForm,
    /// State projection used for comparison.
    pub observable_state: SpuObservableState,
    /// Effect variants this instruction may return in a yield outcome.
    pub effects: &'static [EffectKind],
    /// Result classes accepted from execution.
    pub outcomes: &'static [SpuOutcomeClass],
    /// Relations that apply to this instruction.
    pub relations: &'static [SpuMetamorphicRelation],
}

const NO_EFFECTS: &[EffectKind] = &[];
const RDCH_EFFECTS: &[EffectKind] = &[EffectKind::MailboxReceiveAttempt];
const WRCH_EFFECTS: &[EffectKind] = &[EffectKind::DmaEnqueue, EffectKind::ConditionalStore];
// The fuzz engine rejects outcomes outside these executor-derived sets.
const CONTINUE: &[SpuOutcomeClass] = &[SpuOutcomeClass::Continue];
const FAULT: &[SpuOutcomeClass] = &[SpuOutcomeClass::Fault];
const YIELD: &[SpuOutcomeClass] = &[SpuOutcomeClass::Yield];
const CONTINUE_OR_YIELD: &[SpuOutcomeClass] = &[SpuOutcomeClass::Continue, SpuOutcomeClass::Yield];
const LOAD_STORE: &[SpuOutcomeClass] = &[SpuOutcomeClass::Continue, SpuOutcomeClass::Fault];
const CONDITIONAL_BRANCH: &[SpuOutcomeClass] =
    &[SpuOutcomeClass::Continue, SpuOutcomeClass::Branch];
const UNCONDITIONAL_BRANCH: &[SpuOutcomeClass] = &[SpuOutcomeClass::Branch];
const WRCH: &[SpuOutcomeClass] = &[
    SpuOutcomeClass::Continue,
    SpuOutcomeClass::Yield,
    SpuOutcomeClass::MemoryRead,
    SpuOutcomeClass::Fault,
];
const RELATIONS: &[SpuMetamorphicRelation] = &[SpuMetamorphicRelation::Deterministic];

impl SpuInstruction {
    /// Return the interpreter-owned fuzz contract for this instruction.
    pub fn fuzz_descriptor(&self) -> SpuFuzzDescriptor {
        let kind = SpuInstructionKind::from(*self);
        classify_kind(kind);
        let (effects, outcomes) = effect_and_outcome(self);
        SpuFuzzDescriptor {
            kind,
            form: form_for_kind(kind),
            observable_state: SpuObservableState::Complete,
            effects,
            outcomes,
            relations: RELATIONS,
        }
    }
}

fn effect_and_outcome(
    instruction: &SpuInstruction,
) -> (&'static [EffectKind], &'static [SpuOutcomeClass]) {
    match *instruction {
        SpuInstruction::Lqd { .. }
        | SpuInstruction::Lqx { .. }
        | SpuInstruction::Lqa { .. }
        | SpuInstruction::Lqr { .. }
        | SpuInstruction::Stqd { .. }
        | SpuInstruction::Stqx { .. }
        | SpuInstruction::Stqa { .. }
        | SpuInstruction::Stqr { .. } => (NO_EFFECTS, LOAD_STORE),
        SpuInstruction::Rdch {
            channel: spu::MFC_RD_TAG_STAT,
            ..
        } => (NO_EFFECTS, CONTINUE_OR_YIELD),
        SpuInstruction::Rdch {
            channel: spu::SPU_RD_IN_MBOX,
            ..
        } => (RDCH_EFFECTS, YIELD),
        SpuInstruction::Rdch {
            channel: spu::MFC_RD_ATOMIC_STAT | spu::SPU_RD_MACH_STAT,
            ..
        } => (NO_EFFECTS, CONTINUE),
        SpuInstruction::Rdch { .. } => (NO_EFFECTS, FAULT),
        SpuInstruction::Wrch {
            channel: spu::MFC_CMD,
            ..
        } => (WRCH_EFFECTS, WRCH),
        SpuInstruction::Wrch {
            channel:
                spu::MFC_LSA
                | spu::MFC_EAH
                | spu::MFC_EAL
                | spu::MFC_SIZE
                | spu::MFC_TAG_ID
                | spu::MFC_WR_TAG_MASK
                | spu::MFC_WR_TAG_UPDATE
                | spu::SPU_WR_OUT_MBOX,
            ..
        } => (NO_EFFECTS, CONTINUE),
        SpuInstruction::Wrch { .. } => (NO_EFFECTS, FAULT),
        SpuInstruction::Rchcnt {
            channel: spu::SPU_RD_MACH_STAT,
            ..
        } => (NO_EFFECTS, CONTINUE),
        SpuInstruction::Rchcnt { .. } => (NO_EFFECTS, FAULT),
        SpuInstruction::Br { .. }
        | SpuInstruction::Brsl { .. }
        | SpuInstruction::Bi { .. }
        | SpuInstruction::Bisl { .. } => (NO_EFFECTS, UNCONDITIONAL_BRANCH),
        SpuInstruction::Brz { .. }
        | SpuInstruction::Brnz { .. }
        | SpuInstruction::Brhnz { .. }
        | SpuInstruction::Brhz { .. }
        | SpuInstruction::Biz { .. }
        | SpuInstruction::Binz { .. }
        | SpuInstruction::Bihz { .. }
        | SpuInstruction::Bihnz { .. } => (NO_EFFECTS, CONDITIONAL_BRANCH),
        SpuInstruction::Stop { .. } => (NO_EFFECTS, YIELD),
        _ => (NO_EFFECTS, CONTINUE),
    }
}

/// Produces a simpler encoding when it keeps the instruction kind.
pub fn simplify_instruction_bit(raw: u32, bit_index: u32) -> Option<u32> {
    let bit = 1u32.checked_shl(bit_index)?;
    (raw & bit != 0).then_some(())?;
    let instruction = crate::decode::decode(raw).ok()?;
    let kind = SpuInstructionKind::from(instruction);
    let candidate = raw & !bit;
    let decoded = crate::decode::decode(candidate).ok()?;
    (SpuInstructionKind::from(decoded) == kind).then_some(candidate)
}

/// Produces valid same-kind candidates for fuzz-engine shrinking.
pub fn shrink_instruction(raw: u32) -> Vec<u32> {
    if crate::decode::decode(raw).is_err() {
        return Vec::new();
    }
    (0..u32::BITS)
        .filter_map(|bit_index| simplify_instruction_bit(raw, bit_index))
        .collect()
}

fn form_for_kind(kind: SpuInstructionKind) -> SpuEncodingForm {
    use SpuInstructionKind as K;
    match kind {
        K::Selb | K::Shufb => SpuEncodingForm::Rrrr,
        // [SPU-ISA p:29 s:2.3 Instruction Formats] RI10 carries I10 between its opcode and RA fields.
        K::Lqd | K::Stqd | K::Ai | K::Ori | K::Andi | K::Ceqi | K::Ceqbi | K::Cgti => {
            SpuEncodingForm::Ri10
        }
        K::Cbd
        | K::Chd
        | K::Cwd
        | K::Cdd
        | K::Shlqbyi
        | K::Rotqbyi
        | K::Rotqmbyi
        | K::Shli
        | K::Rotmi
        | K::Rotmai => SpuEncodingForm::Ri7,
        K::Lqa | K::Stqa | K::Lqr | K::Stqr | K::Il | K::Ilh | K::Ilhu | K::Iohl | K::Fsmbi => {
            SpuEncodingForm::Ri16
        }
        K::Ila => SpuEncodingForm::Ri18,
        K::Br
        | K::Brsl
        | K::Brz
        | K::Brnz
        | K::Bi
        | K::Bisl
        | K::Brhnz
        | K::Brhz
        | K::Biz
        | K::Binz
        | K::Bihz
        | K::Bihnz => SpuEncodingForm::Branch,
        K::Rdch | K::Wrch | K::Rchcnt => SpuEncodingForm::Channel,
        K::Nop | K::Lnop | K::Hbr | K::Hbra | K::Hbrr | K::Sync | K::Dsync | K::Heq | K::Stop => {
            SpuEncodingForm::Control
        }
        _ => SpuEncodingForm::Rrr,
    }
}

fn classify_kind(kind: SpuInstructionKind) {
    match kind {
        SpuInstructionKind::Lqd
        | SpuInstructionKind::Lqx
        | SpuInstructionKind::Lqa
        | SpuInstructionKind::Stqd
        | SpuInstructionKind::Stqx
        | SpuInstructionKind::Stqa
        | SpuInstructionKind::Lqr
        | SpuInstructionKind::Stqr
        | SpuInstructionKind::Il
        | SpuInstructionKind::Ila
        | SpuInstructionKind::Ilh
        | SpuInstructionKind::Ilhu
        | SpuInstructionKind::Iohl
        | SpuInstructionKind::Fsmbi
        | SpuInstructionKind::A
        | SpuInstructionKind::Ai
        | SpuInstructionKind::Sf
        | SpuInstructionKind::And
        | SpuInstructionKind::Or
        | SpuInstructionKind::Selb
        | SpuInstructionKind::Xsbh
        | SpuInstructionKind::Gb
        | SpuInstructionKind::Gbh
        | SpuInstructionKind::Ori
        | SpuInstructionKind::Nor
        | SpuInstructionKind::Andi
        | SpuInstructionKind::Shufb
        | SpuInstructionKind::Shlqbyi
        | SpuInstructionKind::Rotqby
        | SpuInstructionKind::Rotqbyi
        | SpuInstructionKind::Rotqmbyi
        | SpuInstructionKind::Shl
        | SpuInstructionKind::Shli
        | SpuInstructionKind::Rotmi
        | SpuInstructionKind::Rotmai
        | SpuInstructionKind::Cbd
        | SpuInstructionKind::Cbx
        | SpuInstructionKind::Chd
        | SpuInstructionKind::Chx
        | SpuInstructionKind::Cwd
        | SpuInstructionKind::Cwx
        | SpuInstructionKind::Cdd
        | SpuInstructionKind::Cdx
        | SpuInstructionKind::Ceq
        | SpuInstructionKind::Ceqi
        | SpuInstructionKind::Ceqbi
        | SpuInstructionKind::Cgti
        | SpuInstructionKind::Clgt
        | SpuInstructionKind::Br
        | SpuInstructionKind::Brsl
        | SpuInstructionKind::Brz
        | SpuInstructionKind::Brnz
        | SpuInstructionKind::Bi
        | SpuInstructionKind::Bisl
        | SpuInstructionKind::Brhnz
        | SpuInstructionKind::Brhz
        | SpuInstructionKind::Biz
        | SpuInstructionKind::Binz
        | SpuInstructionKind::Bihz
        | SpuInstructionKind::Bihnz
        | SpuInstructionKind::Rdch
        | SpuInstructionKind::Wrch
        | SpuInstructionKind::Rchcnt
        | SpuInstructionKind::Nop
        | SpuInstructionKind::Lnop
        | SpuInstructionKind::Hbr
        | SpuInstructionKind::Hbra
        | SpuInstructionKind::Hbrr
        | SpuInstructionKind::Sync
        | SpuInstructionKind::Dsync
        | SpuInstructionKind::Heq
        | SpuInstructionKind::Stop => {}
    }
}

#[cfg(test)]
#[path = "tests/fuzz_tests.rs"]
mod tests;
