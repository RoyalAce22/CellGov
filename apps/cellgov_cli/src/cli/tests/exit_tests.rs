//! PPU-image decrypt helpers: plaintext passthrough and the empty vault
//! a plaintext image is given.

use super::*;

#[test]
fn decrypt_passes_plaintext_elf_through_unchanged() {
    // Non-SCE bytes (ELF magic) are returned verbatim without a RAP
    // lookup, so this never touches the filesystem.
    let mut elf = vec![0x7F, b'E', b'L', b'F'];
    elf.extend_from_slice(&[0u8; 60]);
    let out = decrypt_ppu_self(&elf, "fixture.elf", Path::new("/nonexistent"))
        .expect("plaintext ELF passes through");
    assert_eq!(out, elf);
}

#[test]
fn decrypt_passes_short_non_sce_bytes_through() {
    let bytes = vec![1u8, 2, 3];
    let out = decrypt_ppu_self(&bytes, "x", Path::new("/nonexistent"))
        .expect("short plaintext input passes through");
    assert_eq!(out, bytes);
}

#[test]
fn a_plaintext_image_is_given_an_empty_vault_without_reading_any_keys() {
    let mut elf = vec![0x7F, b'E', b'L', b'F'];
    elf.extend_from_slice(&[0u8; 60]);
    let keys = crate::cli::keys::key_vault_for(&elf)
        .expect("plaintext image gets an empty vault without loading keys");
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

/// A stopped walk ends with the keys hint only when the vault refused;
/// a refused RAP names the file, and a hint about keys would mislead.
#[test]
fn only_a_vault_refusal_that_stops_the_walk_carries_the_keys_hint() {
    let stopped = |source| EbootLoadError::Stopped {
        path: PathBuf::from("EBOOT.BIN"),
        title: "t".to_string(),
        source: Box::new(source),
    };
    assert!(stopped_by_the_vault(&stopped(SceError::NoAppKey {
        revision: 0x0A
    })));
    assert!(!stopped_by_the_vault(&stopped(SceError::RapRead {
        content_id: "NPAA00001".to_string(),
        source: cellgov_install::npdrm::RapReadError::Missing {
            path: PathBuf::from("NPAA00001.rap"),
        },
    })));
    assert!(!stopped_by_the_vault(&stopped(
        SceError::DecryptFeatureDisabled
    )));
}

#[cfg(not(feature = "decrypt"))]
#[test]
fn without_the_decrypt_feature_an_sce_wrapper_is_given_an_empty_vault() {
    let keys = crate::cli::keys::key_vault_for(&cellgov_ps3_abi::format::sce::SCE_MAGIC)
        .expect("decrypt-disabled build does not consult the vault");
    assert!(keys.sources().is_empty());
    let same = crate::cli::keys::try_key_vault_for(&cellgov_ps3_abi::format::sce::SCE_MAGIC)
        .expect("no vault is consulted without the feature");
    assert!(same.sources().is_empty());
}
