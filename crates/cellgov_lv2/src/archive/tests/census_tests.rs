use super::*;
use crate::archive::{parse, CENSUS, KERNEL, STUB, SUBENTRY};

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
    };
    let kernel_text = kernel_tsv(std::slice::from_ref(&kernel)).expect("render kernel");
    assert_eq!(
        kernel_text,
        concat!(
            "pup_sha256\tkernel_elf_sha256\ttable_base\tentry_width\tentry_format\tentry_count\tdiscovery_method\tconfidence\tcensus_sha256\tsubentry_sha256\n",
            "1111111111111111111111111111111111111111111111111111111111111111\t",
            "2222222222222222222222222222222222222222222222222222222222222222\t",
            "0x8000000000346570\t8\tppc64_descriptor_pointer\t1024\t",
            "sc_vector_descriptor_array\thigh\t",
            "3333333333333333333333333333333333333333333333333333333333333333\t",
            "4444444444444444444444444444444444444444444444444444444444444444\n"
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
