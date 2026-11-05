use std::path::PathBuf;

use super::*;
use crate::ppu_census::{PpuCensus, PpuCensusTitle};

/// `addi r3, r0, 7`.
const ADDI: u32 = 0x3860_0007;
/// `addis r3, r0, 7`.
const ADDIS: u32 = 0x3C60_0007;

fn reference_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ppu_reference")
}

fn row<'a>(rows: &'a [PpuCoverageRow], unit: &str) -> &'a PpuCoverageRow {
    rows.iter()
        .find(|row| row.unit == unit)
        .unwrap_or_else(|| panic!("no row for {unit}"))
}

#[test]
fn the_table_has_one_row_per_reference_unit_in_listing_order() {
    let rows = coverage_rows(&PpuCoverageEvidence::default());
    let units: Vec<String> = all_units().iter().map(unit_key).collect();
    let keys: Vec<String> = rows.iter().map(|row| row.unit.clone()).collect();
    assert!(units.len() > 300, "{} units", units.len());
    assert_eq!(keys, units);
}

#[test]
fn each_unit_status_follows_what_the_interpreter_does_with_its_words() {
    let rows = coverage_rows(&PpuCoverageEvidence::default());
    assert_eq!(
        row(&rows, "instruction:addi").status,
        PpuUnitStatus::Executes
    );
    assert!(row(&rows, "instruction:addi").fuzz_descriptor);
    // [CBE-Handbook p:738 s:A.2.4.1] The PPE does not implement popcntb,
    // one word of the PPE-unimplemented unit.
    assert_eq!(
        row(&rows, "ppe_unimplemented").status,
        PpuUnitStatus::Refuses(BTreeSet::from(["illegal"]))
    );
    assert_eq!(
        row(&rows, "privileged").status,
        PpuUnitStatus::Refuses(BTreeSet::from(["privileged"]))
    );
    assert_eq!(
        row(&rows, "unassigned_opcodes").status,
        PpuUnitStatus::NotDecoded
    );
    assert_eq!(
        row(&rows, "facility:reservation").status,
        PpuUnitStatus::Facility
    );
}

#[test]
fn the_stream_mnemonics_bit_6_selects_each_have_a_decoded_row() {
    let rows = coverage_rows(&PpuCoverageEvidence::default());
    for unit in [
        "instruction:dss",
        "instruction:dssall",
        "instruction:dst",
        "instruction:dstt",
        "instruction:dstst",
        "instruction:dststt",
    ] {
        assert_eq!(row(&rows, unit).status, PpuUnitStatus::Executes, "{unit}");
        assert!(row(&rows, unit).fuzz_descriptor, "{unit}");
    }
}

#[test]
fn a_case_counts_once_for_each_unit_its_words_reach() {
    let cases: [&[u32]; 3] = [&[ADDI, ADDI], &[ADDI, ADDIS], &[0x0400_0000]];
    let counts = console_case_units(cases);
    assert_eq!(
        counts,
        BTreeMap::from([
            ("instruction:addi".to_owned(), 2),
            ("instruction:addis".to_owned(), 1),
        ])
    );
}

#[test]
fn a_reference_file_is_found_under_the_unit_it_covers() {
    let files = reference_files(&reference_dir()).unwrap();
    assert_eq!(
        files.get("instruction:add").map(String::as_str),
        Some("add.json")
    );
    assert_eq!(
        files.get("facility:reservation").map(String::as_str),
        Some("facility_reservation.json")
    );
}

#[test]
fn a_documented_open_result_set_is_listed_under_the_units_it_reaches() {
    let rows = coverage_rows(&PpuCoverageEvidence::default());
    assert!(row(&rows, "instruction:divw")
        .open_sets
        .contains("muldiv-undef"));
    assert!(row(&rows, "instruction:vsl")
        .open_sets
        .contains("vshift-nonuniform"));
    assert!(row(&rows, "instruction:addi").open_sets.is_empty());
}

#[test]
fn the_evidence_reaches_its_row_and_the_summary() {
    let mut census = PpuCensus::new();
    census.add(ADDI);
    census.add(ADDI);
    let title = PpuCensusTitle {
        content_id: "TEST00000".into(),
        skipped: None,
        images: 1,
        words: 2,
        unread: 0,
    };
    let evidence = PpuCoverageEvidence {
        reference_files: BTreeMap::from([(
            "instruction:addis".to_owned(),
            "addis.json".to_owned(),
        )]),
        console_cases: BTreeMap::from([("instruction:addi".to_owned(), 5)]),
        census: Some(PpuCensusRecord::new(vec![title], &census)),
    };
    let rows = coverage_rows(&evidence);
    let addi = row(&rows, "instruction:addi");
    assert_eq!(addi.console_cases, 5);
    assert_eq!(addi.title_words, Some(2));
    assert_eq!(addi.reference_file, None);
    assert_eq!(row(&rows, "instruction:addis").title_words, Some(0));
    let doc = render_coverage(&rows, evidence.census.as_ref());
    assert!(
        doc.contains("| `instruction:addi` | executes | yes | - | 5 | - | 2 |"),
        "{doc}"
    );
    assert!(doc.contains("| exercised by the titles | 1 |"), "{doc}");
    assert!(
        doc.contains("| exercised with no reference file | 1 |"),
        "{doc}"
    );
    assert!(doc.contains("| with a reference file | 1 |"), "{doc}");
}

#[test]
fn a_census_unit_outside_the_listing_is_listed_on_its_own() {
    let mut record = PpuCensusRecord::new(Vec::new(), &PpuCensus::new());
    record.units.insert(
        "instruction:nonesuch".into(),
        crate::ppu_census::PpuCensusCounts {
            implemented: 0,
            refused: BTreeMap::new(),
            not_recognized: 3,
        },
    );
    let doc = render_coverage(&[], Some(&record));
    assert!(doc.contains("## Census words outside the units"), "{doc}");
    assert!(doc.contains("| `instruction:nonesuch` | 3 |"), "{doc}");
}
