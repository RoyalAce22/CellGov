//! RAP -> klicensee derivation and NPDRM SELF decrypt prefix.
//!
//! NPDRM-wrapped SELFs carry an extra AES-128-CBC layer over the SCE
//! metadata-info envelope. Peeling it needs a 16-byte klicensee
//! derived from a RAP file the operator ships alongside the title.
//! This module owns that derivation, the NPD header interpretation
//! ([`find_npd_header_info`] over the generic supplemental walk in
//! [`crate::sce`]), and the prefix decrypt; the post-envelope flow
//! rejoins `crate::sce::decrypt_self_to_elf`'s shared CTR path.
//!
//! The NPD header interpretation is available in every build; the
//! derivation and the decrypt are behind the `decrypt` feature.
//! Witness vectors over the operator's installed corpus live in
//! `tests/npdrm_oracle_vectors.rs` behind the `npdrm-oracle-vectors`
//! feature.

#[cfg(feature = "decrypt")]
use aes::cipher::{BlockDecrypt, KeyInit};
use cellgov_ps3_abi::format::sce::SCE_SUPPLEMENTAL_KIND_NPDRM;

#[cfg(feature = "decrypt")]
use crate::keys::{KeyVault, KeyVaultError, SelfClass};
#[cfg(feature = "decrypt")]
use crate::sce::{
    assemble_elf_from_sections, decrypt_sections_from_envelope, inner_elf_segment_file_sizes,
    open_envelope_with, parse_sce_header,
};
use crate::sce::{find_supplemental_body, SceError};

/// Validated NPDRM license type; discriminants match the u32 BE wire
/// encoding, and any other wire value is rejected with
/// [`SceError::NpdrmBadLicense`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum NpdLicense {
    /// License type 1: network license, klicensee derived from a
    /// per-account RAP file.
    Network = 1,
    /// License type 2: local license, klicensee derived from a
    /// per-account RAP file.
    Local = 2,
    /// License type 3: free license; klicensee defaults to the vault's
    /// free klicensee when no RAP is supplied.
    Free = 3,
}

impl TryFrom<u32> for NpdLicense {
    type Error = SceError;
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(NpdLicense::Network),
            2 => Ok(NpdLicense::Local),
            3 => Ok(NpdLicense::Free),
            got => Err(SceError::NpdrmBadLicense { got }),
        }
    }
}

/// NPDRM control info extracted from a type-3 supplemental header.
#[derive(Debug, Clone)]
pub struct NpdHeaderInfo {
    /// Title `content_id` (up to 48 bytes, NUL-trimmed).
    pub content_id: String,
    /// License type, already validated by [`find_npd_header_info`].
    pub license: NpdLicense,
}

/// The 16 bytes of one title's RAP file, as read from disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rap(pub [u8; 16]);

/// Find and interpret an SELF's NPDRM (type 3) supplemental header.
///
/// Returns `Ok(None)` for SELFs that have no NPDRM supplemental
/// (APP-keyed retail / disc binaries take this path).
pub fn find_npd_header_info(data: &[u8]) -> Result<Option<NpdHeaderInfo>, SceError> {
    let Some(body) = find_supplemental_body(data, SCE_SUPPLEMENTAL_KIND_NPDRM)? else {
        return Ok(None);
    };
    // Type-3 body is the 0x80-byte NPD_HEADER: content_id is 48
    // bytes at NPD+0x10 (NUL-trimmed), license is u32 BE at NPD+0x08.
    if body.len() < 0x80 {
        return Err(SceError::HeaderOffsetOutOfRange {
            what: "NPDRM supplemental NPD body",
        });
    }
    let cid_bytes = &body[0x10..0x40];
    let cid_end = cid_bytes.iter().position(|&b| b == 0).unwrap_or(0x30);
    let content_id = String::from_utf8_lossy(&cid_bytes[..cid_end]).into_owned();
    let license_raw = u32::from_be_bytes(
        body[0x08..0x0C]
            .try_into()
            .expect("invariant: fixed 4-byte slice always converts to [u8; 4]"),
    );
    let license = NpdLicense::try_from(license_raw)?;
    Ok(Some(NpdHeaderInfo {
        content_id,
        license,
    }))
}

/// Derive the 16-byte intermediate klicensee (RIF key) from a 16-byte
/// RAP under the vault's RAP key and round tables.
///
/// The envelope-peel step further ECB-decrypts the output with the
/// vault's klicensee key to produce the layer key.
///
/// # Errors
///
/// [`SceError::Keys`] naming the first of the four RAP slots the vault
/// lacks, and [`SceError::RapPboxNotAPermutation`] when the vault's
/// permutation table is not one.
#[cfg(feature = "decrypt")]
pub fn rap_to_klic(keys: &KeyVault, rap: &[u8; 16]) -> Result<[u8; 16], SceError> {
    let rap_key = keys.rap_key()?;
    let pbox = keys.rap_pbox()?;
    let e1 = keys.rap_e1()?;
    let e2 = keys.rap_e2()?;
    // A digit order that skips or repeats a byte would derive a wrong
    // klicensee that only fails, later, as an envelope padding
    // mismatch, so it is refused here by name.
    let mut seen = [false; 16];
    for (index, &p) in pbox.iter().enumerate() {
        let p = usize::from(p);
        if p >= 16 || seen[p] {
            return Err(SceError::RapPboxNotAPermutation { index });
        }
        seen[p] = true;
    }
    let cipher = aes::Aes128::new_from_slice(rap_key).expect("invariant: the slot is 16 bytes");
    let mut state = *rap;
    cipher.decrypt_block((&mut state).into());

    let e1 = to_digits(e1, pbox);
    let e2 = u128::from_le_bytes(to_digits(e2, pbox));
    let mut digits = to_digits(&state, pbox);
    for _round in 0..5 {
        for (digit, e) in digits.iter_mut().zip(&e1) {
            *digit ^= e;
        }
        let before = digits;
        for (i, digit) in digits.iter_mut().enumerate().skip(1) {
            *digit = before[i] ^ before[i - 1];
        }
        digits = u128::from_le_bytes(digits).wrapping_sub(e2).to_le_bytes();
    }
    Ok(from_digits(&digits, pbox))
}

/// `bytes` in the RAP derivation's digit order: digit `i` is byte
/// `pbox[i]`, least significant first.
#[cfg(feature = "decrypt")]
fn to_digits(bytes: &[u8; 16], pbox: &[u8; 16]) -> [u8; 16] {
    let mut digits = [0u8; 16];
    for (digit, &p) in digits.iter_mut().zip(pbox) {
        *digit = bytes[usize::from(p)];
    }
    digits
}

/// Inverse of [`to_digits`] for a `pbox` that is a permutation.
#[cfg(feature = "decrypt")]
fn from_digits(digits: &[u8; 16], pbox: &[u8; 16]) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    for (&digit, &p) in digits.iter().zip(pbox) {
        bytes[usize::from(p)] = digit;
    }
    bytes
}

/// Derive the AES-128 layer key (which decrypts the NPDRM-wrapped
/// metadata-info envelope) by ECB-decrypting `klicensee` with the
/// vault's klicensee key.
#[cfg(feature = "decrypt")]
fn klicensee_to_layer_key(
    keys: &KeyVault,
    klicensee: &[u8; 16],
) -> Result<[u8; 16], KeyVaultError> {
    let cipher =
        aes::Aes128::new_from_slice(keys.np_klic_key()?).expect("invariant: the slot is 16 bytes");
    let mut layer_key = [0u8; 16];
    layer_key.copy_from_slice(klicensee);
    cipher.decrypt_block((&mut layer_key).into());
    Ok(layer_key)
}

/// Decrypt an NPDRM-wrapped SELF using the supplied 16-byte klicensee
/// and the vault's NPDRM keysets, and reconstruct the plaintext ELF.
///
/// # Errors
///
/// [`SceError::NoNpdrmKey`] when the vault has no keyset for the
/// revision; [`SceError::KeyEnvelopePadding`] when the one keyset or
/// the klicensee is wrong -- envelope zero-padding self-certifies a
/// correct decrypt -- and [`SceError::NoCandidateOpensEnvelope`] when
/// several keysets were tried.
#[cfg(feature = "decrypt")]
pub fn decrypt_self_to_elf_npdrm(
    data: &[u8],
    keys: &KeyVault,
    klicensee: &[u8; 16],
) -> Result<Vec<u8>, SceError> {
    let hdr = parse_sce_header(data)?;
    // High bit of revision_flags marks an unencrypted debug SELF.
    if hdr.revision_flags & 0x8000 != 0 {
        return Err(SceError::DebugSelfUnsupported {
            revision_flags: hdr.revision_flags,
        });
    }
    let revision = hdr.revision_flags & 0x7FFF;
    let layer_key = klicensee_to_layer_key(keys, klicensee)?;
    let envelope = open_envelope_with(
        data,
        &hdr,
        keys.self_key_candidates(SelfClass::Npdrm, revision),
        Some(&layer_key),
        "NPDRM",
        revision,
        || SceError::NoNpdrmKey { revision },
    )?;
    let segment_file_sizes = inner_elf_segment_file_sizes(data)?;
    let sections =
        decrypt_sections_from_envelope(data, &hdr, &envelope, Some(&segment_file_sizes))?;
    assemble_elf_from_sections(data, &sections)
}

/// Decrypt a SELF whose key class is not known up-front, dispatching
/// APP-keyed vs NPDRM via the presence of a type-3 supplemental header.
///
/// `rap_lookup` is invoked only for NPDRM-wrapped SELFs and returns
/// the title's [`Rap`]; the klicensee is derived here. Returning
/// `None` errors with [`SceError::NoRapForNpdrmTitle`] naming the
/// `content_id`. License-3 (free) titles fall back to the vault's
/// free klicensee when the lookup returns `None`.
#[cfg(feature = "decrypt")]
pub fn decrypt_self_to_elf_auto(
    data: &[u8],
    keys: &KeyVault,
    rap_lookup: impl FnOnce(&NpdHeaderInfo) -> Option<Rap>,
) -> Result<Vec<u8>, SceError> {
    match find_npd_header_info(data)? {
        None => crate::sce::decrypt_self_to_elf(data, keys),
        Some(npd) => {
            let klicensee = resolve_npdrm_klicensee(keys, &npd, rap_lookup)?;
            decrypt_self_to_elf_npdrm(data, keys, &klicensee)
        }
    }
}

/// Resolve the klicensee bytes for an NPDRM SELF given its NPD header.
#[cfg(feature = "decrypt")]
fn resolve_npdrm_klicensee(
    keys: &KeyVault,
    npd: &NpdHeaderInfo,
    rap_lookup: impl FnOnce(&NpdHeaderInfo) -> Option<Rap>,
) -> Result<[u8; 16], SceError> {
    let klicensee = match rap_lookup(npd) {
        Some(rap) => Some(rap_to_klic(keys, &rap.0)?),
        None => None,
    };
    match npd.license {
        NpdLicense::Network | NpdLicense::Local => {
            klicensee.ok_or_else(|| SceError::NoRapForNpdrmTitle {
                content_id: npd.content_id.clone(),
            })
        }
        NpdLicense::Free => match klicensee {
            Some(k) => Ok(k),
            None => Ok(*keys.np_klic_free()?),
        },
    }
}

#[cfg(test)]
#[path = "tests/npdrm_tests.rs"]
mod tests;

#[cfg(all(test, feature = "decrypt"))]
#[path = "tests/npdrm_klic_tests.rs"]
mod klic_known_answer_tests;
