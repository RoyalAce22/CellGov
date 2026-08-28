//! PPU-image decrypt helpers: plaintext passthrough and key-vault
//! refusal classification.

use super::*;

#[test]
fn decrypt_passes_plaintext_elf_through_unchanged() {
    // Non-SCE bytes (ELF magic) are returned verbatim without a RAP
    // lookup, so this never touches the filesystem.
    let mut elf = vec![0x7F, b'E', b'L', b'F'];
    elf.extend_from_slice(&[0u8; 60]);
    let out = decrypt_ppu_self_or_die(&elf, "fixture.elf", Path::new("/nonexistent"));
    assert_eq!(out, elf);
}

#[test]
fn decrypt_passes_short_non_sce_bytes_through() {
    let bytes = vec![1u8, 2, 3];
    let out = decrypt_ppu_self_or_die(&bytes, "x", Path::new("/nonexistent"));
    assert_eq!(out, bytes);
}

#[test]
fn a_plaintext_image_is_given_an_empty_vault_without_reading_any_keys() {
    let mut elf = vec![0x7F, b'E', b'L', b'F'];
    elf.extend_from_slice(&[0u8; 60]);
    let keys = crate::cli::keys::key_vault_for(&elf);
    assert!(
        keys.sources().is_empty(),
        "no keyfile is read for a plaintext image"
    );
    assert!(
        !keys.missing_for_decrypt().is_empty(),
        "the vault a plaintext image gets holds nothing"
    );
    let plain = crate::cli::keys::try_key_vault_for(&elf).expect("plaintext never fails");
    assert!(plain.sources().is_empty());
}

#[cfg(not(feature = "decrypt"))]
#[test]
fn without_the_decrypt_feature_an_sce_wrapper_is_given_an_empty_vault() {
    let keys = crate::cli::keys::key_vault_for(&cellgov_ps3_abi::sce::SCE_MAGIC);
    assert!(keys.sources().is_empty());
    let same = crate::cli::keys::try_key_vault_for(&cellgov_ps3_abi::sce::SCE_MAGIC)
        .expect("no vault is consulted without the feature");
    assert!(same.sources().is_empty());
}

#[test]
fn a_vault_that_lacks_the_keyset_is_a_run_level_refusal() {
    use cellgov_install::keys::{KeyVaultError, Slot};
    let refusals = [
        SceError::Keys(Box::new(KeyVaultError::MissingSlot {
            slot: Slot::NpKlicFree,
        })),
        SceError::Keys(Box::new(KeyVaultError::MissingScepkg)),
        SceError::NoAppKey { revision: 0x0A },
        SceError::NoNpdrmKey { revision: 0x0A },
        SceError::RapPboxNotAPermutation { index: 3 },
    ];
    for e in &refusals {
        assert!(is_key_vault_refusal(e), "{e}");
    }
}

#[test]
fn a_refusal_that_names_the_image_or_its_rap_is_not_a_run_level_refusal() {
    let image_side = [
        SceError::KeyEnvelopePadding,
        SceError::AesCbcDecryptFailed,
        SceError::NoCandidateOpensEnvelope {
            class: "APP",
            revision: 0x0A,
            tried: 2,
        },
        SceError::NoRapForNpdrmTitle {
            content_id: "UP0001-CGOV00001_00-TESTTESTTESTTEST".into(),
        },
        SceError::DecryptFeatureDisabled,
        SceError::DebugSelfUnsupported {
            revision_flags: 0x8001,
        },
        SceError::BadMagic { got: 0 },
    ];
    for e in &image_side {
        assert!(!is_key_vault_refusal(e), "{e}");
    }
}
