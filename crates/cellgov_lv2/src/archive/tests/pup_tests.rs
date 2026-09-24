use super::*;
use crate::archive::{parse, render};

fn table_text(rows: &[PupRow]) -> String {
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            vec![
                row.pup_sha256.clone(),
                row.fw.clone(),
                row.size_bytes.to_string(),
                row.image_version.clone(),
                row.source_note.clone(),
                row.acquired.clone().unwrap_or_else(|| NONE.to_string()),
            ]
        })
        .collect();
    render(&PUP, &cells).unwrap()
}

#[test]
fn checked_pup_rows_reads_a_table_and_holds_its_rows_to_the_invariants() {
    assert_eq!(checked_pup_rows(&table_text(&[row()])), Ok(vec![row()]));
    assert!(matches!(
        checked_pup_rows("not a table\n"),
        Err(PupTsvError::Parse(_))
    ));
    let bad = PupRow {
        size_bytes: 0,
        ..row()
    };
    // The schema admits the cell; the row invariant refuses it.
    let text = table_text(&[bad]);
    assert!(parse(&PUP, &text).is_ok());
    assert!(matches!(
        checked_pup_rows(&text),
        Err(PupTsvError::Rows(PupTableError::ZeroSize { .. }))
    ));
}

fn row() -> PupRow {
    PupRow {
        pup_sha256: "01".repeat(32),
        fw: "4.93".to_string(),
        size_bytes: 206_197_916,
        image_version: "0x0000000000010b94".to_string(),
        source_note: "operator_archive".to_string(),
        acquired: Some("2026-09-11".to_string()),
    }
}

#[test]
fn an_empty_table_is_valid_and_round_trips() {
    let text = "pup_sha256\tfw\tsize_bytes\timage_version\tsource_note\tacquired\n";
    let table = parse(&PUP, text).unwrap_or_else(|error| panic!("{error}"));
    let firmware = parse(
        &crate::archive::FIRMWARE,
        "fw\torder\trelease_date\tpriority\trole\n\
         4.93\t493\t2026-03-17\t1\tfinal\n",
    )
    .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(pup_rows(&table), Vec::new());
    assert_eq!(check_pup_rows(&[]), Ok(()));
    assert_eq!(
        crate::archive::check_references(&[firmware, table.clone()]),
        Ok(())
    );
    assert_eq!(render(&PUP, &table.rows), Ok(text.to_string()));
}

#[test]
fn a_well_formed_row_decodes_and_round_trips() {
    let text = format!(
        "pup_sha256\tfw\tsize_bytes\timage_version\tsource_note\tacquired\n\
         {}\t4.93\t206197916\t0x0000000000010b94\toperator_archive\t2026-09-11\n",
        "01".repeat(32)
    );
    let table = parse(&PUP, &text).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(pup_rows(&table), [row()]);
    assert_eq!(check_pup_rows(&pup_rows(&table)), Ok(()));
    assert_eq!(render(&PUP, &table.rows), Ok(text));
}

#[test]
fn malformed_domain_values_are_refused_by_name() {
    let mut bad = row();
    bad.pup_sha256 = "A1".repeat(32);
    assert_eq!(
        check_pup_rows(&[bad.clone()]),
        Err(PupTableError::BadSha256 {
            pup_sha256: bad.pup_sha256,
        })
    );

    let mut bad = row();
    bad.size_bytes = 0;
    assert_eq!(
        check_pup_rows(&[bad.clone()]),
        Err(PupTableError::ZeroSize {
            pup_sha256: bad.pup_sha256,
        })
    );

    let mut bad = row();
    bad.image_version = "0x10b94".to_string();
    assert_eq!(
        check_pup_rows(&[bad.clone()]),
        Err(PupTableError::BadImageVersion {
            pup_sha256: bad.pup_sha256,
            image_version: bad.image_version,
        })
    );

    let mut bad = row();
    bad.acquired = Some("2026-02-29".to_string());
    assert_eq!(
        check_pup_rows(&[bad.clone()]),
        Err(PupTableError::BadAcquired {
            pup_sha256: bad.pup_sha256,
            acquired: "2026-02-29".to_string(),
        })
    );
}

#[test]
fn a_size_shorter_than_the_pup_header_is_refused() {
    let mut bad = row();
    bad.size_bytes = PUP_HEADER_SIZE as u64 - 1;
    assert_eq!(
        check_pup_rows(&[bad.clone()]),
        Err(PupTableError::TooSmall {
            pup_sha256: bad.pup_sha256,
            size_bytes: bad.size_bytes,
        })
    );
}

#[test]
fn year_zero_is_not_a_gregorian_acquisition_date() {
    let mut bad = row();
    bad.acquired = Some("0000-01-01".to_string());
    assert_eq!(
        check_pup_rows(&[bad.clone()]),
        Err(PupTableError::BadAcquired {
            pup_sha256: bad.pup_sha256,
            acquired: "0000-01-01".to_string(),
        })
    );
}

#[test]
fn a_source_note_cannot_be_a_url() {
    let text = format!(
        "pup_sha256\tfw\tsize_bytes\timage_version\tsource_note\tacquired\n\
         {}\t4.93\t206197916\t0x0000000000010b94\thttps://example.com/file\tnone\n",
        "01".repeat(32)
    );
    assert!(matches!(
        parse(&PUP, &text),
        Err(crate::archive::ArchiveError::BadCell {
            column: "source_note",
            ..
        })
    ));
}
