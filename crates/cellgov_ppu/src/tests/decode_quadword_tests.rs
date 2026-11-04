//! `lq` and `stq`: privileged, with `lq` RT = RA illegal, and each
//! rendered with its operands.

use super::*;
use crate::instruction::AsmText;

fn text(raw: u32) -> String {
    let insn = decode(raw).unwrap_or_else(|e| panic!("0x{raw:08x}: {e}"));
    AsmText {
        insn: &insn,
        addr: 0,
        symbols: None,
    }
    .to_string()
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ")
}

#[test]
fn lq_decodes_as_privileged() {
    // lq r4, 32(r10)
    let raw = 0xE08A_0020;
    assert_eq!(decode(raw), Ok(PpuInstruction::PpePrivileged { raw }));
    assert_eq!(text(raw), "lq r4, 32(r10)");
}

#[test]
fn lq_with_rt_equal_to_ra_decodes_as_illegal() {
    // lq r4, -16(r4)
    let raw = 0xE084_FFF0;
    assert_eq!(decode(raw), Ok(PpuInstruction::PpeIllegal { raw }));
    assert_eq!(text(raw), "lq r4, -16(r4)");
}

#[test]
fn stq_decodes_as_privileged() {
    // stq r4, 8(r10)
    let raw = 0xF88A_000A;
    assert_eq!(decode(raw), Ok(PpuInstruction::PpePrivileged { raw }));
    assert_eq!(text(raw), "stq r4, 8(r10)");
}

#[test]
fn the_reserved_primary_62_extended_opcode_stays_unrecognized() {
    let raw = 0xF88A_0003;
    assert_eq!(
        decode(raw),
        Err(PpuDecodeError::EncodingNotRecognized { raw })
    );
}
