//! The conversion family: csflt, cuflt, cflts and cfltu over the
//! defined scales and every undefined I8.

use std::collections::BTreeMap;

use cellgov_ps3_abi::hw::spu_isa::{TO_FLOAT_SCALE_BIAS, TO_INTEGER_SCALE_BIAS};

use super::builder::{from_words, Builder, RA, RT};
use crate::spu_documented::model::{Insn, Op};
use crate::spu_documented::question::VectorClass;

/// The scales the conversion vectors cover inside 0 to 127.
pub(crate) const TO_FLOAT_SCALES: [u8; 11] = [0, 1, 2, 23, 24, 31, 32, 64, 100, 126, 127];
pub(crate) const TO_INTEGER_SCALES: [u8; 10] = [0, 1, 8, 23, 24, 31, 32, 64, 126, 127];

/// [SPU-ISA p:220 s:9] csflt and [SPU-ISA p:222 s:9] cuflt: scale = 155 - I8; [SPU-ISA p:221 s:9] cflts and [SPU-ISA p:223 s:9] cfltu: scale = 173 - I8; outside 0 to 127 the result is undefined.
pub(super) fn conversion(b: &mut Builder) {
    for (op, bias) in [
        (Op::Csflt, TO_FLOAT_SCALE_BIAS),
        (Op::Cuflt, TO_FLOAT_SCALE_BIAS),
    ] {
        let mut integers = vec![
            0,
            1,
            0xFFFF_FFFF,
            2,
            0x7F_FFFF,
            0xFF_FFFF,
            0x100_0000,
            0x100_0001,
            0x7FFF_FFFF,
            0x8000_0000,
            0x8000_0001,
            0xFFFF_FF00,
        ];
        integers.extend((0..4).map(|_| b.rng.next() as u32));
        for &scale in &TO_FLOAT_SCALES {
            let insn = Insn::scaled(op, RT, RA, bias - scale);
            let slots = integers
                .iter()
                .map(|&i| (format!("{}:scale{scale:03}", op.mnemonic()), [i, 0, 0]))
                .collect();
            b.single_vectors(
                &format!("scale{scale:03}"),
                VectorClass::Conversion,
                insn,
                slots,
            );
        }
        undefined_scales(b, op, bias, [1, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF_FFFF]);
    }
    for (op, bias) in [
        (Op::Cflts, TO_INTEGER_SCALE_BIAS),
        (Op::Cfltu, TO_INTEGER_SCALE_BIAS),
    ] {
        let mut floats = vec![
            0,
            0x8000_0000,
            0x0000_0001,
            0x3F00_0000,
            0x3F80_0000,
            0xBF80_0000,
            0x3FC0_0000,
            0x4B00_0000,
            0x4EFF_FFFF,
            0x4F00_0000,
            0xCF00_0000,
            0xCF00_0001,
            0x4F7F_FFFF,
            0x4F80_0000,
            0x7FFF_FFFF,
            0xFFFF_FFFF,
            0x7F80_0001,
        ];
        floats.extend((0..3).map(|_| b.rng.next() as u32));
        for &scale in &TO_INTEGER_SCALES {
            let insn = Insn::scaled(op, RT, RA, bias - scale);
            let slots = floats
                .iter()
                .map(|&f| (format!("{}:scale{scale:03}", op.mnemonic()), [f, 0, 0]))
                .collect();
            b.single_vectors(
                &format!("scale{scale:03}"),
                VectorClass::Conversion,
                insn,
                slots,
            );
        }
        undefined_scales(b, op, bias, [0x3F80_0000, 0xBF80_0000, 0x4F00_0000, 0]);
    }
}

/// One vector per I8 whose scale falls outside 0 to 127.
fn undefined_scales(b: &mut Builder, op: Op, bias: u8, operands: [u32; 4]) {
    for imm in 0..=255u8 {
        let scale = i32::from(bias) - i32::from(imm);
        if (0..=127).contains(&scale) {
            continue;
        }
        let insn = Insn::scaled(op, RT, RA, imm);
        let registers = BTreeMap::from([(RA, from_words(operands))]);
        let fpscr = b.start_fpscr();
        let cell = format!("{}:undefined-i8-{imm:03}", op.mnemonic());
        let id = format!("{}/undefined-i8/{imm:03}", op.mnemonic());
        b.push(
            id,
            VectorClass::Conversion,
            vec![cell; 4],
            &[insn],
            registers,
            fpscr,
        );
    }
}
