//! Every SPU fault class has a reader's name.

use super::{describe_guest_fault, EVERY_FAULT_CLASS};

#[test]
fn every_raised_class_is_named_and_no_other_is() {
    for class in EVERY_FAULT_CLASS {
        assert!(
            describe_guest_fault(class).is_some(),
            "0x{class:08x} has no name"
        );
    }
    assert_eq!(describe_guest_fault(0x000F_0000), None);
    assert_eq!(describe_guest_fault(0x0105_0000), None);
}
