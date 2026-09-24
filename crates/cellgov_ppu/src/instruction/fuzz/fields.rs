//! Operand fields, their candidate values, and operand validity.

use crate::instruction::ops::{Fp59Shape, Fp63Op, Fp63Shape, VaShape, VxShape};
use crate::instruction::{PpuInstruction, PpuInstructionKind};

use super::types::{
    PpuEncodingForm, PpuFuzzDescriptor, PpuFuzzKind, PpuOperandClass, PpuOperandField,
};

pub(super) fn operand_fields(
    raw: u32,
    instruction: PpuInstruction,
    descriptor: PpuFuzzDescriptor,
) -> Vec<PpuOperandField> {
    field_candidates(descriptor)
        .into_iter()
        .filter_map(|(mask, class)| {
            let active = (active_bits(raw, instruction, descriptor.kind)
                | decoder_opaque_operand_bits(descriptor.kind))
                & mask;
            (active != 0).then_some(PpuOperandField {
                class,
                mask: active,
            })
        })
        .collect()
}

fn active_bits(raw: u32, instruction: PpuInstruction, kind: PpuFuzzKind) -> u32 {
    (0..u32::BITS).fold(0, |mask, bit| {
        let candidate = raw ^ (1u32 << bit);
        match crate::decode::decode(candidate) {
            Ok(decoded)
                if decoded != instruction && decoded.fuzz_descriptor(candidate).kind == kind =>
            {
                mask | (1u32 << bit)
            }
            _ => mask,
        }
    })
}

fn field_candidates(descriptor: PpuFuzzDescriptor) -> Vec<(u32, PpuOperandClass)> {
    use PpuEncodingForm as F;
    use PpuFuzzKind as K;
    use PpuInstructionKind as I;
    use PpuOperandClass as C;

    const BIT_0: u32 = 0x0000_0001;
    const BITS_1_5: u32 = 0x0000_003e;
    const BITS_5_10: u32 = 0x0000_07e0;
    const BITS_6_10: u32 = 0x0000_07c0;
    const BITS_11_15: u32 = 0x0000_f800;
    const BITS_16_20: u32 = 0x001f_0000;
    const BITS_21_25: u32 = 0x03e0_0000;

    match descriptor.form {
        // [PPC-Book1 p:8 s:1.7.4] Compare-immediate uses BF in the
        // D-form RT slot; the other D-form instructions use a register.
        F::D => match descriptor.kind {
            K::Ordinary(I::Cmpwi | I::Cmplwi | I::Cmpdi | I::Cmpldi) => vec![
                (0x0000_ffff, C::Immediate),
                (BITS_16_20, C::Register),
                (BITS_21_25, C::Condition),
            ],
            _ => vec![
                (0x0000_ffff, C::Immediate),
                (BITS_16_20, C::Register),
                (BITS_21_25, C::Register),
            ],
        },
        F::Ds => vec![
            (0x0000_fffc, C::Immediate),
            (BITS_16_20, C::Register),
            (BITS_21_25, C::Register),
        ],
        F::I => vec![
            (0x0000_0001, C::Flag),
            (0x0000_0002, C::Flag),
            (0x03ff_fffc, C::Immediate),
        ],
        F::B => vec![
            (0x0000_0001, C::Flag),
            (0x0000_0002, C::Flag),
            (0x0000_fffc, C::Immediate),
            (0x001f_0000, C::Condition),
            (0x03e0_0000, C::Condition),
        ],
        // [PPC-Book1 p:9 s:1.7.6] X-form reuses its register slots for
        // condition, shift, count, and selector operands in named forms.
        F::X => match descriptor.kind {
            K::Ordinary(I::Cmpw | I::Cmplw | I::Cmpd | I::Cmpld | I::Mcrxr) => vec![
                (BITS_11_15, C::Register),
                (BITS_16_20, C::Register),
                (BITS_21_25, C::Condition),
            ],
            K::Ordinary(I::Tw | I::Td) => vec![
                (BITS_11_15, C::Register),
                (BITS_16_20, C::Register),
                (BITS_21_25, C::Selector),
            ],
            K::Ordinary(I::Lswi | I::Stswi) => vec![
                (BITS_11_15, C::Immediate),
                (BITS_16_20, C::Register),
                (BITS_21_25, C::Register),
            ],
            K::Ordinary(I::Srawi) => vec![
                (BIT_0, C::Flag),
                (BITS_11_15, C::Immediate),
                (BITS_16_20, C::Register),
                (BITS_21_25, C::Register),
            ],
            K::Ordinary(I::Sradi) => vec![
                (BIT_0, C::Flag),
                (0x0000_f802, C::Immediate),
                (BITS_16_20, C::Register),
                (BITS_21_25, C::Register),
            ],
            K::Ordinary(I::Mfocrf | I::Mtcrf | I::Mtocrf) => {
                vec![(0x000f_f000, C::Selector), (BITS_21_25, C::Register)]
            }
            _ => vec![
                (BIT_0, C::Flag),
                (0x0000_0400, C::Flag),
                (BITS_11_15, C::Register),
                (BITS_16_20, C::Register),
                (BITS_21_25, C::Register),
            ],
        },
        // [PPC-Book1 p:9 s:1.7.7] XL-form operands are CR selectors or
        // branch conditions; it has no general-register operand slot.
        F::Xl => match descriptor.kind {
            K::Ordinary(I::Bclr | I::Bcctr) => vec![
                (BIT_0, C::Flag),
                (0x0000_1800, C::Selector),
                (BITS_16_20, C::Condition),
                (BITS_21_25, C::Condition),
            ],
            _ => vec![
                (BIT_0, C::Flag),
                (BITS_6_10, C::Condition),
                (BITS_11_15, C::Condition),
                (BITS_16_20, C::Condition),
                (BITS_21_25, C::Condition),
            ],
        },
        // [PPC-Book1 p:10 s:1.7.13] The third M-form slot is SH for
        // immediate rotates and RB for rlwnm.
        F::M => vec![
            (BIT_0, C::Flag),
            (BITS_1_5, C::Immediate),
            (BITS_6_10, C::Immediate),
            (
                BITS_11_15,
                if descriptor.kind == K::Ordinary(I::Rlwnm) {
                    C::Register
                } else {
                    C::Immediate
                },
            ),
            (BITS_16_20, C::Register),
            (BITS_21_25, C::Register),
        ],
        // [PPC-Book1 p:10 s:1.7.14] MD-form splits SH and its mask
        // bound; MDS-form replaces SH with RB.
        F::Md => match descriptor.kind {
            K::Ordinary(I::Rldcl | I::Rldcr) => vec![
                (BIT_0, C::Flag),
                (BITS_5_10, C::Immediate),
                (BITS_11_15, C::Register),
                (BITS_16_20, C::Register),
                (BITS_21_25, C::Register),
            ],
            _ => vec![
                (BIT_0, C::Flag),
                (0x0000_f802, C::Immediate),
                (BITS_5_10, C::Immediate),
                (BITS_16_20, C::Register),
                (BITS_21_25, C::Register),
            ],
        },
        F::Vector => vector_field_candidates(descriptor.kind),
        F::Float => float_field_candidates(descriptor.kind),
        F::SystemCall => vec![(0x0000_0fe0, C::Selector)],
        F::Synthetic => Vec::new(),
    }
}

fn vector_field_candidates(kind: PpuFuzzKind) -> Vec<(u32, PpuOperandClass)> {
    use PpuOperandClass as C;

    const VC: u32 = 0x0000_07c0;
    const VB: u32 = 0x0000_f800;
    const VA: u32 = 0x001f_0000;
    const VD: u32 = 0x03e0_0000;

    // [AltiVec-PEM p:A-21 s:A.5] VA/VX forms reserve unused slots and
    // place splat immediates in vA; VXR compares carry Rc in raw bit 10.
    match kind {
        PpuFuzzKind::Ordinary(PpuInstructionKind::Vsldoi) => vec![
            (0x0000_03c0, C::Immediate),
            (VB, C::Register),
            (VA, C::Register),
            (VD, C::Register),
        ],
        PpuFuzzKind::Ordinary(PpuInstructionKind::Vxor) => {
            vec![(VB, C::Register), (VA, C::Register), (VD, C::Register)]
        }
        PpuFuzzKind::Vx(op) => {
            let mut fields = Vec::new();
            if op.is_vxr_compare() {
                fields.push((0x0000_0400, C::Flag));
            }
            match op.shape() {
                VxShape::VdVaVb => {
                    fields.extend([(VB, C::Register), (VA, C::Register), (VD, C::Register)]);
                }
                VxShape::VdVb => fields.extend([(VB, C::Register), (VD, C::Register)]),
                VxShape::VdVbUimm => {
                    fields.extend([(VB, C::Register), (VA, C::Immediate), (VD, C::Register)]);
                }
                VxShape::VdSimm => {
                    fields.extend([(VA, C::Immediate), (VD, C::Register)]);
                }
            }
            fields
        }
        PpuFuzzKind::Va(op) => match op.shape() {
            VaShape::VdVaVbVc | VaShape::VdVaVcVb => vec![
                (VC, C::Register),
                (VB, C::Register),
                (VA, C::Register),
                (VD, C::Register),
            ],
            VaShape::VdVaVbShb => vec![
                (0x0000_03c0, C::Immediate),
                (VB, C::Register),
                (VA, C::Register),
                (VD, C::Register),
            ],
        },
        _ => Vec::new(),
    }
}

fn float_field_candidates(kind: PpuFuzzKind) -> Vec<(u32, PpuOperandClass)> {
    use PpuOperandClass as C;

    const RC: (u32, C) = (0x0000_0001, C::Flag);
    const FRC: (u32, C) = (0x0000_07c0, C::Register);
    const FRB: (u32, C) = (0x0000_f800, C::Register);
    const FRA: (u32, C) = (0x001f_0000, C::Register);
    const FRT: (u32, C) = (0x03e0_0000, C::Register);

    // [PPC-Book1 p:9 s:1.7.6] The floating X forms reuse register
    // slots for CR/FPSCR selectors and reserve every unused slot.
    match kind {
        PpuFuzzKind::Fp59(op) => match op.shape() {
            Fp59Shape::FrtFraFrb => vec![RC, FRB, FRA, FRT],
            Fp59Shape::FrtFrb => vec![RC, FRB, FRT],
            Fp59Shape::FrtFraFrc => vec![RC, FRC, FRA, FRT],
            Fp59Shape::FrtFraFrcFrb => vec![RC, FRC, FRB, FRA, FRT],
        },
        PpuFuzzKind::Fp63(op) => match op.shape() {
            Fp63Shape::FrtFraFrb => vec![RC, FRB, FRA, FRT],
            Fp63Shape::FrtFrb => vec![RC, FRB, FRT],
            Fp63Shape::FrtFraFrc => vec![RC, FRC, FRA, FRT],
            Fp63Shape::FrtFraFrcFrb => vec![RC, FRC, FRB, FRA, FRT],
            Fp63Shape::CrfFraFrb => vec![FRB, FRA, (0x0380_0000, C::Condition)],
            Fp63Shape::Frt => vec![RC, FRT],
            Fp63Shape::CrfCrf => vec![(0x001c_0000, C::Condition), (0x0380_0000, C::Condition)],
            Fp63Shape::CrfImm => vec![RC, (0x0000_f000, C::Immediate), (0x0380_0000, C::Condition)],
            Fp63Shape::Crb => vec![RC, (0x03e0_0000, C::Selector)],
            Fp63Shape::FmFrb => vec![RC, FRB, (0x01fe_0000, C::Selector)],
        },
        _ => Vec::new(),
    }
}

pub(super) fn semantic_reserved_bits(kind: PpuFuzzKind) -> u32 {
    // Some family decoders retain every physical slot so execution can share
    // one variant. The form shape remains authoritative for unused slots.
    match kind {
        PpuFuzzKind::Vx(op) => match op.shape() {
            VxShape::VdVb => 0x001f_0000,
            VxShape::VdSimm => 0x0000_f800,
            VxShape::VdVaVb | VxShape::VdVbUimm => 0,
        },
        // [PPC-Book1 p:10 s:1.7.12] A-form diagrams mark the unused
        // FRA, FRB, or FRC slots as reserved for the corresponding shape.
        PpuFuzzKind::Fp59(op) => match op.shape() {
            Fp59Shape::FrtFraFrb => 0x0000_07c0,
            Fp59Shape::FrtFrb => 0x001f_07c0,
            Fp59Shape::FrtFraFrc => 0x0000_f800,
            Fp59Shape::FrtFraFrcFrb => 0,
        },
        // [PPC-Book1 p:9 s:1.7.6] Floating X/XFL forms reserve the
        // unused portions of their CR, FPSCR, and register slots.
        PpuFuzzKind::Fp63(op) => match op.shape() {
            Fp63Shape::FrtFraFrb => 0x0000_07c0,
            Fp63Shape::FrtFrb if matches!(op, Fp63Op::Fsqrt | Fp63Op::Frsqrte) => 0x001f_07c0,
            Fp63Shape::FrtFrb => 0x001f_0000,
            Fp63Shape::FrtFraFrc => 0x0000_f800,
            Fp63Shape::FrtFraFrcFrb => 0,
            Fp63Shape::CrfFraFrb => 0x0060_0001,
            Fp63Shape::Frt => 0x001f_f800,
            Fp63Shape::CrfCrf => 0x0063_f801,
            Fp63Shape::CrfImm => 0x007f_0800,
            Fp63Shape::Crb => 0x001f_f800,
            Fp63Shape::FmFrb => 0x0201_0000,
        },
        _ => 0,
    }
}

fn decoder_opaque_operand_bits(kind: PpuFuzzKind) -> u32 {
    // [PPC-Book1 p:10 s:1.7.16] BH is an architected branch-hint
    // selector even though the deterministic decoder does not retain it.
    match kind {
        PpuFuzzKind::Ordinary(PpuInstructionKind::Bclr | PpuInstructionKind::Bcctr) => 0x0000_1800,
        _ => 0,
    }
}

pub(super) fn discriminator_bits(kind: PpuFuzzKind, form: PpuEncodingForm) -> u32 {
    0xfc00_0000
        | match form {
            PpuEncodingForm::Ds => 0x0000_0003,
            PpuEncodingForm::X | PpuEncodingForm::Xl => 0x0000_07fe,
            PpuEncodingForm::Md => match kind {
                PpuFuzzKind::Ordinary(PpuInstructionKind::Rldcl | PpuInstructionKind::Rldcr) => {
                    0x0000_001e
                }
                _ => 0x0000_001c,
            },
            PpuEncodingForm::Vector => match kind {
                PpuFuzzKind::Ordinary(PpuInstructionKind::Vsldoi) | PpuFuzzKind::Va(_) => {
                    0x0000_003f
                }
                _ => 0x0000_07ff,
            },
            PpuEncodingForm::Float => match kind {
                PpuFuzzKind::Fp59(_) => 0x0000_003e,
                PpuFuzzKind::Fp63(
                    Fp63Op::Fdiv
                    | Fp63Op::Fsub
                    | Fp63Op::Fadd
                    | Fp63Op::Fsqrt
                    | Fp63Op::Fsel
                    | Fp63Op::Fmul
                    | Fp63Op::Frsqrte
                    | Fp63Op::Fmsub
                    | Fp63Op::Fmadd
                    | Fp63Op::Fnmsub
                    | Fp63Op::Fnmadd,
                ) => 0x0000_003e,
                _ => 0x0000_07fe,
            },
            PpuEncodingForm::SystemCall => 0x0000_0002,
            PpuEncodingForm::D
            | PpuEncodingForm::I
            | PpuEncodingForm::B
            | PpuEncodingForm::M
            | PpuEncodingForm::Synthetic => 0,
        }
}

pub(super) fn generation_operands_are_valid(instruction: PpuInstruction) -> bool {
    use PpuInstruction as I;

    match instruction {
        // [PPC-Book1 p:33 s:3.3.2] A fixed-point load-update form
        // requires a nonzero RA distinct from RT.
        I::Lhau { rt, ra, .. }
        | I::Lwzu { rt, ra, .. }
        | I::Lbzu { rt, ra, .. }
        | I::Lhzu { rt, ra, .. }
        | I::Ldu { rt, ra, .. }
        | I::Lwzux { rt, ra, .. }
        | I::Lbzux { rt, ra, .. }
        | I::Lhzux { rt, ra, .. }
        | I::Ldux { rt, ra, .. }
        | I::Lhaux { rt, ra, .. }
        | I::Lwaux { rt, ra, .. } => ra != 0 && ra != rt,
        // [PPC-Book1 p:46 s:3.3.5] lmw is invalid when RA is zero or
        // names any register in the RT..=31 load range.
        I::Lmw { rt, ra, .. } => ra != 0 && ra < rt,
        // [PPC-Book1 p:48 s:3.3.6] lswi is invalid when RA is zero or
        // names one of the wrapping destination registers.
        I::Lswi { rt, ra, nb } => {
            let bytes = if nb == 0 { 32 } else { nb };
            let registers = bytes.div_ceil(4);
            ra != 0 && !(0..registers).any(|index| rt.wrapping_add(index) & 31 == ra)
        }
        // [PPC-Book1 p:48 s:3.3.6] lswx is always invalid when the
        // first destination names either address register.
        I::Lswx { rt, ra, rb } => rt != ra && rt != rb,
        // [PPC-Book1 p:104 s:4.6.2] Single-precision load-update requires nonzero RA.
        // [PPC-Book1 p:105 s:4.6.2] Double-precision load-update requires nonzero RA.
        // FRT is in a separate register file, so it may share RA's number.
        I::Lfsu { ra, .. } | I::Lfdu { ra, .. } | I::Lfsux { ra, .. } | I::Lfdux { ra, .. } => {
            ra != 0
        }
        // [PPC-Book1 p:40 s:3.3.3] Fixed-point store-update permits
        // RS=RA but requires nonzero RA.
        I::Stwu { ra, .. }
        | I::Stdu { ra, .. }
        | I::Stbu { ra, .. }
        | I::Sthu { ra, .. }
        | I::Stdux { ra, .. }
        | I::Sthux { ra, .. }
        | I::Stwux { ra, .. }
        | I::Stbux { ra, .. } => ra != 0,
        // [PPC-Book1 p:107 s:4.6] Single-precision store-update requires nonzero RA.
        // [PPC-Book1 p:108 s:4.6] Double-precision store-update requires nonzero RA.
        I::Stfsu { ra, .. } | I::Stfdu { ra, .. } | I::Stfsux { ra, .. } | I::Stfdux { ra, .. } => {
            ra != 0
        }
        // [PPC-Book1 p:25 s:2.4] bcctr cannot request CTR decrement.
        I::Bcctr { bo, .. } => bo & 0x04 != 0,
        _ => true,
    }
}
