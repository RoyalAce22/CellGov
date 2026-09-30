//! Quadword byte shifts with register counts: the byte-count and bit-count
//! fields, the zero result above 15, and only the preferred slot read.

use super::*;
use crate::state::SpuState;

/// [SPU-ISA p:28 s:2.3] RR: 11-bit opcode, RB, RA, RT.
fn rr(op: u32, rt: u32, ra: u32, rb: u32) -> u32 {
    op << 21 | rb << 14 | ra << 7 | rt
}

const SHLQBY: u32 = 0x1DF;
const SHLQBYBI: u32 = 0x1CF;
const ROTQBYBI: u32 = 0x1CC;
const ROTQMBY: u32 = 0x1DD;
const ROTQMBYBI: u32 = 0x1CD;
const ROTQBY: u32 = 0x1DC;

/// Bytes 0x10 to 0x1F, so every output byte names its source.
const A: [u8; 16] = [
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F,
];

/// Runs `op` with `A` in r1, `count` in r2's preferred slot beside junk
/// slots, and junk in r3, and returns r3.
fn run(op: u32, count: u32) -> [u8; 16] {
    let mut s = SpuState::new();
    s.regs[1] = A;
    s.regs[2] = from_words([count, u32::MAX, u32::MAX, u32::MAX]);
    s.regs[3] = [0xAA; 16];
    let insn = crate::decode::decode(rr(op, 3, 1, 2)).expect("decodes");
    assert_eq!(
        execute(&insn, &mut s, UnitId::new(0)),
        SpuStepOutcome::Continue
    );
    s.regs[3]
}

/// `A` moved left `n` bytes with zero fill.
fn left(n: usize) -> [u8; 16] {
    std::array::from_fn(|i| A.get(i + n).copied().unwrap_or(0))
}

/// `A` moved right `n` bytes with zero fill.
fn right(n: usize) -> [u8; 16] {
    std::array::from_fn(|i| if i >= n { A[i - n] } else { 0 })
}

#[test]
fn shlqby_reads_a_5_bit_byte_count_and_zeroes_above_15() {
    assert_eq!(run(SHLQBY, 3), left(3));
    assert_eq!(run(SHLQBY, 0xFFFF_FFE3), left(3));
    assert_eq!(run(SHLQBY, 16), [0; 16]);
}

/// [SPU-ISA p:131 s:6. Shift and Rotate Instructions] rotqby rotates left by
/// bits 28 to 31 of RB's preferred slot.
#[test]
fn rotqby_rotates_left_by_the_low_4_bits() {
    let rotated: [u8; 16] = std::array::from_fn(|i| A[(i + 3) % 16]);
    assert_eq!(run(ROTQBY, 3), rotated);
    assert_eq!(run(ROTQBY, 19), rotated);
}

#[test]
fn the_bit_count_forms_ignore_bits_outside_the_count_field() {
    assert_eq!(run(SHLQBYBI, 3 << 3 | 7), left(3));
    assert_eq!(run(SHLQBYBI, 0xFFFF_FF00 | 3 << 3), left(3));
    assert_eq!(run(SHLQBYBI, 16 << 3), [0; 16]);
    let rotated: [u8; 16] = std::array::from_fn(|i| A[(i + 3) % 16]);
    assert_eq!(run(ROTQBYBI, 3 << 3 | 7), rotated);
    // The RTL reads bits 24 to 28 and rotates modulo 16, so a count of 19
    // rotates like 3: bit 24 of the count has no effect.
    assert_eq!(run(ROTQBYBI, 19 << 3), rotated);
}

#[test]
fn the_rotate_and_mask_forms_shift_right_by_the_negated_count() {
    assert_eq!(run(ROTQMBY, 0u32.wrapping_sub(3)), right(3));
    assert_eq!(run(ROTQMBY, 0u32.wrapping_sub(16)), [0; 16]);
    assert_eq!(run(ROTQMBYBI, 0u32.wrapping_sub(3) << 3 | 7), right(3));
    assert_eq!(run(ROTQMBYBI, 0u32.wrapping_sub(16) << 3), [0; 16]);
}
