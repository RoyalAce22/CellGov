use super::*;
use std::collections::BTreeMap;

use crate::{parse, CAPABILITY_GATE, CENSUS, KERNEL, PRESENCE, STUB, SUBENTRY};

#[test]
fn kernel_stub_and_census_rows_round_trip_byte_identically() {
    let kernel = KernelRow {
        pup_sha256: "11".repeat(32),
        kernel_elf_sha256: "22".repeat(32),
        table_base: 0x8000_0000_0034_6570,
        entry_width: 8,
        entry_format: "ppc64_descriptor_pointer".to_string(),
        entry_count: 1024,
        discovery_method: "sc_vector_descriptor_array".to_string(),
        confidence: "high".to_string(),
        census_sha256: "33".repeat(32),
        subentry_sha256: "44".repeat(32),
        gate_sha256: "55".repeat(32),
    };
    let kernel_text = kernel_tsv(std::slice::from_ref(&kernel)).expect("render kernel");
    assert_eq!(
        kernel_text,
        concat!(
            "pup_sha256\tkernel_elf_sha256\ttable_base\tentry_width\tentry_format\tentry_count\tdiscovery_method\tconfidence\tcensus_sha256\tsubentry_sha256\tgate_sha256\n",
            "1111111111111111111111111111111111111111111111111111111111111111\t",
            "2222222222222222222222222222222222222222222222222222222222222222\t",
            "0x8000000000346570\t8\tppc64_descriptor_pointer\t1024\t",
            "sc_vector_descriptor_array\thigh\t",
            "3333333333333333333333333333333333333333333333333333333333333333\t",
            "4444444444444444444444444444444444444444444444444444444444444444\t",
            "5555555555555555555555555555555555555555555555555555555555555555\n"
        )
    );
    assert_eq!(
        kernel_rows(&parse(&KERNEL, &kernel_text).expect("parse kernel")),
        [kernel]
    );

    let stub = StubRow {
        pup_sha256: "11".repeat(32),
        descriptor: 0x8000_0000_0032_4968,
        target: 0x8000_0000_0029_04b0,
        errno: 0x8001_0003,
        errno_symbol: "CELL_ENOSYS".to_string(),
        references: 388,
        primary: true,
    };
    let stub_text = stub_tsv(std::slice::from_ref(&stub)).expect("render stub");
    assert_eq!(
        stub_text,
        concat!(
            "pup_sha256\tdescriptor\ttarget\terrno\terrno_symbol\treferences\tprimary\n",
            "1111111111111111111111111111111111111111111111111111111111111111\t",
            "0x8000000000324968\t0x80000000002904b0\t0x80010003\tCELL_ENOSYS\t388\tyes\n"
        )
    );
    assert_eq!(
        stub_rows(&parse(&STUB, &stub_text).expect("parse stub")),
        [stub]
    );

    let census = vec![
        CensusRow {
            fw: "3.55".to_string(),
            ordinal: 0,
            class: CensusClass::Stub,
            target: Some(0x8000_0000_0029_04b0),
            dispatch: DispatchShape::Flat,
        },
        CensusRow {
            fw: "3.55".to_string(),
            ordinal: 1,
            class: CensusClass::Absent,
            target: None,
            dispatch: DispatchShape::ChainIncomplete,
        },
    ];
    let census_text = census_tsv(&census).expect("render census");
    assert_eq!(
        census_text,
        concat!(
            "fw\tordinal\tclass\ttarget\tdispatch\n",
            "3.55\t0\tstub\t0x80000000002904b0\tflat\n",
            "3.55\t1\tabsent\tnone\tchain_incomplete\n"
        )
    );
    assert_eq!(
        census_rows(&parse(&CENSUS, &census_text).expect("parse census")),
        census
    );
}

#[test]
fn census_file_uses_the_firmware_key_verbatim() {
    assert_eq!(census_file("3.56"), "census/fw-3.56.tsv");
}

#[test]
fn renderers_sort_rows_before_serialization() {
    let first = CensusRow {
        fw: "3.55".to_string(),
        ordinal: 1,
        class: CensusClass::Implemented,
        target: Some(0x8000_0000_0000_1000),
        dispatch: DispatchShape::Flat,
    };
    let second = CensusRow {
        fw: "3.55".to_string(),
        ordinal: 2,
        class: CensusClass::Stub,
        target: Some(0x8000_0000_0000_2000),
        dispatch: DispatchShape::Flat,
    };
    assert_eq!(
        census_tsv(&[second.clone(), first.clone()]).expect("render reverse order"),
        census_tsv(&[first, second]).expect("render forward order")
    );
}

#[test]
fn subentry_rows_round_trip_with_decimal_packets() {
    let rows = vec![SubentryRow {
        pup_sha256: "44".repeat(32),
        ordinal: 863,
        selector_slot: "r3".to_string(),
        packet: 0x6001,
        class: CensusClass::Implemented,
        target: 0x8000_0000_0024_6458,
    }];
    let text = subentry_tsv(&rows).expect("render subentry");
    assert_eq!(
        text,
        concat!(
            "pup_sha256\tordinal\tselector_slot\tpacket\tclass\ttarget\n",
            "4444444444444444444444444444444444444444444444444444444444444444\t",
            "863\tr3\t24577\timplemented\t0x8000000000246458\n"
        )
    );
    assert_eq!(
        subentry_rows(&parse(&SUBENTRY, &text).expect("parse subentry")),
        rows
    );
}

#[test]
fn gate_rows_round_trip_all_three_states() {
    let rows = vec![
        GateRow {
            pup_sha256: "44".repeat(32),
            ordinal: 119,
            state: GateState::Gated,
            reads: Some("ctrl_flags1_0x40000000".to_string()),
            fail_errno: Some(0x8001_0003),
        },
        GateRow {
            pup_sha256: "44".repeat(32),
            ordinal: 120,
            state: GateState::Ungated,
            reads: None,
            fail_errno: None,
        },
        GateRow {
            pup_sha256: "44".repeat(32),
            ordinal: 121,
            state: GateState::NotAnalysed,
            reads: None,
            fail_errno: None,
        },
    ];
    let text = gate_tsv(&rows).expect("render gates");
    assert_eq!(
        gate_rows(&parse(&CAPABILITY_GATE, &text).expect("parse gates")),
        rows
    );
}

#[test]
fn presence_reduction_keeps_an_explicit_class_for_each_version() {
    let by_version = BTreeMap::from([
        (
            "3.55".to_string(),
            vec![
                CensusRow {
                    fw: "3.55".to_string(),
                    ordinal: 0,
                    class: CensusClass::Implemented,
                    target: Some(0x1000),
                    dispatch: DispatchShape::Flat,
                },
                CensusRow {
                    fw: "3.55".to_string(),
                    ordinal: 1,
                    class: CensusClass::Stub,
                    target: Some(0x2000),
                    dispatch: DispatchShape::Flat,
                },
            ],
        ),
        (
            "3.56".to_string(),
            vec![
                CensusRow {
                    fw: "3.56".to_string(),
                    ordinal: 0,
                    class: CensusClass::Absent,
                    target: None,
                    dispatch: DispatchShape::Flat,
                },
                CensusRow {
                    fw: "3.56".to_string(),
                    ordinal: 1,
                    class: CensusClass::Stub,
                    target: Some(0x2000),
                    dispatch: DispatchShape::Flat,
                },
            ],
        ),
    ]);
    let rows = presence_rows(&by_version).expect("reduce presence");
    assert_eq!(
        rows,
        [
            PresenceRow {
                ordinal: 0,
                implemented_versions: vec!["3.55".to_string()],
                stub_versions: Vec::new(),
                absent_versions: vec!["3.56".to_string()],
            },
            PresenceRow {
                ordinal: 1,
                implemented_versions: Vec::new(),
                stub_versions: vec!["3.55".to_string(), "3.56".to_string()],
                absent_versions: Vec::new(),
            },
        ]
    );
    let text = presence_tsv(&rows).expect("render presence");
    assert_eq!(
        text,
        concat!(
            "ordinal\timplemented_versions\tstub_versions\tabsent_versions\n",
            "0\t3.55\tnone\t3.56\n",
            "1\tnone\t3.55,3.56\tnone\n"
        )
    );
    assert_eq!(
        parse(&PRESENCE, &text).expect("parse presence").rows.len(),
        2
    );
}

#[test]
fn presence_reduction_refuses_versions_with_different_ordinal_ranges() {
    let row = |fw: &str, ordinal| CensusRow {
        fw: fw.to_string(),
        ordinal,
        class: CensusClass::Absent,
        target: None,
        dispatch: DispatchShape::Flat,
    };
    let by_version = BTreeMap::from([
        ("3.55".to_string(), vec![row("3.55", 0)]),
        ("3.56".to_string(), vec![row("3.56", 0), row("3.56", 1)]),
    ]);
    assert_eq!(
        presence_rows(&by_version),
        Err(PresenceError::EntryCount {
            fw: "3.55".to_string(),
            expected: 2,
            found: 1,
        })
    );
}

#[test]
fn presence_reduction_refuses_a_row_at_the_wrong_ordinal() {
    let by_version = BTreeMap::from([(
        "3.55".to_string(),
        vec![CensusRow {
            fw: "3.55".to_string(),
            ordinal: 1,
            class: CensusClass::Absent,
            target: None,
            dispatch: DispatchShape::Flat,
        }],
    )]);
    assert_eq!(
        presence_rows(&by_version),
        Err(PresenceError::Ordinal {
            fw: "3.55".to_string(),
            index: 0,
            found: 1,
        })
    );
}

#[test]
fn presence_reduction_refuses_a_row_from_another_version() {
    let by_version = BTreeMap::from([(
        "3.55".to_string(),
        vec![CensusRow {
            fw: "3.56".to_string(),
            ordinal: 0,
            class: CensusClass::Absent,
            target: None,
            dispatch: DispatchShape::Flat,
        }],
    )]);
    assert_eq!(
        presence_rows(&by_version),
        Err(PresenceError::Firmware {
            fw: "3.55".to_string(),
            ordinal: 0,
            found: "3.56".to_string(),
        })
    );
}

#[test]
fn presence_parser_refuses_a_noncanonical_version_key() {
    let text = concat!(
        "ordinal\timplemented_versions\tstub_versions\tabsent_versions\n",
        "0\t123.45\tnone\tnone\n"
    );
    assert!(matches!(
        parse(&PRESENCE, text),
        Err(ArchiveError::BadCell {
            column: "implemented_versions",
            ..
        })
    ));
}
