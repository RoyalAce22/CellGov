//! Boot flag-conflict checks.

use super::{check_strict_reserved_vs_rsx_mirror, StrictReservedConflict};

#[test]
fn rejects_strict_reserved_with_rsx_mirror() {
    let err = check_strict_reserved_vs_rsx_mirror(true, true).unwrap_err();
    assert_eq!(err, StrictReservedConflict::RsxMirror);
    let msg = err.to_string();
    assert!(msg.contains("--strict-reserved"));
    assert!(msg.contains("rsx_mirror"));
}

#[test]
fn accepts_strict_reserved_alone() {
    assert!(check_strict_reserved_vs_rsx_mirror(true, false).is_ok());
}

#[test]
fn accepts_rsx_mirror_alone() {
    assert!(check_strict_reserved_vs_rsx_mirror(false, true).is_ok());
}

#[test]
fn accepts_neither() {
    assert!(check_strict_reserved_vs_rsx_mirror(false, false).is_ok());
}
