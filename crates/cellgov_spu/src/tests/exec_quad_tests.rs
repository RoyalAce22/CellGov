//! The eight quadword load / store forms through the shared helpers.

use crate::exec::{execute, SpuFault, SpuStepOutcome};
use crate::instruction::SpuInstruction;
use crate::state::SpuState;
use cellgov_event::UnitId;

const PATTERN: [u8; 16] = [
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF,
];

fn uid() -> UnitId {
    UnitId::new(1)
}

// [SPU-ISA p:32 s:3. Memory-Load/Store Instructions] lqd: I10 gains four zero bits before the add, and the sum drops its low four bits.
#[test]
fn lqd_scales_a_negative_immediate_by_sixteen_and_masks_the_low_bits() {
    let mut s = SpuState::new();
    s.set_reg_word_splat(1, 0x1008);
    s.ls[0x0FE0..0x0FF0].copy_from_slice(&PATTERN);
    // 0x1008 + (-2 << 4) = 0x0FE8; the quadword mask folds it to 0x0FE0.
    let outcome = execute(
        &SpuInstruction::Lqd {
            rt: 3,
            ra: 1,
            imm: -2,
        },
        &mut s,
        uid(),
    );
    assert!(matches!(outcome, SpuStepOutcome::Continue));
    assert_eq!(s.regs[3], PATTERN);
}

// [SPU-ISA p:36 s:3. Memory-Load/Store Instructions] stqd: the same scaled and masked address as lqd, written from RT.
#[test]
fn stqd_writes_the_register_at_the_scaled_offset() {
    let mut s = SpuState::new();
    s.set_reg_word_splat(1, 0x2000);
    s.regs[4] = PATTERN;
    let outcome = execute(
        &SpuInstruction::Stqd {
            rt: 4,
            ra: 1,
            imm: 3,
        },
        &mut s,
        uid(),
    );
    assert!(matches!(outcome, SpuStepOutcome::Continue));
    assert_eq!(&s.ls[0x2030..0x2040], &PATTERN);
    assert!(s.ls[0x2020..0x2030].iter().all(|&b| b == 0));
    assert!(s.ls[0x2040..0x2050].iter().all(|&b| b == 0));
}

// [SPU-ISA p:33 s:3. Memory-Load/Store Instructions] lqx: the preferred slots of RA and RB add, and the sum drops its low four bits.
#[test]
fn lqx_and_stqx_sum_the_preferred_slots_of_ra_and_rb() {
    let mut s = SpuState::new();
    s.set_reg_word_splat(1, 0x3004);
    s.set_reg_word_splat(2, 0x0018);
    s.regs[5] = PATTERN;
    // 0x3004 + 0x0018 = 0x301C; the quadword mask folds it to 0x3010.
    execute(
        &SpuInstruction::Stqx {
            rt: 5,
            ra: 1,
            rb: 2,
        },
        &mut s,
        uid(),
    );
    assert_eq!(&s.ls[0x3010..0x3020], &PATTERN);
    let outcome = execute(
        &SpuInstruction::Lqx {
            rt: 6,
            ra: 1,
            rb: 2,
        },
        &mut s,
        uid(),
    );
    assert!(matches!(outcome, SpuStepOutcome::Continue));
    assert_eq!(s.regs[6], PATTERN);
}

// [SPU-ISA p:34 s:3. Memory-Load/Store Instructions] lqa: I16 with two zero bits appended is the whole address.
#[test]
fn lqa_and_stqa_ignore_every_register_for_the_address() {
    let mut s = SpuState::new();
    s.set_reg_word_splat(1, 0xDEAD_0000);
    s.regs[7] = PATTERN;
    let imm: i16 = (0x0400u16 >> 2) as i16;
    execute(&SpuInstruction::Stqa { rt: 7, imm }, &mut s, uid());
    assert_eq!(&s.ls[0x0400..0x0410], &PATTERN);
    let outcome = execute(&SpuInstruction::Lqa { rt: 8, imm }, &mut s, uid());
    assert!(matches!(outcome, SpuStepOutcome::Continue));
    assert_eq!(s.regs[8], PATTERN);
}

// [SPU-ISA p:35 s:3. Memory-Load/Store Instructions] lqr: I16 with two zero bits appended adds to the PC.
#[test]
fn lqr_and_stqr_use_the_pc_not_ra() {
    let mut s = SpuState::new();
    s.pc = 0x0500;
    s.set_reg_word_splat(1, 0xDEAD_0000);
    s.regs[9] = PATTERN;
    execute(&SpuInstruction::Stqr { rt: 9, imm: 4 }, &mut s, uid());
    assert_eq!(&s.ls[0x0510..0x0520], &PATTERN);
    let outcome = execute(&SpuInstruction::Lqr { rt: 10, imm: 4 }, &mut s, uid());
    assert!(matches!(outcome, SpuStepOutcome::Continue));
    assert_eq!(s.regs[10], PATTERN);
}

#[test]
fn load_past_a_short_local_store_faults_and_leaves_rt_unchanged() {
    let mut s = SpuState::new();
    s.ls.truncate(0x1000);
    s.regs[3] = PATTERN;
    s.set_reg_word_splat(1, 0x1000);
    let outcome = execute(
        &SpuInstruction::Lqd {
            rt: 3,
            ra: 1,
            imm: 0,
        },
        &mut s,
        uid(),
    );
    assert!(matches!(
        outcome,
        SpuStepOutcome::Fault(SpuFault::LsOutOfRange(0x1000))
    ));
    assert_eq!(s.regs[3], PATTERN);
}

#[test]
fn store_past_a_short_local_store_faults_and_leaves_ls_unchanged() {
    let mut s = SpuState::new();
    s.ls.truncate(0x1000);
    s.regs[3] = PATTERN;
    let before = s.ls.clone();
    // I16 = 0x400 -> LSA 0x1000, the first quadword past the end.
    let outcome = execute(&SpuInstruction::Stqa { rt: 3, imm: 0x400 }, &mut s, uid());
    assert!(matches!(
        outcome,
        SpuStepOutcome::Fault(SpuFault::LsOutOfRange(0x1000))
    ));
    assert_eq!(s.ls, before);
}
