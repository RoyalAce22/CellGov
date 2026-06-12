//! Add / subtract / negate families: carry in/out, OE overflow, and Rc CR0 recording.

use super::*;

#[test]
fn addi_with_ra_zero_is_li() {
    let mut s = PpuState::new();
    exec_no_mem(
        &PpuInstruction::Addi {
            rt: 3,
            ra: 0,
            imm: 42,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 42);
}

#[test]
fn addi_with_ra_nonzero_adds() {
    let mut s = PpuState::new();
    s.gpr[5] = 100;
    exec_no_mem(
        &PpuInstruction::Addi {
            rt: 3,
            ra: 5,
            imm: -10,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 90);
}

#[test]
fn addis_shifts_left_16() {
    let mut s = PpuState::new();
    exec_no_mem(
        &PpuInstruction::Addis {
            rt: 3,
            ra: 0,
            imm: 1,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 0x10000);
}

#[test]
fn adde_adds_with_carry_in_and_sets_carry_out() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
    s.gpr[4] = 0;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Adde {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0);
    assert!(s.xer_ca());
}

#[test]
fn adde_without_carry_clears_ca() {
    let mut s = PpuState::new();
    s.gpr[3] = 5;
    s.gpr[4] = 3;
    s.set_xer_ca(false);
    exec_no_mem(
        &PpuInstruction::Adde {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 8);
    assert!(!s.xer_ca());
}

#[test]
fn addze_with_ca_zero_copies_ra() {
    let mut s = PpuState::new();
    s.gpr[4] = 42;
    s.set_xer_ca(false);
    exec_no_mem(
        &PpuInstruction::Addze {
            rt: 3,
            ra: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 42);
    assert!(!s.xer_ca());
}

#[test]
fn addze_with_ca_set_adds_one() {
    let mut s = PpuState::new();
    s.gpr[4] = 42;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Addze {
            rt: 3,
            ra: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 43);
    assert!(!s.xer_ca());
}

#[test]
fn addze_overflow_sets_ca() {
    let mut s = PpuState::new();
    s.gpr[4] = u64::MAX;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Addze {
            rt: 3,
            ra: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 0);
    assert!(s.xer_ca());
}

#[test]
fn addze_oe_signed_overflow_sets_ov_and_so() {
    // Max positive i64 + CA=1 wraps to min i64: signed overflow.
    let mut s = PpuState::new();
    s.gpr[4] = 0x7FFF_FFFF_FFFF_FFFF;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Addze {
            rt: 3,
            ra: 4,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 0x8000_0000_0000_0000);
    assert_eq!(s.xer & (1u64 << 30), 1u64 << 30, "OV set");
    assert_eq!(s.xer & (1u64 << 31), 1u64 << 31, "SO set");
}

#[test]
fn addze_oe_no_overflow_clears_ov_keeps_so_sticky() {
    // u64::MAX + 1 carries (CA=1) and wraps to 0, but signed:
    // -1 + 1 = 0, no signed overflow. OV must be cleared while
    // any pre-existing sticky SO is preserved.
    let mut s = PpuState::new();
    s.gpr[4] = u64::MAX;
    s.set_xer_ca(true);
    // Pre-set sticky SO via set_xer_ov round-trip so the entry
    // state has SO=1, OV=0; the round-trip itself is covered in
    // state.rs tests.
    s.set_xer_ov(true);
    s.set_xer_ov(false);
    exec_no_mem(
        &PpuInstruction::Addze {
            rt: 3,
            ra: 4,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 0);
    assert_eq!(s.xer & (1u64 << 30), 0, "OV cleared");
    assert_eq!(s.xer & (1u64 << 31), 1u64 << 31, "SO sticky");
}

#[test]
fn subfc_computes_rb_minus_ra_and_sets_ca_on_no_borrow() {
    let mut s = PpuState::new();
    s.gpr[3] = 3;
    s.gpr[4] = 10;
    exec_no_mem(
        &PpuInstruction::Subfc {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 7);
    assert!(s.xer_ca());
}

#[test]
fn subfc_borrow_clears_ca() {
    let mut s = PpuState::new();
    s.gpr[3] = 10;
    s.gpr[4] = 3;
    exec_no_mem(
        &PpuInstruction::Subfc {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 3u64.wrapping_sub(10));
    assert!(!s.xer_ca());
}

#[test]
fn subfe_uses_carry_in() {
    // rt = ~ra + rb + CA: CA=1 gives rb - ra, CA=0 gives rb - ra - 1.
    let mut s = PpuState::new();
    s.gpr[3] = 3;
    s.gpr[4] = 10;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Subfe {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 7);

    s.set_xer_ca(false);
    exec_no_mem(
        &PpuInstruction::Subfe {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 6);
}

// -- Rc / OE regression tests --
// Record form (Rc=1) must set CR0 LT/GT/EQ from the signed 64-bit
// result, plus the sticky SO from XER. OE=1 must set XER OV and the
// sticky SO on overflow.

#[test]
fn add_dot_sets_cr0_eq_when_result_is_zero() {
    let mut s = PpuState::new();
    s.gpr[3] = 1;
    s.gpr[4] = (-1i64) as u64;
    exec_no_mem(
        &PpuInstruction::Add {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0);
    assert_eq!(s.cr_field(0), 0b0010);
}

#[test]
fn add_dot_sets_cr0_lt_when_result_is_negative() {
    let mut s = PpuState::new();
    s.gpr[3] = 1;
    s.gpr[4] = (-2i64) as u64;
    exec_no_mem(
        &PpuInstruction::Add {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b1000);
}

#[test]
fn add_rc_zero_leaves_cr0_untouched() {
    let mut s = PpuState::new();
    s.set_cr_field(0, 0b0100);
    s.gpr[3] = 1;
    s.gpr[4] = 2;
    exec_no_mem(
        &PpuInstruction::Add {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b0100, "CR0 preserved when Rc=0");
}

#[test]
fn addo_sets_xer_ov_and_sticky_so() {
    let mut s = PpuState::new();
    s.gpr[3] = i64::MAX as u64;
    s.gpr[4] = 1;
    exec_no_mem(
        &PpuInstruction::Add {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.xer & (1u64 << 30), 1u64 << 30, "OV set");
    assert_eq!(s.xer & (1u64 << 31), 1u64 << 31, "SO set");

    // Non-overflow op clears OV but SO stays sticky.
    s.gpr[3] = 1;
    s.gpr[4] = 2;
    exec_no_mem(
        &PpuInstruction::Add {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.xer & (1u64 << 30), 0, "OV cleared");
    assert_eq!(s.xer & (1u64 << 31), 1u64 << 31, "SO remains sticky");
}

#[test]
fn nego_of_int_min_sets_ov() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x8000_0000_0000_0000;
    exec_no_mem(
        &PpuInstruction::Neg {
            rt: 5,
            ra: 3,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.xer & (1u64 << 30), 1u64 << 30, "OV set");
}

#[test]
fn addo_dot_combined_sets_both_ov_and_cr0() {
    // oe=rc=true: executor must act on both bits independently.
    let mut s = PpuState::new();
    s.gpr[3] = i64::MAX as u64;
    s.gpr[4] = 1;
    exec_no_mem(
        &PpuInstruction::Add {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: true,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.xer & (1u64 << 30), 1u64 << 30, "OV set");
    assert_eq!(s.xer & (1u64 << 31), 1u64 << 31, "SO set");
    // Result is INT_MIN, negative -- CR0 = LT plus sticky SO.
    assert_eq!(s.cr_field(0), 0b1001);
}

#[test]
fn subfic_sets_xer_ca_on_no_borrow() {
    let mut s = PpuState::new();
    s.gpr[3] = 5;
    s.set_xer_ca(false);
    exec_no_mem(
        &PpuInstruction::Subfic {
            rt: 4,
            ra: 3,
            imm: 10,
        },
        &mut s,
    );
    assert_eq!(s.gpr[4], 5);
    assert!(s.xer_ca(), "subfic sets CA when there is no borrow");
}

#[test]
fn subfic_clears_xer_ca_on_borrow() {
    let mut s = PpuState::new();
    s.gpr[3] = 10;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Subfic {
            rt: 4,
            ra: 3,
            imm: 5,
        },
        &mut s,
    );
    assert_eq!(s.gpr[4], 5u64.wrapping_sub(10));
    assert!(!s.xer_ca(), "subfic clears stale CA when borrow occurs");
}

#[test]
fn addic_sets_xer_ca_on_carry() {
    let mut s = PpuState::new();
    s.gpr[3] = u64::MAX;
    s.set_xer_ca(false);
    exec_no_mem(
        &PpuInstruction::Addic {
            rt: 4,
            ra: 3,
            imm: 1,
        },
        &mut s,
    );
    assert_eq!(s.gpr[4], 0);
    assert!(s.xer_ca(), "addic sets CA when carry out");
}

#[test]
fn addic_clears_xer_ca_on_no_carry() {
    let mut s = PpuState::new();
    s.gpr[3] = 1;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Addic {
            rt: 4,
            ra: 3,
            imm: 1,
        },
        &mut s,
    );
    assert_eq!(s.gpr[4], 2);
    assert!(!s.xer_ca(), "addic clears stale CA when no carry");
}

#[test]
fn addic_negative_immediate_sign_extends_and_clears_ca() {
    // RA=0, imm=-1: the sign-extended -1 is 0xFFFF_FFFF_FFFF_FFFF.
    // 0 + (-1 sign-ext) wraps to 0xFFFF... with no carry out (the
    // unsigned add of 0 + 0xFFFF... is below 2^64).
    let mut s = PpuState::new();
    s.gpr[3] = 0;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Addic {
            rt: 4,
            ra: 3,
            imm: -1,
        },
        &mut s,
    );
    assert_eq!(s.gpr[4], 0xFFFF_FFFF_FFFF_FFFF);
    assert!(!s.xer_ca(), "0 + (-1 sign-ext) does not generate carry");
}

#[test]
fn addic_dot_records_to_cr0_and_sets_ca() {
    let mut s = PpuState::new();
    s.gpr[3] = u64::MAX;
    s.set_xer_ca(false);
    exec_no_mem(
        &PpuInstruction::AddicDot {
            rt: 4,
            ra: 3,
            imm: 1,
        },
        &mut s,
    );
    assert_eq!(s.gpr[4], 0);
    assert!(s.xer_ca(), "addic. sets CA on carry out");
    assert_eq!(s.cr_field(0), 0b0010, "addic. records EQ for zero result");
}

// -- Level-2 side-effect gap fillers --
// Per-instruction side-effect coverage for Rc=1 (CR0), OE=1
// (XER[OV]/[SO]), CA-bearing semantics, and compare SO propagation.
// Each test exercises ONE side effect; result correctness is covered
// by Level-1 tests elsewhere.

// -- Subf OE/Rc --

#[test]
fn subfo_signed_overflow_sets_ov_and_so() {
    // RB=i64::MAX, RA=-1: MAX - (-1) overflows.
    let mut s = PpuState::new();
    s.gpr[3] = (-1i64) as u64;
    s.gpr[4] = i64::MAX as u64;
    exec_no_mem(
        &PpuInstruction::Subf {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.xer & (1u64 << 30), 1u64 << 30, "OV set");
    assert_eq!(s.xer & (1u64 << 31), 1u64 << 31, "SO set");
}

#[test]
fn subf_dot_sets_cr0_lt_on_negative_result() {
    let mut s = PpuState::new();
    s.gpr[3] = 5; // RA
    s.gpr[4] = 2; // RB; result = 2 - 5 = -3
    exec_no_mem(
        &PpuInstruction::Subf {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b1000);
}

// -- Subfc OE/Rc --

#[test]
fn subfco_signed_overflow_sets_ov() {
    // RB=i64::MAX, RA=-1: MAX - (-1) overflows.
    let mut s = PpuState::new();
    s.gpr[3] = (-1i64) as u64;
    s.gpr[4] = i64::MAX as u64;
    exec_no_mem(
        &PpuInstruction::Subfc {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.xer & (1u64 << 30), 1u64 << 30, "OV set");
}

#[test]
fn subfc_dot_sets_cr0_eq_on_zero_result() {
    let mut s = PpuState::new();
    s.gpr[3] = 7;
    s.gpr[4] = 7;
    exec_no_mem(
        &PpuInstruction::Subfc {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b0010);
}

// -- Subfe CA / OE / Rc --

#[test]
fn subfe_sets_ca_on_no_borrow() {
    let mut s = PpuState::new();
    s.gpr[3] = 3; // RA
    s.gpr[4] = 10; // RB
    s.set_xer_ca(true); // CA=1: rb - ra
    exec_no_mem(
        &PpuInstruction::Subfe {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    // 10 - 3 = 7, no borrow -> CA=1.
    assert!(s.xer_ca());
}

#[test]
fn subfe_clears_ca_on_borrow() {
    let mut s = PpuState::new();
    s.gpr[3] = 10; // RA
    s.gpr[4] = 3; // RB; rb - ra borrows
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Subfe {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert!(!s.xer_ca(), "borrow clears CA");
}

#[test]
fn subfeo_signed_overflow_sets_ov() {
    // RB=i64::MAX, RA=-1, CA=1: MAX - (-1) overflows.
    let mut s = PpuState::new();
    s.gpr[3] = (-1i64) as u64;
    s.gpr[4] = i64::MAX as u64;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Subfe {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.xer & (1u64 << 30), 1u64 << 30, "OV set");
}

#[test]
fn subfe_dot_sets_cr0_lt_on_negative_result() {
    let mut s = PpuState::new();
    s.gpr[3] = 10; // RA
    s.gpr[4] = 3; // RB
    s.set_xer_ca(true); // rb - ra = -7
    exec_no_mem(
        &PpuInstruction::Subfe {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b1000);
}

// -- Neg Rc --

#[test]
fn neg_dot_sets_cr0_lt_on_negative_result() {
    let mut s = PpuState::new();
    s.gpr[3] = 5; // -5 result
    exec_no_mem(
        &PpuInstruction::Neg {
            rt: 5,
            ra: 3,
            oe: false,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b1000);
}

// -- Adde OE/Rc --

#[test]
fn addeo_signed_overflow_sets_ov() {
    let mut s = PpuState::new();
    s.gpr[3] = i64::MAX as u64;
    s.gpr[4] = 1;
    s.set_xer_ca(false);
    exec_no_mem(
        &PpuInstruction::Adde {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.xer & (1u64 << 30), 1u64 << 30, "OV set");
}

#[test]
fn adde_dot_sets_cr0_eq_on_zero() {
    let mut s = PpuState::new();
    s.gpr[3] = u64::MAX; // -1
    s.gpr[4] = 0;
    s.set_xer_ca(true); // -1 + 0 + 1 = 0
    exec_no_mem(
        &PpuInstruction::Adde {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b0010);
}

// -- Addze Rc --

#[test]
fn addze_dot_sets_cr0_gt_on_positive() {
    let mut s = PpuState::new();
    s.gpr[3] = 41;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Addze {
            rt: 5,
            ra: 3,
            oe: false,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b0100);
}

// -- Addme CA/OE/Rc --

#[test]
fn addme_sets_ca_on_carry_out() {
    // RA = 1, CA_in = 1: 1 + (-1) + 1 = 1, with carry out of u64 add.
    let mut s = PpuState::new();
    s.gpr[3] = 1;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Addme {
            rt: 5,
            ra: 3,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert!(s.xer_ca(), "carry out from 1 + (-1) + 1");
}

#[test]
fn addme_clears_ca_when_no_carry() {
    // RA = 0, CA_in = 0: 0 + (-1) + 0 = -1, no carry.
    let mut s = PpuState::new();
    s.gpr[3] = 0;
    s.set_xer_ca(false);
    exec_no_mem(
        &PpuInstruction::Addme {
            rt: 5,
            ra: 3,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert!(!s.xer_ca());
}

#[test]
fn addmeo_signed_overflow_sets_ov() {
    // i64::MIN + (-1) overflows in signed: MIN - 1 -> MAX (wraparound).
    let mut s = PpuState::new();
    s.gpr[3] = i64::MIN as u64;
    s.set_xer_ca(false);
    exec_no_mem(
        &PpuInstruction::Addme {
            rt: 5,
            ra: 3,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.xer & (1u64 << 30), 1u64 << 30, "OV set");
}

#[test]
fn addme_dot_sets_cr0_lt_on_negative() {
    let mut s = PpuState::new();
    s.gpr[3] = 0;
    s.set_xer_ca(false); // result = -1
    exec_no_mem(
        &PpuInstruction::Addme {
            rt: 5,
            ra: 3,
            oe: false,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b1000);
}

// -- Subfze CA/OE/Rc --

#[test]
fn subfze_sets_ca_on_carry_out() {
    // ~RA + CA: RA = 0 -> ~0 = u64::MAX; + CA(1) = 0 with carry.
    let mut s = PpuState::new();
    s.gpr[3] = 0;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Subfze {
            rt: 5,
            ra: 3,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert!(s.xer_ca());
}

#[test]
fn subfze_clears_ca_when_no_carry() {
    // RA = 0 -> ~0 = u64::MAX; + CA(0) = u64::MAX, no carry.
    let mut s = PpuState::new();
    s.gpr[3] = 0;
    s.set_xer_ca(false);
    exec_no_mem(
        &PpuInstruction::Subfze {
            rt: 5,
            ra: 3,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert!(!s.xer_ca());
}

#[test]
fn subfzeo_signed_overflow_sets_ov() {
    // ~RA + CA: RA = i64::MAX -> ~RA = i64::MIN; + 1 = MIN + 1 -> no
    // overflow. Use RA such that ~RA + 1 overflows: RA=0x8000... -> ~RA
    // = 0x7FFF... = i64::MAX; + 1 (CA=1) = i64::MIN -> signed overflow
    // because MAX + 1 wraps.
    let mut s = PpuState::new();
    s.gpr[3] = 0x8000_0000_0000_0000;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Subfze {
            rt: 5,
            ra: 3,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.xer & (1u64 << 30), 1u64 << 30, "OV set");
}

#[test]
fn subfze_dot_sets_cr0_lt_on_negative_result() {
    // RA = 0 -> ~RA = u64::MAX = -1; + CA(0) = -1.
    let mut s = PpuState::new();
    s.gpr[3] = 0;
    s.set_xer_ca(false);
    exec_no_mem(
        &PpuInstruction::Subfze {
            rt: 5,
            ra: 3,
            oe: false,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b1000);
}

// -- Subfme CA/OE/Rc --

#[test]
fn subfme_sets_ca_on_carry_out() {
    // ~RA + CA + (-1): RA = 0 -> ~RA = u64::MAX; + CA(1) = 0 (carry);
    // + (-1) = u64::MAX. Result u64::MAX, intermediate carry set CA.
    let mut s = PpuState::new();
    s.gpr[3] = 0;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Subfme {
            rt: 5,
            ra: 3,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert!(s.xer_ca(), "carry from u64::MAX + 1");
}

#[test]
fn subfme_clears_ca_when_no_carry() {
    // RA = u64::MAX -> ~RA = 0; + CA(0) + (-1) = -1, no carry.
    let mut s = PpuState::new();
    s.gpr[3] = u64::MAX;
    s.set_xer_ca(false);
    exec_no_mem(
        &PpuInstruction::Subfme {
            rt: 5,
            ra: 3,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert!(!s.xer_ca());
}

#[test]
fn subfmeo_signed_overflow_sets_ov() {
    // RA = 1 -> ~RA = u64::MAX - 1 = i64 -2; + CA(0) + (-1) = -3, no OV.
    // Use RA where ~RA = i64::MIN: RA=0x7FFF_FFFF_FFFF_FFFF -> ~RA =
    // 0x8000_0000_0000_0000 = i64::MIN; + CA(0) + (-1) = MIN - 1 ->
    // overflow.
    let mut s = PpuState::new();
    s.gpr[3] = 0x7FFF_FFFF_FFFF_FFFF;
    s.set_xer_ca(false);
    exec_no_mem(
        &PpuInstruction::Subfme {
            rt: 5,
            ra: 3,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.xer & (1u64 << 30), 1u64 << 30, "OV set");
}

#[test]
fn subfme_dot_sets_cr0_lt_on_negative_result() {
    // RA = 0 -> ~RA = u64::MAX; + CA(0) + (-1) = u64::MAX - 1 = -2.
    let mut s = PpuState::new();
    s.gpr[3] = 0;
    s.set_xer_ca(false);
    exec_no_mem(
        &PpuInstruction::Subfme {
            rt: 5,
            ra: 3,
            oe: false,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b1000);
}
