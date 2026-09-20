//! Defines interpreter-owned PPU instruction contracts for fuzzers.

use cellgov_effects::EffectKind;

use super::ops::{Fp59Op, Fp63Op, VaOp, VxOp};
use super::{PpuInstruction, PpuInstructionKind};

/// Encoding form used by a decoded PPU instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuEncodingForm {
    /// PowerPC D-form encoding.
    D,
    /// PowerPC DS-form encoding.
    Ds,
    /// PowerPC I-form encoding.
    I,
    /// PowerPC B-form encoding.
    B,
    /// PowerPC X or XO-form encoding.
    X,
    /// PowerPC XL-form encoding.
    Xl,
    /// PowerPC M-form encoding.
    M,
    /// PowerPC MD or MDS-form encoding.
    Md,
    /// AltiVec VA or VX-form encoding.
    Vector,
    /// Floating-point A or X-form encoding.
    Float,
    /// System-call encoding.
    SystemCall,
    /// A predecoded synthetic instruction with no standalone encoding.
    Synthetic,
}

/// Exact fuzz identity for a decoded PPU instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuFuzzKind {
    /// An instruction represented by its ordinary typed variant.
    Ordinary(PpuInstructionKind),
    /// One operation in the generic VX family.
    Vx(VxOp),
    /// One operation in the generic VA family.
    Va(VaOp),
    /// One operation in the generic primary-59 floating-point family.
    Fp59(Fp59Op),
    /// One operation in the generic primary-63 floating-point family.
    Fp63(Fp63Op),
}

impl PartialOrd for PpuFuzzKind {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PpuFuzzKind {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        fuzz_kind_order(*self).cmp(&fuzz_kind_order(*other))
    }
}

fn fuzz_kind_order(kind: PpuFuzzKind) -> (u8, u16) {
    match kind {
        PpuFuzzKind::Ordinary(kind) => (0, kind as u16),
        PpuFuzzKind::Vx(op) => (1, op as u16),
        PpuFuzzKind::Va(op) => (2, op as u16),
        PpuFuzzKind::Fp59(op) => (3, op as u16),
        PpuFuzzKind::Fp63(op) => (4, op as u16),
    }
}

/// Observable state compared after PPU execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuObservableState {
    /// GPR, FPR, VR, CR, LR, CTR, XER, PC, reservation, memory, and effects.
    Complete,
}

/// Legal result class for one PPU instruction step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuOutcomeClass {
    /// Ordinary completion.
    Continue,
    /// Explicit PC change.
    Branch,
    /// System-call boundary.
    Syscall,
    /// Architectural fault.
    Fault,
    /// Unmapped-memory fault.
    MemoryFault,
    /// Caller must flush the store buffer and retry.
    BufferFull,
}

impl PpuOutcomeClass {
    /// Classify an executor verdict exhaustively.
    pub fn from_verdict(verdict: &crate::exec::ExecuteVerdict) -> Self {
        match verdict {
            crate::exec::ExecuteVerdict::Continue => Self::Continue,
            crate::exec::ExecuteVerdict::Branch => Self::Branch,
            crate::exec::ExecuteVerdict::Syscall { .. } => Self::Syscall,
            crate::exec::ExecuteVerdict::Fault(_) => Self::Fault,
            crate::exec::ExecuteVerdict::MemFault(_) => Self::MemoryFault,
            crate::exec::ExecuteVerdict::BufferFull => Self::BufferFull,
        }
    }
}

/// Interpreter self-relation suitable for fuzz checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuMetamorphicRelation {
    /// Identical inputs give identical outputs.
    Deterministic,
}

/// Complete fuzz contract for one decoded PPU instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PpuFuzzDescriptor {
    /// Stable instruction identity.
    pub kind: PpuFuzzKind,
    /// Encoding form of the decoded instruction.
    pub form: PpuEncodingForm,
    /// State projection used for comparison.
    pub observable_state: PpuObservableState,
    /// Effect variants this instruction may emit.
    pub effects: &'static [EffectKind],
    /// Result classes accepted from execution.
    pub outcomes: &'static [PpuOutcomeClass],
    /// Relations that apply to this instruction.
    pub relations: &'static [PpuMetamorphicRelation],
}

const NO_EFFECTS: &[EffectKind] = &[];
const READ_EFFECTS: &[EffectKind] = &[EffectKind::SharedReadIntent];
// A successful committed-memory load-reserve step emits both effect packets.
const RESERVATION_READ_EFFECTS: &[EffectKind] =
    &[EffectKind::SharedReadIntent, EffectKind::ReservationAcquire];
const WRITE_EFFECTS: &[EffectKind] = &[EffectKind::SharedWriteIntent];
const ATOMIC_STORE_EFFECTS: &[EffectKind] = &[EffectKind::ConditionalStore];
const CLOCK_EFFECTS: &[EffectKind] = &[EffectKind::ClockRead];
const CONTINUE: &[PpuOutcomeClass] = &[PpuOutcomeClass::Continue];
const FAULT: &[PpuOutcomeClass] = &[PpuOutcomeClass::Fault];
const CONTINUE_OR_FAULT: &[PpuOutcomeClass] = &[PpuOutcomeClass::Continue, PpuOutcomeClass::Fault];
const LOAD: &[PpuOutcomeClass] = &[PpuOutcomeClass::Continue, PpuOutcomeClass::MemoryFault];
const STORE: &[PpuOutcomeClass] = &[PpuOutcomeClass::Continue, PpuOutcomeClass::BufferFull];
const ATOMIC_LOAD: &[PpuOutcomeClass] = &[
    PpuOutcomeClass::Continue,
    PpuOutcomeClass::Fault,
    PpuOutcomeClass::MemoryFault,
];
const ATOMIC_STORE: &[PpuOutcomeClass] = &[
    PpuOutcomeClass::Continue,
    PpuOutcomeClass::Fault,
    PpuOutcomeClass::MemoryFault,
    PpuOutcomeClass::BufferFull,
];
const UNCONDITIONAL_BRANCH: &[PpuOutcomeClass] = &[PpuOutcomeClass::Branch];
const BRANCH: &[PpuOutcomeClass] = &[PpuOutcomeClass::Continue, PpuOutcomeClass::Branch];
const SYSCALL: &[PpuOutcomeClass] = &[PpuOutcomeClass::Syscall];
const RELATIONS: &[PpuMetamorphicRelation] = &[PpuMetamorphicRelation::Deterministic];

impl PpuInstruction {
    /// Return the interpreter-owned fuzz contract for this instruction.
    pub fn fuzz_descriptor(&self, raw: u32) -> PpuFuzzDescriptor {
        let instruction_kind = PpuInstructionKind::from(*self);
        classify_kind(instruction_kind);
        let (effects, outcomes) = effect_and_outcome(instruction_kind);
        PpuFuzzDescriptor {
            kind: fuzz_kind(*self),
            form: form_for_word(instruction_kind, raw),
            observable_state: PpuObservableState::Complete,
            effects,
            outcomes,
            relations: RELATIONS,
        }
    }
}

/// Clear one raw bit and retain only a decodable encoding of the exact same kind.
pub fn simplify_bit(raw: u32, bit: u8) -> Option<u32> {
    if bit >= u32::BITS as u8 || raw & (1u32 << bit) == 0 {
        return None;
    }
    let instruction = crate::decode::decode(raw).ok()?;
    let kind = instruction.fuzz_descriptor(raw).kind;
    let candidate = raw & !(1u32 << bit);
    let decoded = crate::decode::decode(candidate).ok()?;
    (decoded.fuzz_descriptor(candidate).kind == kind).then_some(candidate)
}

/// Return every valid exact-kind encoding produced by clearing one set bit.
pub fn simplify_encoding(raw: u32) -> Vec<u32> {
    if crate::decode::decode(raw).is_err() {
        return Vec::new();
    }
    (0..u32::BITS as u8)
        .filter_map(|bit| simplify_bit(raw, bit))
        .collect()
}

fn fuzz_kind(instruction: PpuInstruction) -> PpuFuzzKind {
    match instruction {
        PpuInstruction::Vx { op, .. } => PpuFuzzKind::Vx(op),
        PpuInstruction::Va { op, .. } => PpuFuzzKind::Va(op),
        PpuInstruction::Fp59 { op, .. } => PpuFuzzKind::Fp59(op),
        PpuInstruction::Fp63 { op, .. } => PpuFuzzKind::Fp63(op),
        ordinary => PpuFuzzKind::Ordinary(PpuInstructionKind::from(ordinary)),
    }
}

fn form_for_word(kind: PpuInstructionKind, raw: u32) -> PpuEncodingForm {
    use PpuInstructionKind as K;
    if matches!(
        kind,
        K::Li
            | K::Mr
            | K::Slwi
            | K::Srwi
            | K::Clrlwi
            | K::Nop
            | K::CmpwZero
            | K::Clrldi
            | K::Sldi
            | K::Srdi
            | K::LwzCmpwi
            | K::LiStw
            | K::MflrStw
            | K::LwzMtlr
            | K::MflrStd
            | K::LdMtlr
            | K::StdStd
            | K::CmpwiBc
            | K::CmpwBc
            | K::Consumed
    ) {
        return PpuEncodingForm::Synthetic;
    }
    match raw >> 26 {
        4 => PpuEncodingForm::Vector,
        16 => PpuEncodingForm::B,
        18 => PpuEncodingForm::I,
        19 => PpuEncodingForm::Xl,
        20 | 21 | 23 => PpuEncodingForm::M,
        30 => PpuEncodingForm::Md,
        31 => PpuEncodingForm::X,
        58 | 62 => PpuEncodingForm::Ds,
        59 | 63 => PpuEncodingForm::Float,
        17 => PpuEncodingForm::SystemCall,
        _ => PpuEncodingForm::D,
    }
}

fn effect_and_outcome(
    kind: PpuInstructionKind,
) -> (&'static [EffectKind], &'static [PpuOutcomeClass]) {
    use PpuInstructionKind as K;
    match kind {
        K::Lwz
        | K::Lbz
        | K::Lhz
        | K::Lha
        | K::Lhau
        | K::Lmw
        | K::Lwzu
        | K::Lbzu
        | K::Lhzu
        | K::Ldu
        | K::Ld
        | K::Lwa
        | K::Lwzx
        | K::Lbzx
        | K::Ldx
        | K::Lhzx
        | K::Lwzux
        | K::Lbzux
        | K::Lhzux
        | K::Ldux
        | K::Lhax
        | K::Lhaux
        | K::Lwax
        | K::Lwaux
        | K::Lswi
        | K::Lswx
        | K::Ldbrx
        | K::Lwbrx
        | K::Lhbrx
        | K::Lvlx
        | K::Lvrx
        | K::Lvlxl
        | K::Lvrxl
        | K::Lvsl
        | K::Lvebx
        | K::Lvsr
        | K::Lvehx
        | K::Lvewx
        | K::Lvx
        | K::Lvxl
        | K::Lfs
        | K::Lfsu
        | K::Lfd
        | K::Lfdu
        | K::Lfsx
        | K::Lfsux
        | K::Lfdx
        | K::Lfdux
        | K::LwzCmpwi
        | K::LwzMtlr
        | K::LdMtlr => (READ_EFFECTS, LOAD),
        K::Stw
        | K::Stwu
        | K::Stdu
        | K::Stb
        | K::Stbu
        | K::Stmw
        | K::Sth
        | K::Sthu
        | K::Std
        | K::Stwx
        | K::Stdx
        | K::Stdux
        | K::Stbx
        | K::Sthx
        | K::Sthux
        | K::Stwux
        | K::Stbux
        | K::Stswi
        | K::Stswx
        | K::Sdbrx
        | K::Stwbrx
        | K::Sthbrx
        | K::Stvlx
        | K::Stvrx
        | K::Stvlxl
        | K::Stvrxl
        | K::Stvebx
        | K::Stvehx
        | K::Stvewx
        | K::Stvx
        | K::Stvxl
        | K::Stfs
        | K::Stfd
        | K::Stfsu
        | K::Stfdu
        | K::Stfiwx
        | K::Stfsx
        | K::Stfsux
        | K::Stfdx
        | K::Stfdux
        | K::Dcbz
        | K::LiStw
        | K::MflrStw
        | K::MflrStd
        | K::StdStd => (WRITE_EFFECTS, STORE),
        K::Ldarx | K::Lwarx => (RESERVATION_READ_EFFECTS, ATOMIC_LOAD),
        K::Stdcx | K::Stwcx => (ATOMIC_STORE_EFFECTS, ATOMIC_STORE),
        K::Mftb | K::Mftbu => (CLOCK_EFFECTS, CONTINUE),
        K::B => (NO_EFFECTS, UNCONDITIONAL_BRANCH),
        K::Bc | K::Bclr | K::Bcctr | K::CmpwiBc | K::CmpwBc => (NO_EFFECTS, BRANCH),
        K::Sc => (NO_EFFECTS, SYSCALL),
        K::Popcntb => (NO_EFFECTS, FAULT),
        K::Tw | K::Td | K::Mfocrf | K::Mtocrf | K::Vx | K::Va => (NO_EFFECTS, CONTINUE_OR_FAULT),
        _ => (NO_EFFECTS, CONTINUE),
    }
}

fn classify_kind(kind: PpuInstructionKind) {
    match kind {
        PpuInstructionKind::B
        | PpuInstructionKind::Lwz
        | PpuInstructionKind::Lbz
        | PpuInstructionKind::Lhz
        | PpuInstructionKind::Lha
        | PpuInstructionKind::Lhau
        | PpuInstructionKind::Lmw
        | PpuInstructionKind::Lwzu
        | PpuInstructionKind::Lbzu
        | PpuInstructionKind::Lhzu
        | PpuInstructionKind::Ldu
        | PpuInstructionKind::Ld
        | PpuInstructionKind::Lwa
        | PpuInstructionKind::Stw
        | PpuInstructionKind::Stwu
        | PpuInstructionKind::Stdu
        | PpuInstructionKind::Stb
        | PpuInstructionKind::Stbu
        | PpuInstructionKind::Stmw
        | PpuInstructionKind::Sth
        | PpuInstructionKind::Sthu
        | PpuInstructionKind::Std
        | PpuInstructionKind::Addi
        | PpuInstructionKind::Addis
        | PpuInstructionKind::Subfic
        | PpuInstructionKind::Mulli
        | PpuInstructionKind::Addic
        | PpuInstructionKind::AddicDot
        | PpuInstructionKind::Add
        | PpuInstructionKind::Or
        | PpuInstructionKind::Subf
        | PpuInstructionKind::Subfc
        | PpuInstructionKind::Subfe
        | PpuInstructionKind::Neg
        | PpuInstructionKind::Mullw
        | PpuInstructionKind::Mulhwu
        | PpuInstructionKind::Mulhw
        | PpuInstructionKind::Mulhdu
        | PpuInstructionKind::Mulhd
        | PpuInstructionKind::Adde
        | PpuInstructionKind::Addze
        | PpuInstructionKind::Subfze
        | PpuInstructionKind::Subfme
        | PpuInstructionKind::Addme
        | PpuInstructionKind::Mulld
        | PpuInstructionKind::Ldarx
        | PpuInstructionKind::Stdcx
        | PpuInstructionKind::Lwarx
        | PpuInstructionKind::Stwcx
        | PpuInstructionKind::Xori
        | PpuInstructionKind::Xoris
        | PpuInstructionKind::Divw
        | PpuInstructionKind::Divwu
        | PpuInstructionKind::Divd
        | PpuInstructionKind::Divdu
        | PpuInstructionKind::And
        | PpuInstructionKind::Andc
        | PpuInstructionKind::Nor
        | PpuInstructionKind::Xor
        | PpuInstructionKind::Eqv
        | PpuInstructionKind::Nand
        | PpuInstructionKind::AndiDot
        | PpuInstructionKind::AndisDot
        | PpuInstructionKind::Slw
        | PpuInstructionKind::Srw
        | PpuInstructionKind::Srawi
        | PpuInstructionKind::Sraw
        | PpuInstructionKind::Srad
        | PpuInstructionKind::Sradi
        | PpuInstructionKind::Sld
        | PpuInstructionKind::Srd
        | PpuInstructionKind::Cntlzw
        | PpuInstructionKind::Cntlzd
        | PpuInstructionKind::Popcntb
        | PpuInstructionKind::Tw
        | PpuInstructionKind::Td
        | PpuInstructionKind::Mcrxr
        | PpuInstructionKind::Orc
        | PpuInstructionKind::Extsh
        | PpuInstructionKind::Extsb
        | PpuInstructionKind::Extsw
        | PpuInstructionKind::Ori
        | PpuInstructionKind::Oris
        | PpuInstructionKind::Cmpwi
        | PpuInstructionKind::Cmplwi
        | PpuInstructionKind::Cmpdi
        | PpuInstructionKind::Cmpldi
        | PpuInstructionKind::Cmpw
        | PpuInstructionKind::Cmplw
        | PpuInstructionKind::Cmpd
        | PpuInstructionKind::Cmpld
        | PpuInstructionKind::Bc
        | PpuInstructionKind::Bclr
        | PpuInstructionKind::Bcctr
        | PpuInstructionKind::Mcrf
        | PpuInstructionKind::Crand
        | PpuInstructionKind::Crandc
        | PpuInstructionKind::Cror
        | PpuInstructionKind::Crorc
        | PpuInstructionKind::Crxor
        | PpuInstructionKind::Crnand
        | PpuInstructionKind::Crnor
        | PpuInstructionKind::Creqv
        | PpuInstructionKind::Lwzx
        | PpuInstructionKind::Lbzx
        | PpuInstructionKind::Ldx
        | PpuInstructionKind::Lhzx
        | PpuInstructionKind::Stwx
        | PpuInstructionKind::Stdx
        | PpuInstructionKind::Stdux
        | PpuInstructionKind::Stbx
        | PpuInstructionKind::Lwzux
        | PpuInstructionKind::Lbzux
        | PpuInstructionKind::Lhzux
        | PpuInstructionKind::Ldux
        | PpuInstructionKind::Lhax
        | PpuInstructionKind::Lhaux
        | PpuInstructionKind::Lwax
        | PpuInstructionKind::Lwaux
        | PpuInstructionKind::Sthx
        | PpuInstructionKind::Sthux
        | PpuInstructionKind::Stwux
        | PpuInstructionKind::Stbux
        | PpuInstructionKind::Lswi
        | PpuInstructionKind::Lswx
        | PpuInstructionKind::Stswi
        | PpuInstructionKind::Stswx
        | PpuInstructionKind::Ldbrx
        | PpuInstructionKind::Lwbrx
        | PpuInstructionKind::Lhbrx
        | PpuInstructionKind::Sdbrx
        | PpuInstructionKind::Stwbrx
        | PpuInstructionKind::Sthbrx
        | PpuInstructionKind::Mftb
        | PpuInstructionKind::Mftbu
        | PpuInstructionKind::Mfcr
        | PpuInstructionKind::Mtcrf
        | PpuInstructionKind::Mfocrf
        | PpuInstructionKind::Mtocrf
        | PpuInstructionKind::Mflr
        | PpuInstructionKind::Mtlr
        | PpuInstructionKind::Mfctr
        | PpuInstructionKind::Mtctr
        | PpuInstructionKind::Mfxer
        | PpuInstructionKind::Mtxer
        | PpuInstructionKind::Mfvrsave
        | PpuInstructionKind::Mtvrsave
        | PpuInstructionKind::Rlwinm
        | PpuInstructionKind::Rlwimi
        | PpuInstructionKind::Rlwnm
        | PpuInstructionKind::Rldicl
        | PpuInstructionKind::Rldicr
        | PpuInstructionKind::Rldic
        | PpuInstructionKind::Rldimi
        | PpuInstructionKind::Rldcl
        | PpuInstructionKind::Rldcr
        | PpuInstructionKind::Vx
        | PpuInstructionKind::Va
        | PpuInstructionKind::Vxor
        | PpuInstructionKind::Vsldoi
        | PpuInstructionKind::Lvlx
        | PpuInstructionKind::Lvrx
        | PpuInstructionKind::Lvlxl
        | PpuInstructionKind::Lvrxl
        | PpuInstructionKind::Stvlx
        | PpuInstructionKind::Stvrx
        | PpuInstructionKind::Stvlxl
        | PpuInstructionKind::Stvrxl
        | PpuInstructionKind::Lvsl
        | PpuInstructionKind::Lvebx
        | PpuInstructionKind::Lvsr
        | PpuInstructionKind::Lvehx
        | PpuInstructionKind::Lvewx
        | PpuInstructionKind::Lvx
        | PpuInstructionKind::Stvebx
        | PpuInstructionKind::Stvehx
        | PpuInstructionKind::Stvewx
        | PpuInstructionKind::Lvxl
        | PpuInstructionKind::Stvx
        | PpuInstructionKind::Stvxl
        | PpuInstructionKind::Lfs
        | PpuInstructionKind::Lfsu
        | PpuInstructionKind::Lfd
        | PpuInstructionKind::Lfdu
        | PpuInstructionKind::Stfs
        | PpuInstructionKind::Stfd
        | PpuInstructionKind::Stfsu
        | PpuInstructionKind::Stfdu
        | PpuInstructionKind::Stfiwx
        | PpuInstructionKind::Lfsx
        | PpuInstructionKind::Lfsux
        | PpuInstructionKind::Lfdx
        | PpuInstructionKind::Lfdux
        | PpuInstructionKind::Stfsx
        | PpuInstructionKind::Stfsux
        | PpuInstructionKind::Stfdx
        | PpuInstructionKind::Stfdux
        | PpuInstructionKind::Fp63
        | PpuInstructionKind::Fp59
        | PpuInstructionKind::Li
        | PpuInstructionKind::Mr
        | PpuInstructionKind::Slwi
        | PpuInstructionKind::Srwi
        | PpuInstructionKind::Clrlwi
        | PpuInstructionKind::Nop
        | PpuInstructionKind::CmpwZero
        | PpuInstructionKind::Clrldi
        | PpuInstructionKind::Sldi
        | PpuInstructionKind::Srdi
        | PpuInstructionKind::LwzCmpwi
        | PpuInstructionKind::LiStw
        | PpuInstructionKind::MflrStw
        | PpuInstructionKind::LwzMtlr
        | PpuInstructionKind::MflrStd
        | PpuInstructionKind::LdMtlr
        | PpuInstructionKind::StdStd
        | PpuInstructionKind::CmpwiBc
        | PpuInstructionKind::CmpwBc
        | PpuInstructionKind::Consumed
        | PpuInstructionKind::Dcbz
        | PpuInstructionKind::Sc => {}
    }
}

#[cfg(test)]
#[path = "tests/fuzz_tests.rs"]
mod tests;
