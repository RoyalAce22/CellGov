//! The 16-bit multiplies: signed and unsigned halves at 0x8000 and 0xFFFF,
//! mpys's sign extension, the addend forms, and the ISA's 32-bit multiply.

use super::*;
use crate::state::SpuState;

// [SPU-ISA p:28 s:2.3] RR: 11-bit opcode, RB, RA, RT; RRR: 4-bit opcode, RT, RB, RA, RC.
// [SPU-ISA p:29 s:2.3] RI10: 8-bit opcode, I10, RA, RT.
fn rr(op: u32, rt: u32, ra: u32, rb: u32) -> u32 {
    op << 21 | rb << 14 | ra << 7 | rt
}

fn ri10(op: u32, rt: u32, ra: u32, i10: u32) -> u32 {
    op << 24 | (i10 & 0x3FF) << 14 | ra << 7 | rt
}

const MPY: u32 = 0x3C4;
const MPYU: u32 = 0x3CC;
const MPYH: u32 = 0x3C5;
const MPYS: u32 = 0x3C7;
const MPYHH: u32 = 0x3C6;
const MPYHHA: u32 = 0x346;
const MPYHHU: u32 = 0x3CE;
const MPYHHAU: u32 = 0x34E;
const MPYI: u32 = 0x74;
const MPYUI: u32 = 0x75;
const A: u32 = 0x0C0;

fn set(s: &mut SpuState, r: u8, w: [u32; 4]) {
    for (slot, v) in w.into_iter().enumerate() {
        s.set_reg_word_slot(r, slot, v);
    }
}

fn get(s: &SpuState, r: u8) -> [u32; 4] {
    [0, 1, 2, 3].map(|slot| s.reg_word_slot(r, slot))
}

fn step(s: &mut SpuState, raw: u32) {
    let insn = crate::decode::decode(raw).expect("decodes");
    assert_eq!(execute(&insn, s, UnitId::new(0)), SpuStepOutcome::Continue);
}

/// Runs `raw` with `a` in r1, `b` in r2 and `t` in r3, and returns r3.
fn run(raw: u32, a: [u32; 4], b: [u32; 4], t: [u32; 4]) -> [u32; 4] {
    let mut s = SpuState::new();
    set(&mut s, 1, a);
    set(&mut s, 2, b);
    set(&mut s, 3, t);
    step(&mut s, raw);
    get(&s, 3)
}

// The high halves carry junk the low-half forms must ignore.
const A_LOW: [u32; 4] = [0xABCD_8000, 0x1234_FFFF, 0x5555_7FFF, 0x0101_0002];
const B_LOW: [u32; 4] = [0xFEDC_8000, 0x4321_FFFF, 0xAAAA_0002, 0x0202_FFFF];

#[test]
fn mpy_and_mpyu_read_the_low_halves_signed_and_unsigned() {
    assert_eq!(
        run(rr(MPY, 3, 1, 2), A_LOW, B_LOW, [0; 4]),
        [0x4000_0000, 1, 0x0000_FFFE, 0xFFFF_FFFE]
    );
    assert_eq!(
        run(rr(MPYU, 3, 1, 2), A_LOW, B_LOW, [0; 4]),
        [0x4000_0000, 0xFFFE_0001, 0x0000_FFFE, 0x0001_FFFE]
    );
}

#[test]
fn mpyi_sign_extends_the_immediate_and_mpyui_reads_it_unsigned() {
    // I10 0x3FF is -1 as a signed halfword and 0xFFFF as an unsigned one.
    assert_eq!(
        run(ri10(MPYI, 3, 1, 0x3FF), A_LOW, B_LOW, [0; 4]),
        [0x0000_8000, 1, 0xFFFF_8001, 0xFFFF_FFFE]
    );
    assert_eq!(
        run(ri10(MPYUI, 3, 1, 0x3FF), A_LOW, B_LOW, [0; 4]),
        [0x7FFF_8000, 0xFFFE_0001, 0x7FFE_8001, 0x0001_FFFE]
    );
}

#[test]
fn mpys_sign_extends_the_high_half_of_the_product() {
    assert_eq!(
        run(rr(MPYS, 3, 1, 2), A_LOW, B_LOW, [0; 4]),
        [0x0000_4000, 0, 0, 0xFFFF_FFFF]
    );
}

#[test]
fn mpyhh_and_mpyhhu_read_the_high_halves_signed_and_unsigned() {
    let a = [0x8000_1234, 0xFFFF_0000, 0x7FFF_FFFF, 0x0002_0000];
    let b = [0x8000_5678, 0xFFFF_0000, 0x0002_FFFF, 0xFFFF_0000];
    assert_eq!(
        run(rr(MPYHH, 3, 1, 2), a, b, [0; 4]),
        [0x4000_0000, 1, 0x0000_FFFE, 0xFFFF_FFFE]
    );
    assert_eq!(
        run(rr(MPYHHU, 3, 1, 2), a, b, [0; 4]),
        [0x4000_0000, 0xFFFE_0001, 0x0000_FFFE, 0x0001_FFFE]
    );
}

#[test]
fn the_high_high_add_forms_add_the_rt_input() {
    let a = [0xFFFF_0000; 4];
    let b = [0x0002_0000; 4];
    let t = [1, 0xFFFF_FFFF, 0x10, 0];
    assert_eq!(
        run(rr(MPYHHA, 3, 1, 2), a, b, t),
        [0xFFFF_FFFF, 0xFFFF_FFFD, 0xE, 0xFFFF_FFFE]
    );
    assert_eq!(
        run(rr(MPYHHAU, 3, 1, 2), a, b, t),
        [0x0001_FFFF, 0x0001_FFFD, 0x0002_000E, 0x0001_FFFE]
    );
}

#[test]
fn mpya_adds_rc_to_the_signed_low_product() {
    let mut s = SpuState::new();
    set(&mut s, 1, A_LOW);
    set(&mut s, 2, B_LOW);
    set(&mut s, 4, [1, 1, 1, 1]);
    // [SPU-ISA p:76 s:5] mpya: RRR opcode 0b1100, RT in bits 4:10, RC in bits 25:31.
    step(&mut s, 0xC << 28 | 3 << 21 | 2 << 14 | 1 << 7 | 4);
    assert_eq!(get(&s, 3), [0x4000_0001, 2, 0x0000_FFFF, 0xFFFF_FFFF]);
}

#[test]
fn mpyh_moves_the_low_product_half_into_the_high_half() {
    assert_eq!(
        run(
            rr(MPYH, 3, 1, 2),
            [0x0003_0000, 0xFFFF_0000, 0, 0],
            [0x0000_0005, 0x0000_0002, 0, 0],
            [0; 4]
        ),
        [0x000F_0000, 0xFFFE_0000, 0, 0]
    );
}

// [SPU-ISA p:77 s:5] a 32-bit multiply is mpyh t1,ra,rb; mpyh t2,rb,ra; mpyu t3,ra,rb; a rt,t1,t2; a rt,rt,t3.
#[test]
fn the_isa_sequence_builds_a_full_32_bit_product() {
    let a = [0x1234_5678, 0xFFFF_FFFF, 0x0001_0001, 0x8000_0000];
    let b = [0x9ABC_DEF0, 0x0000_0003, 0x0002_0003, 0x0000_0002];
    let mut s = SpuState::new();
    set(&mut s, 1, a);
    set(&mut s, 2, b);
    for raw in [
        rr(MPYH, 5, 1, 2),
        rr(MPYH, 6, 2, 1),
        rr(MPYU, 7, 1, 2),
        rr(A, 3, 5, 6),
        rr(A, 3, 3, 7),
    ] {
        step(&mut s, raw);
    }
    assert_eq!(get(&s, 3), std::array::from_fn(|i| a[i].wrapping_mul(b[i])));
}
