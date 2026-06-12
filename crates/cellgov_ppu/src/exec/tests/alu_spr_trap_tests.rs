//! SPR moves (LR / TB / XER / VRSAVE) and tw / td trap conditions.

use super::*;

#[test]
fn mflr_mtlr_roundtrip() {
    let mut s = PpuState::new();
    s.gpr[5] = 0xABCD;
    exec_no_mem(&PpuInstruction::Mtlr { rs: 5 }, &mut s);
    assert_eq!(s.lr, 0xABCD);
    exec_no_mem(&PpuInstruction::Mflr { rt: 3 }, &mut s);
    assert_eq!(s.gpr[3], 0xABCD);
}

#[test]
fn mftbu_returns_upper_32_bits_of_tb() {
    let mut s = PpuState::new();
    s.tb = 0xAAAA_BBBB_0000_0000 - 1; // post-increment lands at 0xAAAA_BBBB_0000_0000
    exec_no_mem(&PpuInstruction::Mftbu { rt: 6 }, &mut s);
    assert_eq!(s.gpr[6], 0xAAAA_BBBB);
}

#[test]
fn mftb_returns_strictly_increasing_values_within_step() {
    // Two consecutive mftb reads in the same step must differ so a
    // guest doing `delta = t2 - t1` never observes zero.
    let mut s = PpuState::new();
    s.tb = 100;
    exec_no_mem(&PpuInstruction::Mftb { rt: 3 }, &mut s);
    let t1 = s.gpr[3];
    exec_no_mem(&PpuInstruction::Mftb { rt: 4 }, &mut s);
    let t2 = s.gpr[4];
    assert!(
        t2 > t1,
        "mftb must strictly increase per read: {t1} -> {t2}"
    );
}

#[test]
fn tw_no_condition_selected_never_traps() {
    let mut s = PpuState::new();
    s.gpr[3] = 1;
    s.gpr[4] = 2;
    let v = exec_no_mem(
        &PpuInstruction::Tw {
            to: 0,
            ra: 3,
            rb: 4,
        },
        &mut s,
    );
    assert!(matches!(v, ExecuteVerdict::Continue));
}

#[test]
fn tw_equal_with_to_equal_bit_traps() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xDEAD_BEEF;
    s.gpr[4] = 0xDEAD_BEEF;
    // TO bit 2 = equal, no other bits. TO = 0b00100 = 4.
    let v = exec_no_mem(
        &PpuInstruction::Tw {
            to: 4,
            ra: 3,
            rb: 4,
        },
        &mut s,
    );
    assert!(matches!(v, ExecuteVerdict::Fault(PpuFault::ProgramTrap(4))));
}

#[test]
fn tw_compares_low_32_bits_only() {
    // High halves differ but low 32 bits compare equal under both
    // signed and unsigned comparison; the 32-bit `tw` arm must not
    // surface the high-half divergence as inequality.
    let mut s = PpuState::new();
    s.gpr[3] = 0xAAAA_AAAA_0000_0001;
    s.gpr[4] = 0x5555_5555_0000_0001;
    // TO = all-bits-but-equal = 0b11011 = 27; only the equal arm fires.
    let v = exec_no_mem(
        &PpuInstruction::Tw {
            to: 27,
            ra: 3,
            rb: 4,
        },
        &mut s,
    );
    assert!(matches!(v, ExecuteVerdict::Continue));
    // With TO bit 2 (equal) included, the equal condition fires.
    let v = exec_no_mem(
        &PpuInstruction::Tw {
            to: 31,
            ra: 3,
            rb: 4,
        },
        &mut s,
    );
    assert!(matches!(
        v,
        ExecuteVerdict::Fault(PpuFault::ProgramTrap(31))
    ));
}

#[test]
fn td_unsigned_greater_traps_when_high_bits_differ() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x8000_0000_0000_0000; // very large unsigned, also negative signed
    s.gpr[4] = 0x0000_0000_0000_0001;
    // TO bit 0 (signed less) AND bit 4 (unsigned greater): TO = 0b10001 = 17.
    // For 64-bit: a is < b signed AND a > b unsigned. Either selected
    // condition that matches fires the trap.
    let v = exec_no_mem(
        &PpuInstruction::Td {
            to: 17,
            ra: 3,
            rb: 4,
        },
        &mut s,
    );
    assert!(matches!(
        v,
        ExecuteVerdict::Fault(PpuFault::ProgramTrap(17))
    ));
}

#[test]
fn mfxer_reads_full_64_bit_xer_into_rt() {
    let mut s = PpuState::new();
    s.xer = 0xDEAD_BEEF_CAFE_BABE;
    exec_no_mem(&PpuInstruction::Mfxer { rt: 4 }, &mut s);
    assert_eq!(s.gpr[4], 0xDEAD_BEEF_CAFE_BABE);
}

#[test]
fn mtxer_writes_rs_into_xer() {
    let mut s = PpuState::new();
    s.gpr[5] = 0x1234_5678_9ABC_DEF0;
    exec_no_mem(&PpuInstruction::Mtxer { rs: 5 }, &mut s);
    assert_eq!(s.xer, 0x1234_5678_9ABC_DEF0);
}

#[test]
fn vrsave_round_trips_through_gpr() {
    let mut s = PpuState::new();
    s.gpr[6] = 0x0000_0000_DEAD_BEEF;
    exec_no_mem(&PpuInstruction::Mtvrsave { rs: 6 }, &mut s);
    assert_eq!(s.vrsave, 0xDEAD_BEEF);
    // Distinct destination GPR so the test catches a buggy
    // mfvrsave that returns the wrong register's contents.
    exec_no_mem(&PpuInstruction::Mfvrsave { rt: 7 }, &mut s);
    assert_eq!(s.gpr[7], 0xDEAD_BEEF);
}

#[test]
fn mtvrsave_truncates_upper_half_of_rs() {
    let mut s = PpuState::new();
    s.gpr[3] = 0xFFFF_FFFF_8000_0001;
    exec_no_mem(&PpuInstruction::Mtvrsave { rs: 3 }, &mut s);
    assert_eq!(s.vrsave, 0x8000_0001);
}

#[test]
fn mfvrsave_zero_extends_into_rt() {
    // Set vrsave_written directly (not via Mtvrsave) so the test
    // bypasses the tripwire and pins read-side widening only.
    let mut s = PpuState::new();
    s.vrsave = 0xCAFE_BABE;
    s.vrsave_written = true;
    s.gpr[8] = 0xFFFF_FFFF_FFFF_FFFF;
    exec_no_mem(&PpuInstruction::Mfvrsave { rt: 8 }, &mut s);
    assert_eq!(s.gpr[8], 0x0000_0000_CAFE_BABE);
}

#[test]
fn vrsave_write_then_read_does_not_trip_tripwire() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x0000_0000_1234_5678;
    exec_no_mem(&PpuInstruction::Mtvrsave { rs: 3 }, &mut s);
    exec_no_mem(&PpuInstruction::Mfvrsave { rt: 4 }, &mut s);
    assert_eq!(s.gpr[4], 0x1234_5678);
    assert_eq!(s.mfvrsave_executed, 1, "witness counts the read");
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "never-written VRSAVE")]
fn mfvrsave_before_any_mtvrsave_trips_tripwire() {
    let mut s = PpuState::new();
    exec_no_mem(&PpuInstruction::Mfvrsave { rt: 5 }, &mut s);
}

#[test]
fn mfvrsave_counter_increments_on_each_read() {
    let mut s = PpuState::new();
    s.gpr[3] = 0x42;
    exec_no_mem(&PpuInstruction::Mtvrsave { rs: 3 }, &mut s);
    exec_no_mem(&PpuInstruction::Mfvrsave { rt: 4 }, &mut s);
    exec_no_mem(&PpuInstruction::Mfvrsave { rt: 5 }, &mut s);
    assert_eq!(s.mfvrsave_executed, 2);
}
