//! Bit counts, form-select masks and byte gather: the bit-to-slot direction
//! and which bits of the preferred slot each form reads.

use super::*;
use crate::state::SpuState;

/// [SPU-ISA p:28 s:2.3] RR: 11-bit opcode, RB, RA, RT.
fn rr(op: u32, rt: u32, ra: u32) -> u32 {
    op << 21 | ra << 7 | rt
}

const CLZ: u32 = 0x2A5;
const CNTB: u32 = 0x2B4;
const FSMB: u32 = 0x1B6;
const FSMH: u32 = 0x1B5;
const FSM: u32 = 0x1B4;
const GBB: u32 = 0x1B2;

/// Runs `op` with `a` in r1 and junk in r3, and returns r3.
fn run(op: u32, a: [u8; 16]) -> [u8; 16] {
    let mut s = SpuState::new();
    s.regs[1] = a;
    s.regs[3] = [0xAA; 16];
    let insn = crate::decode::decode(rr(op, 3, 1)).expect("decodes");
    assert_eq!(
        execute(&insn, &mut s, UnitId::new(0)),
        SpuStepOutcome::Continue
    );
    s.regs[3]
}

#[test]
fn clz_counts_32_for_a_zero_word() {
    assert_eq!(
        run(CLZ, from_words([0, 1, 0x8000_0000, 0x0001_0000])),
        from_words([32, 31, 0, 15])
    );
}

#[test]
fn cntb_counts_each_byte() {
    let a = [
        0x00, 0xFF, 0x01, 0x80, 0xAA, 0x0F, 0x7E, 0x10, 0, 0, 0, 0, 0xFF, 0x03, 0x07, 0xFE,
    ];
    assert_eq!(
        run(CNTB, a),
        [0, 8, 1, 1, 4, 4, 6, 1, 0, 0, 0, 0, 8, 2, 3, 7]
    );
}

// Each mask source carries set bits above the field the form reads, and a
// field that is not a palindrome, so a reversed or widened read differs.
#[test]
fn fsmb_reads_the_low_16_bits_leftmost_to_byte_0() {
    let mut want = [0u8; 16];
    for byte in [0, 1, 15] {
        want[byte] = 0xFF;
    }
    assert_eq!(
        run(
            FSMB,
            from_words([0xABCD_C001, u32::MAX, u32::MAX, u32::MAX])
        ),
        want
    );
}

#[test]
fn fsmh_reads_the_low_8_bits_leftmost_to_halfword_0() {
    let mut want = [0u8; 16];
    for half in [0, 1, 7] {
        want[half * 2] = 0xFF;
        want[half * 2 + 1] = 0xFF;
    }
    assert_eq!(
        run(
            FSMH,
            from_words([0x1234_56C1, u32::MAX, u32::MAX, u32::MAX])
        ),
        want
    );
}

#[test]
fn fsm_reads_the_low_4_bits_leftmost_to_word_0() {
    assert_eq!(
        run(FSM, from_words([0xABCD_EF12, u32::MAX, u32::MAX, u32::MAX])),
        from_words([0, 0, u32::MAX, 0])
    );
}

#[test]
fn gbb_gathers_byte_low_bits_into_the_right_half_of_the_preferred_slot() {
    // Byte low bits 1100_0000_0000_0001; the other bits of each byte are set.
    let mut a = [0xFE; 16];
    for byte in [0, 1, 15] {
        a[byte] = 0xFF;
    }
    assert_eq!(run(GBB, a), from_words([0x0000_C001, 0, 0, 0]));
}
