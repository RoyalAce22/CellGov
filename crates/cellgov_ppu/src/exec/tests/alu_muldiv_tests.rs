//! Multiply and divide: high-half products, divide-by-zero OV, and Rc CR0 recording.

use super::*;

#[test]
fn divdu_basic() {
    let mut s = PpuState::new();
    s.gpr[3] = 100;
    s.gpr[4] = 7;
    exec_no_mem(
        &PpuInstruction::Divdu {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 14);
}

#[test]
fn divdu_divide_by_zero() {
    let mut s = PpuState::new();
    s.gpr[3] = 100;
    s.gpr[4] = 0;
    exec_no_mem(
        &PpuInstruction::Divdu {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0);
}

#[test]
fn divdu_large_values() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
    s.gpr[4] = 2;
    exec_no_mem(
        &PpuInstruction::Divdu {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0x7FFF_FFFF_FFFF_FFFF);
}

#[test]
fn divd_signed() {
    let mut s = PpuState::new();
    s.gpr[3] = (-100i64) as u64;
    s.gpr[4] = 7;
    exec_no_mem(
        &PpuInstruction::Divd {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5] as i64, -14);
}

#[test]
fn divd_small_dividend_returns_zero() {
    // Hex-format conversion routines do `value / base` until
    // value reaches 0; the last iteration always has dividend
    // < divisor (e.g. 0xF / 16, 1 / 16). Verify those produce
    // zero quotient.
    for (a, b) in [(0u64, 16u64), (1, 16), (0xFu64, 16), (15, 16)] {
        let mut s = PpuState::new();
        s.gpr[3] = a;
        s.gpr[4] = b;
        exec_no_mem(
            &PpuInstruction::Divd {
                rt: 5,
                ra: 3,
                rb: 4,
                oe: false,
                rc: false,
            },
            &mut s,
        );
        assert_eq!(s.gpr[5], 0, "divd({a:#x}, {b}) expected 0");
    }
}

#[test]
fn divd_divide_by_zero() {
    let mut s = PpuState::new();
    s.gpr[3] = 100;
    s.gpr[4] = 0;
    exec_no_mem(
        &PpuInstruction::Divd {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0);
}

#[test]
fn mulld_basic() {
    let mut s = PpuState::new();
    s.gpr[3] = 7;
    s.gpr[4] = 8;
    exec_no_mem(
        &PpuInstruction::Mulld {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 56);
}

#[test]
fn mulld_wraps_on_overflow() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
    s.gpr[4] = 2;
    exec_no_mem(
        &PpuInstruction::Mulld {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: false,
        },
        &mut s,
    );
    // -1 * 2 = -2 (wrapping) = 0xFFFF_FFFF_FFFF_FFFE
    assert_eq!(s.gpr[5], 0xFFFF_FFFF_FFFF_FFFE);
}

#[test]
fn mulhdu_takes_high_64_bits_of_u128_product() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
    s.gpr[4] = 2;
    exec_no_mem(
        &PpuInstruction::Mulhdu {
            rt: 5,
            ra: 3,
            rb: 4,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 1);
}

#[test]
fn mulhdu_small_product_is_zero() {
    let mut s = PpuState::new();
    s.gpr[3] = 7;
    s.gpr[4] = 8;
    exec_no_mem(
        &PpuInstruction::Mulhdu {
            rt: 5,
            ra: 3,
            rb: 4,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0);
}

#[test]
fn mulhw_signed_high_32_bits() {
    let mut s = PpuState::new();
    s.gpr[4] = (-2i32) as u32 as u64;
    s.gpr[5] = 3;
    exec_no_mem(
        &PpuInstruction::Mulhw {
            rt: 3,
            ra: 4,
            rb: 5,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 0xFFFFFFFF_FFFFFFFFu64);
}

#[test]
fn mulhw_positive_produces_zero_high_bits() {
    let mut s = PpuState::new();
    s.gpr[4] = 0x0001_0000;
    s.gpr[5] = 0x0001_0000;
    exec_no_mem(
        &PpuInstruction::Mulhw {
            rt: 3,
            ra: 4,
            rb: 5,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 1);
}

#[test]
fn mulhd_signed_high_doubleword() {
    let mut s = PpuState::new();
    s.gpr[3] = u64::MAX;
    s.gpr[4] = u64::MAX;
    exec_no_mem(
        &PpuInstruction::Mulhd {
            rt: 5,
            ra: 3,
            rb: 4,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0);

    s.gpr[3] = u64::MAX;
    s.gpr[4] = 2;
    exec_no_mem(
        &PpuInstruction::Mulhd {
            rt: 5,
            ra: 3,
            rb: 4,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], u64::MAX);
}

#[test]
fn divwo_div_by_zero_sets_ov() {
    let mut s = PpuState::new();
    s.gpr[3] = 100;
    s.gpr[4] = 0;
    exec_no_mem(
        &PpuInstruction::Divw {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0);
    assert_eq!(s.xer & (1u64 << 30), 1u64 << 30);
}

#[test]
fn mullwo_with_overflow_sets_ov() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x1_0000;
    s.gpr[4] = 0x1_0000;
    exec_no_mem(
        &PpuInstruction::Mullw {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    // 0x1_0000 * 0x1_0000 = 0x1_0000_0000, overflows 32-bit signed.
    assert_eq!(s.xer & (1u64 << 30), 1u64 << 30);
}

#[test]
fn mulhwu_cr0_treats_high_bit_result_as_positive() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xFFFF_FFFFu32 as u64;
    s.gpr[4] = 0xFFFF_FFFFu32 as u64;
    exec_no_mem(
        &PpuInstruction::Mulhwu {
            rt: 5,
            ra: 3,
            rb: 4,
            rc: true,
        },
        &mut s,
    );
    // (0xFFFF_FFFF * 0xFFFF_FFFF) >> 32 = 0xFFFF_FFFE.
    assert_eq!(s.gpr[5], 0xFFFF_FFFE);
    assert_eq!(s.cr_field(0), 0b0100, "GT, not LT");
}

#[test]
fn divwu_cr0_treats_high_bit_result_as_positive() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xFFFF_FFFFu32 as u64;
    s.gpr[4] = 1;
    exec_no_mem(
        &PpuInstruction::Divwu {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0xFFFF_FFFF);
    assert_eq!(s.cr_field(0), 0b0100, "GT, not LT");
}

// -- Mullw Rc --

#[test]
fn mullw_dot_sets_cr0_lt_on_negative_product() {
    let mut s = PpuState::new();
    s.gpr[1] = 0xFFFF_FFFF_FFFF_FFFE; // i32 -2
    s.gpr[2] = 0x0000_0000_0000_0003;
    exec_no_mem(
        &PpuInstruction::Mullw {
            rt: 3,
            ra: 1,
            rb: 2,
            oe: false,
            rc: true,
        },
        &mut s,
    );
    // -6 sign-extended is negative.
    assert_eq!(s.cr_field(0), 0b1000);
}

// -- Mulld OE/Rc --

#[test]
fn mulldo_signed_overflow_sets_ov() {
    let mut s = PpuState::new();
    s.gpr[3] = i64::MAX as u64;
    s.gpr[4] = 2;
    exec_no_mem(
        &PpuInstruction::Mulld {
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
fn mulld_dot_sets_cr0_gt_on_positive_product() {
    let mut s = PpuState::new();
    s.gpr[3] = 7;
    s.gpr[4] = 8;
    exec_no_mem(
        &PpuInstruction::Mulld {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b0100);
}

// -- Mulhw / Mulhd Rc --

#[test]
fn mulhw_dot_sets_cr0_lt_on_negative_high() {
    let mut s = PpuState::new();
    s.gpr[3] = (-2i32) as u32 as u64;
    s.gpr[4] = 3;
    exec_no_mem(
        &PpuInstruction::Mulhw {
            rt: 5,
            ra: 3,
            rb: 4,
            rc: true,
        },
        &mut s,
    );
    // High 32 of (-2 * 3) = high(-6 i64) = 0xFFFF_FFFF, sign-ext negative.
    assert_eq!(s.cr_field(0), 0b1000);
}

#[test]
fn mulhd_dot_sets_cr0_eq_on_small_product() {
    let mut s = PpuState::new();
    s.gpr[3] = 7;
    s.gpr[4] = 8;
    exec_no_mem(
        &PpuInstruction::Mulhd {
            rt: 5,
            ra: 3,
            rb: 4,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b0010);
}

// -- Divw / Divd / Divdu OE / Rc --

#[test]
fn divw_dot_sets_cr0_lt_on_negative_quotient() {
    let mut s = PpuState::new();
    s.gpr[3] = (-12i32) as u32 as u64;
    s.gpr[4] = 4;
    exec_no_mem(
        &PpuInstruction::Divw {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: false,
            rc: true,
        },
        &mut s,
    );
    // -3 sign-extended is negative.
    assert_eq!(s.cr_field(0), 0b1000);
}

#[test]
fn divwuo_div_by_zero_sets_ov() {
    let mut s = PpuState::new();
    s.gpr[3] = 100;
    s.gpr[4] = 0;
    exec_no_mem(
        &PpuInstruction::Divwu {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.xer & (1u64 << 30), 1u64 << 30);
}

#[test]
fn divdo_div_by_zero_sets_ov() {
    let mut s = PpuState::new();
    s.gpr[3] = 100;
    s.gpr[4] = 0;
    exec_no_mem(
        &PpuInstruction::Divd {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.xer & (1u64 << 30), 1u64 << 30);
}

#[test]
fn divdo_min_div_neg1_sets_ov() {
    let mut s = PpuState::new();
    s.gpr[3] = i64::MIN as u64;
    s.gpr[4] = (-1i64) as u64;
    exec_no_mem(
        &PpuInstruction::Divd {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.xer & (1u64 << 30), 1u64 << 30);
}

#[test]
fn divd_dot_sets_cr0_lt_on_negative_quotient() {
    let mut s = PpuState::new();
    s.gpr[3] = (-12i64) as u64;
    s.gpr[4] = 4;
    exec_no_mem(
        &PpuInstruction::Divd {
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
fn divduo_div_by_zero_sets_ov() {
    let mut s = PpuState::new();
    s.gpr[3] = 100;
    s.gpr[4] = 0;
    exec_no_mem(
        &PpuInstruction::Divdu {
            rt: 5,
            ra: 3,
            rb: 4,
            oe: true,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.xer & (1u64 << 30), 1u64 << 30);
}

#[test]
fn divdu_dot_sets_cr0_eq_on_zero_quotient() {
    let mut s = PpuState::new();
    s.gpr[3] = 1;
    s.gpr[4] = 100; // 1 / 100 = 0
    exec_no_mem(
        &PpuInstruction::Divdu {
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
