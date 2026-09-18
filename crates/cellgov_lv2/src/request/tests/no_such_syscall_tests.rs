//! Out-of-table syscall classification through both public classifiers.

use super::*;
use crate::syscall_classification::{self, SyscallClassification};

#[test]
fn request_classifier_preserves_out_of_table_numbers_and_arguments() {
    let args = [0xAA, 0xBB, 0xCC, 0, 0, 0, 0, 0];
    for number in [syscall::SYSCALL_TABLE_SLOTS, 0xffff, 0x80000] {
        assert_eq!(
            classify(number, &args),
            Lv2Request::NoSuchSyscall { number, args },
        );
    }
}

#[test]
fn dispatch_hint_classifier_separates_the_table_and_private_namespace() {
    for r11 in [syscall::SYSCALL_TABLE_SLOTS, 0xffff, 0x80000] {
        assert_eq!(
            syscall_classification::classify(0, r11),
            SyscallClassification::NoSuchSyscall { r11 },
        );
    }
    assert_eq!(
        syscall_classification::classify(0, syscall::UNRESOLVED_IMPORT),
        SyscallClassification::UnresolvedImport { index: 0 },
    );
}
