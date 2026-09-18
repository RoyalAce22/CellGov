use super::*;

use std::path::PathBuf;

use cellgov_install::manifest::Sha256 as HexSha256;

use crate::cli::store::read::collect::StoreView;
use crate::cli::store::read::model::STORE_FORMAT_VERSION;

fn hash(byte: u8) -> HexSha256 {
    HexSha256([byte; 32])
}

fn view() -> StoreView {
    StoreView {
        root: PathBuf::from("vfs"),
        inventory: crate::composition::inventory::StoreInventory::read(std::path::Path::new(
            "no-such-store-root",
        ))
        .expect("an absent root reads as an empty store"),
        registry: cellgov_boot::manifest::TitleRegistry::default(),
        fixtures: PathBuf::from("no-such-fixtures"),
    }
}

fn doc(entries: Vec<VerifiedEntryDoc>) -> VerifyDoc {
    let clean = entries.iter().all(|e| e.divergences.is_empty());
    VerifyDoc {
        format_version: STORE_FORMAT_VERSION,
        store: "vfs".to_string(),
        subject: "TEST00000".to_string(),
        entries,
        clean,
    }
}

#[test]
fn a_missing_file_carries_no_hash_to_compare() {
    let rendered = divergence_doc(
        &view(),
        &Divergence {
            path: PathBuf::from("vfs").join("dev_hdd0/game/TEST00000/PARAM.SFO"),
            kind: DivergenceKind::Missing,
        },
    );
    assert_eq!(rendered.kind, "missing");
    assert_eq!(rendered.path, "dev_hdd0/game/TEST00000/PARAM.SFO");
    assert!(rendered.expected.is_none() && rendered.found.is_none());
}

#[test]
fn a_modified_file_carries_both_hashes() {
    let rendered = divergence_doc(
        &view(),
        &Divergence {
            path: PathBuf::from("vfs").join("EBOOT.BIN"),
            kind: DivergenceKind::Modified {
                expected: hash(0xaa),
                found: hash(0xbb),
            },
        },
    );
    assert_eq!(rendered.kind, "modified");
    assert_eq!(
        rendered.expected.as_deref(),
        Some(hash(0xaa).to_hex()).as_deref()
    );
    assert_eq!(
        rendered.found.as_deref(),
        Some(hash(0xbb).to_hex()).as_deref()
    );
}

/// A firmware manifest hashes the decrypted module, so a file that
/// yields no image has no `found` hash the `expected` one compares to.
#[test]
fn a_module_with_no_image_reports_the_reason_and_no_found_hash() {
    let rendered = module_fault_doc(
        &view(),
        &ModuleFault {
            path: PathBuf::from("vfs").join("firmware/4.91/dev_flash/sys/external/liblv2.sprx"),
            kind: ModuleDivergence::NoImage {
                reason: "no key for revision 0x0001".to_string(),
            },
        },
    );
    assert_eq!(rendered.kind, "no-image");
    assert!(rendered.found.is_none() && rendered.expected.is_none());
    assert_eq!(
        rendered.reason.as_deref(),
        Some("no key for revision 0x0001")
    );
}

#[test]
fn a_clean_pass_reports_what_it_checked() {
    let rendered = render(
        &doc(vec![VerifiedEntryDoc {
            entry: "base".to_string(),
            matched: 42,
            divergences: Vec::new(),
            kernel_omission: None,
        }]),
        "title TEST00000",
    );
    assert_eq!(
        rendered,
        "title TEST00000: 42 artefact(s) match their record\n"
    );
}

#[test]
fn a_kernel_the_entry_does_not_store_is_named_as_unchecked_not_as_a_divergence() {
    let d = doc(vec![VerifiedEntryDoc {
        entry: "4.93".to_string(),
        matched: 370,
        divergences: Vec::new(),
        kernel_omission: Some("update_files carries no CORE_OS_PACKAGE.pkg".to_string()),
    }]);
    assert!(d.clean);
    let rendered = render(&d, "firmware 4.93");
    let lines: Vec<&str> = rendered.lines().collect();
    assert_eq!(
        lines,
        [
            "4.93: kernel not checked: update_files carries no CORE_OS_PACKAGE.pkg",
            "firmware 4.93: 370 artefact(s) match their record",
        ]
    );
}

#[test]
fn a_pass_that_examined_no_artefact_is_not_a_clean_verdict() {
    let empty = doc(vec![VerifiedEntryDoc {
        entry: "base".to_string(),
        matched: 0,
        divergences: Vec::new(),
        kernel_omission: None,
    }]);
    assert!(empty.clean);
    assert!(checked_nothing(&empty));

    assert!(!checked_nothing(&doc(vec![VerifiedEntryDoc {
        entry: "base".to_string(),
        matched: 1,
        divergences: Vec::new(),
        kernel_omission: None,
    }])));
}

#[test]
fn every_divergence_gets_its_own_line() {
    let rendered = render(
        &doc(vec![VerifiedEntryDoc {
            entry: "base".to_string(),
            matched: 1,
            kernel_omission: None,
            divergences: vec![
                DivergenceDoc {
                    path: "a/PARAM.SFO".to_string(),
                    kind: "missing".to_string(),
                    expected: None,
                    found: None,
                    reason: None,
                },
                DivergenceDoc {
                    path: "a/EBOOT.BIN".to_string(),
                    kind: "modified".to_string(),
                    expected: Some(hash(0xaa).to_hex()),
                    found: Some(hash(0xbb).to_hex()),
                    reason: None,
                },
            ],
        }]),
        "title TEST00000",
    );
    let lines: Vec<&str> = rendered.lines().collect();
    assert_eq!(
        lines.len(),
        3,
        "one line per divergence plus a total: {rendered}"
    );
    assert_eq!(lines[0], "a/PARAM.SFO: missing");
    assert!(
        lines[1].starts_with("a/EBOOT.BIN: modified (recorded "),
        "{rendered}"
    );
    assert_eq!(lines[2], "title TEST00000: 2 of 3 artefact(s) diverged");
}
