//! The shared OE / Rc tail: which XER and CR0 bits each flag touches.

use super::*;

fn xer_ov(s: &PpuState) -> bool {
    (s.xer() >> 30) & 1 != 0
}

#[test]
fn retire_writes_the_target_register() {
    let mut s = PpuState::new();
    let verdict = retire(&mut s, 7, 0x1234, None, false);
    assert!(matches!(verdict, ExecuteVerdict::Continue));
    assert_eq!(s.gpr[7], 0x1234);
}

#[test]
fn retire_without_oe_leaves_ov_and_so_untouched() {
    let mut s = PpuState::new();
    s.set_xer_ov(true);
    retire(&mut s, 3, 1, None, false);
    assert!(xer_ov(&s));
    assert!(s.xer_so());
}

// [PPC-Book1 p:32 s:3.2.2] OE=1 with no overflow writes OV to 0; SO stays set until mtspr or mcrxr clears it.
#[test]
fn retire_with_oe_false_clears_ov_but_keeps_so_sticky() {
    let mut s = PpuState::new();
    s.set_xer_ov(true);
    retire(&mut s, 3, 1, Some(false), false);
    assert!(!xer_ov(&s));
    assert!(s.xer_so());
}

#[test]
fn retire_without_rc_leaves_cr0_untouched() {
    let mut s = PpuState::new();
    s.set_cr_field(0, 0b0100);
    retire(&mut s, 3, u64::MAX, None, false);
    assert_eq!(s.cr_field(0), 0b0100);
}

// [PPC-Book1 p:18 s:2.3.1] CR0[0:2] is LT / GT / EQ from a signed compare of the 64-bit result with zero; CR0[3] copies XER[SO].
#[test]
fn retire_with_rc_records_lt_gt_eq_from_the_signed_result() {
    let mut s = PpuState::new();
    retire(&mut s, 3, u64::MAX, None, true);
    assert_eq!(s.cr_field(0), 0b1000);
    retire(&mut s, 3, 5, None, true);
    assert_eq!(s.cr_field(0), 0b0100);
    retire(&mut s, 3, 0, None, true);
    assert_eq!(s.cr_field(0), 0b0010);
}

#[test]
fn retire_sets_ov_before_cr0_so_the_new_so_bit_reaches_cr0() {
    let mut s = PpuState::new();
    assert!(!s.xer_so());
    retire(&mut s, 3, 5, Some(true), true);
    assert_eq!(s.cr_field(0), 0b0101);
}
