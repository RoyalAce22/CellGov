//! Logical ops, count-leading-zeros, and sign extension with Rc CR0 recording.

use super::*;

#[test]
fn ori_zero_is_move() {
    let mut s = PpuState::new();
    s.gpr[5] = 0xCAFE;
    exec_no_mem(
        &PpuInstruction::Ori {
            ra: 3,
            rs: 5,
            imm: 0,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 0xCAFE);
}

#[test]
fn extsw_sign_extends_low_32_bits() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x0000_0000_8000_0000;
    exec_no_mem(
        &PpuInstruction::Extsw {
            ra: 4,
            rs: 3,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[4], 0xFFFF_FFFF_8000_0000);
}

#[test]
fn cntlzd_counts_64_for_zero() {
    let mut s = PpuState::new();
    s.gpr[5] = 0;
    exec_no_mem(
        &PpuInstruction::Cntlzd {
            ra: 3,
            rs: 5,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 64);
}

#[test]
fn cntlzd_high_bit_set_returns_zero() {
    let mut s = PpuState::new();
    s.gpr[5] = 1u64 << 63;
    exec_no_mem(
        &PpuInstruction::Cntlzd {
            ra: 3,
            rs: 5,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 0);
}

#[test]
fn orc_is_or_with_complement_rb() {
    let mut s = PpuState::new();
    s.gpr[4] = 0x00FF_0000;
    s.gpr[5] = 0x0000_00FF;
    exec_no_mem(
        &PpuInstruction::Orc {
            ra: 3,
            rs: 4,
            rb: 5,
            rc: false,
        },
        &mut s,
    );
    // orc is 32-bit, result sign-extended to 64 bits on this operand.
    assert_eq!(s.gpr[3], 0xFFFF_FFFF_FFFF_FF00);
}

#[test]
fn or_dot_sets_cr0_without_touching_result() {
    // `or. rA, rS, rS` must update CR0; quickening it to plain
    // `Mr` (move register) is incorrect because Mr has no Rc form.
    let mut s = PpuState::new();
    s.gpr[4] = (-5i64) as u64;
    exec_no_mem(
        &PpuInstruction::Or {
            ra: 3,
            rs: 4,
            rb: 4,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], (-5i64) as u64);
    assert_eq!(s.cr_field(0), 0b1000, "LT from negative result");
}

#[test]
fn and_dot_sets_cr0_eq_on_zero() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xFF00;
    s.gpr[4] = 0x00FF;
    exec_no_mem(
        &PpuInstruction::And {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0);
    assert_eq!(s.cr_field(0), 0b0010);
}

#[test]
fn cntlzd_dot_sets_cr0_gt_when_value_nonzero() {
    let mut s = PpuState::new();
    s.gpr[3] = 1u64 << 40;
    exec_no_mem(
        &PpuInstruction::Cntlzd {
            ra: 5,
            rs: 3,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 23);
    assert_eq!(s.cr_field(0), 0b0100);
}

#[test]
fn andi_dot_propagates_xer_so_into_cr0() {
    // andi. routes through set_cr0_from_result, which OR-in's SO.
    // Hand-rolled CR0 construction that ignores XER[SO] would
    // produce a CR0 with the SO bit always zero.
    let mut s = PpuState::new();
    s.gpr[3] = 0xFF;
    s.set_xer_ov(true);
    exec_no_mem(
        &PpuInstruction::AndiDot {
            ra: 4,
            rs: 3,
            imm: 0x0F,
        },
        &mut s,
    );
    assert_eq!(s.gpr[4], 0x0F);
    // GT (positive non-zero) + SO: 0b0100 | 0b0001 = 0b0101.
    assert_eq!(s.cr_field(0), 0b0101);
}

#[test]
fn andis_dot_shifts_immediate_left_16() {
    // andis. masks RS with (UI << 16). Reading andis. as andi.
    // would mask with 0x0F instead of 0x000F_0000 here.
    let mut s = PpuState::new();
    s.gpr[3] = 0x00FF_00FF;
    exec_no_mem(
        &PpuInstruction::AndisDot {
            ra: 4,
            rs: 3,
            imm: 0x0F,
        },
        &mut s,
    );
    // 0x00FF_00FF & 0x000F_0000 = 0x000F_0000.
    assert_eq!(s.gpr[4], 0x000F_0000);
    assert_eq!(s.cr_field(0), 0b0100); // GT (positive nonzero)
}

#[test]
fn andis_dot_zero_result_sets_eq() {
    // No bit overlap between RS and (UI << 16) -> result 0 -> EQ.
    let mut s = PpuState::new();
    s.gpr[3] = 0x0000_FFFF; // bits 0..16 only
    exec_no_mem(
        &PpuInstruction::AndisDot {
            ra: 4,
            rs: 3,
            imm: 0x0F, // shifted to 0x000F_0000 -- no overlap
        },
        &mut s,
    );
    assert_eq!(s.gpr[4], 0);
    assert_eq!(s.cr_field(0), 0b0010); // EQ
}

#[test]
fn popcntb_faults_on_cell_ppe() {
    // [CBE-Handbook p:738 s:A.2.4.1] Cell PPE does not implement popcntb.
    let mut s = PpuState::new();
    s.gpr[5] = 0x3f1f_0f07_0301_ff00u64;
    let v = exec_no_mem(&PpuInstruction::Popcntb { ra: 3, rs: 5 }, &mut s);
    assert!(matches!(
        v,
        ExecuteVerdict::Fault(PpuFault::UnimplementedInstruction(122))
    ));
    // RA must be left untouched -- a fault discards all effects.
    assert_eq!(s.gpr[3], 0);
}

// -- Logical Rc=1 fillers (skipping And, Or which exist) --

#[test]
fn andc_dot_sets_cr0_eq_on_zero() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xFF;
    s.gpr[4] = 0xFF; // ~RB clears RS bits
    exec_no_mem(
        &PpuInstruction::Andc {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b0010);
}

#[test]
fn orc_dot_sets_cr0_lt_on_negative_result() {
    let mut s = PpuState::new();
    s.gpr[3] = 0;
    s.gpr[4] = 0; // ~RB = u64::MAX -> negative
    exec_no_mem(
        &PpuInstruction::Orc {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b1000);
}

#[test]
fn xor_dot_sets_cr0_eq_when_operands_equal() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xDEAD_BEEF;
    s.gpr[4] = 0xDEAD_BEEF;
    exec_no_mem(
        &PpuInstruction::Xor {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b0010);
}

#[test]
fn nor_dot_sets_cr0_lt_on_negative_result() {
    let mut s = PpuState::new();
    s.gpr[3] = 0;
    s.gpr[4] = 0; // ~(0 | 0) = u64::MAX -> negative
    exec_no_mem(
        &PpuInstruction::Nor {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b1000);
}

#[test]
fn nand_dot_sets_cr0_eq_when_both_all_ones() {
    let mut s = PpuState::new();
    s.gpr[3] = u64::MAX;
    s.gpr[4] = u64::MAX; // ~(MAX & MAX) = 0
    exec_no_mem(
        &PpuInstruction::Nand {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b0010);
}

#[test]
fn eqv_dot_sets_cr0_lt_when_operands_equal() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xDEAD_BEEF;
    s.gpr[4] = 0xDEAD_BEEF; // ~(RS ^ RB) = u64::MAX
    exec_no_mem(
        &PpuInstruction::Eqv {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b1000);
}

// -- Cntlzw / Extsb / Extsh / Extsw Rc --

#[test]
fn cntlzw_dot_sets_cr0_gt_on_nonzero_count() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x0000_0000_0010_0000;
    exec_no_mem(
        &PpuInstruction::Cntlzw {
            ra: 5,
            rs: 3,
            rc: true,
        },
        &mut s,
    );
    // 0x10_0000 has 11 leading zeros in 32-bit.
    assert_eq!(s.gpr[5], 11);
    assert_eq!(s.cr_field(0), 0b0100);
}

#[test]
fn extsb_dot_sets_cr0_lt_on_negative_byte() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x80; // i8 negative
    exec_no_mem(
        &PpuInstruction::Extsb {
            ra: 5,
            rs: 3,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b1000);
}

#[test]
fn extsh_dot_sets_cr0_lt_on_negative_halfword() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x8000; // i16 negative
    exec_no_mem(
        &PpuInstruction::Extsh {
            ra: 5,
            rs: 3,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b1000);
}

#[test]
fn extsw_dot_sets_cr0_lt_on_negative_word() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x8000_0000; // i32 negative
    exec_no_mem(
        &PpuInstruction::Extsw {
            ra: 5,
            rs: 3,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b1000);
}
