//! Decode of `mfvscr` and `mtvscr`, and their reserved fields.

use crate::decode::decode;
use crate::instruction::PpuInstruction;

/// [AltiVec-PEM p:6-23 s:6.2] mfvscr: primary 4, vD, XO 1540.
#[test]
fn xo_1540_decodes_to_mfvscr() {
    assert_eq!(
        decode((4 << 26) | (7 << 21) | 1540),
        Ok(PpuInstruction::Mfvscr { vt: 7 })
    );
}

/// [AltiVec-PEM p:6-24 s:6.2] mtvscr: primary 4, vB, XO 1604.
#[test]
fn xo_1604_decodes_to_mtvscr() {
    assert_eq!(
        decode((4 << 26) | (9 << 11) | 1604),
        Ok(PpuInstruction::Mtvscr { vb: 9 })
    );
}

/// [PPC-Book1 p:3 s:1.5.2] A reserved field is ignored on read.
#[test]
fn the_reserved_fields_do_not_change_the_decode() {
    assert_eq!(
        decode((4 << 26) | (7 << 21) | (31 << 16) | (31 << 11) | 1540),
        Ok(PpuInstruction::Mfvscr { vt: 7 })
    );
    assert_eq!(
        decode((4 << 26) | (31 << 21) | (31 << 16) | (9 << 11) | 1604),
        Ok(PpuInstruction::Mtvscr { vb: 9 })
    );
}
