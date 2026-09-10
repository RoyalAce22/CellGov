//! Execute semantics of the forms a C compiler's startup code and
//! runtime emit.

use super::*;
use crate::state::SpuState;

fn uid() -> UnitId {
    UnitId::new(0)
}

const PATTERN: [u8; 16] = [
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F,
];

#[test]
fn lqr_loads_relative_to_pc() {
    let mut s = SpuState::new();
    s.pc = 0x100;
    s.ls[0x120..0x130].copy_from_slice(&PATTERN);
    let outcome = execute(&SpuInstruction::Lqr { rt: 3, imm: 8 }, &mut s, uid());
    assert!(matches!(outcome, SpuStepOutcome::Continue));
    assert_eq!(s.regs[3], PATTERN);
}

#[test]
fn lqr_negative_offset_reaches_below_pc() {
    let mut s = SpuState::new();
    s.pc = 0x200;
    s.ls[0x1F0..0x200].copy_from_slice(&PATTERN);
    execute(&SpuInstruction::Lqr { rt: 3, imm: -4 }, &mut s, uid());
    assert_eq!(s.regs[3], PATTERN);
}

#[test]
fn stqr_stores_relative_to_pc_with_low_bits_masked() {
    let mut s = SpuState::new();
    s.pc = 0x104;
    s.regs[6] = PATTERN;
    // 0x104 + 8*4 = 0x124; the quadword mask folds it to 0x120.
    let outcome = execute(&SpuInstruction::Stqr { rt: 6, imm: 8 }, &mut s, uid());
    assert!(matches!(outcome, SpuStepOutcome::Continue));
    assert_eq!(&s.ls[0x120..0x130], &PATTERN);
    assert!(s.ls[0x110..0x120].iter().all(|&b| b == 0));
    assert!(s.ls[0x130..0x140].iter().all(|&b| b == 0));
}

#[test]
fn stqr_past_ls_end_wraps_through_the_ls_mask() {
    let mut s = SpuState::new();
    s.pc = 0x3FFF0;
    s.regs[6] = PATTERN;
    // 0x3FFF0 + 4*4 = 0x40000, which the local-store mask folds to 0.
    let outcome = execute(&SpuInstruction::Stqr { rt: 6, imm: 4 }, &mut s, uid());
    assert!(matches!(outcome, SpuStepOutcome::Continue));
    assert_eq!(&s.ls[0..16], &PATTERN);
}

#[test]
fn and_or_operate_on_all_sixteen_bytes() {
    let mut s = SpuState::new();
    s.regs[1] = [0xF0; 16];
    s.regs[2] = [0x3C; 16];
    execute(
        &SpuInstruction::And {
            rt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        uid(),
    );
    assert_eq!(s.regs[3], [0x30; 16]);
    execute(
        &SpuInstruction::Or {
            rt: 4,
            ra: 1,
            rb: 2,
        },
        &mut s,
        uid(),
    );
    assert_eq!(s.regs[4], [0xFC; 16]);
}

#[test]
fn selb_takes_rb_where_rc_is_set() {
    let mut s = SpuState::new();
    s.regs[1] = [0xAA; 16];
    s.regs[2] = [0x55; 16];
    s.regs[3] = [0x0F; 16];
    execute(
        &SpuInstruction::Selb {
            rt: 4,
            ra: 1,
            rb: 2,
            rc: 3,
        },
        &mut s,
        uid(),
    );
    assert_eq!(s.regs[4], [0xA5; 16]);
}

#[test]
fn xsbh_sign_extends_the_right_byte_of_each_halfword() {
    let mut s = SpuState::new();
    s.regs[1] = [
        0x12, 0x80, 0x34, 0x7F, 0x56, 0xFF, 0x78, 0x00, 0x9A, 0x01, 0xBC, 0xFE, 0xDE, 0x40, 0xF0,
        0xC0,
    ];
    execute(&SpuInstruction::Xsbh { rt: 2, ra: 1 }, &mut s, uid());
    assert_eq!(
        s.regs[2],
        [
            0xFF, 0x80, 0x00, 0x7F, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x01, 0xFF, 0xFE, 0x00, 0x40,
            0xFF, 0xC0,
        ]
    );
}

#[test]
fn shl_uses_the_low_six_count_bits_per_slot() {
    let mut s = SpuState::new();
    s.set_reg_word_splat(1, 0x8000_0001);
    s.set_reg_word_slot(2, 0, 1);
    s.set_reg_word_slot(2, 1, 32);
    s.set_reg_word_slot(2, 2, 0x41);
    s.set_reg_word_slot(2, 3, 0);
    execute(
        &SpuInstruction::Shl {
            rt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        uid(),
    );
    assert_eq!(s.reg_word_slot(3, 0), 0x0000_0002);
    assert_eq!(s.reg_word_slot(3, 1), 0);
    assert_eq!(s.reg_word_slot(3, 2), 0x0000_0002);
    assert_eq!(s.reg_word_slot(3, 3), 0x8000_0001);
}

#[test]
fn shli_shifts_left_and_zeroes_past_thirty_one() {
    let mut s = SpuState::new();
    s.set_reg_word_splat(1, 0x4000_0001);
    execute(
        &SpuInstruction::Shli {
            rt: 2,
            ra: 1,
            imm: 2,
        },
        &mut s,
        uid(),
    );
    assert_eq!(s.reg_word(2), 0x0000_0004);
    execute(
        &SpuInstruction::Shli {
            rt: 3,
            ra: 1,
            imm: 0x20,
        },
        &mut s,
        uid(),
    );
    assert_eq!(s.reg_word(3), 0);
}

#[test]
fn rotmi_is_a_logical_right_shift_by_the_negated_immediate() {
    let mut s = SpuState::new();
    s.set_reg_word_splat(1, 0x8000_0000);
    // I7 = 0x61 is -31 in seven bits: shift right by 31.
    execute(
        &SpuInstruction::Rotmi {
            rt: 2,
            ra: 1,
            imm: 0x61,
        },
        &mut s,
        uid(),
    );
    assert_eq!(s.reg_word(2), 1);
    // I7 = 0x7F is -1: shift right by one, zero fill.
    execute(
        &SpuInstruction::Rotmi {
            rt: 3,
            ra: 1,
            imm: 0x7F,
        },
        &mut s,
        uid(),
    );
    assert_eq!(s.reg_word(3), 0x4000_0000);
    // I7 = 0x20 gives count 32: the whole word clears.
    execute(
        &SpuInstruction::Rotmi {
            rt: 4,
            ra: 1,
            imm: 0x20,
        },
        &mut s,
        uid(),
    );
    assert_eq!(s.reg_word(4), 0);
    // I7 = 0 gives count 0: unchanged.
    execute(
        &SpuInstruction::Rotmi {
            rt: 5,
            ra: 1,
            imm: 0,
        },
        &mut s,
        uid(),
    );
    assert_eq!(s.reg_word(5), 0x8000_0000);
}

#[test]
fn rotmai_replicates_the_sign_bit() {
    let mut s = SpuState::new();
    s.set_reg_word_slot(1, 0, 0x8000_0000);
    s.set_reg_word_slot(1, 1, 0x7FFF_FFFF);
    s.set_reg_word_slot(1, 2, 0x8000_0000);
    s.set_reg_word_slot(1, 3, 0x7FFF_FFFF);
    // I7 = 0x7E is -2: arithmetic shift right by two.
    execute(
        &SpuInstruction::Rotmai {
            rt: 2,
            ra: 1,
            imm: 0x7E,
        },
        &mut s,
        uid(),
    );
    assert_eq!(s.reg_word_slot(2, 0), 0xE000_0000);
    assert_eq!(s.reg_word_slot(2, 1), 0x1FFF_FFFF);
    // Count 32 fills every bit with the sign.
    execute(
        &SpuInstruction::Rotmai {
            rt: 3,
            ra: 1,
            imm: 0x20,
        },
        &mut s,
        uid(),
    );
    assert_eq!(s.reg_word_slot(3, 2), 0xFFFF_FFFF);
    assert_eq!(s.reg_word_slot(3, 3), 0);
}

#[test]
fn rotqbyi_rotates_left_by_the_low_nibble() {
    let mut s = SpuState::new();
    s.regs[1] = PATTERN;
    execute(
        &SpuInstruction::Rotqbyi {
            rt: 2,
            ra: 1,
            imm: 12,
        },
        &mut s,
        uid(),
    );
    assert_eq!(
        s.regs[2],
        [
            0x1C, 0x1D, 0x1E, 0x1F, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19,
            0x1A, 0x1B,
        ]
    );
    execute(
        &SpuInstruction::Rotqbyi {
            rt: 3,
            ra: 1,
            imm: 0x10,
        },
        &mut s,
        uid(),
    );
    assert_eq!(s.regs[3], PATTERN);
}

#[test]
fn cgti_compares_signed() {
    let mut s = SpuState::new();
    s.set_reg_word_slot(1, 0, 0xFFFF_FFFF);
    s.set_reg_word_slot(1, 1, 1);
    s.set_reg_word_slot(1, 2, 0);
    s.set_reg_word_slot(1, 3, 0x7FFF_FFFF);
    execute(
        &SpuInstruction::Cgti {
            rt: 2,
            ra: 1,
            imm: 0,
        },
        &mut s,
        uid(),
    );
    assert_eq!(s.reg_word_slot(2, 0), 0, "-1 > 0 is false");
    assert_eq!(s.reg_word_slot(2, 1), 0xFFFF_FFFF);
    assert_eq!(s.reg_word_slot(2, 2), 0);
    assert_eq!(s.reg_word_slot(2, 3), 0xFFFF_FFFF);
    execute(
        &SpuInstruction::Cgti {
            rt: 3,
            ra: 1,
            imm: -1,
        },
        &mut s,
        uid(),
    );
    assert_eq!(s.reg_word_slot(3, 0), 0, "-1 > -1 is false");
    assert_eq!(s.reg_word_slot(3, 2), 0xFFFF_FFFF, "0 > -1 is true");
}

#[test]
fn clgt_compares_unsigned() {
    let mut s = SpuState::new();
    s.set_reg_word_slot(1, 0, 0xFFFF_FFFF);
    s.set_reg_word_slot(1, 1, 1);
    s.set_reg_word_slot(1, 2, 5);
    s.set_reg_word_slot(1, 3, 0);
    s.set_reg_word_slot(2, 0, 1);
    s.set_reg_word_slot(2, 1, 0xFFFF_FFFF);
    s.set_reg_word_slot(2, 2, 5);
    s.set_reg_word_slot(2, 3, 0);
    execute(
        &SpuInstruction::Clgt {
            rt: 3,
            ra: 1,
            rb: 2,
        },
        &mut s,
        uid(),
    );
    assert_eq!(s.reg_word_slot(3, 0), 0xFFFF_FFFF);
    assert_eq!(s.reg_word_slot(3, 1), 0);
    assert_eq!(s.reg_word_slot(3, 2), 0);
    assert_eq!(s.reg_word_slot(3, 3), 0);
}

#[test]
fn bisl_links_in_the_preferred_slot_only_and_branches() {
    let mut s = SpuState::new();
    s.pc = 0x100;
    s.set_reg_word_splat(4, 0x3A3);
    s.regs[0] = [0xEE; 16];
    let outcome = execute(&SpuInstruction::Bisl { rt: 0, ra: 4 }, &mut s, uid());
    assert!(matches!(outcome, SpuStepOutcome::Branch));
    assert_eq!(s.pc, 0x3A0);
    assert_eq!(s.reg_word_slot(0, 0), 0x104);
    assert_eq!(s.reg_word_slot(0, 1), 0);
    assert_eq!(s.reg_word_slot(0, 2), 0);
    assert_eq!(s.reg_word_slot(0, 3), 0);
}

#[test]
fn bisl_link_at_the_top_of_ls_wraps_through_the_ls_mask() {
    let mut s = SpuState::new();
    // The last instruction word in a 256 KB local store: PC + 4 is
    // 0x40000, which LSLR folds to 0.
    s.pc = 0x3FFFC;
    s.set_reg_word_splat(4, 0x200);
    let outcome = execute(&SpuInstruction::Bisl { rt: 0, ra: 4 }, &mut s, uid());
    assert!(matches!(outcome, SpuStepOutcome::Branch));
    assert_eq!(s.pc, 0x200);
    assert_eq!(s.reg_word_slot(0, 0), 0);
}

#[test]
fn bisl_target_above_ls_wraps_through_the_ls_mask() {
    let mut s = SpuState::new();
    s.pc = 0x100;
    // Bit 18 of the target falls to LSLR and the low two bits to the
    // instruction-alignment mask.
    s.set_reg_word_splat(4, 0x4_03A3);
    let outcome = execute(&SpuInstruction::Bisl { rt: 0, ra: 4 }, &mut s, uid());
    assert!(matches!(outcome, SpuStepOutcome::Branch));
    assert_eq!(s.pc, 0x3A0);
    assert_eq!(s.reg_word_slot(0, 0), 0x104);
}

#[test]
fn bisl_reads_the_target_before_writing_the_link_when_rt_is_ra() {
    let mut s = SpuState::new();
    s.pc = 0x100;
    s.set_reg_word_splat(4, 0x3A0);
    let outcome = execute(&SpuInstruction::Bisl { rt: 4, ra: 4 }, &mut s, uid());
    assert!(matches!(outcome, SpuStepOutcome::Branch));
    assert_eq!(s.pc, 0x3A0);
    assert_eq!(s.reg_word_slot(4, 0), 0x104);
    assert_eq!(s.reg_word_slot(4, 1), 0);
}

#[test]
fn brsl_links_in_the_preferred_slot_only_and_masks_at_the_top_of_ls() {
    let mut s = SpuState::new();
    s.pc = 0x100;
    s.regs[0] = [0xEE; 16];
    execute(&SpuInstruction::Brsl { rt: 0, offset: 16 }, &mut s, uid());
    assert_eq!(s.reg_word_slot(0, 0), 0x104);
    assert_eq!(s.reg_word_slot(0, 1), 0);
    assert_eq!(s.reg_word_slot(0, 2), 0);
    assert_eq!(s.reg_word_slot(0, 3), 0);

    s.pc = 0x3FFFC;
    execute(&SpuInstruction::Brsl { rt: 0, offset: -64 }, &mut s, uid());
    assert_eq!(s.reg_word_slot(0, 0), 0);
    assert_eq!(s.pc, 0x3FEFC);
}

#[test]
fn brhnz_tests_only_the_low_halfword() {
    let mut s = SpuState::new();
    s.pc = 0x100;
    s.set_reg_word_splat(4, 0x0001_0000);
    let outcome = execute(&SpuInstruction::Brhnz { rt: 4, offset: 20 }, &mut s, uid());
    assert!(matches!(outcome, SpuStepOutcome::Continue));
    assert_eq!(s.pc, 0x100);

    s.set_reg_word_splat(4, 0x0000_0001);
    let outcome = execute(&SpuInstruction::Brhnz { rt: 4, offset: 20 }, &mut s, uid());
    assert!(matches!(outcome, SpuStepOutcome::Branch));
    assert_eq!(s.pc, 0x150);
}
