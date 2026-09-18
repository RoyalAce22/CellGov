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
    for (n, arm, _) in ROUTED_UNSUPPORTED_ARMS {
        assert!(seen.insert(*n), "syscall {n} ({arm}) listed twice");
    }
}

/// Only the abi crate's routed list gives a routed number its name. An
/// arm on a number outside that list would render no name row.
#[test]
fn routed_arms_are_exactly_the_abi_routed_numbers() {
    let mut arms: Vec<u64> = ROUTED_UNSUPPORTED_ARMS.iter().map(|(n, ..)| *n).collect();
    arms.sort_unstable();
    let mut abi = cellgov_ps3_abi::lv2::syscall::ALL_LV2_UNSUPPORTED_ROUTED_NUMBERS.to_vec();
    abi.sort_unstable();
    assert_eq!(arms, abi);
}

#[test]
fn routed_arm_identifiers_are_unique_and_name_no_typed_variant() {
    let variants: Vec<&str> = Lv2RequestKind::VARIANTS
        .iter()
        .map(|k| <&'static str>::from(*k))
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    for (n, arm, _) in ROUTED_UNSUPPORTED_ARMS {
        assert!(seen.insert(*arm), "arm {arm} ({n}) listed twice");
        assert!(
            !variants.contains(arm),
            "arm {arm} ({n}) collides with a typed variant"
        );
        assert!(
            arm.bytes().all(|b| b.is_ascii_alphanumeric()),
            "arm {arm} ({n}) is not a bare identifier"
        );
    }
}

#[test]
fn every_tag_is_listed_once_with_a_distinct_label() {
    let labels: Vec<&str> = ArmFidelity::ALL.iter().map(|f| f.label()).collect();
    assert_eq!(
        labels,
        ["modeled", "partial-state", "abi-only", "null-backend"]
    );
}

#[test]
fn unlisted_number_reads_null_backend() {
    assert_eq!(unsupported_arm_fidelity(9999), ArmFidelity::NullBackend);
    assert_eq!(
        unsupported_arm_fidelity(cellgov_ps3_abi::lv2::syscall::TTY_READ),
        ArmFidelity::Modeled
    );
}
