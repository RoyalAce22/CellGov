//! PpuState CR field/bit accessors and effective-address computation.

use super::*;

#[test]
fn new_state_is_zeroed() {
    let s = PpuState::new();
    assert_eq!(s.pc, 0);
    assert_eq!(s.lr(), 0);
    assert_eq!(s.ctr(), 0);
    assert_eq!(s.cr(), 0);
    assert!(s.gpr.as_array().iter().all(|&r| r == 0));
}

/// Golden pin on the hash byte stream: reordering the fingerprint
/// fold (field order or per-field encoding) shifts every recorded
/// per-step hash in every trace and anchor, so it must fail HERE with
/// a named cause, not later as an unexplained cross-runner mismatch.
#[test]
fn state_hash_byte_stream_is_pinned() {
    let mut s = PpuState::new();
    for i in 0..32 {
        s.set_gpr(i, 0x0101_0101_0101_0101u64.wrapping_mul(i as u64 + 1));
    }
    s.set_lr(0x1122_3344_5566_7788);
    s.set_ctr(0x99AA_BBCC_DDEE_FF00);
    s.set_xer((1 << 29) | (1 << 31));
    s.set_cr(0xA5A5_5A5A);
    s.set_reservation(Some(ReservedLine::containing(0x3000_1080)));
    assert_eq!(s.state_hash(), 0xCDB1_0BA6_1479_AD7A);
}

#[test]
fn cr_field_roundtrip() {
    let mut s = PpuState::new();
    s.set_cr_field(0, 0b1010);
    assert_eq!(s.cr_field(0), 0b1010);
    assert_eq!(s.cr_field(1), 0);
    assert_eq!(s.cr_field(7), 0);
}

#[test]
fn cr_bit_reads_correct_position() {
    let mut s = PpuState::new();
    // CR field 0 = LT(1) GT(0) EQ(1) SO(0) = 0b1010
    s.set_cr_field(0, 0b1010);
    assert!(s.cr_bit(0));
    assert!(!s.cr_bit(1));
    assert!(s.cr_bit(2));
    assert!(!s.cr_bit(3));
}

#[test]
fn ea_d_form_ra_zero_uses_literal_zero() {
    let mut s = PpuState::new();
    s.set_gpr(0, 0xDEAD);
    assert_eq!(s.ea_d_form(0, 100), 100);
}

#[test]
fn ea_x_form_ra_zero_uses_literal_zero() {
    let mut s = PpuState::new();
    s.set_gpr(0, 0xDEAD);
    s.set_gpr(5, 200);
    assert_eq!(s.ea_x_form(0, 5), 200);
}

#[test]
fn set_cr_field_preserves_other_fields() {
    let mut s = PpuState::new();
    s.set_cr_field(3, 0b1111);
    s.set_cr_field(5, 0b0101);
    assert_eq!(s.cr_field(3), 0b1111);
    assert_eq!(s.cr_field(5), 0b0101);
    s.set_cr_field(3, 0b1010);
    assert_eq!(s.cr_field(3), 0b1010);
    assert_eq!(s.cr_field(5), 0b0101);
    assert_eq!(s.cr_field(0), 0);
    assert_eq!(s.cr_field(7), 0);
}

#[test]
fn ea_d_form_negative_displacement() {
    let mut s = PpuState::new();
    s.set_gpr(1, 1000);
    assert_eq!(s.ea_d_form(1, -4), 996);
}

#[test]
fn xer_ca_round_trips() {
    let mut s = PpuState::new();
    assert!(!s.xer_ca(), "fresh state has CA cleared");
    s.set_xer_ca(true);
    assert!(s.xer_ca());
    s.set_xer_ca(false);
    assert!(!s.xer_ca());
}

#[test]
fn set_xer_ca_does_not_touch_other_bits() {
    let mut s = PpuState::new();
    s.set_xer(!(1u64 << 29));
    s.set_xer_ca(true);
    assert_eq!(s.xer(), !0u64, "set CA should preserve all other bits");
    s.set_xer_ca(false);
    assert_eq!(
        s.xer(),
        !(1u64 << 29),
        "clear CA should preserve all other bits"
    );
}

#[test]
fn state_hash_is_reproducible_for_same_state() {
    let mut a = PpuState::new();
    let mut b = PpuState::new();
    a.set_gpr(3, 0x1234_5678_9abc_def0);
    a.set_lr(0x42);
    a.set_ctr(0x84);
    a.set_xer(1 << 29);
    a.set_cr(0xa5a5_a5a5);
    b.set_gpr(3, 0x1234_5678_9abc_def0);
    b.set_lr(0x42);
    b.set_ctr(0x84);
    b.set_xer(1 << 29);
    b.set_cr(0xa5a5_a5a5);
    assert_eq!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_distinguishes_every_covered_field() {
    let base = PpuState::new();
    let baseline = base.state_hash();

    for i in 0..GPR_COUNT {
        let mut s = base.clone();
        s.set_gpr(i, 1);
        assert_ne!(
            s.state_hash(),
            baseline,
            "GPR[{i}] must influence state_hash"
        );
    }

    let mut s = base.clone();
    s.set_lr(1);
    assert_ne!(s.state_hash(), baseline, "LR must influence state_hash");

    let mut s = base.clone();
    s.set_ctr(1);
    assert_ne!(s.state_hash(), baseline, "CTR must influence state_hash");

    let mut s = base.clone();
    s.set_xer(1);
    assert_ne!(s.state_hash(), baseline, "XER must influence state_hash");

    let mut s = base.clone();
    s.set_cr(1);
    assert_ne!(s.state_hash(), baseline, "CR must influence state_hash");
}

#[test]
fn state_hash_ignores_pc_fpr_vr() {
    let base = PpuState::new();
    let baseline = base.state_hash();

    let mut s = base.clone();
    s.pc = 0xdead_beef;
    assert_eq!(s.state_hash(), baseline, "PC is excluded");

    let mut s = base.clone();
    s.set_fpr(7, 0xffff_ffff_ffff_ffff);
    assert_eq!(s.state_hash(), baseline, "FPR is excluded");

    let mut s = base.clone();
    s.set_vr(0, u128::MAX);
    assert_eq!(s.state_hash(), baseline, "VR is excluded");
}

#[test]
fn state_hash_ignores_instrument_counters() {
    let base = PpuState::new();
    let baseline = base.state_hash();

    let mut s = base.clone();
    s.vrsave = 0xffff_ffff;
    assert_eq!(s.state_hash(), baseline, "VRSAVE is excluded");

    let mut s = base.clone();
    s.vrsave_written = true;
    assert_eq!(
        s.state_hash(),
        baseline,
        "vrsave_written instrument flag is excluded"
    );

    let mut s = base.clone();
    s.mfvrsave_executed = 1;
    assert_eq!(
        s.state_hash(),
        baseline,
        "mfvrsave_executed counter is excluded"
    );

    let mut s = base.clone();
    s.ldarx_executed = 1;
    assert_eq!(
        s.state_hash(),
        baseline,
        "ldarx_executed counter is excluded"
    );

    let mut s = base.clone();
    s.stdcx_executed = 1;
    assert_eq!(
        s.state_hash(),
        baseline,
        "stdcx_executed counter is excluded"
    );

    let mut s = base.clone();
    s.lwarx_executed = 1;
    assert_eq!(
        s.state_hash(),
        baseline,
        "lwarx_executed counter is excluded"
    );

    let mut s = base.clone();
    s.stwcx_executed = 1;
    assert_eq!(
        s.state_hash(),
        baseline,
        "stwcx_executed counter is excluded"
    );

    let mut s = base.clone();
    s.mem_fault_arm_entries = 1;
    assert_eq!(
        s.state_hash(),
        baseline,
        "mem_fault_arm_entries counter is excluded"
    );

    let mut s = base.clone();
    s.mem_fault_unmapped_routed = 1;
    assert_eq!(
        s.state_hash(),
        baseline,
        "mem_fault_unmapped_routed counter is excluded"
    );

    let mut s = base.clone();
    s.dcbz_executed = 1;
    assert_eq!(
        s.state_hash(),
        baseline,
        "dcbz_executed counter is excluded"
    );
}

#[test]
fn state_hash_tracks_reservation_register() {
    let base = PpuState::new();
    let baseline = base.state_hash();

    let mut s = base.clone();
    s.set_reservation(Some(ReservedLine::containing(0x1000)));
    let h_a = s.state_hash();
    assert_ne!(h_a, baseline, "setting a reservation must flip the hash");

    let mut s = base.clone();
    s.set_reservation(Some(ReservedLine::containing(0x2000)));
    let h_b = s.state_hash();
    assert_ne!(h_a, h_b, "different reserved lines must hash distinctly");

    let mut s = base.clone();
    s.set_reservation(Some(ReservedLine::containing(0x1000)));
    s.set_reservation(None);
    assert_eq!(s.state_hash(), baseline);
}

#[test]
fn set_xer_ov_sets_ov_and_sticky_so() {
    let mut s = PpuState::new();
    s.set_xer_ov(true);
    assert_eq!(s.xer() & (1u64 << 30), 1u64 << 30, "OV set");
    assert_eq!(s.xer() & (1u64 << 31), 1u64 << 31, "SO set");
    s.set_xer_ov(false);
    assert_eq!(s.xer() & (1u64 << 30), 0, "OV cleared");
    assert_eq!(
        s.xer() & (1u64 << 31),
        1u64 << 31,
        "SO remains sticky across clear"
    );
}

#[test]
fn set_cr0_from_result_negative_gt_eq() {
    let mut s = PpuState::new();
    s.set_cr0_from_result((-1i64) as u64);
    assert_eq!(s.cr_field(0), 0b1000);
    s.set_cr0_from_result(1);
    assert_eq!(s.cr_field(0), 0b0100);
    s.set_cr0_from_result(0);
    assert_eq!(s.cr_field(0), 0b0010);
}

#[test]
fn set_cr0_from_result_copies_sticky_so() {
    let mut s = PpuState::new();
    s.set_xer_ov(true);
    s.set_xer_ov(false);
    s.set_cr0_from_result(0);
    assert_eq!(s.cr_field(0), 0b0011, "EQ set plus SO copied from XER");
}

#[test]
fn xer_ca_reads_only_bit_29() {
    let mut s = PpuState::new();
    s.set_xer(!(1u64 << 29));
    assert!(!s.xer_ca());
    s.set_xer(1u64 << 29);
    assert!(s.xer_ca());
}

#[test]
fn ppu_syscall_args_maps_r11_to_index_0_and_r3_through_r10_to_1_through_8() {
    let mut s = PpuState::new();
    s.set_gpr(3, 0xA300_0000_0000_0003);
    s.set_gpr(4, 0xA400_0000_0000_0004);
    s.set_gpr(5, 0xA500_0000_0000_0005);
    s.set_gpr(6, 0xA600_0000_0000_0006);
    s.set_gpr(7, 0xA700_0000_0000_0007);
    s.set_gpr(8, 0xA800_0000_0000_0008);
    s.set_gpr(9, 0xA900_0000_0000_0009);
    s.set_gpr(10, 0xAA00_0000_0000_000A);
    s.set_gpr(11, 0xAB00_0000_0000_000B);
    s.set_gpr(0, 0xDEAD_BEEF_DEAD_BEEF);
    s.set_gpr(2, 0xDEAD_BEEF_DEAD_BEEF);
    s.set_gpr(12, 0xDEAD_BEEF_DEAD_BEEF);
    s.set_gpr(31, 0xDEAD_BEEF_DEAD_BEEF);

    let args = ppu_syscall_args(&s);
    assert_eq!(args[0], 0xAB00_0000_0000_000B, "args[0] must be r11");
    assert_eq!(args[1], 0xA300_0000_0000_0003, "args[1] must be r3");
    assert_eq!(args[2], 0xA400_0000_0000_0004);
    assert_eq!(args[3], 0xA500_0000_0000_0005);
    assert_eq!(args[4], 0xA600_0000_0000_0006);
    assert_eq!(args[5], 0xA700_0000_0000_0007);
    assert_eq!(args[6], 0xA800_0000_0000_0008);
    assert_eq!(args[7], 0xA900_0000_0000_0009);
    assert_eq!(args[8], 0xAA00_0000_0000_000A, "args[8] must be r10");
    assert!(
        !args.contains(&0xDEAD_BEEF_DEAD_BEEF),
        "no register outside r3..=r11 may leak into the args array",
    );
}
