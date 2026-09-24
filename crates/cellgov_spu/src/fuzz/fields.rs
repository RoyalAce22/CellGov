//! Operand fields, their candidate values, and operand validity.

use crate::instruction::{SpuInstruction, SpuInstructionKind};

use super::types::{SpuEncodingForm, SpuFuzzDescriptor, SpuOperandClass, SpuOperandField};

pub(super) fn operand_fields(
    raw: u32,
    instruction: SpuInstruction,
    descriptor: SpuFuzzDescriptor,
) -> Vec<SpuOperandField> {
    field_candidates(descriptor.kind, descriptor.form)
        .into_iter()
        .filter_map(|(mask, class)| {
            let decoded_field_is_ignored = matches!(
                descriptor.kind,
                SpuInstructionKind::Nop
                    | SpuInstructionKind::Bi
                    | SpuInstructionKind::Bisl
                    | SpuInstructionKind::Biz
                    | SpuInstructionKind::Binz
                    | SpuInstructionKind::Bihz
                    | SpuInstructionKind::Bihnz
                    | SpuInstructionKind::Hbr
                    | SpuInstructionKind::Hbra
                    | SpuInstructionKind::Hbrr
                    | SpuInstructionKind::Sync
                    | SpuInstructionKind::Heq
            );
            let active = if decoded_field_is_ignored {
                mask
            } else {
                active_bits(raw, instruction, descriptor.kind) & mask
            };
            (active != 0).then_some(SpuOperandField {
                class,
                mask: active,
            })
        })
        .collect()
}

pub(super) fn active_bits(raw: u32, instruction: SpuInstruction, kind: SpuInstructionKind) -> u32 {
    (0..u32::BITS).fold(0, |mask, bit| {
        let candidate = raw ^ (1u32 << bit);
        match crate::decode::decode(candidate) {
            Ok(decoded) if decoded != instruction && SpuInstructionKind::from(decoded) == kind => {
                mask | (1u32 << bit)
            }
            _ => mask,
        }
    })
}

fn field_candidates(
    kind: SpuInstructionKind,
    form: SpuEncodingForm,
) -> Vec<(u32, SpuOperandClass)> {
    use SpuEncodingForm as F;
    use SpuInstructionKind as K;
    use SpuOperandClass as C;
    match form {
        F::Rrr => vec![
            (0x0000_007f, C::Register),
            (0x0000_3f80, C::Register),
            (0x001f_c000, C::Register),
        ],
        F::Rrrr => vec![
            (0x0000_007f, C::Register),
            (0x0000_3f80, C::Register),
            (0x001f_c000, C::Register),
            (0x0fe0_0000, C::Register),
        ],
        F::Ri7 => vec![
            (0x0000_007f, C::Register),
            (0x0000_3f80, C::Register),
            (0x001f_c000, C::Immediate),
        ],
        F::Ri10 => vec![
            (0x0000_007f, C::Register),
            (0x0000_3f80, C::Register),
            (0x00ff_c000, C::Immediate),
        ],
        F::Ri16 => vec![(0x0000_007f, C::Register), (0x007f_ff80, C::Immediate)],
        F::Ri18 => vec![(0x0000_007f, C::Register), (0x01ff_ff80, C::Immediate)],
        F::Channel => vec![(0x0000_007f, C::Register), (0x0000_3f80, C::Channel)],
        F::Branch => match kind {
            K::Br => vec![(0x007f_ff80, C::Immediate)],
            K::Brsl | K::Brz | K::Brnz | K::Brhnz | K::Brhz => {
                vec![(0x0000_007f, C::Register), (0x007f_ff80, C::Immediate)]
            }
            // [SPU-ISA p:178 s:7 Compare, Branch, and Halt Instructions] BI encodes E and D options.
            K::Bi => vec![
                (0x0000_3f80, C::Register),
                (0x0004_0000, C::Flag),
                (0x0008_0000, C::Flag),
            ],
            K::Bisl | K::Biz | K::Binz | K::Bihz | K::Bihnz => {
                // [SPU-ISA p:181 s:7 Compare, Branch, and Halt Instructions] BISL encodes E and D options.
                // [SPU-ISA p:186 s:7 Compare, Branch, and Halt Instructions] BIZ encodes E and D options.
                // [SPU-ISA p:187 s:7 Compare, Branch, and Halt Instructions] BINZ encodes E and D options.
                // [SPU-ISA p:188 s:7 Compare, Branch, and Halt Instructions] BIHZ encodes E and D options.
                // [SPU-ISA p:189 s:7 Compare, Branch, and Halt Instructions] BIHNZ encodes E and D options.
                vec![
                    (0x0000_007f, C::Register),
                    (0x0000_3f80, C::Register),
                    (0x0004_0000, C::Flag),
                    (0x0008_0000, C::Flag),
                ]
            }
            _ => Vec::new(),
        },
        F::Control => match kind {
            // [SPU-ISA p:238 s:10 Control Instructions] STOP carries one 14-bit signal operand.
            K::Stop => vec![(0x0000_3fff, C::Immediate)],
            // [SPU-ISA p:241 s:10 Control Instructions] NOP carries an RT false target.
            K::Nop => vec![(0x0000_007f, C::Register)],
            // [SPU-ISA p:150 s:7 Compare, Branch, and Halt Instructions] HEQ encodes RB, RA, and a false RT.
            K::Heq => vec![
                (0x0000_007f, C::Register),
                (0x0000_3f80, C::Register),
                (0x001f_c000, C::Register),
            ],
            // [SPU-ISA p:192 s:8 Hint-for-Branch Instructions] HBR encodes RO around RA and a P option.
            K::Hbr => vec![
                (0x0000_c07f, C::Immediate),
                (0x0000_3f80, C::Register),
                (0x0010_0000, C::Flag),
            ],
            // [SPU-ISA p:193 s:8 Hint-for-Branch Instructions] HBRA splits RO around its I16 field.
            K::Hbra => vec![(0x0180_007f, C::Immediate), (0x007f_ff80, C::Immediate)],
            // [SPU-ISA p:194 s:8 Hint-for-Branch Instructions] HBRR uses the HBRA field layout.
            K::Hbrr => vec![(0x0180_007f, C::Immediate), (0x007f_ff80, C::Immediate)],
            // [SPU-ISA p:242 s:10 Control Instructions] SYNC's C bit is an encoding option.
            K::Sync => vec![(0x0010_0000, C::Flag)],
            _ => Vec::new(),
        },
    }
}

pub(super) fn operand_combination_is_valid(kind: SpuInstructionKind, word: u32) -> bool {
    use SpuInstructionKind as K;
    match kind {
        // [SPU-ISA p:178 s:7 Compare, Branch, and Halt Instructions] BI reserves E and D set together.
        K::Bi => word & 0x000c_0000 != 0x000c_0000,
        // [SPU-ISA p:181 s:7 Compare, Branch, and Halt Instructions] BISL reserves E and D set together.
        // [SPU-ISA p:186 s:7 Compare, Branch, and Halt Instructions] BIZ reserves E and D set together.
        // [SPU-ISA p:187 s:7 Compare, Branch, and Halt Instructions] BINZ reserves E and D set together.
        // [SPU-ISA p:188 s:7 Compare, Branch, and Halt Instructions] BIHZ reserves E and D set together.
        // [SPU-ISA p:189 s:7 Compare, Branch, and Halt Instructions] BIHNZ reserves E and D set together.
        K::Bisl | K::Biz | K::Binz | K::Bihz | K::Bihnz => word & 0x000c_0000 != 0x000c_0000,
        // [SPU-ISA p:192 s:8 Hint-for-Branch Instructions] P requires the split RO field to be zero.
        K::Hbr => word & 0x0010_0000 == 0 || word & 0x0000_c07f == 0,
        _ => true,
    }
}
