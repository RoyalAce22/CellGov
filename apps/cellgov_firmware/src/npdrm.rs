//! RAP -> klicensee derivation and NPDRM SELF decrypt prefix.
//!
//! NPDRM-wrapped SELFs carry an extra AES-128-CBC layer over the SCE
//! metadata-info envelope. Peeling it needs a 16-byte klicensee
//! derived from a RAP file the operator ships alongside the title.
//! This module owns that derivation and the prefix decrypt; the
//! post-envelope flow rejoins [`crate::sce::decrypt_self_to_elf`]'s
//! shared CTR path.
//!
//! Fixture-dependent tests (`include_bytes!` of operator-supplied
//! RAP and EBOOT files under `tools/rpcs3/dev_hdd0/`) live behind
//! the `npdrm-oracle-vectors` feature.

use aes::cipher::{BlockDecrypt, KeyInit};
use cellgov_ps3_abi::sce::{NP_KLIC_FREE, NP_KLIC_KEY, RAP_E1, RAP_E2, RAP_KEY, RAP_PBOX};

use crate::sce::{
    assemble_elf_from_sections, decrypt_envelope, decrypt_sections_from_envelope,
    find_npd_header_info, parse_sce_header, NpdHeaderInfo, NpdLicense, SceError,
};

/// Derive the 16-byte intermediate klicensee (RIF key) from a 16-byte RAP.
///
/// The envelope-peel step further ECB-decrypts the output with
/// `NP_KLIC_KEY` to produce the layer key.
#[must_use]
pub fn rap_to_klic(rap: &[u8; 16]) -> [u8; 16] {
    let cipher = aes::Aes128::new_from_slice(&RAP_KEY).expect("RAP_KEY is 16 bytes");
    let mut key = [0u8; 16];
    key.copy_from_slice(rap);
    cipher.decrypt_block((&mut key).into());

    for _round in 0..5 {
        for &p in RAP_PBOX.iter() {
            let p = p as usize;
            key[p] ^= RAP_E1[p];
        }
        for i in (1..16).rev() {
            let p = RAP_PBOX[i] as usize;
            let pp = RAP_PBOX[i - 1] as usize;
            key[p] ^= key[pp];
        }
        let mut o: u8 = 0;
        for &pi in RAP_PBOX.iter() {
            let p = pi as usize;
            let kc = key[p].wrapping_sub(o);
            let ec2 = RAP_E2[p];
            if o != 1 || kc != 0xFF {
                o = u8::from(kc < ec2);
                key[p] = kc.wrapping_sub(ec2);
            } else {
                // The C reference's else-if chain collapses here:
                // reaching this branch requires o==1 && kc==0xFF, so
                // only the kc-ec2 arm (no `o` update) survives.
                key[p] = kc.wrapping_sub(ec2);
            }
        }
    }

    key
}

/// Derive the AES-128 layer key (which decrypts the NPDRM-wrapped
/// metadata-info envelope) by ECB-decrypting `klicensee` with `NP_KLIC_KEY`.
fn klicensee_to_layer_key(klicensee: &[u8; 16]) -> [u8; 16] {
    let cipher = aes::Aes128::new_from_slice(&NP_KLIC_KEY).expect("NP_KLIC_KEY is 16 bytes");
    let mut layer_key = [0u8; 16];
    layer_key.copy_from_slice(klicensee);
    cipher.decrypt_block((&mut layer_key).into());
    layer_key
}

/// Decrypt an NPDRM-wrapped SELF using the supplied 16-byte klicensee
/// and reconstruct the plaintext ELF.
///
/// # Errors
///
/// Returns [`SceError::AesCbcDecryptFailed`] or
/// [`SceError::KeyEnvelopePadding`] when either layer key is wrong --
/// envelope zero-padding self-certifies a correct decrypt.
pub fn decrypt_self_to_elf_npdrm(data: &[u8], klicensee: &[u8; 16]) -> Result<Vec<u8>, SceError> {
    let hdr = parse_sce_header(data)?;
    // High bit of revision_flags marks an unencrypted debug SELF.
    if hdr.revision_flags & 0x8000 != 0 {
        return Err(SceError::DebugSelfUnsupported {
            revision_flags: hdr.revision_flags,
        });
    }
    let revision = hdr.revision_flags & 0x7FFF;
    let key =
        crate::crypto::npdrm_key_for_revision(revision).ok_or(SceError::NoAppKey { revision })?;
    let layer_key = klicensee_to_layer_key(klicensee);
    let envelope = decrypt_envelope(data, &hdr, &key.erk, &key.riv, Some(&layer_key))?;
    let sections = decrypt_sections_from_envelope(data, &hdr, &envelope)?;
    assemble_elf_from_sections(data, &sections)
}

/// Decrypt a SELF whose key class is not known up-front, dispatching
/// APP-keyed vs NPDRM via the presence of a type-3 supplemental header.
///
/// `klicensee_lookup` is invoked only for NPDRM-wrapped SELFs; the
/// caller is responsible for running [`rap_to_klic`] on the RAP
/// material first. Returning `None` errors with
/// [`SceError::NoRapForNpdrmTitle`] naming the `content_id`.
/// License-3 (free) titles fall back to `NP_KLIC_FREE` when the
/// lookup returns `None`.
pub fn decrypt_self_to_elf_auto(
    data: &[u8],
    klicensee_lookup: impl FnOnce(&NpdHeaderInfo) -> Option<[u8; 16]>,
) -> Result<Vec<u8>, SceError> {
    match find_npd_header_info(data)? {
        None => crate::sce::decrypt_self_to_elf(data),
        Some(npd) => {
            let klicensee = resolve_npdrm_klicensee(&npd, klicensee_lookup)?;
            decrypt_self_to_elf_npdrm(data, &klicensee)
        }
    }
}

/// Resolve the klicensee bytes for an NPDRM SELF given its NPD header.
///
/// `NpdLicense::Network` and `NpdLicense::Local` both resolve from
/// RAP material via the lookup; `None` surfaces as
/// [`SceError::NoRapForNpdrmTitle`]. `NpdLicense::Free` falls back
/// to `NP_KLIC_FREE` when the lookup returns `None`, but honours a
/// supplied klic if one is returned.
fn resolve_npdrm_klicensee(
    npd: &NpdHeaderInfo,
    klicensee_lookup: impl FnOnce(&NpdHeaderInfo) -> Option<[u8; 16]>,
) -> Result<[u8; 16], SceError> {
    match npd.license {
        NpdLicense::Network | NpdLicense::Local => {
            klicensee_lookup(npd).ok_or_else(|| SceError::NoRapForNpdrmTitle {
                content_id: npd.content_id.clone(),
            })
        }
        NpdLicense::Free => Ok(klicensee_lookup(npd).unwrap_or(NP_KLIC_FREE)),
    }
}

#[cfg(test)]
mod tests {
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
        let got = resolve_npdrm_klicensee(&npd(NpdLicense::Network, "NPUA80001"), |_| Some(want))
            .unwrap();
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
        let err =
            resolve_npdrm_klicensee(&npd(NpdLicense::Local, "NPUA80068"), |_| None).unwrap_err();
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
        let got =
            resolve_npdrm_klicensee(&npd(NpdLicense::Free, "NPEA00000"), |_| Some(want)).unwrap();
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
}

#[cfg(all(test, feature = "npdrm-oracle-vectors"))]
mod oracle_vectors {
    use super::*;

    const FLOW_RAP: [u8; 16] = include_bytes_array_flow_rap();
    /// flOw klic witness, frozen against `FLOW_RAP` and a fixed
    /// reverse-engineered algorithm. Localization aid: if the
    /// end-to-end ELF test flips and this test also flips, the
    /// regression is in `rap_to_klic`; otherwise downstream.
    const FLOW_EXPECTED_KLIC: [u8; 16] = [
        0x04, 0x43, 0xFA, 0x57, 0x9C, 0xB8, 0xEF, 0xBF, 0xE5, 0xA8, 0x98, 0xAE, 0xF2, 0x81, 0x8E,
        0xC1,
    ];

    const SSHD_RAP: [u8; 16] = include_bytes_array_sshd_rap();
    const SSHD_EXPECTED_KLIC: [u8; 16] = [
        0xDA, 0x60, 0x18, 0x39, 0xD4, 0x18, 0xCF, 0x8C, 0x91, 0xEC, 0xDE, 0x76, 0x92, 0xED, 0xCB,
        0x47,
    ];

    const FLOW_EBOOT: &[u8] =
        include_bytes!("../../../tools/rpcs3/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.BIN");
    const SSHD_EBOOT: &[u8] =
        include_bytes!("../../../tools/rpcs3/dev_hdd0/game/NPUA80068/USRDIR/EBOOT.BIN");

    const fn include_bytes_array_flow_rap() -> [u8; 16] {
        *include_bytes!(
            "../../../tools/rpcs3/dev_hdd0/home/00000001/exdata/UP9000-NPUA80001_00-FLOWPS3PROMOTION.rap"
        )
    }

    const fn include_bytes_array_sshd_rap() -> [u8; 16] {
        *include_bytes!(
            "../../../tools/rpcs3/dev_hdd0/home/00000001/exdata/UP9000-NPUA80068_00-STARDUSTFULL0001.rap"
        )
    }

    /// Assert that `elf` is a 64-bit big-endian PS3 ELF (magic +
    /// EI_CLASS=ELFCLASS64 + EI_DATA=ELFDATA2MSB + e_machine=EM_PPC64)
    /// and that the declared phdr table fits within the buffer.
    fn assert_is_ps3_ppc64_elf(elf: &[u8]) {
        assert!(elf.len() >= 0x40, "ELF shorter than ehdr");
        assert_eq!(&elf[..4], b"\x7fELF", "ELF magic");
        assert_eq!(elf[4], 2, "EI_CLASS must be ELFCLASS64");
        assert_eq!(elf[5], 2, "EI_DATA must be ELFDATA2MSB (big-endian)");
        let e_machine = u16::from_be_bytes([elf[0x12], elf[0x13]]);
        assert_eq!(e_machine, 21, "e_machine must be EM_PPC64 (21)");
        let e_phoff = u64::from_be_bytes(
            elf[0x20..0x28]
                .try_into()
                .expect("invariant: 8-byte slice converts to [u8; 8]"),
        ) as usize;
        let e_phentsize = u16::from_be_bytes([elf[0x36], elf[0x37]]) as usize;
        let e_phnum = u16::from_be_bytes([elf[0x38], elf[0x39]]) as usize;
        assert!(e_phnum > 0, "ELF must have at least one program header");
        let phdr_table_end = e_phoff
            .checked_add(
                e_phnum
                    .checked_mul(e_phentsize)
                    .expect("phdr size overflow"),
            )
            .expect("phdr offset overflow");
        assert!(
            phdr_table_end <= elf.len(),
            "phdr table extends past ELF buffer: e_phoff=0x{e_phoff:x} + \
             e_phnum={e_phnum} * e_phentsize={e_phentsize} = {phdr_table_end} \
             > {} (ELF length)",
            elf.len(),
        );
    }

    #[test]
    fn rap_to_klic_matches_witness_flow() {
        let got = rap_to_klic(&FLOW_RAP);
        assert_eq!(got, FLOW_EXPECTED_KLIC, "flOw klic drift");
    }

    #[test]
    fn rap_to_klic_matches_witness_sshd() {
        let got = rap_to_klic(&SSHD_RAP);
        assert_eq!(got, SSHD_EXPECTED_KLIC, "SSHD klic drift");
    }

    #[test]
    fn flow_eboot_decrypts_to_parseable_elf() {
        let klic = rap_to_klic(&FLOW_RAP);
        let elf = decrypt_self_to_elf_npdrm(FLOW_EBOOT, &klic)
            .expect("flOw NPDRM decrypt: padding + section hashes self-certify");
        assert_is_ps3_ppc64_elf(&elf);
    }

    #[test]
    fn sshd_eboot_decrypts_to_parseable_elf() {
        let klic = rap_to_klic(&SSHD_RAP);
        let elf = decrypt_self_to_elf_npdrm(SSHD_EBOOT, &klic)
            .expect("SSHD NPDRM decrypt: padding + section hashes self-certify");
        assert_is_ps3_ppc64_elf(&elf);
    }
}
