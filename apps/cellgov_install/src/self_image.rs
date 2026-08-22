//! One place that decides "is this SELF-wrapped, and with which key
//! path do I open it".
//!
//! Every consumer that hands a PPU image to a loader goes through
//! here, so the SCE-magic probe and the APP-vs-NPDRM choice exist
//! once rather than once per call site. The choice is a named
//! [`KeyPolicy`] argument because it is a caller policy, not a
//! property of the bytes: firmware is APP-keyed by construction,
//! while an installed title may be either.

use std::borrow::Cow;

use cellgov_ps3_abi::sce::{SCE_MAGIC, SCE_SUPPLEMENTAL_KIND_NPDRM};

use crate::npdrm::{decrypt_self_to_elf_auto, find_npd_header_info, NpdHeaderInfo};
use crate::sce::{decrypt_self_to_elf, find_supplemental_body, SceError};

/// Which key path [`to_plaintext_elf`] may use to open a SELF.
pub enum KeyPolicy<'a> {
    /// APP keys only. An NPDRM-wrapped SELF is refused with
    /// [`SceError::NpdrmUnderAppOnlyPolicy`] rather than attempted.
    AppOnly,
    /// Detect APP vs NPDRM, resolving the klicensee through the
    /// supplied lookup. Returning `None` falls back to `NP_KLIC_FREE`
    /// for license-3 titles and fails for the others.
    Auto(&'a dyn Fn(&NpdHeaderInfo) -> Option<[u8; 16]>),
}

/// True when `bytes` opens with the SCE container magic.
pub fn is_sce_wrapped(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[..4] == SCE_MAGIC
}

/// Borrow `bytes` through unchanged when they are already a plaintext
/// image, or decrypt the SELF wrapper under `policy`.
///
/// # Errors
///
/// Any [`SceError`] from the underlying decrypt, plus
/// [`SceError::NpdrmUnderAppOnlyPolicy`] when an NPDRM SELF meets
/// [`KeyPolicy::AppOnly`]. An NPDRM record whose NPD body will not
/// parse surfaces that parse error (e.g.
/// [`SceError::NpdrmBadLicense`]), not the APP decrypt's.
pub fn to_plaintext_elf<'a>(
    bytes: &'a [u8],
    policy: KeyPolicy<'_>,
) -> Result<Cow<'a, [u8]>, SceError> {
    if !is_sce_wrapped(bytes) {
        return Ok(Cow::Borrowed(bytes));
    }
    match policy {
        KeyPolicy::AppOnly => {
            // Name the NPDRM refusal before attempting the decrypt.
            // The NPD supplemental header is plaintext, so spotting one
            // needs no key material; without this probe an NPDRM image
            // surfaces as a key-envelope padding failure that reads
            // like a corrupt file rather than an unsupported one.
            match find_npd_header_info(bytes) {
                Ok(Some(npd)) => Err(SceError::NpdrmUnderAppOnlyPolicy {
                    content_id: npd.content_id,
                    license: npd.license as u32,
                }),
                Ok(None) => decrypt_self_to_elf(bytes).map(Cow::Owned),
                Err(e) => match find_supplemental_body(bytes, SCE_SUPPLEMENTAL_KIND_NPDRM) {
                    // The record is there and only its body would not
                    // parse, so the key class is already settled: the
                    // named error stands instead of being retried
                    // under APP keys that cannot open the image. RPCS3
                    // `unself.cpp` `SELFDecrypter::DecryptNPDRM` draws
                    // the same line -- absent control info means "not
                    // NPDRM, carry on", control info it cannot
                    // interpret aborts the decrypt.
                    Ok(Some(_)) => Err(e),
                    // The chain itself would not walk, so nothing
                    // establishes the image as NPDRM. The APP decrypt
                    // reads no supplemental record, so it still runs
                    // and names its own refusal.
                    _ => decrypt_self_to_elf(bytes).map(Cow::Owned),
                },
            }
        }
        KeyPolicy::Auto(resolver) => decrypt_self_to_elf_auto(bytes, resolver).map(Cow::Owned),
    }
}

/// Owned counterpart to [`to_plaintext_elf`] that moves an already
/// plaintext image through instead of copying it.
///
/// # Errors
///
/// Same as [`to_plaintext_elf`].
pub fn into_plaintext_elf(bytes: Vec<u8>, policy: KeyPolicy<'_>) -> Result<Vec<u8>, SceError> {
    if !is_sce_wrapped(&bytes) {
        return Ok(bytes);
    }
    Ok(to_plaintext_elf(&bytes, policy)?.into_owned())
}

#[cfg(test)]
#[path = "tests/self_image_tests.rs"]
mod tests;
