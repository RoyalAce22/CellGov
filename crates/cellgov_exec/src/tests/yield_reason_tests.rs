//! YieldReason discriminant locking, variant distinctness, and copy semantics.

use super::*;

#[test]
fn discriminants_are_locked() {
    assert_eq!(YieldReason::BudgetExhausted as u8, 0);
    assert_eq!(YieldReason::MailboxAccess as u8, 1);
    assert_eq!(YieldReason::DmaSubmitted as u8, 2);
    assert_eq!(YieldReason::DmaWait as u8, 3);
    assert_eq!(YieldReason::WaitingSync as u8, 4);
    assert_eq!(YieldReason::Syscall as u8, 5);
    assert_eq!(YieldReason::InterruptBoundary as u8, 6);
    assert_eq!(YieldReason::Fault as u8, 7);
    assert_eq!(YieldReason::Finished as u8, 8);
}

#[test]
fn variants_are_distinct() {
    use strum::VariantArray;
    let unique: std::collections::BTreeSet<u8> =
        YieldReason::VARIANTS.iter().map(|y| *y as u8).collect();
    assert_eq!(unique.len(), YieldReason::VARIANTS.len());
}

#[test]
fn equality_is_reflexive_and_distinguishing() {
    assert_eq!(YieldReason::Fault, YieldReason::Fault);
    assert_ne!(YieldReason::Fault, YieldReason::Finished);
    assert_ne!(YieldReason::DmaSubmitted, YieldReason::DmaWait);
}

#[test]
fn copy_semantics_hold() {
    let r = YieldReason::WaitingSync;
    let s = r;
    assert_eq!(r, s);
}
