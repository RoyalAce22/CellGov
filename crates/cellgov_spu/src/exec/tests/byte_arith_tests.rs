//! Byte arithmetic: avgb without overflow, absdb unsigned in both operand
//! orders, and sumb's RB-high / RA-low halfword placement.

use super::*;
use crate::state::SpuState;

// [SPU-ISA p:28 s:2.3] RR: 11-bit opcode, RB, RA, RT.
fn rr(op: u32, rt: u32, ra: u32, rb: u32) -> u32 {
    op << 21 | rb << 14 | ra << 7 | rt
}

const AVGB: u32 = 0x0D3;
const ABSDB: u32 = 0x053;
const SUMB: u32 = 0x253;

/// Runs `op` with `a` in r1, `b` in r2 and junk in r3, and returns r3.
fn run(op: u32, a: [u8; 16], b: [u8; 16]) -> [u8; 16] {
    let mut s = SpuState::new();
    s.regs[1] = a;
    s.regs[2] = b;
    s.regs[3] = [0xAA; 16];
    let insn = crate::decode::decode(rr(op, 3, 1, 2)).expect("decodes");
    assert_eq!(
        execute(&insn, &mut s, UnitId::new(0)),
        SpuStepOutcome::Continue
    );
    s.regs[3]
}

fn bytes(head: [u8; 4]) -> [u8; 16] {
    std::array::from_fn(|j| head.get(j).copied().unwrap_or(0))
}

#[test]
fn avgb_keeps_the_ninth_bit_and_rounds_up() {
    assert_eq!(
        run(
            AVGB,
            bytes([0xFF, 0x00, 0x80, 0x01]),
            bytes([0xFF, 0x01, 0x7F, 0x02])
        ),
        bytes([0xFF, 0x01, 0x80, 0x02])
    );
}

#[test]
fn absdb_is_unsigned_in_both_operand_orders() {
    let a = bytes([10, 3, 0x00, 0xFF]);
    let b = bytes([3, 10, 0xFF, 0x00]);
    assert_eq!(run(ABSDB, a, b), bytes([7, 7, 0xFF, 0xFF]));
}

#[test]
fn sumb_puts_the_rb_sum_high_and_the_ra_sum_low() {
    let a = [0xFF; 16];
    let b: [u8; 16] = std::array::from_fn(|j| j as u8);
    assert_eq!(
        run(SUMB, a, b),
        from_words([
            6 << 16 | 0x3FC,
            22 << 16 | 0x3FC,
            38 << 16 | 0x3FC,
            54 << 16 | 0x3FC
        ])
    );
}
