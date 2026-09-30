//! Sign extension: xshw and xswd with the sign bit set and clear in every
//! slot, and junk in the bits each form replaces.

use super::*;
use crate::state::SpuState;

/// [SPU-ISA p:28 s:2.3] RR: 11-bit opcode, RB, RA, RT.
fn rr(op: u32, rt: u32, ra: u32) -> u32 {
    op << 21 | ra << 7 | rt
}

const XSHW: u32 = 0x2AE;
const XSWD: u32 = 0x2A6;

/// Runs `op` with `a` in r1 and junk in r3, and returns r3's words.
fn run(op: u32, a: [u32; 4]) -> [u32; 4] {
    let mut s = SpuState::new();
    s.regs[1] = from_words(a);
    s.regs[3] = [0xAA; 16];
    let insn = crate::decode::decode(rr(op, 3, 1)).expect("decodes");
    assert_eq!(
        execute(&insn, &mut s, UnitId::new(0)),
        SpuStepOutcome::Continue
    );
    words(s.regs[3])
}

#[test]
fn xshw_extends_each_right_halfword() {
    assert_eq!(
        run(XSHW, [0x1234_8000, 0xFFFF_7FFF, 0x0000_FFFF, 0x8000_0001]),
        [0xFFFF_8000, 0x0000_7FFF, 0xFFFF_FFFF, 0x0000_0001]
    );
}

#[test]
fn xswd_extends_each_right_word() {
    assert_eq!(
        run(XSWD, [0x1234_5678, 0x8000_0000, 0xFFFF_FFFF, 0x7FFF_FFFF]),
        [0xFFFF_FFFF, 0x8000_0000, 0x0000_0000, 0x7FFF_FFFF]
    );
    assert_eq!(
        run(XSWD, [0, 0x0000_0001, 0, 0xFFFF_FFFE]),
        [0, 0x0000_0001, 0xFFFF_FFFF, 0xFFFF_FFFE]
    );
}
