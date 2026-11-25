//! The family rule reads the name that `hypercall_name` gives, so an
//! alias decides the family of its call.

use super::*;

/// Each family and work item, with the aliased numbers that their
/// aliases put in it, ascending.
const PLACEMENTS: &[(&str, &str, &[u64])] = &[
    ("console", "dispatch", &[105, 106, 107, 108, 109, 231]),
    ("device", "boot", &[230]),
    (
        "logical_spe",
        "logical_spe",
        &[62, 89, 137, 138, 167, 168, 200, 201, 209],
    ),
    ("other", "boot", &[99, 195, 196]),
];

/// The numbers that have an alias in the call table or in
/// `UNLISTED_ALIASES`, ascending.
fn aliased_numbers() -> Vec<u64> {
    let listed = call::LV1_CALLS
        .iter()
        .filter(|row| row.alias.is_some())
        .map(|row| u64::from(row.number));
    let unlisted = call::UNLISTED_ALIASES
        .iter()
        .map(|(number, _)| u64::from(*number));
    let mut numbers: Vec<u64> = listed.chain(unlisted).collect();
    numbers.sort_unstable();
    numbers
}

#[test]
fn each_aliased_number_has_the_family_and_work_item_of_its_alias() {
    let mut placed: Vec<u64> = PLACEMENTS
        .iter()
        .flat_map(|(_, _, numbers)| numbers.iter().copied())
        .collect();
    placed.sort_unstable();
    assert_eq!(placed.len(), 19);
    assert_eq!(aliased_numbers(), placed, "a new alias needs a placement");
    for &(family, leaf, numbers) in PLACEMENTS {
        for &number in numbers {
            assert_eq!(hypercall_family(number), (family, leaf), "{number}");
        }
    }
}

#[test]
fn only_a_header_placeholder_with_no_alias_is_undocumented() {
    let undocumented: Vec<u64> = (0..=255)
        .filter(|number| hypercall_family(*number).0 == "undocumented")
        .collect();
    assert_eq!(undocumented, [114, 115]);
    for number in undocumented {
        assert_eq!(hypercall_family(number).1, "boot", "{number}");
    }
}

#[test]
fn a_number_is_unnamed_exactly_when_no_source_names_it() {
    let unnamed: Vec<u64> = (0..=255)
        .filter(|number| hypercall_family(*number).0 == "unnamed")
        .collect();
    let nameless: Vec<u64> = (0..=255)
        .filter(|number| hypercall_name(*number).is_none())
        .collect();
    assert!(nameless.contains(&5), "5 has no name");
    // 256 numbers, less the 137 call table rows and the 6 unlisted aliases.
    assert_eq!(nameless.len(), 113);
    assert_eq!(unnamed, nameless);
}
