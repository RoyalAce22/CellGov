//! Disc-install refusal of an image that still carries its disc
//! encryption.

#![cfg_attr(not(feature = "decrypt"), allow(unused_imports, dead_code))]

use super::*;
use crate::scratch_dir::scratch;
use crate::test_support::{build_iso, build_param_sfo, IsoNode};

#[cfg(feature = "decrypt")]
fn keys() -> KeyVault {
    crate::test_support::synthetic_vault()
}

/// A disc tree whose PARAM.SFO is well-formed and whose EBOOT is
/// `eboot`.
fn disc_with_eboot(eboot: Vec<u8>) -> Vec<u8> {
    build_iso(vec![IsoNode::Dir(
        "PS3_GAME",
        vec![
            IsoNode::File(
                "PARAM.SFO",
                build_param_sfo(&[("TITLE_ID", "BCES00664"), ("CATEGORY", "DG")]),
            ),
            IsoNode::Dir("USRDIR", vec![IsoNode::File("EBOOT.BIN", eboot)]),
        ],
    )])
}

#[cfg(feature = "decrypt")]
#[test]
fn an_eboot_without_an_sce_or_elf_magic_is_refused_as_still_encrypted_before_staging() {
    let image = disc_with_eboot(vec![0x8a, 0x3f, 0xc1, 0x07, 0x55, 0x19]);
    let out = scratch();
    let vfs = out.join("vfs");
    let err = install_iso(&image, &keys(), &vfs, InstallOptions::default()).unwrap_err();
    assert!(
        matches!(
            err,
            GameInstallError::DiscImageEncrypted {
                path: "PS3_GAME/USRDIR/EBOOT.BIN",
                head: [0x8a, 0x3f, 0xc1, 0x07],
            }
        ),
        "{err:?}"
    );
    let rendered = err.to_string();
    assert!(rendered.contains("0x8a3fc107"), "{rendered}");
    assert!(rendered.contains("decrypted dump"), "{rendered}");
    assert!(
        !vfs.join("dev_bdvd").exists(),
        "refused before the tree was staged"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn a_param_sfo_without_its_magic_is_refused_as_still_encrypted() {
    let image = build_iso(vec![IsoNode::Dir(
        "PS3_GAME",
        vec![IsoNode::File("PARAM.SFO", {
            // Header-sized, so it reaches the magic check instead of
            // failing the length check.
            let mut ciphertext = vec![0xde, 0xad, 0xbe, 0xef];
            ciphertext.extend((4..32u8).map(|i| i.wrapping_mul(0x9d)));
            ciphertext
        })],
    )]);
    let out = scratch();
    let err =
        install_iso(&image, &keys(), &out.join("vfs"), InstallOptions::default()).unwrap_err();
    assert!(
        matches!(
            err,
            GameInstallError::DiscImageEncrypted {
                path: "PS3_GAME/PARAM.SFO",
                head: [0xde, 0xad, 0xbe, 0xef],
            }
        ),
        "{err:?}"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn a_param_sfo_that_is_plaintext_but_malformed_keeps_its_own_error() {
    let image = build_iso(vec![IsoNode::Dir(
        "PS3_GAME",
        vec![IsoNode::File("PARAM.SFO", b"\0PSF".to_vec())],
    )]);
    let out = scratch();
    let err =
        install_iso(&image, &keys(), &out.join("vfs"), InstallOptions::default()).unwrap_err();
    assert!(
        matches!(
            err,
            GameInstallError::Sfo(param_sfo::SfoError::TooSmall { .. })
        ),
        "{err:?}"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn a_plain_elf_eboot_passes_the_encryption_check_and_reaches_the_proof() {
    let mut eboot = ELF_MAGIC.to_vec();
    eboot.extend_from_slice(b" not a SELF either");
    let image = disc_with_eboot(eboot);
    let out = scratch();
    let err =
        install_iso(&image, &keys(), &out.join("vfs"), InstallOptions::default()).unwrap_err();
    assert!(
        matches!(err, GameInstallError::DecryptProof(_)),
        "a plaintext ELF is not an encrypted image; the proof decides it: {err:?}"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn an_eboot_shorter_than_a_magic_is_truncated_not_encrypted() {
    // Per-sector encryption keeps a file's length, so three bytes can
    // not be ciphertext of a SELF; the proof refuses it by length.
    for eboot in [Vec::new(), vec![0x8a, 0x3f, 0xc1]] {
        let image = disc_with_eboot(eboot);
        let out = scratch();
        let err =
            install_iso(&image, &keys(), &out.join("vfs"), InstallOptions::default()).unwrap_err();
        assert!(
            matches!(
                err,
                GameInstallError::DecryptProof(sce::SceError::TooSmall { .. })
            ),
            "{err:?}"
        );
    }
}
