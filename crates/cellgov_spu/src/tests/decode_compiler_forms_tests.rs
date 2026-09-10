//! Decode of the forms a C compiler's startup code and runtime emit,
//! from words taken out of the micro-test SPU images.

use super::*;

#[test]
fn stqr_from_binary() {
    // stqr $6, 0xa80 -> 0x23815006 (i16 = 672 words)
    let insn = decode(0x2381_5006).unwrap();
    assert_eq!(insn, SpuInstruction::Stqr { rt: 6, imm: 672 });
}

#[test]
fn lqr_from_binary() {
    // lqr $4, 0x14 -> 0x33800284
    let insn = decode(0x3380_0284).unwrap();
    assert_eq!(insn, SpuInstruction::Lqr { rt: 4, imm: 5 });
}

#[test]
fn lqr_negative_offset_sign_extends() {
    // i16 = 0xFFFC (-4)
    let insn = decode((0x067 << 23) | (0xFFFC << 7) | 9).unwrap();
    assert_eq!(insn, SpuInstruction::Lqr { rt: 9, imm: -4 });
}

#[test]
fn brhnz_from_binary() {
    // brhnz $4, +0x50 -> 0x23000a04
    let insn = decode(0x2300_0a04).unwrap();
    assert_eq!(insn, SpuInstruction::Brhnz { rt: 4, offset: 20 });
}

#[test]
fn bisl_from_binary() {
    // bisl $0, $4 -> 0x35200200
    let insn = decode(0x3520_0200).unwrap();
    assert_eq!(insn, SpuInstruction::Bisl { rt: 0, ra: 4 });
}

#[test]
fn bisl_interrupt_bits_do_not_change_the_decode() {
    // [SPU-ISA p:181 s:7 Bisl] D is bit 12 and E is bit 13 (big-endian);
    // bit 11 is reserved. The test sets each bit on its own because the
    // D = E = 1 combination is undefined.
    let d_bit = 1 << (31 - 12);
    let e_bit = 1 << (31 - 13);
    let with_d = decode(0x3520_0200 | d_bit).unwrap();
    assert_eq!(with_d, SpuInstruction::Bisl { rt: 0, ra: 4 });
    let with_e = decode(0x3520_0200 | e_bit).unwrap();
    assert_eq!(with_e, SpuInstruction::Bisl { rt: 0, ra: 4 });
}

#[test]
fn or_from_binary() {
    // or $33, $32, $28 -> 0x08271021
    let insn = decode(0x0827_1021).unwrap();
    assert_eq!(
        insn,
        SpuInstruction::Or {
            rt: 33,
            ra: 32,
            rb: 28
        }
    );
}

#[test]
fn and_from_binary() {
    // and $24, $22, $23 -> 0x1825cb18
    let insn = decode(0x1825_cb18).unwrap();
    assert_eq!(
        insn,
        SpuInstruction::And {
            rt: 24,
            ra: 22,
            rb: 23
        }
    );
}

#[test]
fn shl_from_binary() {
    // shl $28, $27, $19 -> 0x0b64cd9c
    let insn = decode(0x0b64_cd9c).unwrap();
    assert_eq!(
        insn,
        SpuInstruction::Shl {
            rt: 28,
            ra: 27,
            rb: 19
        }
    );
}

#[test]
fn rotmi_from_binary() {
    // rotmi $8, $7, -31 -> 0x0f384388 (I7 = 0x61)
    let insn = decode(0x0f38_4388).unwrap();
    assert_eq!(
        insn,
        SpuInstruction::Rotmi {
            rt: 8,
            ra: 7,
            imm: 0x61
        }
    );
}

#[test]
fn rotmai_from_binary() {
    // rotmai $7, $6, -2 -> 0x0f5f8307 (I7 = 0x7E)
    let insn = decode(0x0f5f_8307).unwrap();
    assert_eq!(
        insn,
        SpuInstruction::Rotmai {
            rt: 7,
            ra: 6,
            imm: 0x7E
        }
    );
}

#[test]
fn shli_from_binary() {
    // shli $82, $2, 2 -> 0x0f608152
    let insn = decode(0x0f60_8152).unwrap();
    assert_eq!(
        insn,
        SpuInstruction::Shli {
            rt: 82,
            ra: 2,
            imm: 2
        }
    );
}

#[test]
fn rotqbyi_from_binary() {
    // rotqbyi $4, $2, 12 -> 0x3f830104
    let insn = decode(0x3f83_0104).unwrap();
    assert_eq!(
        insn,
        SpuInstruction::Rotqbyi {
            rt: 4,
            ra: 2,
            imm: 12
        }
    );
}

#[test]
fn rotqbyi_is_distinct_from_shlqbyi() {
    // Same fields, opcode 0x1FF -> shlqbyi, so the two never cross.
    let insn = decode(0x3fe3_0104).unwrap();
    assert_eq!(
        insn,
        SpuInstruction::Shlqbyi {
            rt: 4,
            ra: 2,
            imm: 12
        }
    );
}

#[test]
fn cgti_from_binary() {
    // cgti $16, $19, 31 -> 0x4c07c990
    let insn = decode(0x4c07_c990).unwrap();
    assert_eq!(
        insn,
        SpuInstruction::Cgti {
            rt: 16,
            ra: 19,
            imm: 31
        }
    );
}

#[test]
fn cgti_negative_immediate_sign_extends() {
    // cgti $3, $81, -1 -> 0x4cffe883
    let insn = decode(0x4cff_e883).unwrap();
    assert_eq!(
        insn,
        SpuInstruction::Cgti {
            rt: 3,
            ra: 81,
            imm: -1
        }
    );
}

#[test]
fn xsbh_from_binary() {
    // xsbh $4, $2 -> 0x56c00104
    let insn = decode(0x56c0_0104).unwrap();
    assert_eq!(insn, SpuInstruction::Xsbh { rt: 4, ra: 2 });
}

#[test]
fn clgt_from_binary() {
    // clgt $13, $80, $10 -> 0x5802a80d
    let insn = decode(0x5802_a80d).unwrap();
    assert_eq!(
        insn,
        SpuInstruction::Clgt {
            rt: 13,
            ra: 80,
            rb: 10
        }
    );
}

#[test]
fn selb_from_binary() {
    // selb $1, $1, $3, $4 -> 0x8020c084
    let insn = decode(0x8020_c084).unwrap();
    assert_eq!(
        insn,
        SpuInstruction::Selb {
            rt: 1,
            ra: 1,
            rb: 3,
            rc: 4
        }
    );
}
