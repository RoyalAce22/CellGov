use strum::VariantArray;

use super::{unsupported_arm_fidelity, ArmFidelity, ROUTED_UNSUPPORTED_ARMS};
use crate::request::Lv2RequestKind;

#[test]
fn routing_variants_and_only_routing_variants_are_untagged() {
    let untagged: Vec<&str> = Lv2RequestKind::VARIANTS
        .iter()
        .filter(|k| k.fidelity().is_none())
        .map(|k| <&'static str>::from(*k))
        .collect();
    assert_eq!(
        untagged,
        ["Hypercall", "Unsupported", "UnresolvedImport", "Malformed"],
        "fidelity() returns None exactly for the routing-layer variants"
    );
}

#[test]
fn routed_unsupported_numbers_are_unique() {
    let mut seen = std::collections::BTreeSet::new();
    for (n, name, _) in ROUTED_UNSUPPORTED_ARMS {
        assert!(seen.insert(*n), "syscall {n} ({name}) listed twice");
    }
}

#[test]
fn unlisted_number_reads_null_backend() {
    assert_eq!(unsupported_arm_fidelity(9999), ArmFidelity::NullBackend);
    assert_eq!(
        unsupported_arm_fidelity(cellgov_ps3_abi::syscall::TTY_READ),
        ArmFidelity::Modeled
    );
}
