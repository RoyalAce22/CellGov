//! Firmware-manifest round-trip, canonical file ordering, and format-version gating.

use super::*;

fn sha(b: u8) -> Sha256 {
    Sha256([b; 32])
}

// Kept in sorted path order: `serialize_manifest` canonicalizes,
// so the round-trip test would otherwise compare canonical-vs-
// input and fail. New entries here must remain sorted.
fn sample_manifest() -> FirmwareManifest {
    FirmwareManifest {
        format_version: SUPPORTED_FORMAT_VERSION,
        firmware: FirmwareIdentity {
            image_version: "4.85".into(),
            pup_sha256: sha(0x00),
        },
        files: vec![
            FirmwareFileEntry {
                path: "sys/external/libfs.sprx".into(),
                sha256: sha(0xAA),
                revision: 0x1c,
            },
            FirmwareFileEntry {
                path: "sys/external/liblv2.sprx".into(),
                sha256: sha(0xBB),
                revision: 0x1c,
            },
        ],
    }
}

#[test]
fn round_trip_preserves_every_field() {
    let m = sample_manifest();
    let text = serialize_manifest(&m).expect("ser");
    let parsed = parse_manifest(&text).expect("parse");
    assert_eq!(parsed, m);
}

#[test]
fn serialise_is_deterministic_across_two_calls() {
    let m = sample_manifest();
    let t1 = serialize_manifest(&m).expect("ser1");
    let t2 = serialize_manifest(&m).expect("ser2");
    assert_eq!(t1, t2);
}

#[test]
fn serialise_sorts_files_by_path_regardless_of_input_order() {
    let mut m = sample_manifest();
    let unsorted = vec![m.files[1].clone(), m.files[0].clone()];
    m.files = unsorted;
    let t_unsorted = serialize_manifest(&m).expect("ser");

    let m_sorted = sample_manifest();
    let t_sorted = serialize_manifest(&m_sorted).expect("ser");

    assert_eq!(t_unsorted, t_sorted);
}

#[test]
fn unsupported_format_version_errors_via_parse_manifest() {
    let mut m = sample_manifest();
    m.format_version = 2;
    // Bypass the try_from gate by serialising the raw form
    // through toml::to_string against a struct that mirrors
    // RawManifest's shape.
    #[derive(Serialize)]
    struct Forged<'a> {
        format_version: u32,
        firmware: &'a FirmwareIdentity,
        files: &'a [FirmwareFileEntry],
    }
    let text = toml::to_string(&Forged {
        format_version: m.format_version,
        firmware: &m.firmware,
        files: &m.files,
    })
    .expect("forge");
    let err = parse_manifest(&text).unwrap_err();
    let inner = match err {
        ManifestError::Toml(e) => e.to_string(),
        other => panic!("expected Toml-wrapped UnsupportedFormatVersion, got {other:?}"),
    };
    assert!(
        inner.contains("unsupported firmware.toml format_version 2"),
        "wrong inner message: {inner}"
    );
}

#[test]
fn forged_future_version_via_direct_from_str_is_also_rejected() {
    // The try_from attribute makes the version check structural;
    // a caller bypassing parse_manifest and reaching for
    // toml::from_str directly gets the same rejection.
    let text = "format_version = 2\n[firmware]\nimage_version = \"x\"\npup_sha256 = \"00000000000000000000000000000000000000000000000000000000000000ff\"\n";
    let err = toml::from_str::<FirmwareManifest>(text).unwrap_err();
    assert!(
        err.to_string().contains("unsupported"),
        "expected version rejection from direct toml::from_str, got: {err}"
    );
}

#[test]
fn malformed_toml_surfaces_as_toml_error() {
    let err = parse_manifest("not [valid").unwrap_err();
    assert!(matches!(err, ManifestError::Toml(_)));
}

#[test]
fn missing_required_field_surfaces_as_toml_error() {
    let text = "format_version = 1\n";
    let err = parse_manifest(text).unwrap_err();
    assert!(matches!(err, ManifestError::Toml(_)));
}

#[test]
fn duplicate_path_is_rejected_at_parse_time() {
    let dup = "ee".repeat(32);
    let text = format!(
        r#"format_version = 1

[firmware]
image_version = "x"
pup_sha256 = "{}"

[[files]]
path = "a.sprx"
sha256 = "{dup}"
revision = 0

[[files]]
path = "a.sprx"
sha256 = "{dup}"
revision = 0
"#,
        "00".repeat(32),
    );
    let err = parse_manifest(&text).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("duplicate") && msg.contains("a.sprx"),
        "expected duplicate-path rejection, got: {msg}"
    );
}

#[test]
fn uppercase_hex_is_rejected_at_parse_time() {
    let text = format!(
        "format_version = 1\n[firmware]\nimage_version = \"x\"\npup_sha256 = \"{}\"\n",
        "AA".repeat(32),
    );
    let err = parse_manifest(&text).unwrap_err();
    assert!(matches!(err, ManifestError::Toml(_)));
}

#[test]
fn short_hex_is_rejected_at_parse_time() {
    let text = "format_version = 1\n[firmware]\nimage_version = \"x\"\npup_sha256 = \"deadbeef\"\n";
    let err = parse_manifest(text).unwrap_err();
    assert!(matches!(err, ManifestError::Toml(_)));
}

#[test]
fn non_hex_chars_are_rejected_at_parse_time() {
    let text = format!(
        "format_version = 1\n[firmware]\nimage_version = \"x\"\npup_sha256 = \"{}\"\n",
        "g0".repeat(32),
    );
    let err = parse_manifest(&text).unwrap_err();
    assert!(matches!(err, ManifestError::Toml(_)));
}

#[test]
fn verify_post_decrypt_match_returns_match() {
    let m = sample_manifest();
    let r = verify_post_decrypt(&m, "sys/external/libfs.sprx", &[0xAA; 32]);
    assert_eq!(r, VerifyOutcome::Match);
}

#[test]
fn verify_post_decrypt_unknown_path_returns_not_in_manifest() {
    let m = sample_manifest();
    let r = verify_post_decrypt(&m, "sys/external/nope.sprx", &[0u8; 32]);
    assert_eq!(r, VerifyOutcome::NotInManifest);
}

#[test]
fn verify_post_decrypt_wrong_digest_returns_mismatch() {
    let m = sample_manifest();
    let r = verify_post_decrypt(&m, "sys/external/libfs.sprx", &[0xFF; 32]);
    assert_eq!(
        r,
        VerifyOutcome::Mismatch {
            expected: [0xAA; 32],
            actual: [0xFF; 32],
        }
    );
}

#[test]
fn files_table_can_be_empty() {
    // The cellgov_firmware install pipeline does not contractually
    // guarantee at least one decrypted SPRX -- a PUP without APP
    // keys we recognise produces zero entries, and the manifest
    // should still round-trip rather than refuse to serialise.
    let m = FirmwareManifest {
        format_version: SUPPORTED_FORMAT_VERSION,
        firmware: FirmwareIdentity {
            image_version: "1.00".into(),
            pup_sha256: sha(0x00),
        },
        files: Vec::new(),
    };
    let text = serialize_manifest(&m).expect("ser");
    let parsed = parse_manifest(&text).expect("parse");
    assert_eq!(parsed.files.len(), 0);
}

#[test]
fn manifest_verifier_rejects_empty_manifest() {
    // Schema permits empty files; verifier does not. Empty
    // would finish() Ok against zero verifications, which is
    // the trivially-true case we are ruling out.
    let m = FirmwareManifest {
        format_version: SUPPORTED_FORMAT_VERSION,
        firmware: FirmwareIdentity {
            image_version: "1.00".into(),
            pup_sha256: sha(0x00),
        },
        files: Vec::new(),
    };
    assert_eq!(ManifestVerifier::new(&m).unwrap_err(), EmptyManifest);
}

#[test]
fn manifest_verifier_finish_succeeds_when_every_entry_matched() {
    let m = sample_manifest();
    let mut v = ManifestVerifier::new(&m).expect("non-empty");
    assert_eq!(
        v.verify_one("sys/external/libfs.sprx", &[0xAA; 32]),
        VerifyOutcome::Match
    );
    assert_eq!(
        v.verify_one("sys/external/liblv2.sprx", &[0xBB; 32]),
        VerifyOutcome::Match
    );
    assert!(v.finish().is_ok());
}

/// Extract the path from a single-element [`ManifestError::EntryUnverified`]
/// vec, panicking on any other shape; test-only.
fn single_unverified_path(errs: Vec<ManifestError>) -> String {
    assert_eq!(errs.len(), 1, "expected exactly one unmatched: {errs:?}");
    match errs.into_iter().next().unwrap() {
        ManifestError::EntryUnverified(p) => p,
        other => panic!("expected EntryUnverified, got {other:?}"),
    }
}

#[test]
fn manifest_verifier_finish_returns_unmatched_paths_in_manifest_order() {
    let m = sample_manifest();
    let mut v = ManifestVerifier::new(&m).expect("non-empty");
    assert_eq!(
        v.verify_one("sys/external/libfs.sprx", &[0xAA; 32]),
        VerifyOutcome::Match
    );
    let unmatched = v.finish().unwrap_err();
    assert_eq!(
        single_unverified_path(unmatched),
        "sys/external/liblv2.sprx"
    );
}

#[test]
fn manifest_verifier_mismatch_does_not_count_as_matched() {
    let m = sample_manifest();
    let mut v = ManifestVerifier::new(&m).expect("non-empty");
    // Wrong digest: returns Mismatch and does NOT flip matched[].
    assert!(matches!(
        v.verify_one("sys/external/libfs.sprx", &[0xFF; 32]),
        VerifyOutcome::Mismatch { .. }
    ));
    assert_eq!(
        v.verify_one("sys/external/liblv2.sprx", &[0xBB; 32]),
        VerifyOutcome::Match
    );
    let unmatched = v.finish().unwrap_err();
    assert_eq!(single_unverified_path(unmatched), "sys/external/libfs.sprx");
}

#[test]
fn manifest_verifier_unknown_path_is_not_in_manifest_and_does_not_count() {
    let m = sample_manifest();
    let mut v = ManifestVerifier::new(&m).expect("non-empty");
    assert_eq!(
        v.verify_one("sys/external/nope.sprx", &[0u8; 32]),
        VerifyOutcome::NotInManifest
    );
    assert_eq!(
        v.verify_one("sys/external/libfs.sprx", &[0xAA; 32]),
        VerifyOutcome::Match
    );
    let unmatched = v.finish().unwrap_err();
    assert_eq!(
        single_unverified_path(unmatched),
        "sys/external/liblv2.sprx"
    );
}
