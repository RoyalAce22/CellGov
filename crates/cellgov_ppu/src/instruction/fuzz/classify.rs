//! The kind, encoding-form, effect and outcome classifiers, and the sequence helpers.

use cellgov_effects::EffectKind;

use crate::instruction::{FusedPairKind, PpuInstruction, PpuInstructionKind};
use crate::PpuFaultClass as Class;

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
const READ_WRITE_EFFECTS: &[EffectKind] =
    &[EffectKind::SharedReadIntent, EffectKind::SharedWriteIntent];
const ATOMIC_STORE_EFFECTS: &[EffectKind] = &[EffectKind::ConditionalStore];
const CLOCK_EFFECTS: &[EffectKind] = &[EffectKind::ClockRead];
const CONTINUE: &[PpuOutcomeClass] = &[PpuOutcomeClass::Continue];
const FAULT: &[PpuOutcomeClass] = &[PpuOutcomeClass::Fault];
const CONTINUE_OR_FAULT: &[PpuOutcomeClass] = &[PpuOutcomeClass::Continue, PpuOutcomeClass::Fault];
const LOAD: &[PpuOutcomeClass] = &[PpuOutcomeClass::Continue, PpuOutcomeClass::MemoryFault];
const LOAD_OR_FAULT: &[PpuOutcomeClass] = &[
    PpuOutcomeClass::Continue,
    PpuOutcomeClass::Fault,
    PpuOutcomeClass::MemoryFault,
];
// A store to a byte no region holds, or that a region refuses,
// faults at execute.
const STORE: &[PpuOutcomeClass] = &[
    PpuOutcomeClass::Continue,
    PpuOutcomeClass::MemoryFault,
    PpuOutcomeClass::BufferFull,
];
const STORE_OR_FAULT: &[PpuOutcomeClass] = &[
    PpuOutcomeClass::Continue,
    PpuOutcomeClass::Fault,
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
        PpuInstruction::Fused(pair) => PpuFuzzKind::Fused(FusedPairKind::from(pair)),
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
            | K::CmpdZero
            | K::Clrldi
            | K::Sldi
            | K::Srdi
            | K::Lis
            | K::Bdnz
            | K::BcTrue
            | K::BcFalse
            | K::Blr
            | K::RlwinmMask
            | K::Fused
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
    // [CBE-Handbook p:254 s:9.5.9] The PPE takes an illegal-instruction program interrupt for a load or store with update, an lmw, or a load string in an invalid form, so only the encoding decides whether the form takes that interrupt.
    let (checked_load, checked_store) = if valid_form {
        ((READ_EFFECTS, LOAD), (WRITE_EFFECTS, STORE))
    } else {
        ((NO_EFFECTS, FAULT), (NO_EFFECTS, FAULT))
    };
    // A misaligned EA takes the alignment interrupt: see `word_aligned` in the scalar model, and the lmw arm.
    let (aligned_load, aligned_store) = if valid_form {
        (
            (READ_EFFECTS, LOAD_OR_FAULT),
            (WRITE_EFFECTS, STORE_OR_FAULT),
        )
    } else {
        ((NO_EFFECTS, FAULT), (NO_EFFECTS, FAULT))
    };
    match kind {
        K::Lmw | K::Lfsu | K::Lfdu | K::Lfsux | K::Lfdux => aligned_load,
        K::Lhau
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
        | K::Lswi => checked_load,
        // The XER byte count decides whether RA or RB falls in the range,
        // so a word that passes the RT check may still fault.
        K::Lswx if valid_form => (READ_EFFECTS, LOAD_OR_FAULT),
        K::Lswx => (NO_EFFECTS, FAULT),
        K::Stfsu | K::Stfdu | K::Stfsux | K::Stfdux => aligned_store,
        K::Stwu | K::Stdu | K::Stbu | K::Sthu | K::Stdux | K::Sthux | K::Stwux | K::Stbux => {
            checked_store
        }
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
        | K::Lvxl => (READ_EFFECTS, LOAD),
        K::Lfs | K::Lfd | K::Lfsx | K::Lfdx => (READ_EFFECTS, LOAD_OR_FAULT),
        K::Stmw | K::Stfs | K::Stfd | K::Stfiwx | K::Stfsx | K::Stfdx => {
            (WRITE_EFFECTS, STORE_OR_FAULT)
        }
        K::Stw
        | K::Stb
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
        | K::Stvxl => (WRITE_EFFECTS, STORE),
        // A block in a caching-inhibited view takes an alignment interrupt.
        K::Dcbz => (WRITE_EFFECTS, STORE_OR_FAULT),
        K::Ldarx | K::Lwarx => (RESERVATION_READ_EFFECTS, ATOMIC_LOAD),
        K::Stdcx | K::Stwcx => (ATOMIC_STORE_EFFECTS, ATOMIC_STORE),
        K::Mftb | K::Mftbu => (CLOCK_EFFECTS, CONTINUE),
        // A block whose addressed byte no region holds faults; the check records no read.
        K::Dcbst | K::Dcbf | K::Icbi => (NO_EFFECTS, LOAD),
        K::B => (NO_EFFECTS, UNCONDITIONAL_BRANCH),
        K::Bc | K::Bclr | K::Bcctr | K::Bdnz | K::BcTrue | K::BcFalse | K::Blr => {
            (NO_EFFECTS, BRANCH)
        }
        K::Sc => (NO_EFFECTS, SYSCALL),
        // [CBE-Handbook p:739 s:A.2.4] The PPE does not implement these; each raises an illegal-instruction exception.
        K::Popcntb | K::Mcrxr | K::PpeIllegal => (NO_EFFECTS, FAULT),
        // [CBE-Handbook p:254 s:9.5.9] Each raises a privileged-instruction exception in problem state.
        K::PpePrivileged => (NO_EFFECTS, FAULT),
        K::Tw | K::Td | K::Twi | K::Tdi | K::Vx => (NO_EFFECTS, CONTINUE_OR_FAULT),
        _ => (NO_EFFECTS, CONTINUE),
    }
}

/// The effects and outcomes of the fused pair `kind`: those of its
/// memory halves, or of its branch half.
pub(super) fn fused_effect_and_outcome(
    kind: FusedPairKind,
) -> (&'static [EffectKind], &'static [PpuOutcomeClass]) {
    use FusedPairKind as F;
    match kind {
        F::LwzCmpwi
        | F::LwzMtlr
        | F::LdMtlr
        | F::LdLd
        | F::LbzuCmpwi
        | F::LduLdu
        | F::LdRlwinm
        | F::LwzLwz
        | F::LdLdAny => (READ_EFFECTS, LOAD),
        F::LiStw | F::MflrStw | F::MflrStd | F::StdStd => (WRITE_EFFECTS, STORE),
        F::StdLwz => (READ_WRITE_EFFECTS, STORE),
        F::CmpwiBc | F::CmpwBc | F::CmpdiBc | F::CmpdBc | F::CmplwBc | F::AddiBc | F::ExtswBlr => {
            (NO_EFFECTS, BRANCH)
        }
        F::OrAdd | F::LisOri | F::CmpdNor | F::OrRlwinm => (NO_EFFECTS, CONTINUE),
    }
}

/// The fault classes a fault outcome of `kind` may carry, given the
/// `outcomes` of the instruction.
///
/// A `MemoryFault` outcome is a data storage interrupt. An invalid form
/// that faults is an illegal instruction.
pub(super) fn fault_classes(
    kind: PpuInstructionKind,
    valid_form: bool,
    outcomes: &[PpuOutcomeClass],
) -> &'static [Class] {
    use PpuInstructionKind as K;
    const DATA: &[Class] = &[Class::DataStorage];
    let memory = outcomes.contains(&PpuOutcomeClass::MemoryFault);
    if !outcomes.contains(&PpuOutcomeClass::Fault) {
        return if memory { DATA } else { &[] };
    }
    match kind {
        K::Ldarx | K::Lwarx | K::Stdcx | K::Stwcx | K::Dcbz => {
            &[Class::Alignment, Class::DataStorage]
        }
        K::Lmw
        | K::Stmw
        | K::Lfs
        | K::Lfsu
        | K::Lfsx
        | K::Lfsux
        | K::Lfd
        | K::Lfdu
        | K::Lfdx
        | K::Lfdux
        | K::Stfs
        | K::Stfsu
        | K::Stfsx
        | K::Stfsux
        | K::Stfd
        | K::Stfdu
        | K::Stfdx
        | K::Stfdux
        | K::Stfiwx
            if valid_form =>
        {
            &[Class::Alignment, Class::DataStorage]
        }
        K::PpePrivileged => &[Class::ProgramPrivileged],
        K::Tw | K::Td | K::Twi | K::Tdi => &[Class::ProgramTrap],
        K::Vx => &[Class::Unimplemented],
        K::Sc => &[Class::SystemCall],
        _ if memory => &[Class::ProgramIllegal, Class::DataStorage],
        _ => &[Class::ProgramIllegal],
    }
}

/// The fault classes of `instruction` given its kind's `faults`, under
/// the same two overrides as [`instruction_outcomes`].
pub(super) fn instruction_faults(
    instruction: PpuInstruction,
    faults: &'static [Class],
) -> &'static [Class] {
    match instruction {
        PpuInstruction::Vx { op, rc, .. } if crate::exec::vx_executes(op, rc) => &[],
        PpuInstruction::Sc { lev: 1 } => &[Class::SystemCall],
        _ => faults,
    }
}

/// The outcomes of `instruction` given its kind's `outcomes`.
///
/// Two instruction forms override their kind:
/// - A VX op the interpreter executes always continues.
/// - `sc` with LEV = 1 always faults.
pub(super) fn instruction_outcomes(
    instruction: PpuInstruction,
    outcomes: &'static [PpuOutcomeClass],
) -> &'static [PpuOutcomeClass] {
    match instruction {
        PpuInstruction::Vx { op, rc, .. } if crate::exec::vx_executes(op, rc) => CONTINUE,
        PpuInstruction::Sc { lev: 1 } => FAULT,
        _ => outcomes,
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
        | PpuInstructionKind::Addc
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
        | PpuInstructionKind::Twi
        | PpuInstructionKind::Tdi
        | PpuInstructionKind::Mcrxr
        | PpuInstructionKind::PpeIllegal
        | PpuInstructionKind::PpePrivileged
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
        | PpuInstructionKind::Mfctrl
        | PpuInstructionKind::Mfsprg3
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
        | PpuInstructionKind::Mfvscr
        | PpuInstructionKind::Mtvscr
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
        | PpuInstructionKind::CmpdZero
        | PpuInstructionKind::Clrldi
        | PpuInstructionKind::Sldi
        | PpuInstructionKind::Srdi
        | PpuInstructionKind::Lis
        | PpuInstructionKind::Bdnz
        | PpuInstructionKind::BcTrue
        | PpuInstructionKind::BcFalse
        | PpuInstructionKind::Blr
        | PpuInstructionKind::RlwinmMask
        | PpuInstructionKind::Fused
        | PpuInstructionKind::Consumed
        | PpuInstructionKind::Dcbz
        | PpuInstructionKind::Dcbst
        | PpuInstructionKind::Dcbf
        | PpuInstructionKind::Icbi
        | PpuInstructionKind::Dcbt
        | PpuInstructionKind::Dcbtst
        | PpuInstructionKind::Dst
        | PpuInstructionKind::Dstst
        | PpuInstructionKind::Dss
        | PpuInstructionKind::Sc
        | PpuInstructionKind::Sync
        | PpuInstructionKind::Eieio
        | PpuInstructionKind::Isync => {}
    }
}
