//! A whole-bank GPR swap or move between two states keeps both state
//! hashes current.

use super::*;

fn state_with(seed: u64) -> PpuState {
    let mut s = PpuState::new();
    for k in 0..GPR_COUNT {
        s.set_gpr(k, seed.wrapping_mul(k as u64 + 1));
    }
    s.set_lr(seed);
    s
}

#[test]
fn swapping_the_gpr_banks_keeps_both_hashes_current() {
    let mut a = state_with(0x1111);
    let mut b = state_with(0x2222);
    std::mem::swap(&mut a.gpr, &mut b.gpr);
    assert!(a.hash_is_current(), "a after swap");
    assert!(b.hash_is_current(), "b after swap");
    assert_eq!(a.gpr[3], 0x2222 * 4);
    assert_eq!(a.state_hash(), a.state_hash_from_scratch());
    assert_eq!(b.state_hash(), b.state_hash_from_scratch());
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
