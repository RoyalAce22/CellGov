use cellgov_ps3_abi::lv1::repository::{field, first_field, AbsentNode, RepositoryKey, BUS_WALK};

use super::*;

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

fn region_2() -> RepositoryKey {
    RepositoryKey {
        lpar: 1,
        n1: first_field(b"bus", 1),
        n2: field(b"dev", 1),
        n3: field(b"reg", 2),
        n4: field(b"type", 0),
    }
}

fn absent_at(firmware: &'static [&'static str]) -> AbsentNode {
    AbsentNode {
        key: region_2(),
        firmware,
        walk: BUS_WALK,
    }
}

fn read_row(n: u8, answered: bool) -> RepositoryReadRow {
    RepositoryReadRow {
        pup_sha256: pup(n),
        key: region_2(),
        node: region_2().path(),
        first: 0,
        answered,
    }
}

#[test]
fn an_answered_read_of_an_absent_key_needs_no_node_row() {
    let rows = repository_rows(
        &[],
        &[absent_at(&["4.93"])],
        &[read_row(1, true)],
        &[pup_row(1, "4.93")],
    );
    assert_eq!(rows, Ok(Vec::new()));
}

#[test]
fn an_answered_read_of_a_key_absent_only_at_another_firmware_is_a_stale_read() {
    assert_eq!(
        repository_rows(
            &[],
            &[absent_at(&["4.93"])],
            &[read_row(1, true)],
            &[pup_row(1, "3.70")],
        ),
        Err(RepositoryTableError::StaleRead {
            pup_sha256: pup(1),
            key: region_2(),
            answered: true,
        })
    );
}

#[test]
fn a_refused_read_of_an_absent_key_is_a_stale_read() {
    assert_eq!(
        repository_rows(
            &[],
            &[absent_at(&["4.93"])],
            &[read_row(1, false)],
            &[pup_row(1, "4.93")],
        ),
        Err(RepositoryTableError::StaleRead {
            pup_sha256: pup(1),
            key: region_2(),
            answered: false,
        })
    );
}

#[test]
fn an_absent_key_that_no_logged_boot_reads_is_refused() {
    assert_eq!(
        repository_rows(&[], &[absent_at(&["4.93"])], &[], &[pup_row(1, "4.93")]),
        Err(RepositoryTableError::UnreadAbsentKey {
            key: region_2(),
            fw: "4.93".to_string(),
        })
    );
}
