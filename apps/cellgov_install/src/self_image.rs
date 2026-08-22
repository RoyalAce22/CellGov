//! One place that decides "is this SELF-wrapped, and with which key
//! path do I open it".
//!
//! Every consumer that hands a PPU image to a loader goes through
//! here. Firmware is APP-keyed by construction and an installed title
//! may be either, so the key path arrives as a caller-supplied
//! [`KeyPolicy`].

use std::borrow::Cow;

use cellgov_ps3_abi::sce::SCE_MAGIC;

use crate::npdrm::{decrypt_self_to_elf_auto, find_npd_header_info, NpdHeaderInfo};
use crate::sce::{decrypt_self_to_elf, SceError};

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
/// [`KeyPolicy::AppOnly`]. Under [`KeyPolicy::AppOnly`] any failure to
/// read the supplemental chain -- a truncated extended header, a chain
/// that escapes the buffer, an unparseable NPD body -- surfaces as
/// itself (e.g. [`SceError::NpdrmBadLicense`],
/// [`SceError::HeaderOffsetOutOfRange`]).
pub fn to_plaintext_elf<'a>(
    bytes: &'a [u8],
    policy: KeyPolicy<'_>,
) -> Result<Cow<'a, [u8]>, SceError> {
    if !is_sce_wrapped(bytes) {
        return Ok(Cow::Borrowed(bytes));
    }
    match policy {
        KeyPolicy::AppOnly => {
            // The NPD supplemental header is plaintext, so the NPDRM
            // refusal can be named without any key material.
            match find_npd_header_info(bytes) {
                Ok(Some(npd)) => Err(SceError::NpdrmUnderAppOnlyPolicy {
                    content_id: npd.content_id,
                    license: npd.license as u32,
                }),
                Ok(None) => decrypt_self_to_elf(bytes).map(Cow::Owned),
                // Only a chain that walks and carries no NPDRM record
                // clears the image for APP keys. A present record whose
                // body will not parse has already settled the key
                // class, and a chain that will not walk is a hard load
                // failure in RPCS3 `unself.cpp`
                // `SELFDecrypter::LoadHeaders` too.
                Err(e) => Err(e),
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
