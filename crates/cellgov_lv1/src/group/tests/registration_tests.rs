use std::collections::BTreeMap;

use cellgov_ps3_abi::lv1::call;
use cellgov_ps3_abi::lv1::table::{table_entry, TableEntry, LIVE_TABLE_FIRMWARE};

use super::*;

/// One registration as the guard reads it: its group, number and name.
type Entry = (&'static str, u64, &'static str);

/// A rule a set of registrations breaks.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RegistrationFault {
    /// More than one registration declares the number; the groups that
    /// declare it, in declaration order.
    Duplicate {
        number: u64,
        groups: Vec<&'static str>,
    },
    /// lv1's live 4.93 table holds no handler for the number.
    NotInLiveTable { number: u64 },
    /// The call table documents a name for the number, and the
    /// registration declares another one.
    Name {
        number: u64,
        declared: &'static str,
        table: &'static str,
    },
}

/// The name the call table documents for `number`: `None` when the
/// table does not list it or names it only `undocumented_function_N`.
fn documented_name(number: u64) -> Option<&'static str> {
    call::lookup(number)
        .map(|row| row.name)
        .filter(|name| !name.starts_with("undocumented_function_"))
}

/// The rules `entries` break, ascending by number, duplicates first.
fn registration_faults(entries: &[Entry]) -> Vec<RegistrationFault> {
    let mut by_number: BTreeMap<u64, Vec<&'static str>> = BTreeMap::new();
    for &(group, number, _) in entries {
        by_number.entry(number).or_default().push(group);
    }
    let mut faults = Vec::new();
    for (&number, groups) in &by_number {
        if groups.len() > 1 {
            faults.push(RegistrationFault::Duplicate {
                number,
                groups: groups.clone(),
            });
        }
    }
    for &number in by_number.keys() {
        if !matches!(
            table_entry(LIVE_TABLE_FIRMWARE, number),
            Some(TableEntry::Handler(_))
        ) {
            faults.push(RegistrationFault::NotInLiveTable { number });
        }
    }
    for &(_, number, declared) in entries {
        if let Some(table) = documented_name(number).filter(|table| *table != declared) {
            faults.push(RegistrationFault::Name {
                number,
                declared,
                table,
            });
        }
    }
    faults
}

#[test]
fn the_registration_guard_refuses_each_broken_rule() {
    let fourteen = call::lookup(14).map_or("hypercall_14", |row| row.name);
    let cases: &[(&str, &[Entry], &[RegistrationFault])] = &[
        (
            "one registration of a number in the live table",
            &[("memory", 0, "allocate_memory")],
            &[],
        ),
        (
            "a number two groups register",
            &[
                ("memory", 0, "allocate_memory"),
                ("interrupt", 0, "allocate_memory"),
            ],
            &[RegistrationFault::Duplicate {
                number: 0,
                groups: vec!["memory", "interrupt"],
            }],
        ),
        (
            "a number one group registers twice",
            &[
                ("memory", 0, "allocate_memory"),
                ("memory", 0, "allocate_memory"),
            ],
            &[RegistrationFault::Duplicate {
                number: 0,
                groups: vec!["memory", "memory"],
            }],
        ),
        (
            "a number whose live 4.93 entry is invalid",
            &[("memory", 14, fourteen)],
            &[RegistrationFault::NotInLiveTable { number: 14 }],
        ),
        (
            "a number past the table",
            &[("memory", 256, "past_the_table")],
            &[RegistrationFault::NotInLiveTable { number: 256 }],
        ),
        (
            "a name that differs from the documented one",
            &[("memory", 0, "allocate")],
            &[RegistrationFault::Name {
                number: 0,
                declared: "allocate",
                table: "allocate_memory",
            }],
        ),
        (
            "CellGov's name for a number the table calls undocumented",
            &[("partition", 105, "comm_dev_logical_console_construct")],
            &[],
        ),
        (
            "any name for a number the table does not list",
            &[("partition", 5, "unnamed_5")],
            &[],
        ),
    ];
    assert_eq!(cases.len(), 8);
    for (what, entries, expected) in cases {
        assert_eq!(registration_faults(entries), *expected, "{what}");
    }
}

#[test]
fn every_registration_declares_a_distinct_live_number_under_its_documented_name() {
    let entries: Vec<Entry> = GROUPS
        .iter()
        .flat_map(|group| {
            group
                .registrations()
                .map(|hypercall| (group.name, hypercall.number, hypercall.name))
        })
        .collect();
    assert_eq!(entries.len(), registrations().count());
    assert!(!entries.is_empty(), "no group registers a hypercall");
    assert_eq!(registration_faults(&entries), []);
}

#[test]
fn each_registration_is_the_one_its_number_finds() {
    let count = registrations().count();
    assert!(count > 0, "no group registers a hypercall");
    for hypercall in registrations() {
        let found = registration(hypercall.number).map(|h| h.name);
        assert_eq!(found, Some(hypercall.name), "{}", hypercall.number);
    }
    assert_eq!(registration(u64::MAX).map(|h| h.number), None);
}
