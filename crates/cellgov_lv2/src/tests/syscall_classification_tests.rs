//! Syscall namespace classification for LV2, unresolved imports, and hypercalls.

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
fn classify_hypercall_routes_distinctly_for_lev_1() {
    assert_eq!(
        classify(1, 22),
        SyscallClassification::Hypercall { lev: 1, r11: 22 },
    );
}

/// The decoder passes no nonzero LEV other than 1.
#[test]
fn lev_one_routes_to_hypercall_whatever_r11_names() {
    for r11 in [22, 0x10005, 0x80000] {
        assert!(matches!(
            classify(1, r11),
            SyscallClassification::Hypercall { .. }
        ));
    }
}
