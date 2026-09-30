//! Operand fields, their candidate values, and operand validity.

use cellgov_ps3_abi::hw::spu_isa;

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
            // [SPU-ISA p:150 s:7 Compare, Branch, and Halt Instructions] a halt's RT is a false
            // target the decoder does not carry, so its bits stay a generated field.
            // [SPU-ISA p:151 s:7 Compare, Branch, and Halt Instructions] HEQI's RT is a false target.
            // [SPU-ISA p:152 s:7 Compare, Branch, and Halt Instructions] HGT's RT is a false target.
            // [SPU-ISA p:153 s:7 Compare, Branch, and Halt Instructions] HGTI's RT is a false target.
            // [SPU-ISA p:154 s:7 Compare, Branch, and Halt Instructions] HLGT's RT is a false target.
            // [SPU-ISA p:155 s:7 Compare, Branch, and Halt Instructions] HLGTI's RT is a false target.
            let decoded_field_is_ignored = matches!(
                descriptor.kind,
                SpuInstructionKind::Heq
                    | SpuInstructionKind::Heqi
                    | SpuInstructionKind::Hgt
                    | SpuInstructionKind::Hgti
                    | SpuInstructionKind::Hlgt
                    | SpuInstructionKind::Hlgti
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
        F::Ri8 => vec![
            (0x0000_007f, C::Register),
            (0x0000_3f80, C::Register),
            (0x003f_c000, C::Immediate),
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
            K::Br | K::Bra => vec![(0x007f_ff80, C::Immediate)],
            K::Brsl | K::Brasl | K::Brz | K::Brnz | K::Brhnz | K::Brhz => {
                vec![(0x0000_007f, C::Register), (0x007f_ff80, C::Immediate)]
            }
            // [SPU-ISA p:178 s:7 Compare, Branch, and Halt Instructions] BI encodes E and D options.
            // [SPU-ISA p:179 s:7 Compare, Branch, and Halt Instructions] IRET encodes the same fields.
            K::Bi | K::Iret => vec![
                (0x0000_3f80, C::Register),
                (0x0004_0000, C::Flag),
                (0x0008_0000, C::Flag),
            ],
            K::Bisl | K::Bisled | K::Biz | K::Binz | K::Bihz | K::Bihnz => {
                // [SPU-ISA p:181 s:7 Compare, Branch, and Halt Instructions] BISL encodes E and D options.
                // [SPU-ISA p:180 s:7 Compare, Branch, and Halt Instructions] BISLED encodes E and D options.
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
            // [SPU-ISA p:244 s:10 Control Instructions] MFSPR carries RT and the SPR number SA in the RA field.
            // [SPU-ISA p:245 s:10 Control Instructions] MTSPR uses the same fields.
            K::Mfspr | K::Mtspr => vec![(0x0000_007f, C::Register), (0x0000_3f80, C::Immediate)],
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
        // [SPU-ISA p:179 s:7 Compare, Branch, and Halt Instructions] IRET reserves E and D set together.
        K::Bi | K::Iret => word & 0x000c_0000 != 0x000c_0000,
        // [SPU-ISA p:181 s:7 Compare, Branch, and Halt Instructions] BISL reserves E and D set together.
        // [SPU-ISA p:180 s:7 Compare, Branch, and Halt Instructions] BISLED reserves E and D set together.
        // [SPU-ISA p:186 s:7 Compare, Branch, and Halt Instructions] BIZ reserves E and D set together.
        // [SPU-ISA p:187 s:7 Compare, Branch, and Halt Instructions] BINZ reserves E and D set together.
        // [SPU-ISA p:188 s:7 Compare, Branch, and Halt Instructions] BIHZ reserves E and D set together.
        // [SPU-ISA p:189 s:7 Compare, Branch, and Halt Instructions] BIHNZ reserves E and D set together.
        K::Bisl | K::Bisled | K::Biz | K::Binz | K::Bihz | K::Bihnz => {
            word & 0x000c_0000 != 0x000c_0000
        }
        // [SPU-ISA p:192 s:8 Hint-for-Branch Instructions] P requires the split RO field to be zero.
        K::Hbr => word & 0x0010_0000 == 0 || word & 0x0000_c07f == 0,
        // [SPU-ISA p:220 s:9] and [SPU-ISA p:221 s:9]: an I8 whose scale falls outside 0..=127 has an undefined result.
        K::Csflt | K::Cuflt => {
            crate::exec::scale(spu_isa::TO_FLOAT_SCALE_BIAS, (word >> 14) as u8).is_some()
        }
        K::Cflts | K::Cfltu => {
            crate::exec::scale(spu_isa::TO_INTEGER_SCALE_BIAS, (word >> 14) as u8).is_some()
        }
        _ => true,
    }
}
