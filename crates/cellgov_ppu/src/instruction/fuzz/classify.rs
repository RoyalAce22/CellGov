//! The kind, encoding-form, effect and outcome classifiers, and the sequence helpers.

use cellgov_effects::EffectKind;

use crate::instruction::{PpuInstruction, PpuInstructionKind};

use super::types::{
    PpuEncodingForm, PpuFuzzKind, PpuOutcomeClass, PpuSequenceClass, PpuSequenceDependency,
    PpuSequenceFlow,
};

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
// A store whose last byte passes 2^64 faults at execute.
const STORE: &[PpuOutcomeClass] = &[
    PpuOutcomeClass::Continue,
    PpuOutcomeClass::MemoryFault,
    PpuOutcomeClass::BufferFull,
];
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

pub(super) fn exact_kind(raw: u32) -> Option<PpuFuzzKind> {
    let instruction = crate::decode::decode(raw).ok()?;
    Some(instruction.fuzz_descriptor(raw).kind)
}

pub(super) fn sequence_class(kind: PpuFuzzKind) -> PpuSequenceClass {
    match kind {
        PpuFuzzKind::Ordinary(PpuInstructionKind::Lswx) => PpuSequenceClass::ReadsXerByteCount,
        PpuFuzzKind::Ordinary(PpuInstructionKind::Mtxer) => PpuSequenceClass::ReplacesXer,
        _ => PpuSequenceClass::Independent,
    }
}

pub(super) fn sequence_flow(outcomes: &[PpuOutcomeClass]) -> PpuSequenceFlow {
    if outcomes.contains(&PpuOutcomeClass::Branch) {
        PpuSequenceFlow::ControlTransfer
    } else if outcomes.contains(&PpuOutcomeClass::Syscall) || outcomes == FAULT {
        PpuSequenceFlow::Terminal
    } else if outcomes != CONTINUE {
        PpuSequenceFlow::StateDependent
    } else {
        PpuSequenceFlow::Linear
    }
}

pub(super) fn sequence_dependency(kind: PpuFuzzKind) -> Option<PpuSequenceDependency> {
    // [PPC-Book1 p:66 s:3.3.13] These logical-immediate forms read RS and replace RA.
    matches!(
        kind,
        PpuFuzzKind::Ordinary(
            PpuInstructionKind::Ori
                | PpuInstructionKind::Oris
                | PpuInstructionKind::Xori
                | PpuInstructionKind::Xoris
        )
    )
    .then_some(PpuSequenceDependency::GeneralPurposeRegister)
}

pub(super) fn fuzz_kind(instruction: PpuInstruction) -> PpuFuzzKind {
    match instruction {
        PpuInstruction::Vx { op, .. } => PpuFuzzKind::Vx(op),
        PpuInstruction::Va { op, .. } => PpuFuzzKind::Va(op),
        PpuInstruction::Fp59 { op, .. } => PpuFuzzKind::Fp59(op),
        PpuInstruction::Fp63 { op, .. } => PpuFuzzKind::Fp63(op),
        ordinary => PpuFuzzKind::Ordinary(PpuInstructionKind::from(ordinary)),
    }
}

pub(super) fn form_for_word(kind: PpuInstructionKind, raw: u32) -> PpuEncodingForm {
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

pub(super) fn effect_and_outcome(
    kind: PpuInstructionKind,
    valid_form: bool,
) -> (&'static [EffectKind], &'static [PpuOutcomeClass]) {
    use PpuInstructionKind as K;
    // [CBE-Handbook p:254 s:9.5.9] The PPE takes an illegal-instruction program interrupt for a load or store with update, or an lmw, in an invalid form, so only the encoding decides whether the form faults.
    let (checked_load, checked_store) = if valid_form {
        ((READ_EFFECTS, LOAD), (WRITE_EFFECTS, STORE))
    } else {
        ((NO_EFFECTS, FAULT), (NO_EFFECTS, FAULT))
    };
    match kind {
        K::Lmw
        | K::Lhau
        | K::Lwzu
        | K::Lbzu
        | K::Lhzu
        | K::Ldu
        | K::Lwzux
        | K::Lbzux
        | K::Lhzux
        | K::Ldux
        | K::Lhaux
        | K::Lwaux
        | K::Lfsu
        | K::Lfdu
        | K::Lfsux
        | K::Lfdux => checked_load,
        K::Stwu
        | K::Stdu
        | K::Stbu
        | K::Sthu
        | K::Stdux
        | K::Sthux
        | K::Stwux
        | K::Stbux
        | K::Stfsu
        | K::Stfdu
        | K::Stfsux
        | K::Stfdux => checked_store,
        K::Lwz
        | K::Lbz
        | K::Lhz
        | K::Lha
        | K::Ld
        | K::Lwa
        | K::Lwzx
        | K::Lbzx
        | K::Ldx
        | K::Lhzx
        | K::Lhax
        | K::Lwax
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
        | K::Lfd
        | K::Lfsx
        | K::Lfdx
        | K::LwzCmpwi
        | K::LwzMtlr
        | K::LdMtlr => (READ_EFFECTS, LOAD),
        K::Stw
        | K::Stb
        | K::Stmw
        | K::Sth
        | K::Std
        | K::Stwx
        | K::Stdx
        | K::Stbx
        | K::Sthx
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
        | K::Stfiwx
        | K::Stfsx
        | K::Stfdx
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

pub(super) fn classify_kind(kind: PpuInstructionKind) {
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
