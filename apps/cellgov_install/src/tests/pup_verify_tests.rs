use super::*;

fn row(hash: &str, fw: &str) -> ArchivePup {
    ArchivePup {
        pup_sha256: hash.to_string(),
        fw: fw.to_string(),
        size_bytes: 10,
        image_version: "0x0000000000000001".to_string(),
    }
}

/// A file that parsed as `fw`, or, for `None`, one that is no PUP.
fn found(path: &str, hash: &str, fw: Option<&str>) -> ScannedPup {
    ScannedPup {
        path: path.to_string(),
        sha256: hash.to_string(),
        size_bytes: 10,
        identity: match fw {
            Some(fw) => Ok(PupIdentity {
                fw: fw.to_string(),
                image_version: "0x0000000000000001".to_string(),
            }),
            None => Err(pup::parse(b"not a pup").expect_err("nine bytes are no PUP")),
        },
    }
}

fn kinds(mismatched: &[PupMismatch]) -> Vec<&'static str> {
    mismatched.iter().map(|m| m.kind.label()).collect()
}

#[test]
fn an_empty_archive_and_no_files_classify_to_nothing() {
    let sorted = classify(&[], &[]);
    assert!(sorted.present.is_empty());
    assert!(sorted.missing.is_empty());
    assert!(sorted.mismatched.is_empty());
}

#[test]
fn present_missing_and_mismatched_are_distinct() {
    let rows = [row("00", "1.00"), row("11", "2.00")];
    let scanned = [
        found("one.pup", "00", Some("1.00")),
        found("changed.pup", "22", Some("2.00")),
        found("broken.pup", "33", None),
    ];
    let sorted = classify(&rows, &scanned);
    assert_eq!(sorted.present.len(), 1);
    assert_eq!(sorted.present[0].0.pup_sha256, "00");
    assert_eq!(sorted.missing.len(), 1);
    assert_eq!(sorted.missing[0].pup_sha256, "11");
    assert_eq!(kinds(&sorted.mismatched), ["invalid-pup", "sha256"]);
    assert_eq!(sorted.mismatched[0].expected, Vec::<String>::new());
    assert_eq!(
        sorted.mismatched[0].reason.as_deref(),
        Some("PUP file too small for header (got 9 bytes)")
    );
    assert_eq!(sorted.mismatched[1].expected, ["11"]);
    assert_eq!(sorted.mismatched[1].found.as_deref(), Some("22"));
}

#[test]
fn a_known_hash_with_wrong_metadata_is_mismatched_not_missing() {
    let rows = [row("00", "1.00")];
    let sorted = classify(&rows, &[found("wrong.pup", "00", Some("2.00"))]);
    assert!(sorted.present.is_empty());
    assert!(sorted.missing.is_empty());
    assert_eq!(kinds(&sorted.mismatched), ["metadata"]);
    assert_eq!(
        sorted.mismatched[0].expected,
        ["fw 1.00, size 10, image 0x0000000000000001"]
    );
    assert_eq!(
        sorted.mismatched[0].found.as_deref(),
        Some("fw 2.00, size 10, image 0x0000000000000001")
    );
}

#[test]
fn a_known_hash_that_does_not_parse_names_its_archive_row() {
    let rows = [row("00", "1.00")];
    let sorted = classify(&rows, &[found("broken.pup", "00", None)]);
    assert!(sorted.missing.is_empty(), "the hash still claims the row");
    assert_eq!(kinds(&sorted.mismatched), ["invalid-pup"]);
    assert_eq!(sorted.mismatched[0].expected, ["00"]);
}

#[test]
fn a_matching_file_keeps_the_archive_row_and_its_path() {
    let rows = [row("00", "1.00")];
    let sorted = classify(&rows, &[found("nested/one.pup", "00", Some("1.00"))]);
    let [(entry, path)] = sorted.present.as_slice() else {
        panic!("expected one present archive row, got {:?}", sorted.present);
    };
    assert_eq!(**entry, rows[0]);
    assert_eq!(path, "nested/one.pup");
    assert!(sorted.missing.is_empty());
    assert!(sorted.mismatched.is_empty());
}

#[test]
fn only_the_first_copy_of_a_hash_is_present() {
    let rows = [row("00", "1.00")];
    let sorted = classify(
        &rows,
        &[
            found("a.pup", "00", Some("1.00")),
            found("b.pup", "00", Some("1.00")),
        ],
    );
    assert_eq!(sorted.present.len(), 1);
    assert_eq!(sorted.present[0].1, "a.pup");
    assert!(sorted.mismatched.is_empty());
}

fn claims<'a>(
    version: &'a str,
    record: &'a str,
    manifest_version: &'a str,
    image: &'a str,
) -> InstalledClaims<'a> {
    InstalledClaims {
        version,
        record_sha256: record,
        manifest_version,
        manifest_sha256: "archive-hash",
        manifest_image_version: image,
    }
}

#[test]
fn installed_firmware_cross_checks_all_four_identity_fields_in_order() {
    let archive = row("archive-hash", "4.93");
    let mismatched = installed_mismatches(
        &claims("4.92", "record-hash", "4.91", "0x0000000000000002"),
        &archive,
    );
    assert_eq!(
        kinds(&mismatched),
        [
            "manifest-version",
            "firmware-version",
            "image-version",
            "source-sha256"
        ]
    );
    assert!(mismatched
        .iter()
        .all(|m| m.subject == "installed firmware 4.92"));
    assert_eq!(mismatched[0].fw.as_deref(), Some("4.91"));
    assert_eq!(mismatched[3].expected, ["archive-hash"]);
    assert_eq!(mismatched[3].found.as_deref(), Some("record-hash"));
}

#[test]
fn a_matching_installed_identity_has_no_mismatch() {
    let archive = row("archive-hash", "4.93");
    let mismatched = installed_mismatches(
        &claims("4.93", "archive-hash", "4.93", "0x0000000000000001"),
        &archive,
    );
    assert!(mismatched.is_empty(), "{mismatched:?}");
}

#[test]
fn a_scanned_file_that_is_no_pup_carries_the_parse_refusal() {
    let scanned = ScannedPup::of("x.pup".to_string(), b"not a pup");
    assert_eq!(scanned.size_bytes, 9);
    assert_eq!(scanned.sha256.len(), 64);
    assert!(scanned.identity.is_err());
}
