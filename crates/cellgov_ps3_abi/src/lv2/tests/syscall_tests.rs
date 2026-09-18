//! LV2 syscall number uniqueness and syscall-array consistency.

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
    for entry in ALL_LV2_UNSUPPORTED_ROUTED_SYSCALLS {
        assert!(
            !typed.contains(&entry.number),
            "{} ({}) collides with a typed-arm Lv2Request number; \
             either remove it from ALL_LV2_NUMBERS (if it should route via Unsupported) \
             or add a typed Lv2Request variant (and remove the Unsupported arm)",
            entry.constant,
            entry.number,
        );
    }
    // Also enforce intra-list uniqueness within the unsupported set.
    let mut seen: BTreeSet<u64> = BTreeSet::new();
    for entry in ALL_LV2_UNSUPPORTED_ROUTED_SYSCALLS {
        assert!(
            seen.insert(entry.number),
            "{} duplicates another unsupported-routed syscall number ({})",
            entry.constant,
            entry.number,
        );
    }
    assert_eq!(
        ALL_LV2_UNSUPPORTED_ROUTED_SYSCALLS.len(),
        ALL_LV2_UNSUPPORTED_ROUTED_NUMBERS.len(),
    );
}

#[test]
fn the_syscall_arrays_carry_the_number_arrays_in_order() {
    let typed: Vec<u64> = ALL_LV2_SYSCALLS.iter().map(|e| e.number).collect();
    assert_eq!(typed, ALL_LV2_NUMBERS);
    let routed: Vec<u64> = ALL_LV2_UNSUPPORTED_ROUTED_SYSCALLS
        .iter()
        .map(|e| e.number)
        .collect();
    assert_eq!(routed, ALL_LV2_UNSUPPORTED_ROUTED_NUMBERS);
}

#[test]
fn every_name_is_an_lv2_identifier_and_only_the_unnamed_lack_one() {
    let mut unnamed = Vec::new();
    for entry in ALL_LV2_SYSCALLS
        .iter()
        .chain(ALL_LV2_UNSUPPORTED_ROUTED_SYSCALLS)
    {
        match entry.name {
            Some(name) => {
                let body = name.strip_prefix('_').unwrap_or(name);
                assert!(
                    body.starts_with("sys_")
                        && body
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                    "{} names {name:?}, which is not an LV2 identifier",
                    entry.constant
                );
            }
            None => unnamed.push(entry.constant),
        }
    }
    assert_eq!(unnamed, ["UNS_FUNC_462"]);
}
