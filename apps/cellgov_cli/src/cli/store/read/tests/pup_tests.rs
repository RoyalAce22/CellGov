//! The command's share of PUP verification: the compiled archive table,
//! the clean rule and the human report. The classification itself is
//! `cellgov_install::pup_verify`'s, tested beside it.

use super::*;

fn archive_row(hash: &str, fw: &str) -> ArchivePup {
    ArchivePup {
        pup_sha256: hash.to_string(),
        fw: fw.to_string(),
        size_bytes: 10,
        image_version: "0x0000000000000001".to_string(),
    }
}

#[test]
fn an_empty_archive_and_empty_data_verify_cleanly() {
    use cellgov_lv2_archive::{self as archive, PUP};

    let empty = archive::render(&PUP, &[]).expect("render a zero-row PUP table");
    let rows = archive::checked_pup_rows(&empty).expect("a zero-row PUP table is valid");
    assert!(rows.is_empty());
    assert!(pup_set_is_clean(&[], &[], &[]));
}

/// Any missing row, any mismatch, or any installed divergence leaves the
/// set unclean.
#[test]
fn the_set_is_clean_only_with_nothing_missing_mismatched_or_diverged() {
    let missing = [expected_doc(&archive_row("11", "2.00"), None)];
    let mismatched = [PupMismatchDoc {
        subject: "changed.pup".to_string(),
        kind: "sha256".to_string(),
        fw: None,
        expected: Vec::new(),
        found: None,
        reason: None,
    }];
    let diverged = [VerifiedEntryDoc {
        entry: "4.93".to_string(),
        matched: 1,
        divergences: vec![super::super::model::DivergenceDoc {
            path: "dev_flash/x".to_string(),
            kind: "missing".to_string(),
            expected: None,
            found: None,
            reason: None,
        }],
        kernel_omission: None,
    }];
    assert!(!pup_set_is_clean(&missing, &[], &[]));
    assert!(!pup_set_is_clean(&[], &mismatched, &[]));
    assert!(!pup_set_is_clean(&[], &[], &diverged));
}

#[test]
fn the_compiled_archive_reaches_the_verifier_row_for_row() {
    let rows = archive_rows().expect("the compiled table is valid");
    let raw = crate::lv2_tables::committed_pup_rows().expect("parse the compiled table");
    assert_eq!(rows.len(), raw.len());
    for (row, raw) in rows.iter().zip(&raw) {
        assert_eq!(
            (&row.pup_sha256, &row.fw, row.size_bytes, &row.image_version),
            (&raw.pup_sha256, &raw.fw, raw.size_bytes, &raw.image_version)
        );
    }
}

#[test]
fn human_report_keeps_the_three_categories() {
    let doc = PupVerifyDoc {
        format_version: STORE_FORMAT_VERSION,
        pup_directory: "pups".to_string(),
        present: vec![expected_doc(
            &archive_row("00", "1.00"),
            Some("one.pup".to_string()),
        )],
        missing: vec![expected_doc(&archive_row("11", "2.00"), None)],
        mismatched: vec![PupMismatchDoc {
            subject: "changed.pup".to_string(),
            kind: "sha256".to_string(),
            fw: Some("2.00".to_string()),
            expected: vec!["11".to_string()],
            found: Some("22".to_string()),
            reason: None,
        }],
        installed: Vec::new(),
        clean: false,
    };
    let text = render(&doc);
    assert!(text.contains("present:\n  fw 1.00  00  one.pup\n"));
    assert!(text.contains("missing:\n  fw 2.00  11\n"));
    assert!(text.contains("mismatched:\n  changed.pup: sha256"));
}

#[test]
fn a_mismatch_reaches_its_document_with_the_kind_named() {
    let doc = mismatch_doc(PupMismatch {
        subject: "installed firmware 4.92".to_string(),
        kind: cellgov_install::pup_verify::PupMismatchKind::SourceSha256,
        fw: Some("4.92".to_string()),
        expected: vec!["archive-hash".to_string()],
        found: Some("record-hash".to_string()),
        reason: None,
    });
    assert_eq!(doc.kind, "source-sha256");
    assert_eq!(doc.expected, ["archive-hash"]);
    assert_eq!(doc.found.as_deref(), Some("record-hash"));
}
