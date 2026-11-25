//! The source of each hypercall name, and the `name_source` column.

use super::*;
use cellgov_lv2_archive::parse;

use NameSource::{CellGov, Header, Psdevwiki};

/// Every number that has an alias, ascending, with its alias and source.
/// psdevwiki gives its names in [PS3DEVWIKI-HV-SYSCALLS s:HV_Syscalls].
/// The call table cites the lv1 4.93 handler of each CellGov name.
const ALIASES: &[(u64, &str, NameSource)] = &[
    (62, "set_slb_for_logical_spu", Psdevwiki),
    (89, "invalidate_spe_tlb", CellGov),
    (99, "authenticate_program_segment", Psdevwiki),
    (105, "comm_dev_logical_console_construct", Psdevwiki),
    (106, "comm_dev_logical_console_destruct", Psdevwiki),
    (107, "comm_dev_logical_console_send", Psdevwiki),
    (108, "comm_dev_logical_console_receive", Psdevwiki),
    (109, "comm_dev_logical_console_flush", Psdevwiki),
    (137, "disable_spe_execution_locked", Psdevwiki),
    (138, "logical_spu_set_run_timeout", CellGov),
    (167, "logical_spu_get_mfc_control", CellGov),
    (168, "logical_spu_set_mfc_control", Psdevwiki),
    (195, "net_driver_wlan_eurus_send_command", CellGov),
    (196, "net_driver_wlan_eurus_proxy", Psdevwiki),
    (200, "logical_spu_start_isolated", Psdevwiki),
    (201, "logical_spu_unload_isolated", CellGov),
    (209, "logical_spu_load_isolated", CellGov),
    (230, "gpu_get_info", CellGov),
    (231, "comm_dev_logical_console_try_flush", CellGov),
];

/// The numbers the call table gives an alias, ascending.
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
fn each_aliased_number_takes_its_name_from_its_pinned_source() {
    assert_eq!(ALIASES.len(), 19);
    let pinned: Vec<u64> = ALIASES.iter().map(|(number, _, _)| *number).collect();
    assert_eq!(aliased_numbers(), pinned);
    for &(number, text, source) in ALIASES {
        assert_eq!(
            hypercall_name(number),
            Some(HypercallName { text, source }),
            "{number}"
        );
    }
}

#[test]
fn a_number_with_no_alias_keeps_the_header_name_or_has_none() {
    let cases: &[(u64, Option<HypercallName>)] = &[
        (
            74,
            Some(HypercallName {
                text: "get_logical_partition_id",
                source: Header,
            }),
        ),
        (
            114,
            Some(HypercallName {
                text: "undocumented_function_114",
                source: Header,
            }),
        ),
        (
            250,
            Some(HypercallName {
                text: "storage_create_region",
                source: Header,
            }),
        ),
        (5, None),
    ];
    assert_eq!(cases.len(), 4);
    for (number, expected) in cases {
        assert_eq!(hypercall_name(*number), *expected, "{number}");
    }
}

#[test]
fn an_alias_replaces_only_a_header_placeholder() {
    let aliased: Vec<&call::Lv1Call> = call::LV1_CALLS
        .iter()
        .filter(|row| row.alias.is_some())
        .collect();
    assert_eq!(aliased.len(), 13);
    for row in aliased {
        assert_eq!(
            row.name,
            format!("undocumented_function_{}", row.number),
            "{row:?}"
        );
    }
}

#[test]
fn every_alias_is_a_bare_name_that_no_other_number_takes() {
    let names: Vec<&str> = call::LV1_CALLS
        .iter()
        .filter_map(|row| row.alias)
        .chain(call::UNLISTED_ALIASES.iter().map(|(_, alias)| *alias))
        .map(|alias| alias.name)
        .collect();
    assert_eq!(names.len(), ALIASES.len());
    for name in &names {
        assert!(!name.starts_with("lv1_"), "{name}");
        assert!(!name.starts_with("undocumented_function"), "{name}");
        assert!(
            call::LV1_CALLS.iter().all(|row| row.name != *name),
            "{name} is a header name"
        );
    }
    let distinct: BTreeSet<&str> = names.iter().copied().collect();
    assert_eq!(distinct.len(), names.len(), "an alias repeats");
}

#[test]
fn the_unlisted_aliases_ascend_and_name_only_unlisted_numbers() {
    let numbers: Vec<u16> = call::UNLISTED_ALIASES
        .iter()
        .map(|(number, _)| *number)
        .collect();
    assert_eq!(numbers.len(), 6);
    for pair in numbers.windows(2) {
        assert!(pair[0] < pair[1], "{pair:?}");
    }
    for number in numbers {
        assert_eq!(call::lookup(u64::from(number)), None, "{number}");
    }
}

#[test]
fn each_name_source_label_is_a_name_source_value_of_the_schema() {
    let labels: Vec<&str> = [Header, Psdevwiki, CellGov]
        .iter()
        .map(|source| source.label())
        .collect();
    assert_eq!(labels, crate::spec::HYPERCALL_NAME_SOURCES);
}

#[test]
fn a_census_row_renders_its_name_and_the_source_of_the_name() {
    let answered: BTreeSet<u64> = [62].into_iter().collect();
    let rows = hypercall_rows(&[], &[], &answered).expect("no site row names a PUP");
    assert_eq!(rows.len(), 1);
    let text = hypercall_tsv(&rows).expect("the row fits the schema");
    assert_eq!(
        text.lines().nth(1),
        Some("62\tset_slb_for_logical_spu\tpsdevwiki\t5\t0\tundocumented\tboot\tnone\t0\thandler\tanswered")
    );
    let table = parse(&HYPERCALL, &text).expect("the rendered table parses");
    assert_eq!(census_answered(&table), Ok(answered));
}
