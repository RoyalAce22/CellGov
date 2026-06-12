//! Syscall namespace classification tests -- LV2, unresolved-import, hypercall, and unknown routing.

use super::*;

#[test]
fn classify_lv2_syscall() {
    assert_eq!(classify(0, 22), SyscallClassification::Lv2 { number: 22 },);
}

#[test]
fn classify_unresolved_import_decodes_index() {
    assert_eq!(
        classify(0, 0x10005),
        SyscallClassification::UnresolvedImport { index: 5 },
    );
}

#[test]
fn classify_above_all_namespaces_falls_to_unknown() {
    assert_eq!(
        classify(0, 0x80000),
        SyscallClassification::Unknown { r11: 0x80000 },
    );
}

#[test]
fn classify_hypercall_routes_distinctly_for_lev_1() {
    assert_eq!(
        classify(1, 22),
        SyscallClassification::Hypercall { lev: 1, r11: 22 },
    );
}

#[test]
fn nonzero_lev_always_routes_to_hypercall() {
    for lev in 1..=63u8 {
        assert!(matches!(
            classify(lev, 22),
            SyscallClassification::Hypercall { .. }
        ));
        assert!(matches!(
            classify(lev, 0x80000),
            SyscallClassification::Hypercall { .. }
        ));
    }
}
