//! The hypercall tables: the family rule, the per-number join, the
//! renderers and the per-PUP replacement.

use super::*;
use cellgov_lv2_archive::parse;

fn pup(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}

fn pup_row(n: u8, fw: &str) -> PupRow {
    PupRow {
        pup_sha256: pup(n),
        fw: fw.to_string(),
        size_bytes: 1,
        image_version: "0x0000000000000000".to_string(),
        source_note: "test".to_string(),
        acquired: None,
    }
}

fn site(n: u8, number: u64, sites: &[u64]) -> HypercallSiteRow {
    HypercallSiteRow {
        pup_sha256: pup(n),
        number,
        sites: sites.to_vec(),
    }
}

#[test]
fn each_family_takes_its_numbers_and_its_owning_work_item() {
    let family = |number| hypercall_family(number).0;
    let leaf = |number| hypercall_family(number).1;
    assert_eq!(family(74), "partition");
    assert_eq!(family(91), "repository");
    assert_eq!(family(57), "logical_spe");
    assert_eq!(family(1), "memory");
    assert_eq!(family(12), "interrupt");
    assert_eq!(family(62), "undocumented");
    assert_eq!(family(9), "other");
    assert_eq!(family(145), "other", "add_lpm_event_bookmark");
    assert_eq!(family(154), "other", "set_lpm_debug_bus_control");
    assert_eq!(family(5), "unnamed");
    assert_eq!(family(170), "device", "open_device");
    assert_eq!(family(69), "partition", "get_logical_ppe_id");
    assert_eq!(family(67), "logical_spe", "get_spe_interrupt_status");
    for console in [105, 106, 107, 109] {
        assert_eq!(family(console), "console");
        assert_eq!(leaf(console), "dispatch");
    }
    assert_eq!(leaf(74), "dispatch");
    assert_eq!(leaf(91), "repository");
    assert_eq!(leaf(57), "logical_spe");
    assert_eq!(leaf(1), "translation");
    assert_eq!(leaf(12), "interrupt_delivery");
    assert_eq!(leaf(69), "dispatch");
    assert_eq!(leaf(67), "logical_spe");
    for unowned in [62, 9, 5, 170] {
        assert_eq!(leaf(unowned), "boot");
    }
}

#[test]
fn a_number_row_joins_the_call_table_the_kernels_and_the_hypervisor() {
    let pups = [pup_row(1, "1.50"), pup_row(2, "4.93"), pup_row(3, "4.93")];
    let sites = [
        site(1, 74, &[0x10]),
        site(3, 74, &[0x10, 0x20, 0x30]),
        site(2, 74, &[0x10, 0x20]),
        site(1, 5, &[0x40]),
    ];
    let rows = hypercall_rows(&sites, &pups).expect("every site row names a PUP");
    let numbers: Vec<u64> = rows.iter().map(|r| r.number).collect();
    assert_eq!(
        numbers,
        [0, 1, 2, 5, 7, 11, 12, 18, 69, 74, 91, 95, 105, 106, 107, 109, 122],
        "each called number and each answered one"
    );
    let row = |number: u64| &rows[numbers.iter().position(|n| *n == number).expect("a row")];
    assert_eq!(
        *row(74),
        HypercallRow {
            number: 74,
            name: Some("get_logical_partition_id"),
            inputs: Some(0),
            outputs: Some(1),
            family: "partition",
            leaf: "dispatch",
            fw: vec!["1.50".to_string(), "4.93".to_string()],
            sites: 3,
            lv1_493: Lv1Entry::Handler,
            state: HypercallState::Answered,
        }
    );
    assert_eq!(
        *row(5),
        HypercallRow {
            number: 5,
            name: None,
            inputs: None,
            outputs: None,
            family: "unnamed",
            leaf: "boot",
            fw: vec!["1.50".to_string()],
            sites: 1,
            lv1_493: Lv1Entry::Handler,
            state: HypercallState::NullBackend,
        }
    );
    assert_eq!((row(91).fw.len(), row(91).sites), (0, 0), "91 is uncalled");
    let text = hypercall_tsv(&rows).expect("the rows fit the schema");
    assert!(text.contains(
        "\n74\tget_logical_partition_id\t0\t1\tpartition\tdispatch\t1.50,4.93\t3\thandler\tanswered\n"
    ));
    assert!(text.contains("\n5\tnone\tnone\tnone\tunnamed\tboot\t1.50\t1\thandler\tnull_backend\n"));
}

#[test]
fn a_site_row_that_names_no_pup_is_refused() {
    let pups = [pup_row(1, "4.93")];
    let sites = [site(1, 74, &[0x10]), site(2, 91, &[0x20])];
    assert_eq!(
        hypercall_rows(&sites, &pups),
        Err(ExtractionError::ExistingPupReference {
            table: "hypercall_site",
            pup_sha256: pup(2),
        })
    );
}

#[test]
fn the_site_tables_round_trip_their_typed_rows_in_key_order() {
    let sites = vec![site(2, 7, &[4]), site(1, 74, &[8, 12]), site(1, 5, &[16])];
    let text = hypercall_site_tsv(&sites).expect("render");
    assert_eq!(
        text,
        format!(
            "pup_sha256\tnumber\tsites\n{p1}\t5\t16\n{p1}\t74\t8,12\n{p2}\t7\t4\n",
            p1 = pup(1),
            p2 = pup(2)
        )
    );
    let parsed = hypercall_site_rows(&parse(&HYPERCALL_SITE, &text).expect("parse"));
    assert_eq!(
        parsed,
        [site(1, 5, &[16]), site(1, 74, &[8, 12]), site(2, 7, &[4])]
    );
    let unresolved = vec![
        HypercallUnresolvedRow {
            pup_sha256: pup(2),
            sites: vec![],
        },
        HypercallUnresolvedRow {
            pup_sha256: pup(1),
            sites: vec![20, 24],
        },
    ];
    let text = hypercall_unresolved_tsv(&unresolved).expect("render");
    assert_eq!(
        text,
        format!(
            "pup_sha256\tsites\n{p1}\t20,24\n{p2}\tnone\n",
            p1 = pup(1),
            p2 = pup(2)
        )
    );
    let parsed = hypercall_unresolved_rows(&parse(&HYPERCALL_UNRESOLVED, &text).expect("parse"));
    assert_eq!(parsed, [unresolved[1].clone(), unresolved[0].clone()]);
}

#[test]
fn a_rescan_replaces_every_row_of_its_pup_and_keeps_the_others() {
    let mut census = HypercallCensus {
        sites: vec![site(1, 74, &[8]), site(1, 91, &[12]), site(2, 74, &[8])],
        unresolved: vec![
            HypercallUnresolvedRow {
                pup_sha256: pup(1),
                sites: vec![4],
            },
            HypercallUnresolvedRow {
                pup_sha256: pup(2),
                sites: vec![],
            },
        ],
    };
    census.replace_pup(
        &pup(1),
        HypercallCensus {
            sites: vec![site(1, 105, &[16])],
            unresolved: vec![HypercallUnresolvedRow {
                pup_sha256: pup(1),
                sites: vec![],
            }],
        },
    );
    assert_eq!(census.sites, [site(2, 74, &[8]), site(1, 105, &[16])]);
    assert_eq!(
        census.unresolved,
        [
            HypercallUnresolvedRow {
                pup_sha256: pup(2),
                sites: vec![],
            },
            HypercallUnresolvedRow {
                pup_sha256: pup(1),
                sites: vec![],
            },
        ]
    );
}
