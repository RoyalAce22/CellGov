//! Condition-register move and bit-logic instruction execution.

use super::*;

fn fresh() -> PpuState {
    PpuState::new()
}

fn run(insn: PpuInstruction, state: &mut PpuState) {
    let v = execute(&insn, state);
    assert!(matches!(v, ExecuteVerdict::Continue));
}

#[test]
fn mcrf_copies_cr_field() {
    let mut s = fresh();
    s.set_cr_field(2, 0b1011);
    run(PpuInstruction::Mcrf { crfd: 5, crfs: 2 }, &mut s);
    assert_eq!(s.cr_field(5), 0b1011);
    assert_eq!(s.cr_field(2), 0b1011);
}

#[test]
fn crand_one_and_one_is_one() {
    let mut s = fresh();
    s.set_cr_bit(8, true);
    s.set_cr_bit(9, true);
    run(
        PpuInstruction::Crand {
            bt: 10,
            ba: 8,
            bb: 9,
        },
        &mut s,
    );
    assert!(s.cr_bit(10));
}

#[test]
fn crand_zero_and_one_is_zero() {
    let mut s = fresh();
    s.set_cr_bit(8, false);
    s.set_cr_bit(9, true);
    s.set_cr_bit(10, true);
    run(
        PpuInstruction::Crand {
            bt: 10,
            ba: 8,
            bb: 9,
        },
        &mut s,
    );
    assert!(!s.cr_bit(10));
}

#[test]
fn crandc_anti_polar_b() {
    let mut s = fresh();
    s.set_cr_bit(8, true);
    s.set_cr_bit(9, false);
    run(
        PpuInstruction::Crandc {
            bt: 10,
            ba: 8,
            bb: 9,
        },
        &mut s,
    );
    assert!(s.cr_bit(10));
}

#[test]
fn cror_zero_or_one_is_one() {
    let mut s = fresh();
    s.set_cr_bit(8, false);
    s.set_cr_bit(9, true);
    run(
        PpuInstruction::Cror {
            bt: 10,
            ba: 8,
            bb: 9,
        },
        &mut s,
    );
    assert!(s.cr_bit(10));
}

#[test]
fn crorc_or_with_inverted_b() {
    let mut s = fresh();
    s.set_cr_bit(8, false);
    s.set_cr_bit(9, false);
    run(
        PpuInstruction::Crorc {
            bt: 10,
            ba: 8,
            bb: 9,
        },
        &mut s,
    );
    // !cr[9] = 1; 0 OR 1 = 1.
    assert!(s.cr_bit(10));
}

#[test]
fn crxor_unequal_inputs_yield_one() {
    let mut s = fresh();
    s.set_cr_bit(8, true);
    s.set_cr_bit(9, false);
    run(
        PpuInstruction::Crxor {
            bt: 10,
            ba: 8,
            bb: 9,
        },
        &mut s,
    );
    assert!(s.cr_bit(10));
}

#[test]
fn crxor_equal_inputs_yield_zero() {
    let mut s = fresh();
    s.set_cr_bit(8, true);
    s.set_cr_bit(9, true);
    s.set_cr_bit(10, true);
    run(
        PpuInstruction::Crxor {
            bt: 10,
            ba: 8,
            bb: 9,
        },
        &mut s,
    );
    assert!(!s.cr_bit(10));
}

#[test]
fn crnand_inverts_crand() {
    let mut s = fresh();
    s.set_cr_bit(8, true);
    s.set_cr_bit(9, true);
    run(
        PpuInstruction::Crnand {
            bt: 10,
            ba: 8,
            bb: 9,
        },
        &mut s,
    );
    assert!(!s.cr_bit(10));
}

#[test]
fn crnor_inverts_cror() {
    let mut s = fresh();
    s.set_cr_bit(8, false);
    s.set_cr_bit(9, false);
    run(
        PpuInstruction::Crnor {
            bt: 10,
            ba: 8,
            bb: 9,
        },
        &mut s,
    );
    assert!(s.cr_bit(10));
}

#[test]
fn crnor_self_alias_is_crnot() {
    // The PowerPC `crnot BT, BA` mnemonic decomposes into
    // `crnor BT, BA, BA`, inverting bit BA into BT.
    // [PPC-Book1 p:155 s:B.3] crnot bx,by == crnor bx,by,by.
    let mut s = fresh();
    s.set_cr_bit(29, true);
    run(
        PpuInstruction::Crnor {
            bt: 30,
            ba: 29,
            bb: 29,
        },
        &mut s,
    );
    assert!(!s.cr_bit(30));

    s.set_cr_bit(29, false);
    run(
        PpuInstruction::Crnor {
            bt: 30,
            ba: 29,
            bb: 29,
        },
        &mut s,
    );
    assert!(s.cr_bit(30));
}

#[test]
fn creqv_inverts_crxor() {
    let mut s = fresh();
    s.set_cr_bit(8, true);
    s.set_cr_bit(9, false);
    run(
        PpuInstruction::Creqv {
            bt: 10,
            ba: 8,
            bb: 9,
        },
        &mut s,
    );
    assert!(!s.cr_bit(10));
}

#[test]
fn cr_logical_does_not_touch_pc_or_lr() {
    let mut s = fresh();
    s.pc = 0x1000;
    s.lr = 0xDEADBEEF;
    run(
        PpuInstruction::Crand {
            bt: 0,
            ba: 1,
            bb: 2,
        },
        &mut s,
    );
    assert_eq!(s.pc, 0x1000);
    assert_eq!(s.lr, 0xDEADBEEF);
}

/// Build a non-trivial 32-bit CR pattern so other-bit preservation
/// tests can isolate which single bit the handler touched.
fn alternating_cr_pattern() -> u32 {
    0xA5A5_A5A5
}

fn assert_only_bit_changed(before: u32, after: u32, bt: u8) {
    let mask = 1u32 << (31 - bt);
    assert_eq!(
        before & !mask,
        after & !mask,
        "bits other than BT={} must be preserved",
        bt
    );
}

#[test]
fn crand_preserves_other_cr_bits() {
    let mut s = fresh();
    s.cr = alternating_cr_pattern();
    s.set_cr_bit(10, true);
    s.set_cr_bit(11, true);
    let before = s.cr;
    run(
        PpuInstruction::Crand {
            bt: 5,
            ba: 10,
            bb: 11,
        },
        &mut s,
    );
    assert!(s.cr_bit(5));
    assert_only_bit_changed(before, s.cr, 5);
}

#[test]
fn cror_preserves_other_cr_bits() {
    let mut s = fresh();
    s.cr = alternating_cr_pattern();
    s.set_cr_bit(10, false);
    s.set_cr_bit(11, true);
    let before = s.cr;
    run(
        PpuInstruction::Cror {
            bt: 5,
            ba: 10,
            bb: 11,
        },
        &mut s,
    );
    assert!(s.cr_bit(5));
    assert_only_bit_changed(before, s.cr, 5);
}

#[test]
fn crxor_preserves_other_cr_bits() {
    let mut s = fresh();
    s.cr = alternating_cr_pattern();
    s.set_cr_bit(10, true);
    s.set_cr_bit(11, false);
    let before = s.cr;
    run(
        PpuInstruction::Crxor {
            bt: 5,
            ba: 10,
            bb: 11,
        },
        &mut s,
    );
    assert!(s.cr_bit(5));
    assert_only_bit_changed(before, s.cr, 5);
}

#[test]
fn crnand_preserves_other_cr_bits() {
    let mut s = fresh();
    s.cr = alternating_cr_pattern();
    s.set_cr_bit(10, true);
    s.set_cr_bit(11, true);
    let before = s.cr;
    run(
        PpuInstruction::Crnand {
            bt: 5,
            ba: 10,
            bb: 11,
        },
        &mut s,
    );
    assert!(!s.cr_bit(5));
    assert_only_bit_changed(before, s.cr, 5);
}

#[test]
fn crnor_preserves_other_cr_bits() {
    let mut s = fresh();
    s.cr = alternating_cr_pattern();
    s.set_cr_bit(10, false);
    s.set_cr_bit(11, false);
    let before = s.cr;
    run(
        PpuInstruction::Crnor {
            bt: 5,
            ba: 10,
            bb: 11,
        },
        &mut s,
    );
    assert!(s.cr_bit(5));
    assert_only_bit_changed(before, s.cr, 5);
}

#[test]
fn creqv_preserves_other_cr_bits() {
    let mut s = fresh();
    s.cr = alternating_cr_pattern();
    s.set_cr_bit(10, true);
    s.set_cr_bit(11, true);
    let before = s.cr;
    run(
        PpuInstruction::Creqv {
            bt: 5,
            ba: 10,
            bb: 11,
        },
        &mut s,
    );
    assert!(s.cr_bit(5));
    assert_only_bit_changed(before, s.cr, 5);
}

#[test]
fn crandc_preserves_other_cr_bits() {
    let mut s = fresh();
    s.cr = alternating_cr_pattern();
    s.set_cr_bit(10, true);
    s.set_cr_bit(11, false);
    let before = s.cr;
    run(
        PpuInstruction::Crandc {
            bt: 5,
            ba: 10,
            bb: 11,
        },
        &mut s,
    );
    assert!(s.cr_bit(5));
    assert_only_bit_changed(before, s.cr, 5);
}

#[test]
fn crorc_preserves_other_cr_bits() {
    let mut s = fresh();
    s.cr = alternating_cr_pattern();
    s.set_cr_bit(10, false);
    s.set_cr_bit(11, false);
    let before = s.cr;
    run(
        PpuInstruction::Crorc {
            bt: 5,
            ba: 10,
            bb: 11,
        },
        &mut s,
    );
    assert!(s.cr_bit(5));
    assert_only_bit_changed(before, s.cr, 5);
}

#[test]
fn mcrf_does_not_touch_other_fields() {
    let mut s = fresh();
    s.set_cr_field(0, 0b0001);
    s.set_cr_field(1, 0b0010);
    s.set_cr_field(2, 0b0100);
    s.set_cr_field(3, 0b1000);
    run(PpuInstruction::Mcrf { crfd: 1, crfs: 3 }, &mut s);
    assert_eq!(s.cr_field(0), 0b0001);
    assert_eq!(s.cr_field(1), 0b1000);
    assert_eq!(s.cr_field(2), 0b0100);
    assert_eq!(s.cr_field(3), 0b1000);
}
