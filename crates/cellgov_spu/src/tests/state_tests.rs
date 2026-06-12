//! SpuState register-slot accessors, local-store fetch bounds, and the reservation field.

use super::*;

#[test]
fn new_state_is_zeroed() {
    let s = SpuState::new();
    assert_eq!(s.pc, 0);
    assert_eq!(s.ls.len(), SPU_LS_SIZE);
    assert!(s.ls.iter().all(|&b| b == 0));
    assert!(s.regs.iter().all(|r| r.iter().all(|&b| b == 0)));
    assert!(s.reservation.is_none());
}

#[test]
fn reservation_field_is_settable_and_clearable() {
    let mut s = SpuState::new();
    s.reservation = Some(ReservedLine::containing(0x4000));
    assert_eq!(s.reservation.map(|l| l.addr()), Some(0x4000));
    s.reservation = None;
    assert!(s.reservation.is_none());
}

#[test]
fn reg_word_splat_fills_all_slots() {
    let mut s = SpuState::new();
    s.set_reg_word_splat(5, 0xDEADBEEF);
    assert_eq!(s.reg_word(5), 0xDEADBEEF);
    assert_eq!(s.reg_word_slot(5, 1), 0xDEADBEEF);
    assert_eq!(s.reg_word_slot(5, 2), 0xDEADBEEF);
    assert_eq!(s.reg_word_slot(5, 3), 0xDEADBEEF);
}

#[test]
fn reg_word_slot_independent() {
    let mut s = SpuState::new();
    s.set_reg_word_slot(3, 0, 0xAAAAAAAA);
    s.set_reg_word_slot(3, 2, 0xBBBBBBBB);
    assert_eq!(s.reg_word_slot(3, 0), 0xAAAAAAAA);
    assert_eq!(s.reg_word_slot(3, 1), 0);
    assert_eq!(s.reg_word_slot(3, 2), 0xBBBBBBBB);
    assert_eq!(s.reg_word_slot(3, 3), 0);
}

#[test]
fn fetch_from_ls() {
    let mut s = SpuState::new();
    s.ls[0] = 0x12;
    s.ls[1] = 0x34;
    s.ls[2] = 0x56;
    s.ls[3] = 0x78;
    assert_eq!(s.fetch(), Some(0x12345678));
}

#[test]
fn fetch_out_of_range() {
    let s = SpuState::new();
    let mut s2 = s;
    s2.pc = SPU_LS_SIZE as u32;
    assert_eq!(s2.fetch(), None);
}
