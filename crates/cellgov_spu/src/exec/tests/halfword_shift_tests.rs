//! Halfword shifts and rotates: each halfword's own count, counts at the
//! 15 / 16 edge, and the rotate-and-mask forms' two's-complement count.

use super::*;
use crate::state::SpuState;

/// [SPU-ISA p:28 s:2.3] RR and RI7: 11-bit opcode, RB or I7, RA, RT.
fn rr(op: u32, rt: u32, ra: u32, rb: u32) -> u32 {
    op << 21 | rb << 14 | ra << 7 | rt
}

const SHLH: u32 = 0x05F;
const SHLHI: u32 = 0x07F;
const ROTH: u32 = 0x05C;
const ROTHI: u32 = 0x07C;
const ROTHM: u32 = 0x05D;
const ROTHMI: u32 = 0x07D;
const ROTMAH: u32 = 0x05E;
const ROTMAHI: u32 = 0x07E;

/// Runs `raw` with `a` in r1, `b` in r2 and junk in r3, and returns r3's
/// halfwords.
fn run(raw: u32, a: u16, b: [u16; 8]) -> [u16; 8] {
    let mut s = SpuState::new();
    s.regs[1] = from_halfwords([a; 8]);
    s.regs[2] = from_halfwords(b);
    s.regs[3] = [0xAA; 16];
    let insn = crate::decode::decode(raw).expect("decodes");
    assert_eq!(
        execute(&insn, &mut s, UnitId::new(0)),
        SpuStepOutcome::Continue
    );
    halfwords(s.regs[3])
}

/// Left counts 0, 1, 15, 16, 31, 5, then 35 and 0xFFE1, whose bits above the
/// field the forms ignore.
const LEFT: [u16; 8] = [0, 1, 15, 16, 31, 5, 35, 0xFFE1];
/// The two's complements of right counts 0, 1, 15, 16, 31, 5, then 32 and 63,
/// which reduce to 0 and 1 modulo 32.
const RIGHT: [u16; 8] = [0, 0xFFFF, 0xFFF1, 0xFFF0, 0xFFE1, 0xFFFB, 0x20, 0x3F];

#[test]
fn shlh_and_roth_use_each_halfwords_own_count() {
    assert_eq!(
        run(rr(SHLH, 3, 1, 2), 0x8001, LEFT),
        [0x8001, 0x0002, 0x8000, 0, 0, 0x0020, 0x0008, 0x0002]
    );
    // roth keeps only the low 4 bits: 0, 1, 15, 0, 15, 5, 3, 1.
    assert_eq!(
        run(rr(ROTH, 3, 1, 2), 0x8001, LEFT),
        [0x8001, 0x0003, 0xC000, 0x8001, 0xC000, 0x0030, 0x000C, 0x0003]
    );
}

#[test]
fn rothm_and_rotmah_shift_right_by_the_negated_count() {
    assert_eq!(
        run(rr(ROTHM, 3, 1, 2), 0x8001, RIGHT),
        [0x8001, 0x4000, 0x0001, 0, 0, 0x0400, 0x8001, 0x4000]
    );
    assert_eq!(
        run(rr(ROTMAH, 3, 1, 2), 0x8001, RIGHT),
        [0x8001, 0xC000, 0xFFFF, 0xFFFF, 0xFFFF, 0xFC00, 0x8001, 0xC000]
    );
    assert_eq!(
        run(rr(ROTMAH, 3, 1, 2), 0x4002, RIGHT),
        [0x4002, 0x2001, 0, 0, 0, 0x0200, 0x4002, 0x2001]
    );
}

#[test]
fn the_immediate_forms_read_the_count_from_i7() {
    for (op, i7, want) in [
        (SHLHI, 0x23, 0x0008),
        (SHLHI, 0x10, 0),
        // I7 0x63 is negative; its low 5 bits still give a count of 3.
        (SHLHI, 0x63, 0x0008),
        (ROTHI, 0x13, 0x000C),
        // I7 0x7F is -1, 0x71 is -15 and 0x60 is -32, which is 0 modulo 32.
        (ROTHMI, 0x7F, 0x4000),
        (ROTHMI, 0x71, 0x0001),
        (ROTHMI, 0x60, 0x8001),
        (ROTMAHI, 0x71, 0xFFFF),
        (ROTMAHI, 0x7B, 0xFC00),
        (ROTMAHI, 0x60, 0x8001),
    ] {
        assert_eq!(
            run(rr(op, 3, 1, i7), 0x8001, [0; 8]),
            [want; 8],
            "opcode {op:#05x} I7 {i7:#04x}"
        );
    }
}
