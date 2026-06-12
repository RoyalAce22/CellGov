//! LV2 syscall number uniqueness and named-audit-array consistency.

use super::*;
use std::collections::BTreeSet;

#[test]
fn all_lv2_numbers_are_unique() {
    let set: BTreeSet<u64> = ALL_LV2_NUMBERS.iter().copied().collect();
    assert_eq!(
        set.len(),
        ALL_LV2_NUMBERS.len(),
        "ALL_LV2_NUMBERS contains a duplicate; len()={} unique={}",
        ALL_LV2_NUMBERS.len(),
        set.len(),
    );
}

#[test]
fn unsupported_routed_syscall_numbers_do_not_collide_with_typed_arms() {
    let typed: BTreeSet<u64> = ALL_LV2_NUMBERS.iter().copied().collect();
    for &(name, n) in ALL_LV2_UNSUPPORTED_ROUTED_NAMED {
        assert!(
            !typed.contains(&n),
            "{name} ({n}) collides with a typed-arm Lv2Request number; \
             either remove it from ALL_LV2_NUMBERS (if it should route via Unsupported) \
             or add a typed Lv2Request variant (and remove the Unsupported arm)",
        );
    }
    // Also enforce intra-list uniqueness within the unsupported set.
    let mut seen: BTreeSet<u64> = BTreeSet::new();
    for &(name, n) in ALL_LV2_UNSUPPORTED_ROUTED_NAMED {
        assert!(
            seen.insert(n),
            "{name} duplicates another unsupported-routed syscall number ({n})",
        );
    }
    assert_eq!(
        ALL_LV2_UNSUPPORTED_ROUTED_NAMED.len(),
        ALL_LV2_UNSUPPORTED_ROUTED_NUMBERS.len(),
    );
}

/// Named-array values match `ALL_LV2_NUMBERS` exactly.
#[test]
fn audit_array_matches_all_lv2_numbers() {
    let audit_set: BTreeSet<u64> = ALL_LV2_NAMED.iter().map(|&(_, v)| v).collect();
    let array_set: BTreeSet<u64> = ALL_LV2_NUMBERS.iter().copied().collect();
    let missing_from_array: Vec<&(&str, u64)> = ALL_LV2_NAMED
        .iter()
        .filter(|(_, v)| !array_set.contains(v))
        .collect();
    let missing_from_audit: Vec<u64> = ALL_LV2_NUMBERS
        .iter()
        .copied()
        .filter(|v| !audit_set.contains(v))
        .collect();
    assert!(
        missing_from_array.is_empty(),
        "constants in ALL_LV2_NAMED missing from ALL_LV2_NUMBERS: {missing_from_array:?}",
    );
    assert!(
        missing_from_audit.is_empty(),
        "values in ALL_LV2_NUMBERS missing from ALL_LV2_NAMED: {missing_from_audit:?}",
    );
    assert_eq!(ALL_LV2_NAMED.len(), ALL_LV2_NUMBERS.len());
}
