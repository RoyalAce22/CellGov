//! Rotate-and-mask and shift families: masks, CA from shifted-out bits, Rc CR0.

use super::*;

#[test]
fn rlwinm_mask_contiguous() {
    assert_eq!(rlwinm_mask(0, 31), 0xFFFFFFFF);
    assert_eq!(rlwinm_mask(16, 31), 0x0000FFFF);
    assert_eq!(rlwinm_mask(0, 15), 0xFFFF0000);
}

#[test]
fn rlwinm_mask_wrapped() {
    // mb > me: mask wraps around; here bits [0..3] and [28..31].
    assert_eq!(rlwinm_mask(28, 3), 0xF000000F);
}

#[test]
fn rlwinm_slwi() {
    let mut s = PpuState::new();
    s.gpr[5] = 0x0001;
    // slwi r3, r5, 16 == rlwinm r3, r5, 16, 0, 15
    exec_no_mem(
        &PpuInstruction::Rlwinm {
            ra: 3,
            rs: 5,
            sh: 16,
            mb: 0,
            me: 15,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 0x10000);
}

#[test]
fn rlwnm_rotates_by_rb_low_5_bits() {
    let mut s = PpuState::new();
    s.gpr[0] = 0x0000_0000_1234_5678;
    s.gpr[8] = 8;
    exec_no_mem(
        &PpuInstruction::Rlwnm {
            ra: 0,
            rs: 0,
            rb: 8,
            mb: 0,
            me: 31,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[0], 0x3456_7812);
}

#[test]
fn rlwnm_ignores_high_bits_of_rb() {
    // 0x20 == 32: only low 5 bits feed the rotate, so rotation == 0.
    let mut s = PpuState::new();
    s.gpr[1] = 0x0000_0000_DEAD_BEEF;
    s.gpr[2] = 0x20;
    exec_no_mem(
        &PpuInstruction::Rlwnm {
            ra: 3,
            rs: 1,
            rb: 2,
            mb: 0,
            me: 31,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[3], 0xDEAD_BEEF);
}

#[test]
fn rlwimi_preserves_ra_high_32() {
    // Per PPC-Book1 p:76, rlwimi inserts the rotated/masked source into
    // RA under MASK(MB+32, ME+32); the mask only covers the low 32, so
    // RA[0:31] must be PRESERVED. A prior implementation cast RA to u32
    // before merging, which silently wiped the high half.
    let mut s = PpuState::new();
    s.gpr[0] = 0xCAFE_BABE_DEAD_BEEF;
    s.gpr[1] = 0x0000_0000_0000_00FF;
    exec_no_mem(
        &PpuInstruction::Rlwimi {
            ra: 0,
            rs: 1,
            sh: 0,
            mb: 24,
            me: 31,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(
        s.gpr[0], 0xCAFE_BABE_DEAD_BEFF,
        "rlwimi must preserve RA[0:31] (high 32 unchanged), \
         merge rotated/masked source into RA[32:63] only"
    );
}

#[test]
fn sraw_preserves_sign_and_caps_at_31() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xFFFF_FFFF_8000_0000;
    s.gpr[4] = 4;
    exec_no_mem(
        &PpuInstruction::Sraw {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5] as i32 as i64, -2147483648i64 >> 4);
}

#[test]
fn srad_signed_64_bit_shift() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x8000_0000_0000_0000;
    s.gpr[4] = 4;
    exec_no_mem(
        &PpuInstruction::Srad {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5] as i64, (0x8000_0000_0000_0000u64 as i64) >> 4);
}

#[test]
fn sradi_shift_zero_clears_ca_and_preserves_value() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xDEAD_BEEF_CAFE_F00D;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Sradi {
            ra: 4,
            rs: 3,
            sh: 0,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[4], 0xDEAD_BEEF_CAFE_F00D);
    assert!(!s.xer_ca());
}

#[test]
fn rldic_clears_both_sides() {
    // rldic RA, RS, SH=4, MB=32: rotate left 4, keep bits 32..=(63-4)=59.
    // RS=0xFFFF_FFFF_FFFF_FFFF, rotated left 4 still saturated, mask zeroes
    // bits 0..=31 and 60..=63.
    let mut s = PpuState::new();
    s.gpr[4] = 0xFFFF_FFFF_FFFF_FFFF;
    exec_no_mem(
        &PpuInstruction::Rldic {
            ra: 5,
            rs: 4,
            sh: 4,
            mb: 32,
            rc: false,
        },
        &mut s,
    );
    // bits 32..=59 set, others clear.
    let expected: u64 = ((1u64 << 28) - 1) << 4;
    assert_eq!(s.gpr[5], expected);
}

#[test]
fn rldimi_preserves_prior_ra_outside_mask() {
    // rldimi RA, RS, SH=16, MB=0: mask = 0..=(63-16)=47, preserve 48..=63.
    let mut s = PpuState::new();
    s.gpr[4] = 0xDEAD_BEEF_CAFE_BABE; // RS
    s.gpr[5] = 0x1111_2222_3333_4444; // prior RA
    exec_no_mem(
        &PpuInstruction::Rldimi {
            ra: 5,
            rs: 4,
            sh: 16,
            mb: 0,
            rc: false,
        },
        &mut s,
    );
    // rotated = RS rotl 16 = 0xBEEF_CAFE_BABE_DEAD
    // mask = 0xFFFF_FFFF_FFFF_0000 (bits 0..=47 set)
    // merged = (rotated & mask) | (prior & !mask)
    //        = 0xBEEF_CAFE_BABE_0000 | 0x0000_0000_0000_4444
    //        = 0xBEEF_CAFE_BABE_4444
    assert_eq!(s.gpr[5], 0xBEEF_CAFE_BABE_4444);
}

#[test]
fn srad_shifts_full_64_bits_arithmetically() {
    let mut s = PpuState::new();
    s.gpr[4] = 0xFFFF_FFFF_FFFF_FFF0; // -16
    s.gpr[5] = 4;
    exec_no_mem(
        &PpuInstruction::Srad {
            ra: 3,
            rs: 4,
            rb: 5,
            rc: false,
        },
        &mut s,
    );
    // -16 >> 4 = -1, sign-extended across all 64 bits.
    assert_eq!(s.gpr[3], 0xFFFF_FFFF_FFFF_FFFF);
}

#[test]
fn slw_dot_sets_cr0_from_sign_extended_low_32() {
    // Result is 0x8000_0000 as u32, which sign-extends to a negative
    // i64 -- CR0 should read LT.
    let mut s = PpuState::new();
    s.gpr[3] = 1;
    s.gpr[4] = 31;
    exec_no_mem(
        &PpuInstruction::Slw {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0x8000_0000);
    assert_eq!(s.cr_field(0), 0b1000);
}

#[test]
fn srad_dot_sets_cr0_and_preserves_ca() {
    let mut s = PpuState::new();
    s.gpr[3] = (-1i64) as u64; // all-ones, guaranteed 1-bit shifted out.
    s.gpr[4] = 1;
    exec_no_mem(
        &PpuInstruction::Srad {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: true,
        },
        &mut s,
    );
    // -1 >> 1 = -1, and a 1 bit was shifted out of a negative value: CA set.
    assert!(s.xer_ca(), "CA set from nonzero bits shifted out");
    assert_eq!(s.cr_field(0), 0b1000, "LT from negative result");
}

#[test]
fn sradi_dot_sets_cr0() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x8000_0000_0000_0000;
    exec_no_mem(
        &PpuInstruction::Sradi {
            ra: 5,
            rs: 3,
            sh: 8,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b1000);
}

#[test]
fn rldicl_dot_sets_cr0_and_does_not_quicken_to_clrldi() {
    // Verifies the shadow-layer guard: rldicl. with sh=0 cannot be
    // quickened to Clrldi because Clrldi does not update CR0.
    let mut s = PpuState::new();
    s.gpr[3] = 0;
    exec_no_mem(
        &PpuInstruction::Rldicl {
            ra: 5,
            rs: 3,
            sh: 0,
            mb: 32,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0);
    assert_eq!(s.cr_field(0), 0b0010);
}

#[test]
fn rldimi_dot_sets_cr0_from_merged_value() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x1; // RS
    s.gpr[5] = 0xFFFF_FFFF_FFFF_FFFF; // prior RA (bits outside mask preserved)
                                      // rldimi. rA, rS, 32, 0: mask = bits 0..=31, merge RS<<32 into high half.
    exec_no_mem(
        &PpuInstruction::Rldimi {
            ra: 5,
            rs: 3,
            sh: 32,
            mb: 0,
            rc: true,
        },
        &mut s,
    );
    // rotated = 1 rotl 32 = 0x0000_0001_0000_0000
    // mask = 0xFFFF_FFFF_0000_0000
    // merged = (rotated & mask) | (prior & !mask)
    //        = 0x0000_0001_0000_0000 | 0x0000_0000_FFFF_FFFF
    //        = 0x0000_0001_FFFF_FFFF
    assert_eq!(s.gpr[5], 0x0000_0001_FFFF_FFFF);
    assert_eq!(s.cr_field(0), 0b0100, "positive nonzero");
}

#[test]
fn srawi_dot_sets_both_ca_and_cr0() {
    let mut s = PpuState::new();
    s.gpr[3] = (-1i32) as u32 as u64;
    exec_no_mem(
        &PpuInstruction::Srawi {
            ra: 5,
            rs: 3,
            sh: 1,
            rc: true,
        },
        &mut s,
    );
    // -1 arithmetic-shift-right-by-1 yields -1; negative RS with a
    // 1-bit shifted out sets CA; Rc sets CR0 LT from the negative result.
    assert!(s.xer_ca());
    assert_eq!(s.cr_field(0), 0b1000);
}

#[test]
fn srawi_sh_zero_clears_ca() {
    // [PPC-Book1 p:80 s:3.3.12.2] "A shift amount of zero causes
    // RA to receive EXTS(RS[32:63]), and CA to be set to 0." CA
    // is explicitly cleared, not computed from the (nonexistent)
    // shifted-out bits.
    let mut s = PpuState::new();
    s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Srawi {
            ra: 5,
            rs: 3,
            sh: 0,
            rc: false,
        },
        &mut s,
    );
    assert!(!s.xer_ca(), "sh=0 must clear CA regardless of prior value");
    assert_eq!(s.gpr[5], 0xFFFF_FFFF_FFFF_FFFF, "EXTS of -1 low word");
}

#[test]
fn srad_shift_ge_64_collapses_to_sign_broadcast() {
    // shift >= 64: RA = 64 copies of the sign bit, CA = sign bit.
    let mut s = PpuState::new();
    s.gpr[3] = 0x8000_0000_0000_0000;
    s.gpr[4] = 64;
    exec_no_mem(
        &PpuInstruction::Srad {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0xFFFF_FFFF_FFFF_FFFF);
    assert!(s.xer_ca());

    // shift > 64 with positive RS: all zeros, CA clear.
    s.gpr[3] = 0x1;
    s.gpr[4] = 100;
    exec_no_mem(
        &PpuInstruction::Srad {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: false,
        },
        &mut s,
    );
    assert_eq!(s.gpr[5], 0);
    assert!(!s.xer_ca());
}

// -- 64-bit shift Rc (Sld / Srd) --
// Sld/Srd CR0 was flagged clean by the audit; Slw/Srw are skipped
// (suspect cluster).

#[test]
fn sld_dot_sets_cr0_lt_on_high_bit_result() {
    let mut s = PpuState::new();
    s.gpr[3] = 1;
    s.gpr[4] = 63; // 1 << 63 = i64::MIN
    exec_no_mem(
        &PpuInstruction::Sld {
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
fn sld_dot_sets_cr0_eq_when_shift_ge_64() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xFFFF_FFFF_FFFF_FFFF;
    s.gpr[4] = 64; // shift >= 64 -> result 0
    exec_no_mem(
        &PpuInstruction::Sld {
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
fn srd_dot_sets_cr0_gt_when_high_bit_shifted_into_payload() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x8000_0000_0000_0000;
    s.gpr[4] = 1; // logical shift right -> positive
    exec_no_mem(
        &PpuInstruction::Srd {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b0100);
}

// -- Sraw CA conditions --

#[test]
fn sraw_positive_rs_clears_ca() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x0000_0000_7FFF_FFFF; // positive
    s.gpr[4] = 4;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Sraw {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: false,
        },
        &mut s,
    );
    assert!(!s.xer_ca(), "positive RS cannot set CA");
}

#[test]
fn sraw_sh_zero_clears_ca() {
    // [PPC-Book1 p:79 s:3.3.12.2] "If sh==0, RA=EXTS(RS[32:63]) and CA=0."
    let mut s = PpuState::new();
    s.gpr[3] = (-1i32) as u32 as u64;
    s.gpr[4] = 0;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Sraw {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: false,
        },
        &mut s,
    );
    assert!(!s.xer_ca(), "shift 0 must clear CA");
}

#[test]
fn sraw_negative_rs_with_no_one_bits_shifted_out_clears_ca() {
    // RS = 0xFFFF_FFF0 (negative i32), shift by 4: low 4 bits are zero,
    // so no 1-bits shift out -> CA cleared.
    let mut s = PpuState::new();
    s.gpr[3] = 0xFFFF_FFF0;
    s.gpr[4] = 4;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Sraw {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: false,
        },
        &mut s,
    );
    assert!(!s.xer_ca(), "no 1-bits shifted out -> CA=0");
}

#[test]
fn sraw_negative_rs_with_one_bits_shifted_out_sets_ca() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xFFFF_FFFF; // all ones
    s.gpr[4] = 4;
    s.set_xer_ca(false);
    exec_no_mem(
        &PpuInstruction::Sraw {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: false,
        },
        &mut s,
    );
    assert!(s.xer_ca(), "negative RS + 1-bits shifted out -> CA=1");
}

// -- Srad CA conditions (positive / sh=0) --

#[test]
fn srad_positive_rs_clears_ca() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x7FFF_FFFF_FFFF_FFFF; // positive
    s.gpr[4] = 4;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Srad {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: false,
        },
        &mut s,
    );
    assert!(!s.xer_ca());
}

#[test]
fn srad_sh_zero_clears_ca() {
    let mut s = PpuState::new();
    s.gpr[3] = u64::MAX;
    s.gpr[4] = 0;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Srad {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: false,
        },
        &mut s,
    );
    assert!(!s.xer_ca(), "sh=0 must clear CA");
}

#[test]
fn srad_negative_rs_with_no_one_bits_shifted_out_clears_ca() {
    // Low 4 bits zero, sign bit set.
    let mut s = PpuState::new();
    s.gpr[3] = 0xFFFF_FFFF_FFFF_FFF0;
    s.gpr[4] = 4;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Srad {
            ra: 5,
            rs: 3,
            rb: 4,
            rc: false,
        },
        &mut s,
    );
    assert!(!s.xer_ca());
}

// -- Sradi CA conditions --

#[test]
fn sradi_negative_rs_with_one_bits_shifted_out_sets_ca() {
    let mut s = PpuState::new();
    s.gpr[3] = u64::MAX;
    s.set_xer_ca(false);
    exec_no_mem(
        &PpuInstruction::Sradi {
            ra: 5,
            rs: 3,
            sh: 4,
            rc: false,
        },
        &mut s,
    );
    assert!(s.xer_ca());
}

#[test]
fn sradi_negative_rs_with_no_one_bits_shifted_out_clears_ca() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xFFFF_FFFF_FFFF_FFF0;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Sradi {
            ra: 5,
            rs: 3,
            sh: 4,
            rc: false,
        },
        &mut s,
    );
    assert!(!s.xer_ca());
}

#[test]
fn sradi_positive_rs_clears_ca() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x7FFF_FFFF_FFFF_FFFF;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Sradi {
            ra: 5,
            rs: 3,
            sh: 4,
            rc: false,
        },
        &mut s,
    );
    assert!(!s.xer_ca());
}

// -- Srawi CA: positive RS clears CA --

#[test]
fn srawi_positive_rs_clears_ca() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x0000_0000_7FFF_FFFF;
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Srawi {
            ra: 5,
            rs: 3,
            sh: 4,
            rc: false,
        },
        &mut s,
    );
    assert!(!s.xer_ca());
}

#[test]
fn srawi_negative_rs_with_no_one_bits_shifted_out_clears_ca() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xFFFF_FFF0; // negative i32, low 4 bits zero
    s.set_xer_ca(true);
    exec_no_mem(
        &PpuInstruction::Srawi {
            ra: 5,
            rs: 3,
            sh: 4,
            rc: false,
        },
        &mut s,
    );
    assert!(!s.xer_ca());
}

// -- 64-bit rotate Rc (Rldicr / Rldic / Rldcl / Rldcr) --

#[test]
fn rldicr_dot_sets_cr0_eq_on_zero() {
    let mut s = PpuState::new();
    s.gpr[3] = 0;
    exec_no_mem(
        &PpuInstruction::Rldicr {
            ra: 5,
            rs: 3,
            sh: 0,
            me: 63,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b0010);
}

#[test]
fn rldic_dot_sets_cr0_lt_on_high_bit_set() {
    let mut s = PpuState::new();
    s.gpr[3] = 1;
    // sh=63, mb=0: rotate-left 63 puts bit into MSB; mask MB..=(63-sh)=0..=0
    // keeps only bit 0 (MSB). Result = 0x8000_0000_0000_0000.
    exec_no_mem(
        &PpuInstruction::Rldic {
            ra: 5,
            rs: 3,
            sh: 63,
            mb: 0,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b1000);
}

#[test]
fn rldcl_dot_sets_cr0_eq_on_zero() {
    let mut s = PpuState::new();
    s.gpr[3] = 0;
    s.gpr[4] = 0;
    exec_no_mem(
        &PpuInstruction::Rldcl {
            ra: 5,
            rs: 3,
            rb: 4,
            mb: 0,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b0010);
}

#[test]
fn rldcr_dot_sets_cr0_gt_on_positive() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x1;
    s.gpr[4] = 8; // shift left by 8 -> 0x100
    exec_no_mem(
        &PpuInstruction::Rldcr {
            ra: 5,
            rs: 3,
            rb: 4,
            me: 63,
            rc: true,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b0100);
}
