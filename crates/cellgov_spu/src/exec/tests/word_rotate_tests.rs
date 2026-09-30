//! Word rotates and register-count right shifts: per-slot counts, the
//! 31 / 32 / 63 edges, and the ISA's sfi + rotm idiom.

use super::*;
use crate::state::SpuState;

/// [SPU-ISA p:28 s:2.3] RR and RI7: 11-bit opcode, RB or I7, RA, RT.
/// [SPU-ISA p:29 s:2.3] RI10: 8-bit opcode, I10, RA, RT.
fn rr(op: u32, rt: u32, ra: u32, rb: u32) -> u32 {
    op << 21 | rb << 14 | ra << 7 | rt
}

const ROT: u32 = 0x058;
const ROTI: u32 = 0x078;
const ROTM: u32 = 0x059;
const ROTMA: u32 = 0x05A;
const SFI: u32 = 0x0C;

/// Runs `raw` with `a` in r1, `b` in r2 and junk in r3, and returns r3's words.
fn run(raw: u32, a: u32, b: [u32; 4]) -> [u32; 4] {
    let mut s = SpuState::new();
    s.set_reg(1, from_words([a; 4]));
    s.set_reg(2, from_words(b));
    s.set_reg(3, [0xAA; 16]);
    step(&mut s, raw);
    words(s.regs[3])
}

fn step(s: &mut SpuState, raw: u32) {
    let insn = crate::decode::decode(raw).expect("decodes");
    assert_eq!(execute(&insn, s, UnitId::new(0)), SpuStepOutcome::Continue);
}

/// The two's complements of right counts 31, 32, 63 and 1.
const RIGHT: [u32; 4] = [0xFFFF_FFE1, 0xFFFF_FFE0, 0xFFFF_FFC1, 0xFFFF_FFFF];

#[test]
fn rot_and_roti_rotate_by_the_low_5_bits() {
    assert_eq!(
        run(rr(ROT, 3, 1, 2), 0x8000_0001, [0, 1, 31, 33]),
        [0x8000_0001, 0x0000_0003, 0xC000_0000, 0x0000_0003]
    );
    // I7 0x7F is -1, whose low 5 bits are 31.
    for (i7, want) in [(0x7F, 0xC000_0000), (0x04, 0x0000_0018)] {
        assert_eq!(run(rr(ROTI, 3, 1, i7), 0x8000_0001, [0; 4]), [want; 4]);
    }
}

#[test]
fn rotm_and_rotma_shift_right_by_the_negated_count_modulo_64() {
    assert_eq!(
        run(rr(ROTM, 3, 1, 2), 0x8000_0001, RIGHT),
        [1, 0, 0, 0x4000_0000]
    );
    assert_eq!(
        run(rr(ROTMA, 3, 1, 2), 0x8000_0001, RIGHT),
        [u32::MAX, u32::MAX, u32::MAX, 0xC000_0000]
    );
    assert_eq!(
        run(rr(ROTMA, 3, 1, 2), 0x4000_0002, RIGHT),
        [0, 0, 0, 0x2000_0001]
    );
}

/// [SPU-ISA p:138 s:6. Shift and Rotate Instructions] A logical right shift by
/// a register count is sfi to negate the count, then rotm.
#[test]
fn sfi_then_rotm_is_a_logical_right_shift_by_a_register_count() {
    let mut s = SpuState::new();
    s.set_reg(1, from_words([0, 5, 31, 32]));
    s.set_reg(2, from_words([0x8000_0000; 4]));
    step(&mut s, SFI << 24 | 1 << 7 | 3);
    step(&mut s, rr(ROTM, 4, 2, 3));
    assert_eq!(words(s.regs[4]), [0x8000_0000, 0x0400_0000, 1, 0]);
}
