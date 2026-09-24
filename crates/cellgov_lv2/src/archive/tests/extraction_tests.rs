//! The rules that fold one PUP's extracted kernel rows into the archive.

use super::*;
use crate::archive::{CensusClass, GateState};
use sha2::Digest;

fn sha256_hex(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn pup(pup_sha256: &str, fw: &str) -> PupRow {
    PupRow {
        pup_sha256: pup_sha256.to_string(),
        fw: fw.to_string(),
        size_bytes: 1,
        image_version: "0x0000000000035600".to_string(),
        source_note: "local".to_string(),
        acquired: None,
    }
}

fn kernel(pup_sha256: &str, kernel_elf_sha256: &str, census_sha256: &str) -> KernelRow {
    KernelRow {
        pup_sha256: pup_sha256.to_string(),
        kernel_elf_sha256: kernel_elf_sha256.to_string(),
        table_base: 0x1000,
        entry_width: 8,
        entry_format: "ppc64_descriptor_pointer".to_string(),
        entry_count: 1024,
        discovery_method: "sc_vector_descriptor_array".to_string(),
        confidence: "high".to_string(),
        census_sha256: census_sha256.to_string(),
        subentry_sha256: "00".repeat(32),
        gate_sha256: "00".repeat(32),
    }
}

fn stub(pup_sha256: &str, descriptor: u64) -> StubRow {
    StubRow {
        pup_sha256: pup_sha256.to_string(),
        descriptor,
        target: 2,
        errno: 0x8001_0003,
        errno_symbol: "CELL_ENOSYS".to_string(),
        references: 1,
        primary: true,
    }
}

fn subentry(pup_sha256: &str, packet: u64) -> SubentryRow {
    SubentryRow {
        pup_sha256: pup_sha256.to_string(),
        ordinal: 621,
        selector_slot: "r3".to_string(),
        packet,
        class: CensusClass::Implemented,
        target: 3,
    }
}

fn gate(pup_sha256: &str, ordinal: usize) -> GateRow {
    GateRow {
        pup_sha256: pup_sha256.to_string(),
        ordinal,
        state: GateState::Ungated,
        reads: None,
        fail_errno: None,
    }
}

fn extraction(pup_sha256: &str, kernel_elf: &str, census: &str) -> PupExtraction {
    PupExtraction {
        kernel: kernel(pup_sha256, kernel_elf, census),
        stubs: vec![stub(pup_sha256, 1)],
        subentries: vec![subentry(pup_sha256, 0)],
        gates: vec![gate(pup_sha256, 621)],
    }
}

/// The rows `extraction` would leave in an archive of its own.
fn held(new: &PupExtraction) -> ExtractedRows {
    ExtractedRows {
        kernels: vec![new.kernel.clone()],
        stubs: new.stubs.clone(),
        subentries: new.subentries.clone(),
        gates: new.gates.clone(),
    }
}

#[test]
fn the_encodings_of_a_classification_are_the_archive_cells() {
    assert_eq!(selector_slot_name(0), "r3");
    assert_eq!(selector_slot_name(7), "r10");
    assert_eq!(control_flags1_read(0x40), "ctrl_flags1_0x00000040");
}

#[test]
fn a_pup_is_selected_only_under_its_recorded_firmware() {
    let pups = vec![pup("pup-a", "3.55")];
    assert_eq!(select_pup(&pups, "pup-a", "3.55"), Ok(&pups[0]));
    assert_eq!(
        select_pup(&pups, "pup-b", "3.55"),
        Err(ExtractionError::UnknownPup {
            pup_sha256: "pup-b".to_string()
        })
    );
    assert_eq!(
        select_pup(&pups, "pup-a", "0.00"),
        Err(ExtractionError::FirmwareMismatch {
            pup_sha256: "pup-a".to_string(),
            recorded: "3.55".to_string(),
            requested: "0.00".to_string(),
        })
    );
}

#[test]
fn a_kernel_rows_table_digests_hash_the_tables_as_rendered() {
    let new = extraction(&"11".repeat(32), "kernel-a", "census");
    assert_eq!(
        gate_digest(&new.gates, &sha256_hex),
        Ok(sha256_hex(gate_tsv(&new.gates).unwrap().as_bytes()))
    );
    assert_eq!(
        subentry_digest(&new.subentries, &sha256_hex),
        Ok(sha256_hex(
            subentry_tsv(&new.subentries).unwrap().as_bytes()
        ))
    );
}

#[test]
fn a_held_archive_whose_rows_agree_validates() {
    // Two PUPs, so each kernel row's gate digest covers its own gates.
    let (a, b) = ("11".repeat(32), "22".repeat(32));
    let pups = vec![pup(&a, "3.56"), pup(&b, "3.60")];
    let mut rows = held(&extraction(&a, "kernel-a", "census"));
    let other = held(&extraction(&b, "kernel-b", "census"));
    rows.kernels.extend(other.kernels);
    rows.gates.extend(other.gates);
    for kernel in &mut rows.kernels {
        let own: Vec<GateRow> = rows
            .gates
            .iter()
            .filter(|row| row.pup_sha256 == kernel.pup_sha256)
            .cloned()
            .collect();
        kernel.gate_sha256 = sha256_hex(gate_tsv(&own).unwrap().as_bytes());
    }
    assert_eq!(validate_existing(&rows, &pups, &sha256_hex), Ok(()));
}

#[test]
fn a_kernel_row_naming_an_unrecorded_pup_is_refused() {
    let rows = ExtractedRows {
        kernels: vec![kernel("pup-x", "kernel", "census")],
        ..ExtractedRows::default()
    };
    assert_eq!(
        validate_existing(&rows, &[pup("pup-a", "3.56")], &sha256_hex),
        Err(ExtractionError::ExistingPupReference {
            table: "kernel",
            pup_sha256: "pup-x".to_string(),
        })
    );
}

#[test]
fn a_stub_subentry_or_gate_row_without_a_kernel_row_is_refused() {
    let pups = vec![pup("pup-a", "3.56")];
    let orphans = [
        ExtractedRows {
            stubs: vec![stub("pup-a", 1)],
            ..ExtractedRows::default()
        },
        ExtractedRows {
            subentries: vec![subentry("pup-a", 0)],
            ..ExtractedRows::default()
        },
        ExtractedRows {
            gates: vec![gate("pup-a", 1)],
            ..ExtractedRows::default()
        },
    ];
    for rows in orphans {
        assert_eq!(
            validate_existing(&rows, &pups, &sha256_hex),
            Err(ExtractionError::ExistingKernelReference {
                pup_sha256: "pup-a".to_string()
            }),
            "{rows:?}"
        );
    }
}

#[test]
fn existing_gate_rows_must_match_their_kernel_digest() {
    let pups = vec![pup(&"11".repeat(32), "3.56")];
    let mut recorded = kernel(&pups[0].pup_sha256, &"22".repeat(32), &"33".repeat(32));
    recorded.gate_sha256 = sha256_hex(
        gate_tsv(&[gate(&pups[0].pup_sha256, 0)])
            .expect("render recorded gates")
            .as_bytes(),
    );
    let rows = ExtractedRows {
        kernels: vec![recorded],
        ..ExtractedRows::default()
    };
    assert!(matches!(
        validate_existing(&rows, &pups, &sha256_hex),
        Err(ExtractionError::ExistingGateDigest { pup_sha256, .. })
            if pup_sha256 == pups[0].pup_sha256
    ));
}

#[test]
fn a_matching_version_census_does_not_hide_a_wrong_pup_kernel() {
    let existing = held(&extraction("pup-a", "kernel-a", "same-census"));
    let replacement = extraction("pup-a", "kernel-b", "same-census");
    for allow_movement in [false, true] {
        assert!(matches!(
            refuse_extraction_conflict(&existing, &replacement, allow_movement),
            Err(ExtractionError::KernelDigestConflict { pup_sha256, recorded, extracted })
                if pup_sha256 == "pup-a" && recorded == "kernel-a" && extracted == "kernel-b"
        ));
    }
}

#[test]
fn a_re_extraction_that_moves_is_refused_unless_movement_is_allowed() {
    let existing = held(&extraction("pup-a", "kernel-a", "census"));
    let unchanged = extraction("pup-a", "kernel-a", "census");
    assert_eq!(
        refuse_extraction_conflict(&existing, &unchanged, false),
        Ok(())
    );
    let movements = [
        PupExtraction {
            stubs: vec![stub("pup-a", 9)],
            ..unchanged.clone()
        },
        PupExtraction {
            subentries: vec![subentry("pup-a", 9)],
            ..unchanged.clone()
        },
        PupExtraction {
            gates: vec![gate("pup-a", 9)],
            ..unchanged.clone()
        },
        extraction("pup-a", "kernel-a", "other-census"),
    ];
    for moved in movements {
        assert_eq!(
            refuse_extraction_conflict(&existing, &moved, false),
            Err(ExtractionError::ExtractionConflict {
                pup_sha256: "pup-a".to_string()
            }),
            "{moved:?}"
        );
        assert_eq!(refuse_extraction_conflict(&existing, &moved, true), Ok(()));
    }
}

#[test]
fn row_order_is_not_movement() {
    let mut new = extraction("pup-a", "kernel-a", "census");
    new.stubs = vec![stub("pup-a", 1), stub("pup-a", 2)];
    new.gates = vec![gate("pup-a", 1), gate("pup-a", 2)];
    let mut existing = held(&new);
    existing.stubs.reverse();
    existing.gates.reverse();
    assert_eq!(refuse_extraction_conflict(&existing, &new, false), Ok(()));
}

#[test]
fn version_replacement_removes_every_variant_and_reports_the_other_rows() {
    let pups = vec![
        pup("pup-a", "3.56"),
        pup("pup-b", "3.56"),
        pup("pup-c", "3.60"),
    ];
    let mut existing = ExtractedRows {
        kernels: vec![
            kernel("pup-a", "kernel-a", "census"),
            kernel("pup-b", "kernel-b", "census"),
            kernel("pup-c", "kernel-c", "other"),
        ],
        stubs: vec![stub("pup-b", 1)],
        subentries: vec![subentry("pup-b", 0)],
        gates: vec![gate("pup-b", 621)],
    };
    assert_eq!(
        remove_version_rows(&mut existing, "3.56", "pup-a", &pups),
        1
    );
    assert_eq!(
        existing
            .kernels
            .iter()
            .map(|row| row.pup_sha256.as_str())
            .collect::<Vec<_>>(),
        ["pup-c"]
    );
    assert!(existing.stubs.is_empty());
    assert!(existing.subentries.is_empty());
    assert!(existing.gates.is_empty());
}

#[test]
fn two_pups_of_one_version_must_agree_on_the_census() {
    let pups = vec![pup("pup-a", "3.56"), pup("pup-b", "3.56")];
    let agree = [
        kernel("pup-a", "k-a", "census"),
        kernel("pup-b", "k-b", "census"),
    ];
    assert_eq!(refuse_digest_conflict(&agree, &pups), Ok(()));
    let disagree = [
        kernel("pup-a", "k-a", "census"),
        kernel("pup-b", "k-b", "other"),
    ];
    assert_eq!(
        refuse_digest_conflict(&disagree, &pups),
        Err(ExtractionError::DigestConflict {
            fw: "3.56".to_string()
        })
    );
    assert_eq!(
        refuse_digest_conflict(&[kernel("pup-x", "k", "c")], &pups),
        Err(ExtractionError::ExistingPupReference {
            table: "kernel",
            pup_sha256: "pup-x".to_string(),
        })
    );
}

#[test]
fn a_merge_replaces_the_pups_rows_and_keeps_the_others() {
    let pups = vec![pup("pup-a", "3.56"), pup("pup-c", "3.60")];
    let mut existing = held(&extraction("pup-c", "kernel-c", "c-census"));
    let new = extraction("pup-a", "kernel-a", "census");
    assert_eq!(
        merge_extraction(&mut existing, new.clone(), "3.56", &pups, false),
        Ok(0)
    );
    assert_eq!(existing.kernels.len(), 2);
    assert!(existing.kernels.contains(&new.kernel));
    assert_eq!(existing.stubs.len(), 2);
    assert_eq!(existing.gates.len(), 2);

    // A second fold of the same extraction leaves one copy of its rows.
    merge_extraction(&mut existing, new, "3.56", &pups, false).unwrap();
    assert_eq!(existing.kernels.len(), 2);
    assert_eq!(existing.subentries.len(), 2);
}

#[test]
fn a_replacing_merge_reports_the_other_same_version_pups_it_removed() {
    let pups = vec![pup("pup-a", "3.56"), pup("pup-b", "3.56")];
    // The merge replaces the selected PUP's own held row and leaves it
    // out of the count.
    let mut existing = held(&extraction("pup-a", "kernel-a", "old"));
    existing.kernels.push(kernel("pup-b", "kernel-b", "old"));
    let removed = merge_extraction(
        &mut existing,
        extraction("pup-a", "kernel-a", "new"),
        "3.56",
        &pups,
        true,
    );
    assert_eq!(removed, Ok(1));
    assert_eq!(existing.kernels.len(), 1);
    assert_eq!(existing.kernels[0].pup_sha256, "pup-a");
}

#[test]
fn a_replacing_merge_still_refuses_a_different_kernel_image_and_keeps_the_rows() {
    let pups = vec![pup("pup-a", "3.56"), pup("pup-b", "3.56")];
    let mut existing = held(&extraction("pup-a", "kernel-a", "old"));
    existing.kernels.push(kernel("pup-b", "kernel-b", "old"));
    let before = existing.clone();
    assert!(matches!(
        merge_extraction(
            &mut existing,
            extraction("pup-a", "kernel-z", "new"),
            "3.56",
            &pups,
            true,
        ),
        Err(ExtractionError::KernelDigestConflict { pup_sha256, recorded, extracted })
            if pup_sha256 == "pup-a" && recorded == "kernel-a" && extracted == "kernel-z"
    ));
    assert_eq!(existing, before);
}

#[test]
fn a_merge_refuses_a_census_its_version_disagrees_with() {
    let pups = vec![pup("pup-a", "3.56"), pup("pup-b", "3.56")];
    let mut existing = held(&extraction("pup-b", "kernel-b", "old"));
    assert_eq!(
        merge_extraction(
            &mut existing,
            extraction("pup-a", "kernel-a", "new"),
            "3.56",
            &pups,
            false,
        ),
        Err(ExtractionError::DigestConflict {
            fw: "3.56".to_string()
        })
    );
}

#[test]
fn a_changed_census_requires_an_explicit_version_replacement() {
    assert_eq!(census_needs_write(None, "new\n", "3.55", false), Ok(true));
    assert_eq!(
        census_needs_write(Some("new\n"), "new\n", "3.55", false),
        Ok(false)
    );
    assert_eq!(
        census_needs_write(Some("old\n"), "new\n", "3.55", false),
        Err(ExtractionError::CensusConflict {
            fw: "3.55".to_string()
        })
    );
    assert_eq!(
        census_needs_write(Some("old\n"), "new\n", "3.55", true),
        Ok(true)
    );
}
