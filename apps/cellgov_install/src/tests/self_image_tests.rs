use std::borrow::Cow;

use super::{into_plaintext_elf, is_sce_wrapped, to_plaintext_elf, KeyPolicy};
use crate::sce::SceError;
use crate::test_support::build_npdrm_eboot_header;
use cellgov_ps3_abi::elf::ELF_MAGIC;
use cellgov_ps3_abi::sce::{SCE_MAGIC, SCE_SUPPLEMENTAL_KIND_NPDRM};

fn plaintext_image() -> Vec<u8> {
    let mut v = vec![0u8; 64];
    v[0..4].copy_from_slice(&ELF_MAGIC);
    v
}

/// SCE wrapper carrying one supplemental record of `kind` with a
/// `body_len`-byte body. `body_len` under 0x80 truncates an NPD body;
/// `kind` other than NPDRM exercises the no-NPDRM-record walk.
fn sce_wrapper_with_supplemental(kind: u32, body_len: usize) -> Vec<u8> {
    const SUPP_OFF: usize = 0x80;
    let record_size = 0x10 + body_len;
    let mut buf = vec![0u8; SUPP_OFF + record_size];
    buf[0..4].copy_from_slice(&SCE_MAGIC);
    buf[0x58..0x60].copy_from_slice(&(SUPP_OFF as u64).to_be_bytes());
    buf[0x60..0x68].copy_from_slice(&(record_size as u64).to_be_bytes());
    buf[SUPP_OFF..SUPP_OFF + 4].copy_from_slice(&kind.to_be_bytes());
    buf[SUPP_OFF + 4..SUPP_OFF + 8].copy_from_slice(&(record_size as u32).to_be_bytes());
    buf
}

#[test]
fn a_buffer_shorter_than_the_magic_is_not_sce_wrapped() {
    assert!(!is_sce_wrapped(b"SCE"));
    assert!(!is_sce_wrapped(&[]));
    assert!(is_sce_wrapped(b"SCE\0"));
}

#[test]
fn a_plaintext_image_is_borrowed_through_unchanged() {
    let raw = plaintext_image();
    let out = to_plaintext_elf(&raw, KeyPolicy::AppOnly).expect("plaintext passes through");
    assert!(matches!(out, Cow::Borrowed(_)));
    assert_eq!(&*out, raw.as_slice());
}

#[test]
fn into_plaintext_elf_moves_a_plaintext_image_without_reallocating() {
    let raw = plaintext_image();
    let addr = raw.as_ptr();
    let out = into_plaintext_elf(raw, KeyPolicy::AppOnly).expect("plaintext passes through");
    // Pointer identity is the assertion: the pass-through arm exists
    // to keep a multi-megabyte firmware module off a second copy.
    assert_eq!(out.as_ptr(), addr);
}

#[test]
fn an_npdrm_self_under_app_only_policy_is_refused_by_name() {
    let raw = build_npdrm_eboot_header(1, "UP0001-CGOV00001_00-TESTTESTTESTTEST");
    let err = to_plaintext_elf(&raw, KeyPolicy::AppOnly).expect_err("APP-only cannot open NPDRM");
    match err {
        SceError::NpdrmUnderAppOnlyPolicy {
            content_id,
            license,
        } => {
            assert_eq!(content_id, "UP0001-CGOV00001_00-TESTTESTTESTTEST");
            assert_eq!(license, 1);
        }
        other => panic!("expected NpdrmUnderAppOnlyPolicy, got {other:?}"),
    }
}

#[test]
fn a_non_npdrm_wrapper_under_app_only_policy_reaches_the_decrypt() {
    let mut raw = vec![0u8; 0x20];
    raw[0..4].copy_from_slice(b"SCE\0");
    let err = to_plaintext_elf(&raw, KeyPolicy::AppOnly).expect_err("revision 0 has no APP key");
    assert!(
        !matches!(err, SceError::NpdrmUnderAppOnlyPolicy { .. }),
        "a wrapper with no NPDRM supplemental must fall through to the decrypt, got {err:?}"
    );
}

#[test]
fn a_walkable_chain_with_no_npdrm_record_under_app_only_policy_reaches_the_decrypt() {
    let raw = sce_wrapper_with_supplemental(
        cellgov_ps3_abi::sce::SCE_SUPPLEMENTAL_KIND_PLAINTEXT_CAPABILITY,
        0x20,
    );
    let err = to_plaintext_elf(&raw, KeyPolicy::AppOnly)
        .expect_err("zeroed key material fails the envelope padding check");
    assert!(
        matches!(err, SceError::KeyEnvelopePadding),
        "a capability-only chain must reach the APP decrypt, got {err:?}"
    );
}

#[test]
fn an_npdrm_record_with_an_unrecognized_license_is_not_retried_under_app_keys() {
    for wire in [0u32, 4, u32::MAX] {
        let raw = build_npdrm_eboot_header(wire, "UP0001-CGOV00001_00-TESTTESTTESTTEST");
        let err = to_plaintext_elf(&raw, KeyPolicy::AppOnly)
            .expect_err("an NPDRM record is present, so APP keys cannot open it");
        assert!(
            matches!(err, SceError::NpdrmBadLicense { got } if got == wire),
            "license 0x{wire:x} must surface as NpdrmBadLicense, got {err:?}"
        );
    }
}

#[test]
fn an_npdrm_record_with_a_truncated_npd_body_is_not_retried_under_app_keys() {
    // Record body 0x10 bytes: the NPD needs 0x80, so the body parse
    // fails after the record has already settled the key class.
    let raw = sce_wrapper_with_supplemental(SCE_SUPPLEMENTAL_KIND_NPDRM, 0x10);
    let err = to_plaintext_elf(&raw, KeyPolicy::AppOnly)
        .expect_err("an NPDRM record is present, so APP keys cannot open it");
    assert!(
        matches!(err, SceError::HeaderOffsetOutOfRange { .. }),
        "a truncated NPD body must surface its own error, got {err:?}"
    );
}

#[test]
fn an_npdrm_self_under_auto_policy_is_not_refused_for_lack_of_a_key_path() {
    let raw = build_npdrm_eboot_header(1, "UP0001-CGOV00001_00-TESTTESTTESTTEST");
    let resolver = |_: &crate::npdrm::NpdHeaderInfo| Some([0u8; 16]);
    let err = to_plaintext_elf(&raw, KeyPolicy::Auto(&resolver))
        .expect_err("the synthetic header is not a decryptable SELF");
    assert!(
        !matches!(err, SceError::NpdrmUnderAppOnlyPolicy { .. }),
        "Auto has a klicensee path, so the APP-only refusal must not fire, got {err:?}"
    );
}
