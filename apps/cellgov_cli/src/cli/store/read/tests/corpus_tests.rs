use super::*;

fn row(hash: &str, fw: &str) -> PupRow {
    PupRow {
        pup_sha256: hash.to_string(),
        fw: fw.to_string(),
        size_bytes: 10,
        image_version: "0x0000000000000001".to_string(),
        source_note: "fixture".to_string(),
        acquired: None,
    }
}

fn found(path: &str, hash: &str, fw: Result<&str, &str>) -> ScannedPup {
    ScannedPup {
        path: path.to_string(),
        sha256: hash.to_string(),
        size_bytes: 10,
        fw: fw
            .map(|fw| (fw.to_string(), "0x0000000000000001".to_string()))
            .map_err(str::to_string),
    }
}

#[test]
fn an_empty_archive_and_empty_corpus_verify_cleanly() {
    let header = PUP_TSV
        .lines()
        .next()
        .expect("the compiled table has a header");
    let table = archive::parse(&PUP, &format!("{header}\n")).expect("parse a zero-row PUP table");
    let rows = archive::pup_rows(&table);
    archive::check_pup_rows(&rows).expect("a zero-row PUP table is valid");

    let (present, missing, mismatched) = classify(&rows, &[]);
    assert!(present.is_empty());
    assert!(missing.is_empty());
    assert!(mismatched.is_empty());
    assert!(corpus_is_clean(&missing, &mismatched, &[]));
}

#[test]
fn present_missing_and_mismatched_are_distinct() {
    let rows = [row("00", "1.00"), row("11", "2.00")];
    let scanned = [
        found("one.pup", "00", Ok("1.00")),
        found("changed.pup", "22", Ok("2.00")),
        found("broken.pup", "33", Err("bad PUP magic")),
    ];
    let (present, missing, mismatched) = classify(&rows, &scanned);
    assert_eq!(present.len(), 1);
    assert_eq!(present[0].pup_sha256, "00");
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].pup_sha256, "11");
    assert_eq!(mismatched.len(), 2);
    assert_eq!(mismatched[0].kind, "invalid-pup");
    assert_eq!(mismatched[1].kind, "sha256");
    assert_eq!(mismatched[1].expected, ["11"]);
    assert_eq!(mismatched[1].found.as_deref(), Some("22"));
}

#[test]
fn a_known_hash_with_wrong_metadata_is_mismatched_not_missing() {
    let rows = [row("00", "1.00")];
    let scanned = [found("wrong.pup", "00", Ok("2.00"))];
    let (present, missing, mismatched) = classify(&rows, &scanned);
    assert!(present.is_empty());
    assert!(missing.is_empty());
    assert_eq!(mismatched.len(), 1);
    assert_eq!(mismatched[0].kind, "metadata");
}

#[test]
fn a_matching_file_preserves_the_archive_identity_and_corpus_path() {
    let rows = [row("00", "1.00")];
    let scanned = [found("nested/one.pup", "00", Ok("1.00"))];
    let (present, missing, mismatched) = classify(&rows, &scanned);
    let [entry] = present.as_slice() else {
        panic!("expected one present archive row, got {present:?}");
    };
    assert_eq!(entry.fw, "1.00");
    assert_eq!(entry.pup_sha256, "00");
    assert_eq!(entry.size_bytes, 10);
    assert_eq!(entry.image_version, "0x0000000000000001");
    assert_eq!(entry.path.as_deref(), Some("nested/one.pup"));
    assert!(missing.is_empty());
    assert!(mismatched.is_empty());
}

#[test]
fn human_report_keeps_the_three_categories() {
    let doc = PupCorpusVerifyDoc {
        format_version: STORE_FORMAT_VERSION,
        corpus: "pups".to_string(),
        present: vec![expected_doc(
            &row("00", "1.00"),
            Some("one.pup".to_string()),
        )],
        missing: vec![expected_doc(&row("11", "2.00"), None)],
        mismatched: vec![PupCorpusMismatchDoc {
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
fn installed_firmware_cross_checks_all_three_identity_fields() {
    let archive = row("archive-hash", "4.93");
    let mismatched = installed_identity_mismatches(
        "4.92",
        "record-hash",
        "archive-hash",
        "0x0000000000000002",
        &archive,
    );
    let kinds: Vec<&str> = mismatched.iter().map(|row| row.kind.as_str()).collect();
    assert_eq!(
        kinds,
        ["firmware-version", "image-version", "source-sha256"]
    );
    assert_eq!(mismatched[2].expected, ["archive-hash"]);
    assert_eq!(mismatched[2].found.as_deref(), Some("record-hash"));
}

#[test]
fn a_matching_installed_identity_has_no_mismatch() {
    let archive = row("archive-hash", "4.93");
    let mismatched = installed_identity_mismatches(
        "4.93",
        "archive-hash",
        "archive-hash",
        "0x0000000000000001",
        &archive,
    );
    assert!(mismatched.is_empty(), "{mismatched:?}");
}

#[test]
fn a_manifest_version_that_disagrees_with_its_pup_row_is_mismatched() {
    let archive = row("archive-hash", "4.93");
    let mismatch = installed_manifest_version_mismatch("4.93", "4.92", &archive)
        .expect("wrong manifest version must be a finding");
    assert_eq!(mismatch.kind, "manifest-version");
    assert_eq!(mismatch.fw.as_deref(), Some("4.92"));
    assert_eq!(mismatch.expected, ["4.93"]);
    assert_eq!(mismatch.found.as_deref(), Some("4.92"));
}
