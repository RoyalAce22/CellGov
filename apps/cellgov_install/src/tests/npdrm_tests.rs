//! NPDRM klicensee resolution: RAP-to-klic derivation, NPD header parsing
//! over the supplemental-header walk, and debug-SELF rejection.

use super::*;

#[test]
fn rap_to_klic_is_pure() {
    let rap = [0x42u8; 16];
    let a = rap_to_klic(&rap);
    let b = rap_to_klic(&rap);
    assert_eq!(a, b);
}

fn npd(license: NpdLicense, content_id: &str) -> NpdHeaderInfo {
    NpdHeaderInfo {
        license,
        content_id: content_id.to_string(),
    }
}

#[test]
fn resolve_klicensee_license_network_with_rap_returns_klic() {
    let want = [0xABu8; 16];
    let got =
        resolve_npdrm_klicensee(&npd(NpdLicense::Network, "NPUA80001"), |_| Some(want)).unwrap();
    assert_eq!(got, want);
}

#[test]
fn resolve_klicensee_license_local_with_rap_returns_klic() {
    let want = [0xCDu8; 16];
    let got =
        resolve_npdrm_klicensee(&npd(NpdLicense::Local, "NPUA80068"), |_| Some(want)).unwrap();
    assert_eq!(got, want);
}

#[test]
fn resolve_klicensee_license_network_without_rap_errors_with_content_id() {
    let err =
        resolve_npdrm_klicensee(&npd(NpdLicense::Network, "NPUA80001"), |_| None).unwrap_err();
    match err {
        SceError::NoRapForNpdrmTitle { content_id } => {
            assert_eq!(content_id, "NPUA80001");
        }
        other => panic!("expected NoRapForNpdrmTitle, got {other:?}"),
    }
}

#[test]
fn resolve_klicensee_license_local_without_rap_errors_with_content_id() {
    let err = resolve_npdrm_klicensee(&npd(NpdLicense::Local, "NPUA80068"), |_| None).unwrap_err();
    match err {
        SceError::NoRapForNpdrmTitle { content_id } => {
            assert_eq!(content_id, "NPUA80068");
        }
        other => panic!("expected NoRapForNpdrmTitle, got {other:?}"),
    }
}

#[test]
fn resolve_klicensee_license_free_without_rap_returns_np_klic_free() {
    let got = resolve_npdrm_klicensee(&npd(NpdLicense::Free, "NPEA00000"), |_| None).unwrap();
    assert_eq!(got, NP_KLIC_FREE);
}

#[test]
fn resolve_klicensee_license_free_with_rap_returns_supplied_klic() {
    let want = [0x77u8; 16];
    let got = resolve_npdrm_klicensee(&npd(NpdLicense::Free, "NPEA00000"), |_| Some(want)).unwrap();
    assert_eq!(got, want);
}

/// Minimal SCE header (0x20 bytes) carrying the given
/// `revision_flags`. Satisfies `parse_sce_header`'s magic check
/// so the debug guard can run; all other fields are zero.
fn synthetic_sce_header_with_revision_flags(revision_flags: u16) -> Vec<u8> {
    let mut data = vec![0u8; 0x20];
    data[0..4].copy_from_slice(b"SCE\0");
    data[8..10].copy_from_slice(&revision_flags.to_be_bytes());
    data
}

#[test]
fn decrypt_self_to_elf_npdrm_rejects_debug_self_with_high_bit_set() {
    let data = synthetic_sce_header_with_revision_flags(0x8000);
    let dummy_klic = [0u8; 16];
    let err = decrypt_self_to_elf_npdrm(&data, &dummy_klic).unwrap_err();
    match err {
        SceError::DebugSelfUnsupported { revision_flags } => {
            assert_eq!(revision_flags, 0x8000);
        }
        other => panic!("expected DebugSelfUnsupported, got {other:?}"),
    }
}

#[test]
fn decrypt_self_to_elf_npdrm_rejects_debug_self_with_both_bits_set() {
    // High bit AND a non-zero revision in the low 15 bits:
    // guard must fire on the raw value, error carries it whole.
    let data = synthetic_sce_header_with_revision_flags(0xC042);
    let dummy_klic = [0u8; 16];
    let err = decrypt_self_to_elf_npdrm(&data, &dummy_klic).unwrap_err();
    assert!(matches!(
        err,
        SceError::DebugSelfUnsupported {
            revision_flags: 0xC042
        }
    ));
}

#[test]
fn decrypt_self_to_elf_npdrm_does_not_treat_high_revision_as_debug() {
    // 0x7FFF: highest non-debug revision. Must fall through
    // past the debug guard (the downstream NoAppKey is fine).
    let data = synthetic_sce_header_with_revision_flags(0x7FFF);
    let dummy_klic = [0u8; 16];
    let err = decrypt_self_to_elf_npdrm(&data, &dummy_klic).unwrap_err();
    assert!(!matches!(err, SceError::DebugSelfUnsupported { .. }));
}

/// Build a minimal supplemental-header chain at offset 0x68.
/// Returns the data buffer; caller perturbs records in-place.
fn build_synthetic_supplemental_chain(records_bytes: &[u8]) -> Vec<u8> {
    let mut data = vec![0u8; 0x200];
    let supp_off: u64 = 0x68;
    let supp_size: u64 = records_bytes.len() as u64;
    data[0x58..0x60].copy_from_slice(&supp_off.to_be_bytes());
    data[0x60..0x68].copy_from_slice(&supp_size.to_be_bytes());
    let start = supp_off as usize;
    let end = start + records_bytes.len();
    data[start..end].copy_from_slice(records_bytes);
    data
}

#[test]
fn find_npd_no_npdrm_record_returns_ok_none() {
    // Two non-NPDRM records, both well-formed at the minimum 0x10
    // size. The disc / APP-keyed path takes this branch.
    let mut records = vec![0u8; 0x20];
    records[0..4].copy_from_slice(&1u32.to_be_bytes());
    records[4..8].copy_from_slice(&0x10u32.to_be_bytes());
    records[0x10..0x14].copy_from_slice(&2u32.to_be_bytes());
    records[0x14..0x18].copy_from_slice(&0x10u32.to_be_bytes());
    let data = build_synthetic_supplemental_chain(&records);
    assert!(find_npd_header_info(&data).unwrap().is_none());
}

#[test]
fn find_npd_empty_supplemental_returns_ok_none() {
    let mut data = vec![0u8; 0x80];
    data[0x58..0x60].copy_from_slice(&0u64.to_be_bytes());
    data[0x60..0x68].copy_from_slice(&0u64.to_be_bytes());
    assert!(find_npd_header_info(&data).unwrap().is_none());
}

#[test]
fn find_npd_record_size_under_minimum_returns_typed_error() {
    let mut records = vec![0u8; 0x10];
    records[0..4].copy_from_slice(&1u32.to_be_bytes());
    records[4..8].copy_from_slice(&0x0Fu32.to_be_bytes());
    let data = build_synthetic_supplemental_chain(&records);
    let err = find_npd_header_info(&data).unwrap_err();
    assert!(matches!(err, SceError::HeaderOffsetOutOfRange { .. }));
}

#[test]
fn find_npd_record_size_overflow_returns_typed_error() {
    let mut records = vec![0u8; 0x10];
    records[0..4].copy_from_slice(&1u32.to_be_bytes());
    records[4..8].copy_from_slice(&u32::MAX.to_be_bytes());
    let data = build_synthetic_supplemental_chain(&records);
    let err = find_npd_header_info(&data).unwrap_err();
    assert!(matches!(err, SceError::HeaderOffsetOutOfRange { .. }));
}

#[test]
fn find_npd_npd_body_overflow_returns_typed_error() {
    // Record with kind=NPDRM and size=0x10 (just the record header,
    // no body). NPD body needs 0x80 bytes past cursor+0x10, but
    // supplemental_size = 0x10 means npd_end > supplemental_end.
    let mut records = vec![0u8; 0x10];
    records[0..4].copy_from_slice(&SCE_SUPPLEMENTAL_KIND_NPDRM.to_be_bytes());
    records[4..8].copy_from_slice(&0x10u32.to_be_bytes());
    let data = build_synthetic_supplemental_chain(&records);
    let err = find_npd_header_info(&data).unwrap_err();
    assert!(matches!(err, SceError::HeaderOffsetOutOfRange { .. }));
}

/// Build a single NPDRM record of size 0x90 (record header +
/// 0x80 NPD body), license set, content_id filled with `cid_fill`.
/// Returned buffer is ready for `find_npd_header_info`.
fn build_synthetic_npdrm_record(license_wire: u32, cid_fill: u8) -> Vec<u8> {
    let mut records = vec![0u8; 0x90];
    records[0..4].copy_from_slice(&SCE_SUPPLEMENTAL_KIND_NPDRM.to_be_bytes());
    records[4..8].copy_from_slice(&0x90u32.to_be_bytes());
    // NPD body at +0x10; license at NPD+0x08 = record offset 0x18.
    records[0x18..0x1C].copy_from_slice(&license_wire.to_be_bytes());
    // content_id at NPD+0x10 = record offset 0x20, 0x30 bytes.
    records[0x20..0x50].fill(cid_fill);
    build_synthetic_supplemental_chain(&records)
}

#[test]
fn find_npd_content_id_no_nul_returns_full_48_bytes() {
    let data = build_synthetic_npdrm_record(1, b'X');
    let info = find_npd_header_info(&data).unwrap().unwrap();
    assert_eq!(info.content_id.len(), 0x30);
    assert!(info.content_id.chars().all(|c| c == 'X'));
}

#[test]
fn find_npd_valid_license_values_parse_to_enum_variants() {
    for (wire, expected) in [
        (1u32, NpdLicense::Network),
        (2u32, NpdLicense::Local),
        (3u32, NpdLicense::Free),
    ] {
        let data = build_synthetic_npdrm_record(wire, 0);
        let info = find_npd_header_info(&data).unwrap().unwrap();
        assert_eq!(info.license, expected, "wire value 0x{wire:x}");
    }
}

#[test]
fn find_npd_license_zero_returns_npdrm_bad_license() {
    let data = build_synthetic_npdrm_record(0, 0);
    let err = find_npd_header_info(&data).unwrap_err();
    assert!(matches!(err, SceError::NpdrmBadLicense { got: 0 }));
}

#[test]
fn find_npd_license_four_returns_npdrm_bad_license() {
    let data = build_synthetic_npdrm_record(4, 0);
    let err = find_npd_header_info(&data).unwrap_err();
    assert!(matches!(err, SceError::NpdrmBadLicense { got: 4 }));
}

#[test]
fn find_npd_license_u32_max_returns_npdrm_bad_license() {
    let data = build_synthetic_npdrm_record(u32::MAX, 0);
    let err = find_npd_header_info(&data).unwrap_err();
    assert!(matches!(err, SceError::NpdrmBadLicense { got: u32::MAX }));
}
