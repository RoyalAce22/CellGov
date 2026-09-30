//! Quadword bit shifts and rotates: every count 0 to 7, bits crossing byte
//! and word boundaries, and only the count's low 3 bits read.

use super::*;
use crate::state::SpuState;

// [SPU-ISA p:28 s:2.3] RR and RI7: 11-bit opcode, RB or I7, RA, RT.
fn rr(op: u32, rt: u32, ra: u32, rb: u32) -> u32 {
    op << 21 | rb << 14 | ra << 7 | rt
}

const SHLQBI: u32 = 0x1DB;
const SHLQBII: u32 = 0x1FB;
const ROTQBI: u32 = 0x1D8;
const ROTQBII: u32 = 0x1F8;
const ROTQMBI: u32 = 0x1D9;
const ROTQMBII: u32 = 0x1F9;

/// Bits in byte 0's top, word 1's top (byte 4) and byte 15's bottom, so a
/// one-bit move crosses a byte, a word and the register's ends.
const A: [u8; 16] = [0x80, 0, 0, 0, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01];

/// Runs `raw` with `a` in r1, `count` in r2's preferred slot beside junk
/// slots, and junk in r3, and returns r3.
fn run(raw: u32, a: [u8; 16], count: u32) -> [u8; 16] {
    let mut s = SpuState::new();
    s.regs[1] = a;
    s.regs[2] = from_words([count, u32::MAX, u32::MAX, u32::MAX]);
    s.regs[3] = [0xAA; 16];
    let insn = crate::decode::decode(raw).expect("decodes");
    assert_eq!(
        execute(&insn, &mut s, UnitId::new(0)),
        SpuStepOutcome::Continue
    );
    s.regs[3]
}

#[test]
fn a_one_bit_move_crosses_bytes_words_and_the_register_ends() {
    let mut left = [0u8; 16];
    left[3] = 0x01;
    left[15] = 0x02;
    assert_eq!(run(rr(SHLQBI, 3, 1, 2), A, 1), left);
    left[15] = 0x03;
    assert_eq!(run(rr(ROTQBI, 3, 1, 2), A, 1), left);
    let mut right = [0u8; 16];
    right[0] = 0x40;
    right[4] = 0x40;
    // A rotate-and-mask count of -1 is a right shift by 1.
    assert_eq!(run(rr(ROTQMBI, 3, 1, 2), A, u32::MAX), right);
}

#[test]
fn every_count_reads_only_the_low_3_bits() {
    let q = u128::from_be_bytes(A);
    for count in 0..8u32 {
        let left = (q << count).to_be_bytes();
        let rotated = q.rotate_left(count).to_be_bytes();
        let right = (q >> count).to_be_bytes();
        let negated = 8u32.wrapping_sub(count) & 7;
        // The register forms carry set bits above bit 29 of the count; the
        // immediate forms a negative I7.
        let high = 0xFFFF_FFF8;
        assert_eq!(run(rr(SHLQBI, 3, 1, 2), A, high | count), left, "{count}");
        assert_eq!(
            run(rr(ROTQBI, 3, 1, 2), A, high | count),
            rotated,
            "{count}"
        );
        assert_eq!(
            run(rr(ROTQMBI, 3, 1, 2), A, high | negated),
            right,
            "{count}"
        );
        assert_eq!(run(rr(SHLQBII, 3, 1, 0x78 | count), A, 0), left, "{count}");
        assert_eq!(
            run(rr(ROTQBII, 3, 1, 0x78 | count), A, 0),
            rotated,
            "{count}"
        );
        assert_eq!(
            run(rr(ROTQMBII, 3, 1, 0x78 | negated), A, 0),
            right,
            "{count}"
        );
    }
}
