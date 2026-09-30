//! addx, cg, cgx, sfx, bg and bgx: RT as an input, the borrow split, and a
//! 64-bit add built from them.

use super::*;
use crate::state::SpuState;

/// `op rt, ra, rb` in the RR form.
///
/// [SPU-ISA p:28 s:2.3] RR: 11-bit opcode, RB, RA, RT.
fn rr(op: u32, rt: u32, ra: u32, rb: u32) -> u32 {
    op << 21 | rb << 14 | ra << 7 | rt
}

const ADDX: u32 = 0x340;
const CG: u32 = 0x0C2;
const CGX: u32 = 0x342;
const SFX: u32 = 0x341;
const BG: u32 = 0x042;
const BGX: u32 = 0x343;

fn set(s: &mut SpuState, r: u8, w: [u32; 4]) {
    for (slot, v) in w.into_iter().enumerate() {
        s.set_reg_word_slot(r, slot, v);
    }
}

fn get(s: &SpuState, r: u8) -> [u32; 4] {
    [0, 1, 2, 3].map(|slot| s.reg_word_slot(r, slot))
}

/// Runs `op rt=3, ra=1, rb=2` with RT's input words `t`.
fn run(op: u32, a: [u32; 4], b: [u32; 4], t: [u32; 4]) -> [u32; 4] {
    let mut s = SpuState::new();
    set(&mut s, 1, a);
    set(&mut s, 2, b);
    set(&mut s, 3, t);
    let insn = crate::decode::decode(rr(op, 3, 1, 2)).expect("decodes");
    assert_eq!(
        execute(&insn, &mut s, UnitId::new(0)),
        SpuStepOutcome::Continue
    );
    get(&s, 3)
}

const A: [u32; 4] = [0xFFFF_FFFF, 5, 7, 0x8000_0000];
const B: [u32; 4] = [1, 5, 9, 0x8000_0000];

#[test]
fn addx_and_cgx_take_the_low_bit_of_rt_as_carry_in() {
    assert_eq!(run(ADDX, A, B, [0, 0, 0, 0]), [0, 10, 16, 0]);
    assert_eq!(run(ADDX, A, B, [1, 1, 1, 1]), [1, 11, 17, 1]);
    assert_eq!(run(CGX, A, B, [0, 0, 0, 0]), [1, 0, 0, 1]);
    assert_eq!(
        run(CGX, [0xFFFF_FFFF, 0xFFFF_FFFE, 0, 0], [0, 1, 0, 0], [1; 4]),
        [1, 1, 0, 0]
    );
}

#[test]
fn cg_puts_the_carry_out_in_bit_31_and_zeros_the_rest() {
    assert_eq!(run(CG, A, B, [0xFFFF_FFFF; 4]), [1, 0, 0, 1]);
}

#[test]
fn sfx_subtracts_one_more_when_the_borrow_in_is_clear() {
    assert_eq!(
        run(SFX, [5, 5, 9, 0], [9, 9, 5, 0], [1, 0, 1, 0]),
        [4, 3, 0xFFFF_FFFC, 0xFFFF_FFFF]
    );
}

#[test]
fn bg_is_one_when_rb_is_at_least_ra_unsigned() {
    assert_eq!(
        run(BG, [5, 5, 6, 0xFFFF_FFFF], [5, 6, 5, 1], [0; 4]),
        [1, 1, 0, 0]
    );
}

#[test]
fn bgx_compares_at_least_with_the_bit_set_and_greater_with_it_clear() {
    let a = [5, 5, 5, 5];
    let b = [5, 5, 6, 4];
    assert_eq!(run(BGX, a, b, [1, 0, 0, 1]), [1, 0, 1, 0]);
}

#[test]
fn only_bit_31_of_the_rt_input_counts() {
    // The ISA reserves bits 0 to 30 of the RT input; the RTL reads bit 31 alone.
    assert_eq!(run(ADDX, A, B, [0xFFFF_FFFE; 4]), [0, 10, 16, 0]);
    assert_eq!(run(BGX, [5; 4], [5; 4], [0xFFFF_FFFE; 4]), [0; 4]);
}

#[test]
fn cg_then_addx_add_two_64_bit_numbers_across_the_word_carry() {
    // Doublewords in slots 0:1: 0x1_FFFF_FFFF + 0x1 = 0x2_0000_0000.
    let mut s = SpuState::new();
    set(&mut s, 1, [0x0000_0001, 0xFFFF_FFFF, 0, 0]);
    set(&mut s, 2, [0x0000_0000, 0x0000_0001, 0, 0]);
    // [SPU-ISA p:125 s:6] shlqbyi shifts the carries left one word, from each low word to its high word.
    let program = [
        rr(CG, 4, 1, 2),
        (0x1FF << 21) | (4 << 14) | (4 << 7) | 4,
        rr(ADDX, 4, 1, 2),
    ];
    for raw in program {
        let insn = crate::decode::decode(raw).expect("decodes");
        assert_eq!(
            execute(&insn, &mut s, UnitId::new(0)),
            SpuStepOutcome::Continue
        );
    }
    assert_eq!(get(&s, 4)[..2], [0x0000_0002, 0x0000_0000]);
}

#[test]
fn an_rt_that_is_also_ra_or_rb_supplies_both_inputs_before_the_write() {
    let mut s = SpuState::new();
    // addx r3, r3, r2: r3 is both the RA operand and the carry-in.
    set(&mut s, 2, [10, 10, 0, 5]);
    set(&mut s, 3, [1, 2, 0xFFFF_FFFF, 0]);
    let insn = crate::decode::decode(rr(ADDX, 3, 3, 2)).expect("decodes");
    assert_eq!(
        execute(&insn, &mut s, UnitId::new(0)),
        SpuStepOutcome::Continue
    );
    assert_eq!(get(&s, 3), [12, 12, 0, 5]);
    // bgx r3, r1, r3: r3 is both the RB operand and the borrow-in.
    set(&mut s, 1, [5; 4]);
    set(&mut s, 3, [5, 6, 4, 7]);
    let insn = crate::decode::decode(rr(BGX, 3, 1, 3)).expect("decodes");
    assert_eq!(
        execute(&insn, &mut s, UnitId::new(0)),
        SpuStepOutcome::Continue
    );
    assert_eq!(get(&s, 3), [1, 1, 0, 1]);
}
