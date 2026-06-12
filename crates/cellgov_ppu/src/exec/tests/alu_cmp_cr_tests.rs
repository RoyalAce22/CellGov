//! Compare families with XER.SO propagation and CR-field move instructions.

use super::*;

#[test]
fn cmpwi_sets_cr_field() {
    let mut s = PpuState::new();
    s.gpr[3] = 10;
    exec_no_mem(
        &PpuInstruction::Cmpwi {
            bf: 0,
            ra: 3,
            imm: 10,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b0010); // EQ
}

#[test]
fn cmpdi_compares_full_64_bits() {
    // With only the low 32 bits examined, 0x1_0000_0000 would compare
    // equal to zero. cmpdi must see the full doubleword.
    let mut s = PpuState::new();
    s.gpr[3] = 0x1_0000_0000;
    exec_no_mem(
        &PpuInstruction::Cmpdi {
            bf: 0,
            ra: 3,
            imm: 0,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(0), 0b0100); // GT
}

#[test]
fn cmpldi_compares_full_64_bits_unsigned() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x1_0000_0000;
    exec_no_mem(
        &PpuInstruction::Cmpldi {
            bf: 1,
            ra: 3,
            imm: 0,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(1), 0b0100); // GT
}

#[test]
fn cr0_so_bit_tracks_sticky_xer_so() {
    // After an overflow, every record-form instruction must copy the
    // current (sticky) SO into CR0.SO.
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
    // SO is set. A subsequent dot-form should carry SO into CR0.
    s.gpr[3] = 1;
    s.gpr[4] = 2;
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
    assert_eq!(s.cr_field(0), 0b0101, "GT plus sticky SO");
}

#[test]
fn cmpwi_propagates_xer_so_into_cr_field() {
    // SO is sticky once set; every subsequent compare must copy
    // it into the LSB of the CR field. A compare that drops SO
    // would leave guest branch logic looking at stale data.
    let mut s = PpuState::new();
    s.gpr[3] = 0;
    s.set_xer_ov(true); // sets both OV and SO
    exec_no_mem(
        &PpuInstruction::Cmpwi {
            bf: 0,
            ra: 3,
            imm: 0,
        },
        &mut s,
    );
    // EQ + SO: 0b0010 | 0b0001 = 0b0011.
    assert_eq!(s.cr_field(0), 0b0011);
}

#[test]
fn cmpw_propagates_xer_so_with_lt_result() {
    let mut s = PpuState::new();
    s.gpr[3] = 1;
    s.gpr[4] = 2;
    s.set_xer_ov(true);
    exec_no_mem(
        &PpuInstruction::Cmpw {
            bf: 7,
            ra: 3,
            rb: 4,
        },
        &mut s,
    );
    // LT + SO: 0b1000 | 0b0001 = 0b1001.
    assert_eq!(s.cr_field(7), 0b1001);
}

#[test]
fn mcrxr_copies_xer_status_nibble_and_clears_it() {
    let mut s = PpuState::new();
    // XER bits 32 (SO) and 34 (CA) set; bit 33 (OV) and bit 35 (reserved) clear.
    // Rust positions: SO = bit 31, OV = bit 30, CA = bit 29, reserved = bit 28.
    s.xer = (1u64 << 31) | (1u64 << 29);
    // Pre-seed the target CR field with a sentinel to confirm overwrite.
    s.set_cr_field(5, 0b1111);
    exec_no_mem(&PpuInstruction::Mcrxr { bf: 5 }, &mut s);
    // Expected CR field = SO|OV|CA|res = 1010.
    assert_eq!(s.cr_field(5), 0b1010);
    // XER[32..35] cleared, rest preserved.
    assert_eq!(s.xer & (0xFu64 << 28), 0);
}

#[test]
fn mtcrf_reads_low_32_bits_of_rs() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xAAAA_AAAA_5555_5555;
    s.cr = 0;
    exec_no_mem(
        &PpuInstruction::Mtcrf {
            rs: 3,
            crm: 0xFF, // all 8 fields
        },
        &mut s,
    );
    assert_eq!(s.cr, 0x5555_5555, "mtcrf must take the low 32 bits of RS");
}

#[test]
fn mtocrf_with_multi_bit_crm_diverges_from_mtcrf() {
    // CRM=0xC0: mtcrf updates fields 0 AND 1; mtocrf only the
    // highest set bit (field 0). RS low 32 0x12345678 -> field 0
    // = 0x1, field 1 = 0x2; CR sentinel 0xAAAA_AAAA reveals
    // untouched fields.
    let mut s_mtcrf = PpuState::new();
    s_mtcrf.gpr[3] = 0xAAAA_AAAA_1234_5678;
    s_mtcrf.cr = 0xAAAA_AAAA;
    exec_no_mem(&PpuInstruction::Mtcrf { rs: 3, crm: 0xC0 }, &mut s_mtcrf);
    // mtcrf: fields 0 AND 1 updated.
    assert_eq!(s_mtcrf.cr_field(0), 0x1);
    assert_eq!(s_mtcrf.cr_field(1), 0x2);
    // Fields 2..7 unchanged (sentinel 0xA).
    for f in 2..=7 {
        assert_eq!(s_mtcrf.cr_field(f), 0xA, "mtcrf field {f}");
    }

    let mut s_mtocrf = PpuState::new();
    s_mtocrf.gpr[3] = 0xAAAA_AAAA_1234_5678;
    s_mtocrf.cr = 0xAAAA_AAAA;
    exec_no_mem(&PpuInstruction::Mtocrf { rs: 3, crm: 0xC0 }, &mut s_mtocrf);
    // mtocrf: ONLY field 0 (highest set bit of CRM).
    assert_eq!(s_mtocrf.cr_field(0), 0x1);
    // Field 1 untouched -- stays at sentinel.
    assert_eq!(
        s_mtocrf.cr_field(1),
        0xA,
        "mtocrf must leave field 1 alone (mtcrf would not)"
    );
    for f in 2..=7 {
        assert_eq!(s_mtocrf.cr_field(f), 0xA, "mtocrf field {f}");
    }
    // Final discriminator: the post-states diverge.
    assert_ne!(
        s_mtcrf.cr, s_mtocrf.cr,
        "Mtocrf must NOT be a passthrough to Mtcrf semantics"
    );
}

#[test]
fn mtocrf_one_hot_crm_updates_only_selected_field() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xAAAA_AAAA_1234_5678;
    s.cr = 0xAAAA_AAAA;
    // CRM = 0x10 (bit 4 of 8 => field 3, RS bits 32+12..35+12).
    exec_no_mem(&PpuInstruction::Mtocrf { rs: 3, crm: 0x10 }, &mut s);
    // Field 3 receives RS low 32 bits at field-3 position = 0x4.
    assert_eq!(s.cr_field(3), 0x4);
    for f in (0..=7).filter(|f| *f != 3) {
        assert_eq!(s.cr_field(f), 0xA, "mtocrf field {f} must be untouched");
    }
}

#[test]
fn mfocrf_with_multi_bit_crm_faults() {
    let mut s = PpuState::new();
    s.cr = 0x1234_5678;
    let v = exec_no_mem(&PpuInstruction::Mfocrf { rt: 3, crm: 0xC0 }, &mut s);
    assert!(matches!(
        v,
        ExecuteVerdict::Fault(PpuFault::UnimplementedInstruction(19))
    ));
    // RT must be untouched (effect-discard for fault).
    assert_eq!(s.gpr[3], 0);
}

#[test]
fn mfocrf_with_one_hot_crm_extracts_one_field_distinct_from_mfcr() {
    let mut s_mfcr = PpuState::new();
    s_mfcr.cr = 0x1234_5678;
    exec_no_mem(&PpuInstruction::Mfcr { rt: 3 }, &mut s_mfcr);
    // mfcr: full CR into low 32 bits of RT.
    assert_eq!(s_mfcr.gpr[3], 0x1234_5678);

    let mut s_mfocrf = PpuState::new();
    s_mfocrf.cr = 0x1234_5678;
    exec_no_mem(&PpuInstruction::Mfocrf { rt: 3, crm: 0x80 }, &mut s_mfocrf);
    // mfocrf: field 0 (= 0x1) at position (7-0)*4 = 28, zero
    // elsewhere -> 0x1000_0000.
    assert_eq!(s_mfocrf.gpr[3], 0x1000_0000);
    // Final discriminator: the RT values diverge.
    assert_ne!(
        s_mfcr.gpr[3], s_mfocrf.gpr[3],
        "Mfocrf must NOT be a passthrough to Mfcr semantics"
    );
}

// -- Compare SO propagation: Cmplwi / Cmpdi / Cmpldi / Cmplw / Cmpd / Cmpld --

#[test]
fn cmplwi_propagates_xer_so_into_cr_field() {
    let mut s = PpuState::new();
    s.gpr[3] = 1;
    s.set_xer_ov(true);
    exec_no_mem(
        &PpuInstruction::Cmplwi {
            bf: 0,
            ra: 3,
            imm: 1,
        },
        &mut s,
    );
    // EQ + SO.
    assert_eq!(s.cr_field(0), 0b0011);
}

#[test]
fn cmpdi_propagates_xer_so_into_cr_field() {
    let mut s = PpuState::new();
    s.gpr[3] = 5;
    s.set_xer_ov(true);
    exec_no_mem(
        &PpuInstruction::Cmpdi {
            bf: 2,
            ra: 3,
            imm: 5,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(2), 0b0011);
}

#[test]
fn cmpldi_propagates_xer_so_into_cr_field() {
    let mut s = PpuState::new();
    s.gpr[3] = 9;
    s.set_xer_ov(true);
    exec_no_mem(
        &PpuInstruction::Cmpldi {
            bf: 3,
            ra: 3,
            imm: 9,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(3), 0b0011);
}

#[test]
fn cmplw_propagates_xer_so_with_lt_result() {
    let mut s = PpuState::new();
    s.gpr[3] = 1;
    s.gpr[4] = 2;
    s.set_xer_ov(true);
    exec_no_mem(
        &PpuInstruction::Cmplw {
            bf: 4,
            ra: 3,
            rb: 4,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(4), 0b1001);
}

#[test]
fn cmpd_propagates_xer_so_with_gt_result() {
    let mut s = PpuState::new();
    s.gpr[3] = 10;
    s.gpr[4] = 2;
    s.set_xer_ov(true);
    exec_no_mem(
        &PpuInstruction::Cmpd {
            bf: 5,
            ra: 3,
            rb: 4,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(5), 0b0101);
}

#[test]
fn cmpld_propagates_xer_so_with_eq_result() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x1_0000_0000;
    s.gpr[4] = 0x1_0000_0000;
    s.set_xer_ov(true);
    exec_no_mem(
        &PpuInstruction::Cmpld {
            bf: 6,
            ra: 3,
            rb: 4,
        },
        &mut s,
    );
    assert_eq!(s.cr_field(6), 0b0011);
}
