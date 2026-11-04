//! A whole-bank GPR, FPR or VR swap or move between two states keeps
//! both state hashes current.

use super::*;

fn state_with(seed: u64) -> PpuState {
    let mut s = PpuState::new();
    for k in 0..GPR_COUNT {
        s.set_gpr(k, seed.wrapping_mul(k as u64 + 1));
    }
    for k in 0..FPR_COUNT {
        s.set_fpr(k, seed.wrapping_mul(k as u64 + 7));
    }
    for k in 0..VR_COUNT {
        s.set_vr(k, u128::from(seed) << 64 | (k as u128 + 1));
    }
    s.set_lr(seed);
    s
}

fn assert_both_current(a: &PpuState, b: &PpuState, what: &str) {
    assert!(a.hash_is_current(), "a after {what}");
    assert!(b.hash_is_current(), "b after {what}");
    assert_eq!(
        a.state_hash(),
        a.state_hash_from_scratch(),
        "a after {what}"
    );
    assert_eq!(
        b.state_hash(),
        b.state_hash_from_scratch(),
        "b after {what}"
    );
}

#[test]
fn swapping_the_gpr_banks_keeps_both_hashes_current() {
    let mut a = state_with(0x1111);
    let mut b = state_with(0x2222);
    std::mem::swap(&mut a.gpr, &mut b.gpr);
    assert_eq!(a.gpr[3], 0x2222 * 4);
    assert_both_current(&a, &b, "gpr swap");
}

#[test]
fn swapping_the_fpr_banks_keeps_both_hashes_current() {
    let mut a = state_with(0x1111);
    let mut b = state_with(0x2222);
    std::mem::swap(&mut a.fpr, &mut b.fpr);
    assert_eq!(a.fpr[3], 0x2222 * 10);
    assert_both_current(&a, &b, "fpr swap");
}

#[test]
fn swapping_the_vr_banks_keeps_both_hashes_current() {
    let mut a = state_with(0x1111);
    let mut b = state_with(0x2222);
    std::mem::swap(&mut a.vr, &mut b.vr);
    assert_eq!(a.vr[3], 0x2222u128 << 64 | 4);
    assert_both_current(&a, &b, "vr swap");
}

#[test]
fn moving_a_gpr_bank_out_of_an_owned_state_keeps_the_hash_current() {
    let mut a = state_with(0x3333);
    let b = state_with(0x4444);
    a.gpr = b.gpr;
    assert!(a.hash_is_current(), "a after move");
    assert_eq!(a.gpr[5], 0x4444 * 6);
    assert_eq!(a.state_hash(), a.state_hash_from_scratch());
}

#[test]
fn moving_fpr_and_vr_banks_out_of_an_owned_state_keeps_the_hash_current() {
    let mut a = state_with(0x3333);
    let b = state_with(0x4444);
    a.fpr = b.fpr;
    a.vr = b.vr;
    assert!(a.hash_is_current(), "a after move");
    assert_eq!(a.fpr[5], 0x4444 * 12);
    assert_eq!(a.vr[5], 0x4444u128 << 64 | 6);
    assert_eq!(a.state_hash(), a.state_hash_from_scratch());
}
