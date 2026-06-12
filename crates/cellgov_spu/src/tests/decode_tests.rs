//! SPU instruction decode from raw binary words, including unsupported-opcode rejection.

use super::*;

#[test]
fn unsupported_returns_error() {
    let result = decode(0xFFFF_FFFF);
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err(),
        SpuDecodeError::Unsupported(0xFFFF_FFFF)
    );
}

#[test]
fn stop_zero_decodes() {
    let insn = decode(0x0000_0000).unwrap();
    assert_eq!(insn, SpuInstruction::Stop { signal: 0 });
}

#[test]
fn il_from_binary() {
    // il $3, 12288 -> 0x40980003 (from spu_fixed_value disasm)
    let insn = decode(0x4098_0003).unwrap();
    assert_eq!(insn, SpuInstruction::Il { rt: 3, imm: 12288 });
}

#[test]
fn stqd_from_binary() {
    // stqd $0, 16($1) -> 0x24004080
    let insn = decode(0x2400_4080).unwrap();
    assert_eq!(
        insn,
        SpuInstruction::Stqd {
            rt: 0,
            ra: 1,
            imm: 1
        }
    );
}

#[test]
fn ai_from_binary() {
    // ai $1, $1, -32 -> 0x1cf80081
    let insn = decode(0x1cf8_0081).unwrap();
    assert_eq!(
        insn,
        SpuInstruction::Ai {
            rt: 1,
            ra: 1,
            imm: -32
        }
    );
}

#[test]
fn ilhu_from_binary() {
    // ilhu $16, 4919 -> 0x41099b90
    let insn = decode(0x4109_9b90).unwrap();
    assert_eq!(insn, SpuInstruction::Ilhu { rt: 16, imm: 4919 });
}

#[test]
fn iohl_from_binary() {
    // iohl $16, 47789 -> 0x60dd5690
    let insn = decode(0x60dd_5690).unwrap();
    assert_eq!(insn, SpuInstruction::Iohl { rt: 16, imm: 47789 });
}

#[test]
fn wrch_from_binary() {
    // wrch $ch16, $9 -> 0x21a00809
    let insn = decode(0x21a0_0809).unwrap();
    assert_eq!(insn, SpuInstruction::Wrch { channel: 16, rt: 9 });
}

#[test]
fn rdch_from_binary() {
    // rdch $2, $ch24 -> 0x01a00c02
    let insn = decode(0x01a0_0c02).unwrap();
    assert_eq!(insn, SpuInstruction::Rdch { rt: 2, channel: 24 });
}

#[test]
fn ori_from_binary() {
    // ori $9, $3, 0 -> 0x04000189
    let insn = decode(0x0400_0189).unwrap();
    assert_eq!(
        insn,
        SpuInstruction::Ori {
            rt: 9,
            ra: 3,
            imm: 0
        }
    );
}

#[test]
fn bi_from_binary() {
    // bi $0 -> 0x35000000
    let insn = decode(0x3500_0000).unwrap();
    assert_eq!(insn, SpuInstruction::Bi { ra: 0 });
}

#[test]
fn brsl_from_binary() {
    // brsl $0, 0x378 -> 0x33006d00 (offset from PC)
    let insn = decode(0x3300_6d00).unwrap();
    assert!(matches!(insn, SpuInstruction::Brsl { rt: 0, .. }));
}

#[test]
fn nop_from_binary() {
    // nop $127 -> 0x4020007f
    let insn = decode(0x4020_007f).unwrap();
    assert_eq!(insn, SpuInstruction::Nop);
}

#[test]
fn lnop_from_binary() {
    // lnop -> 0x00200000
    let insn = decode(0x0020_0000).unwrap();
    assert_eq!(insn, SpuInstruction::Lnop);
}

#[test]
fn shufb_from_binary() {
    // shufb $11, $2, $8, $5 -> 0xb1620105
    let insn = decode(0xb162_0105).unwrap();
    assert_eq!(
        insn,
        SpuInstruction::Shufb {
            rt: 11,
            ra: 2,
            rb: 8,
            rc: 5
        }
    );
}

#[test]
fn cwd_from_binary() {
    // cwd $5, 0($3) -> 0x3ec00185
    let insn = decode(0x3ec0_0185).unwrap();
    assert_eq!(
        insn,
        SpuInstruction::Cwd {
            rt: 5,
            ra: 3,
            imm: 0
        }
    );
}
