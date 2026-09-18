//! NPDRM klicensee resolution: RAP-to-klic derivation, NPD header parsing
//! over the supplemental-header walk, and debug-SELF rejection.

use super::*;
#[cfg(feature = "decrypt")]
use crate::keys::Slot;
#[cfg(feature = "decrypt")]
use crate::test_support::synthetic_vault;
#[cfg(feature = "decrypt")]
use std::path::Path;

#[cfg(feature = "decrypt")]
#[test]
fn rap_to_klic_is_pure() {
    let keys = synthetic_vault();
    let rap = [0x42u8; 16];
    let a = rap_to_klic(&keys, &rap).unwrap();
    let b = rap_to_klic(&keys, &rap).unwrap();
    assert_eq!(a, b);
}

#[cfg(feature = "decrypt")]
#[test]
fn rap_to_klic_is_not_a_constant_or_the_identity() {
    let keys = synthetic_vault();
    let a = rap_to_klic(&keys, &[0x42u8; 16]).unwrap();
    let b = rap_to_klic(&keys, &[0x43u8; 16]).unwrap();
    assert_ne!(a, b, "distinct RAPs derive distinct klics");
    assert_ne!(a, [0x42u8; 16], "the RAP is transformed, not echoed");
}

#[cfg(feature = "decrypt")]
#[test]
fn rap_to_klic_on_a_vault_missing_a_rap_slot_refuses_naming_that_slot() {
    let rep = |byte: u8| format!("{byte:02x}").repeat(16);
    let toml = format!(
        "rap_key = \"{}\"\nrap_pbox = \"000102030405060708090a0b0c0d0e0f\"\nrap_e2 = \"{}\"\n",
        rep(0x55),
        rep(0x58)
    );
    let keys = KeyVault::parse(Path::new("partial.toml"), toml.as_bytes()).unwrap();
    let err = rap_to_klic(&keys, &[0x42u8; 16]).unwrap_err();
    let SceError::Keys(inner) = &err else {
        panic!("expected the vault's refusal, got {err:?}");
    };
    assert!(
        matches!(**inner, KeyVaultError::MissingSlot { slot: Slot::RapE1 }),
        "expected the first absent RAP slot to be named, got {inner:?}"
    );
    assert!(err.to_string().contains("rap_e1"), "{err}");
}

#[cfg(feature = "decrypt")]
fn vault_with_pbox(pbox_hex: &str) -> KeyVault {
    let rep = |byte: u8| format!("{byte:02x}").repeat(16);
    let toml = format!(
        "rap_key = \"{}\"\nrap_pbox = \"{pbox_hex}\"\nrap_e1 = \"{}\"\nrap_e2 = \"{}\"\n",
        rep(0x55),
        rep(0x57),
        rep(0x58)
    );
    KeyVault::parse(Path::new("pbox.toml"), toml.as_bytes()).unwrap()
}

#[cfg(feature = "decrypt")]
#[test]
fn rap_to_klic_refuses_a_pbox_entry_past_15_instead_of_masking_it() {
    // Entry 3 is 0x13, past the last table index.
    let keys = vault_with_pbox("000102130405060708090a0b0c0d0e0f");
    let err = rap_to_klic(&keys, &[0x42u8; 16]).unwrap_err();
    assert!(
        matches!(err, SceError::RapPboxNotAPermutation { index: 3 }),
        "got {err:?}"
    );
    assert!(err.to_string().contains("rap_pbox"), "{err}");
}

#[cfg(feature = "decrypt")]
#[test]
fn rap_to_klic_refuses_a_pbox_that_repeats_an_index() {
    // Every entry is in range; entry 5 revisits position 2, so
    // position 5 is never touched by any round.
    let keys = vault_with_pbox("000102030402060708090a0b0c0d0e0f");
    let err = rap_to_klic(&keys, &[0x42u8; 16]).unwrap_err();
    assert!(
        matches!(err, SceError::RapPboxNotAPermutation { index: 5 }),
        "got {err:?}"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn rap_to_klic_accepts_a_pbox_that_permutes_out_of_order() {
    let keys = vault_with_pbox("0f0e0d0c0b0a09080706050403020100");
    rap_to_klic(&keys, &[0x42u8; 16]).expect("a reversed permutation is still a permutation");
}

#[cfg(feature = "decrypt")]
fn npd(license: NpdLicense, content_id: &str) -> NpdHeaderInfo {
    NpdHeaderInfo {
        license,
        content_id: content_id.to_string(),
    }
}

#[cfg(feature = "decrypt")]
#[test]
fn resolve_klicensee_license_network_with_rap_derives_the_klic() {
    let keys = synthetic_vault();
    let rap = [0xABu8; 16];
    let got = resolve_npdrm_klicensee(&keys, &npd(NpdLicense::Network, "NPUA80001"), |_| {
        Some(Rap(rap))
    })
    .unwrap();
    assert_eq!(got, rap_to_klic(&keys, &rap).unwrap());
}

#[cfg(feature = "decrypt")]
#[test]
fn resolve_klicensee_license_local_with_rap_derives_the_klic() {
    let keys = synthetic_vault();
    let rap = [0xCDu8; 16];
    let got = resolve_npdrm_klicensee(&keys, &npd(NpdLicense::Local, "NPUA80068"), |_| {
        Some(Rap(rap))
    })
    .unwrap();
    assert_eq!(got, rap_to_klic(&keys, &rap).unwrap());
}

#[cfg(feature = "decrypt")]
#[test]
fn resolve_klicensee_license_network_without_rap_errors_with_content_id() {
    let keys = synthetic_vault();
    let err = resolve_npdrm_klicensee(&keys, &npd(NpdLicense::Network, "NPUA80001"), |_| None)
        .unwrap_err();
    match err {
        SceError::NoRapForNpdrmTitle { content_id } => {
            assert_eq!(content_id, "NPUA80001");
        }
        other => panic!("expected NoRapForNpdrmTitle, got {other:?}"),
    }
}

#[cfg(feature = "decrypt")]
#[test]
fn resolve_klicensee_license_local_without_rap_errors_with_content_id() {
    let keys = synthetic_vault();
    let err =
        resolve_npdrm_klicensee(&keys, &npd(NpdLicense::Local, "NPUA80068"), |_| None).unwrap_err();
    match err {
        SceError::NoRapForNpdrmTitle { content_id } => {
            assert_eq!(content_id, "NPUA80068");
        }
        other => panic!("expected NoRapForNpdrmTitle, got {other:?}"),
    }
}

#[cfg(feature = "decrypt")]
#[test]
fn resolve_klicensee_license_free_without_rap_returns_the_vaults_free_klicensee() {
    let keys = synthetic_vault();
    let got =
        resolve_npdrm_klicensee(&keys, &npd(NpdLicense::Free, "NPEA00000"), |_| None).unwrap();
    assert_eq!(got, *keys.np_klic_free().unwrap());
}

#[cfg(feature = "decrypt")]
#[test]
fn resolve_klicensee_license_free_with_rap_derives_the_supplied_rap() {
    let keys = synthetic_vault();
    let rap = [0x77u8; 16];
    let got = resolve_npdrm_klicensee(&keys, &npd(NpdLicense::Free, "NPEA00000"), |_| {
        Some(Rap(rap))
    })
    .unwrap();
    assert_eq!(got, rap_to_klic(&keys, &rap).unwrap());
    assert_ne!(
        got,
        *keys.np_klic_free().unwrap(),
        "a supplied RAP wins over the free key"
    );
}

#[cfg(feature = "decrypt")]
/// A SELF-shaped container (0x100 bytes) carrying the given
/// `revision_flags` and a program identification header at 0xC0 typed
/// APP; every other field is zero. The APP type lets the decrypt reach
/// its key lookup.
fn synthetic_sce_header_with_revision_flags(revision_flags: u16) -> Vec<u8> {
    let mut data = vec![0u8; 0x100];
    data[0..4].copy_from_slice(b"SCE\0");
    data[8..10].copy_from_slice(&revision_flags.to_be_bytes());
    data[0x28..0x30].copy_from_slice(&0xC0u64.to_be_bytes());
    data[0xCC..0xD0]
        .copy_from_slice(&cellgov_ps3_abi::format::sce::SELF_PROGRAM_TYPE_APP.to_be_bytes());
    data
}

#[cfg(feature = "decrypt")]
#[test]
fn the_app_decrypt_path_rejects_a_debug_self_by_name() {
    let keys = synthetic_vault();
    let data = synthetic_sce_header_with_revision_flags(0x8000);
    let err = crate::sce::decrypt_self_to_elf(&data, &keys).unwrap_err();
    assert!(matches!(
        err,
        SceError::DebugSelfUnsupported {
            revision_flags: 0x8000
        }
    ));
}

#[cfg(feature = "decrypt")]
#[test]
fn the_app_decrypt_path_names_a_revision_the_vault_has_no_keyset_for() {
    let keys = synthetic_vault();
    // The synthetic vault labels revision 0x0001 only and holds no
    // unlabeled candidate, so 0x0002 reaches the key lookup with
    // nothing to try.
    let data = synthetic_sce_header_with_revision_flags(0x0002);
    let err = crate::sce::decrypt_self_to_elf(&data, &keys).unwrap_err();
    assert!(
        matches!(err, SceError::NoAppKey { revision: 2 }),
        "expected NoAppKey for revision 2, got {err:?}"
    );
    assert!(err.to_string().contains("0x0002"), "{err}");
}

#[cfg(feature = "decrypt")]
#[test]
fn decrypt_self_to_elf_npdrm_rejects_debug_self_with_high_bit_set() {
    let keys = synthetic_vault();
    let data = synthetic_sce_header_with_revision_flags(0x8000);
    let dummy_klic = [0u8; 16];
    let err = decrypt_self_to_elf_npdrm(&data, &keys, &dummy_klic).unwrap_err();
    match err {
        SceError::DebugSelfUnsupported { revision_flags } => {
            assert_eq!(revision_flags, 0x8000);
        }
        other => panic!("expected DebugSelfUnsupported, got {other:?}"),
    }
}

#[cfg(feature = "decrypt")]
#[test]
fn decrypt_self_to_elf_npdrm_rejects_debug_self_with_both_bits_set() {
    // High bit AND a non-zero revision in the low 15 bits:
    // guard must fire on the raw value, error carries it whole.
    let keys = synthetic_vault();
    let data = synthetic_sce_header_with_revision_flags(0xC042);
    let dummy_klic = [0u8; 16];
    let err = decrypt_self_to_elf_npdrm(&data, &keys, &dummy_klic).unwrap_err();
    assert!(matches!(
        err,
        SceError::DebugSelfUnsupported {
            revision_flags: 0xC042
        }
    ));
}

#[cfg(feature = "decrypt")]
#[test]
fn decrypt_self_to_elf_npdrm_does_not_treat_high_revision_as_debug() {
    // 0x7FFF: highest non-debug revision. It clears the debug guard
    // and reaches the key lookup, which has no NPDRM keyset for that
    // revision. Pinning the variant keeps this from passing on any
    // failure that never got past the guard at all.
    let keys = synthetic_vault();
    let data = synthetic_sce_header_with_revision_flags(0x7FFF);
    let dummy_klic = [0u8; 16];
    let err = decrypt_self_to_elf_npdrm(&data, &keys, &dummy_klic).unwrap_err();
    assert!(
        matches!(err, SceError::NoNpdrmKey { revision: 0x7FFF }),
        "expected the key lookup to be reached, got {err:?}"
    );
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
    assert!(
        matches!(
            err,
            SceError::HeaderOffsetOutOfRange {
                what: "SELF supplemental header record body"
            }
        ),
        "a record smaller than its own header must be named as such, got {err:?}"
    );
}

#[test]
fn find_npd_a_record_claiming_more_bytes_than_the_chain_holds_returns_typed_error() {
    let mut records = vec![0u8; 0x10];
    records[0..4].copy_from_slice(&1u32.to_be_bytes());
    records[4..8].copy_from_slice(&u32::MAX.to_be_bytes());
    let data = build_synthetic_supplemental_chain(&records);
    let err = find_npd_header_info(&data).unwrap_err();
    assert!(
        matches!(
            err,
            SceError::HeaderOffsetOutOfRange {
                what: "SELF supplemental header record body"
            }
        ),
        "a record body running past the chain must be named as such, got {err:?}"
    );
}

#[test]
fn find_npd_a_record_whose_body_is_shorter_than_the_npd_header_returns_typed_error() {
    // kind=NPDRM at the minimum record size of 0x10: the record walks
    // and settles the key class, but its body is empty where the NPD
    // header needs 0x80 bytes.
    let mut records = vec![0u8; 0x10];
    records[0..4].copy_from_slice(&SCE_SUPPLEMENTAL_KIND_NPDRM.to_be_bytes());
    records[4..8].copy_from_slice(&0x10u32.to_be_bytes());
    let data = build_synthetic_supplemental_chain(&records);
    let err = find_npd_header_info(&data).unwrap_err();
    assert!(
        matches!(
            err,
            SceError::HeaderOffsetOutOfRange {
                what: "NPDRM supplemental NPD body"
            }
        ),
        "the NPD body parse must name itself, not the chain walk, got {err:?}"
    );
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
