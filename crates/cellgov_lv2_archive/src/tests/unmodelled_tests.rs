//! The unmodelled-syscall join over the archive's rows.

use super::*;
use crate::{GateState, NameSource};

fn name(ordinal: u64, packet: Option<&str>, text: &str) -> NameRow {
    NameRow {
        ordinal,
        packet: packet.map(str::to_string),
        name: text.to_string(),
        source: NameSource::Cellgov,
        reference: None,
        fw_from: None,
        fw_to: None,
    }
}

fn gate(pup_sha256: &str, ordinal: usize) -> GateRow {
    GateRow {
        pup_sha256: pup_sha256.to_string(),
        ordinal,
        state: GateState::Gated,
        reads: Some("ctrl_flags1_0x00000040".to_string()),
        fail_errno: Some(0x8001_0009),
    }
}

fn caller(pup_sha256: &str, module: &str, ordinal: usize, sites: &[u64]) -> CallerRow {
    CallerRow {
        pup_sha256: pup_sha256.to_string(),
        module: module.to_string(),
        ordinal,
        sites: sites.to_vec(),
    }
}

/// The 4.93 PUP, which the compiled-in kernel census covers, as bytes
/// and hex.
fn extracted_pup() -> ([u8; 32], String) {
    let hex = "158471fd834f8ea8036136b6aab43cd86c7ba73d79ca30e0af3c0fe0001cf365".to_string();
    let bytes: [u8; 32] = (0..32)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    assert_ne!(lookup(&bytes, 0), PupCensusClass::NotExtracted);
    (bytes, hex)
}

#[test]
fn a_pup_the_census_covers_gets_its_class_names_and_gate_but_no_callers() {
    let (bytes, hex) = extracted_pup();
    let names = [
        name(9, None, "sys_whole"),
        name(9, Some("1"), "sys_packet"),
        name(10, None, "other"),
    ];
    // Another PUP's row for the same ordinal comes first.
    let gates = [gate(&"ee".repeat(32), 9), gate(&hex, 9)];
    let callers = [caller(&hex, "sys/a.sprx", 9, &[16, 32])];
    let rows = unmodelled_syscalls([(9, 5)], Some(&bytes), &names, &gates, &callers);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!((row.ordinal, row.hits), (9, 5));
    assert_eq!(row.names, [&names[0]]);
    assert_eq!(row.census, lookup(&bytes, 9));
    assert_eq!(row.gate, Some(&gates[1]));
    assert!(
        row.callers.is_empty(),
        "callers are evidence only without a census"
    );
}

#[test]
fn a_pup_without_a_census_gets_every_caller_module_with_all_its_sites() {
    let pup = [0xab; 32];
    let hex = "ab".repeat(32);
    let callers = [
        caller(&hex, "sys/a.sprx", 9, &[16, 32, 48]),
        caller(&hex, "sys/b.sprx", 9, &[64]),
        caller(&hex, "sys/b.sprx", 10, &[80]),
        caller(&"cd".repeat(32), "sys/a.sprx", 9, &[16]),
    ];
    let rows = unmodelled_syscalls([(9, 1)], Some(&pup), &[], &[], &callers);
    assert_eq!(rows[0].census, PupCensusClass::NotExtracted);
    assert_eq!(rows[0].callers, [&callers[0], &callers[1]]);
}

#[test]
fn no_pup_identity_matches_no_gate_or_caller() {
    let hex = "ab".repeat(32);
    let gates = [gate(&hex, 9)];
    let callers = [caller(&hex, "sys/a.sprx", 9, &[16])];
    let rows = unmodelled_syscalls([(9, 1)], None, &[], &gates, &callers);
    assert_eq!(rows[0].census, PupCensusClass::NotExtracted);
    assert_eq!(rows[0].gate, None);
    assert!(rows[0].callers.is_empty());
}

#[test]
fn every_census_class_has_a_label() {
    assert_eq!(
        census_class_label(PupCensusClass::NotExtracted),
        "not_extracted"
    );
    assert_eq!(
        census_class_label(PupCensusClass::OutOfRange),
        "out_of_range"
    );
    assert_eq!(
        census_class_label(PupCensusClass::Implemented),
        "implemented"
    );
    assert_eq!(census_class_label(PupCensusClass::Stub), "stub");
    assert_eq!(census_class_label(PupCensusClass::Absent), "absent");
}

#[test]
fn only_a_pup_without_a_census_takes_caller_evidence() {
    let (bytes, _) = extracted_pup();
    assert!(!takes_caller_evidence(Some(&bytes)));
    assert!(takes_caller_evidence(Some(&[0xab; 32])));
    assert!(!takes_caller_evidence(None));
}

#[test]
fn pup_digest_uses_the_archive_key_spelling() {
    assert_eq!(pup_hex(&[0xab; 32]), "ab".repeat(32));
}
