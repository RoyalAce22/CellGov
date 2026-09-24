//! SELF-vs-plaintext detection and the key policy that decides whether
//! an NPDRM wrapper may be opened at all.

use std::borrow::Cow;

use super::{into_plaintext_elf, is_sce_wrapped, open_ppu_image, to_plaintext_elf, KeyPolicy};
use crate::keys::KeyVault;
use crate::sce::SceError;
use crate::test_support::build_npdrm_eboot_header;
#[cfg(feature = "decrypt")]
use crate::test_support::synthetic_vault;
use cellgov_ps3_abi::format::elf::ELF_MAGIC;
use cellgov_ps3_abi::format::sce::SCE_MAGIC;
#[cfg(feature = "decrypt")]
use cellgov_ps3_abi::format::sce::SCE_SUPPLEMENTAL_KIND_NPDRM;

fn plaintext_image() -> Vec<u8> {
    let mut v = vec![0u8; 64];
    v[0..4].copy_from_slice(&ELF_MAGIC);
    v
}

#[cfg(feature = "decrypt")]
/// The one SELF revision the synthetic vault labels a keyset for, so
/// a wrapper carrying it reaches the envelope decrypt instead of the
/// no-key refusal.
const SYNTHETIC_KEYED_REVISION: u16 = 0x0001;

#[cfg(feature = "decrypt")]
/// SCE wrapper at [`SYNTHETIC_KEYED_REVISION`] carrying one
/// supplemental record of `kind` with a `body_len`-byte body.
/// `body_len` under 0x80 truncates an NPD body; `kind` other than
/// NPDRM exercises the no-NPDRM-record walk.
fn sce_wrapper_with_supplemental(kind: u32, body_len: usize) -> Vec<u8> {
    const SUPP_OFF: usize = 0x80;
    let record_size = 0x10 + body_len;
    let mut buf = vec![0u8; SUPP_OFF + record_size];
    buf[0..4].copy_from_slice(&SCE_MAGIC);
    buf[8..10].copy_from_slice(&SYNTHETIC_KEYED_REVISION.to_be_bytes());
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
    // An empty vault: a plaintext image never asks it for anything.
    let out = to_plaintext_elf(&raw, &KeyVault::empty(), KeyPolicy::AppOnly)
        .expect("plaintext passes through");
    assert!(matches!(out, Cow::Borrowed(_)));
    assert_eq!(&*out, raw.as_slice());
}

#[test]
fn into_plaintext_elf_moves_a_plaintext_image_without_reallocating() {
    let raw = plaintext_image();
    let addr = raw.as_ptr();
    let out = into_plaintext_elf(raw, &KeyVault::empty(), KeyPolicy::AppOnly)
        .expect("plaintext passes through");
    // Pointer identity keeps a multi-megabyte firmware module off a
    // second copy.
    assert_eq!(out.as_ptr(), addr);
}

#[cfg(feature = "decrypt")]
#[test]
fn an_npdrm_self_under_app_only_policy_is_refused_by_name() {
    let raw = build_npdrm_eboot_header(1, "UP0001-CGOV00001_00-TESTTESTTESTTEST");
    let err = to_plaintext_elf(&raw, &synthetic_vault(), KeyPolicy::AppOnly)
        .expect_err("APP-only cannot open NPDRM");
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

#[cfg(feature = "decrypt")]
#[test]
fn a_wrapper_with_an_empty_supplemental_chain_under_app_only_policy_reaches_the_decrypt() {
    // Extended header present, supplemental_hdr_size = 0: the walk
    // completes and finds no NPDRM record, which is the only shape
    // that clears an image for APP keys.
    let mut raw = vec![0u8; 0x68];
    raw[0..4].copy_from_slice(&SCE_MAGIC);
    raw[8..10].copy_from_slice(&SYNTHETIC_KEYED_REVISION.to_be_bytes());
    let err = to_plaintext_elf(&raw, &synthetic_vault(), KeyPolicy::AppOnly)
        .expect_err("zeroed key material fails the envelope padding check");
    assert!(
        matches!(err, SceError::KeyEnvelopePadding),
        "an empty chain must fall through to the APP decrypt, got {err:?}"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn a_wrapper_too_short_for_the_extended_header_is_refused_by_name() {
    // The chain cannot be read at all, so nothing establishes the key
    // class.
    let mut raw = vec![0u8; 0x20];
    raw[0..4].copy_from_slice(&SCE_MAGIC);
    let err = to_plaintext_elf(&raw, &synthetic_vault(), KeyPolicy::AppOnly)
        .expect_err("0x20 bytes is not a SELF");
    assert!(
        matches!(
            err,
            SceError::TooSmall {
                what: "SELF extended header",
                ..
            }
        ),
        "a truncated extended header must surface as itself, got {err:?}"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn a_supplemental_chain_that_escapes_the_buffer_is_refused_by_name() {
    let mut raw = vec![0u8; 0x100];
    raw[0..4].copy_from_slice(&SCE_MAGIC);
    raw[0x58..0x60].copy_from_slice(&0x80u64.to_be_bytes());
    raw[0x60..0x68].copy_from_slice(&0x1000u64.to_be_bytes());
    let err = to_plaintext_elf(&raw, &synthetic_vault(), KeyPolicy::AppOnly)
        .expect_err("the chain runs past the buffer");
    assert!(
        matches!(
            err,
            SceError::HeaderOffsetOutOfRange {
                what: "SELF supplemental headers"
            }
        ),
        "an unwalkable chain must surface as itself, got {err:?}"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn a_walkable_chain_with_no_npdrm_record_under_app_only_policy_reaches_the_decrypt() {
    let raw = sce_wrapper_with_supplemental(
        cellgov_ps3_abi::format::sce::SCE_SUPPLEMENTAL_KIND_PLAINTEXT_CAPABILITY,
        0x20,
    );
    let err = to_plaintext_elf(&raw, &synthetic_vault(), KeyPolicy::AppOnly)
        .expect_err("zeroed key material fails the envelope padding check");
    assert!(
        matches!(err, SceError::KeyEnvelopePadding),
        "a capability-only chain must reach the APP decrypt, got {err:?}"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn an_npdrm_record_with_an_unrecognized_license_is_not_retried_under_app_keys() {
    for wire in [0u32, 4, u32::MAX] {
        let raw = build_npdrm_eboot_header(wire, "UP0001-CGOV00001_00-TESTTESTTESTTEST");
        let err = to_plaintext_elf(&raw, &synthetic_vault(), KeyPolicy::AppOnly)
            .expect_err("an NPDRM record is present, so APP keys cannot open it");
        assert!(
            matches!(err, SceError::NpdrmBadLicense { got } if got == wire),
            "license 0x{wire:x} must surface as NpdrmBadLicense, got {err:?}"
        );
    }
}

#[cfg(feature = "decrypt")]
#[test]
fn an_npdrm_record_with_a_truncated_npd_body_is_not_retried_under_app_keys() {
    // Record body 0x10 bytes: the NPD needs 0x80, so the body parse
    // fails after the record has already settled the key class.
    let raw = sce_wrapper_with_supplemental(SCE_SUPPLEMENTAL_KIND_NPDRM, 0x10);
    let err = to_plaintext_elf(&raw, &synthetic_vault(), KeyPolicy::AppOnly)
        .expect_err("an NPDRM record is present, so APP keys cannot open it");
    assert!(
        matches!(
            err,
            SceError::HeaderOffsetOutOfRange {
                what: "NPDRM supplemental NPD body"
            }
        ),
        "a truncated NPD body must surface its own error, got {err:?}"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn an_npdrm_self_under_auto_policy_consults_the_rap_lookup_instead_of_refusing() {
    let raw = build_npdrm_eboot_header(1, "UP0001-CGOV00001_00-TESTTESTTESTTEST");
    let consulted = std::cell::Cell::new(0usize);
    let resolver = |npd: &crate::npdrm::NpdHeaderInfo| {
        consulted.set(consulted.get() + 1);
        assert_eq!(npd.content_id, "UP0001-CGOV00001_00-TESTTESTTESTTEST");
        Ok(Some(crate::npdrm::Rap([0u8; 16])))
    };
    let err = to_plaintext_elf(&raw, &synthetic_vault(), KeyPolicy::Auto(&resolver))
        .expect_err("the synthetic header is not a decryptable SELF");
    // Without the counter this passes on any early failure that never
    // reaches a key at all.
    assert_eq!(
        consulted.get(),
        1,
        "Auto must resolve the RAP exactly once, got {err:?}"
    );
    assert!(
        !matches!(err, SceError::NpdrmUnderAppOnlyPolicy { .. }),
        "Auto has a klicensee path, so the APP-only refusal must not fire, got {err:?}"
    );
}

#[test]
fn a_plaintext_image_under_auto_policy_is_borrowed_through_without_consulting_the_rap_lookup() {
    // The RAP lookup is reached only through an NPDRM record, and a
    // plaintext image has none.
    let raw = plaintext_image();
    let consulted = std::cell::Cell::new(0usize);
    let resolver = |_: &crate::npdrm::NpdHeaderInfo| {
        consulted.set(consulted.get() + 1);
        Ok(Some(crate::npdrm::Rap([0u8; 16])))
    };
    let out = to_plaintext_elf(&raw, &KeyVault::empty(), KeyPolicy::Auto(&resolver))
        .expect("plaintext passes through");
    assert!(matches!(out, Cow::Borrowed(_)));
    assert_eq!(&*out, raw.as_slice());
    assert_eq!(
        consulted.get(),
        0,
        "no RAP is asked for on a plaintext image"
    );
}

/// A plaintext image has no SELF headers to read an identity from, and
/// never reaches the vault.
#[test]
fn a_plaintext_image_opens_with_no_identity_and_its_bytes_unchanged() {
    let raw = plaintext_image();
    let image = open_ppu_image(raw.clone(), &KeyVault::empty(), KeyPolicy::AppOnly)
        .expect("plaintext passes through");
    assert_eq!(image.elf, raw);
    assert!(image.identity.is_none());
}

#[cfg(not(feature = "decrypt"))]
#[test]
fn without_the_decrypt_feature_every_sce_wrapper_is_refused_naming_the_feature() {
    let consulted = std::cell::Cell::new(0usize);
    let resolver = |_: &crate::npdrm::NpdHeaderInfo| {
        consulted.set(consulted.get() + 1);
        Ok(Some(crate::npdrm::Rap([0u8; 16])))
    };
    let npdrm = build_npdrm_eboot_header(1, "UP0001-CGOV00001_00-TESTTESTTESTTEST");
    let mut app_keyed = vec![0u8; 0x68];
    app_keyed[0..4].copy_from_slice(&SCE_MAGIC);
    // The shortest buffer `is_sce_wrapped` accepts: the magic alone.
    // The decrypt build would name it `TooSmall`; this build must not
    // read past the magic before refusing.
    let magic_only = SCE_MAGIC.to_vec();
    for raw in [&npdrm, &app_keyed, &magic_only] {
        for policy in [KeyPolicy::AppOnly, KeyPolicy::Auto(&resolver)] {
            // An NPDRM image under APP-only keys is refused by its own
            // name in every build; a rebuild would not open it.
            let npdrm_under_app_only =
                std::ptr::eq(raw, &npdrm) && matches!(policy, KeyPolicy::AppOnly);
            let err = to_plaintext_elf(raw, &KeyVault::empty(), policy)
                .expect_err("no decrypt path in this build");
            if npdrm_under_app_only {
                assert!(
                    matches!(err, SceError::NpdrmUnderAppOnlyPolicy { license: 1, .. }),
                    "expected NpdrmUnderAppOnlyPolicy, got {err:?}"
                );
                continue;
            }
            assert!(
                matches!(err, SceError::DecryptFeatureDisabled),
                "expected DecryptFeatureDisabled, got {err:?}"
            );
            assert!(
                err.to_string().contains("`decrypt`"),
                "the refusal names the feature: {err}"
            );
        }
        let err = into_plaintext_elf(raw.clone(), &KeyVault::empty(), KeyPolicy::Auto(&resolver))
            .expect_err("owned path");
        assert!(
            matches!(err, SceError::DecryptFeatureDisabled),
            "the owned path shares the refusal, got {err:?}"
        );
    }
    assert_eq!(consulted.get(), 0, "no RAP is ever asked for");
}
