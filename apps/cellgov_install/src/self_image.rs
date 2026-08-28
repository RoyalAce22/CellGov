//! One place that decides "is this SELF-wrapped, and with which key
//! path do I open it".
//!
//! Every consumer that hands a PPU image to a loader goes through
//! here. Firmware is APP-keyed by construction and an installed title
//! may be either, so the key path arrives as a caller-supplied
//! [`KeyPolicy`], and the key material as the caller's [`KeyVault`].
//! A build without the `decrypt` feature passes plaintext images
//! through and refuses every SCE wrapper with
//! [`SceError::DecryptFeatureDisabled`].

use std::borrow::Cow;

use cellgov_ps3_abi::sce::SCE_MAGIC;

use crate::keys::KeyVault;
use crate::npdrm::{NpdHeaderInfo, Rap};
use crate::sce::SceError;

/// Which key path [`to_plaintext_elf`] may use to open a SELF.
pub enum KeyPolicy<'a> {
    /// APP keys only. An NPDRM-wrapped SELF is refused with
    /// [`SceError::NpdrmUnderAppOnlyPolicy`] rather than attempted.
    AppOnly,
    /// Detect APP vs NPDRM, resolving the title's [`Rap`] through the
    /// supplied lookup. Returning `None` falls back to the vault's
    /// free klicensee for license-3 titles and fails for the others.
    Auto(&'a dyn Fn(&NpdHeaderInfo) -> Option<Rap>),
}

/// True when `bytes` opens with the SCE container magic.
pub fn is_sce_wrapped(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[..4] == SCE_MAGIC
}

/// Borrow `bytes` through unchanged when they are already a plaintext
/// image, or decrypt the SELF wrapper under `policy` with `keys`.
///
/// A plaintext image never consults the vault, so a build or a run
/// with no keys still boots one.
///
/// # Errors
///
/// Any [`SceError`] from the underlying decrypt, plus
/// [`SceError::NpdrmUnderAppOnlyPolicy`] when an NPDRM SELF meets
/// [`KeyPolicy::AppOnly`]. Under [`KeyPolicy::AppOnly`] any failure to
/// read the supplemental chain -- a truncated extended header, a chain
/// that escapes the buffer, an unparseable NPD body -- surfaces as
/// itself (e.g. [`SceError::NpdrmBadLicense`],
/// [`SceError::HeaderOffsetOutOfRange`]). Without the `decrypt`
/// feature every SCE wrapper is [`SceError::DecryptFeatureDisabled`],
/// except an NPDRM image under [`KeyPolicy::AppOnly`], which keeps
/// [`SceError::NpdrmUnderAppOnlyPolicy`]: no build could open it.
pub fn to_plaintext_elf<'a>(
    bytes: &'a [u8],
    keys: &KeyVault,
    policy: KeyPolicy<'_>,
) -> Result<Cow<'a, [u8]>, SceError> {
    if !is_sce_wrapped(bytes) {
        return Ok(Cow::Borrowed(bytes));
    }
    open_sce_wrapper(bytes, keys, policy).map(Cow::Owned)
}

#[cfg(feature = "decrypt")]
fn open_sce_wrapper(
    bytes: &[u8],
    keys: &KeyVault,
    policy: KeyPolicy<'_>,
) -> Result<Vec<u8>, SceError> {
    use crate::npdrm::{decrypt_self_to_elf_auto, find_npd_header_info};
    use crate::sce::decrypt_self_to_elf;

    match policy {
        KeyPolicy::AppOnly => {
            // The NPD supplemental header is plaintext, so the NPDRM
            // refusal can be named without any key material.
            match find_npd_header_info(bytes) {
                Ok(Some(npd)) => Err(SceError::NpdrmUnderAppOnlyPolicy {
                    content_id: npd.content_id,
                    license: npd.license as u32,
                }),
                Ok(None) => decrypt_self_to_elf(bytes, keys),
                // Only a chain that walks and carries no NPDRM record
                // clears the image for APP keys. A present record whose
                // body will not parse has already settled the key
                // class, and a chain that will not walk is a hard load
                // failure in RPCS3 `unself.cpp`
                // `SELFDecrypter::LoadHeaders` too.
                Err(e) => Err(e),
            }
        }
        KeyPolicy::Auto(resolver) => decrypt_self_to_elf_auto(bytes, keys, resolver),
    }
}

#[cfg(not(feature = "decrypt"))]
fn open_sce_wrapper(
    bytes: &[u8],
    _keys: &KeyVault,
    policy: KeyPolicy<'_>,
) -> Result<Vec<u8>, SceError> {
    // The NPD supplemental header is plaintext, so an NPDRM image under
    // APP-only keys gets the refusal no build could lift rather than a
    // rebuild hint.
    if let KeyPolicy::AppOnly = policy {
        if let Ok(Some(npd)) = crate::npdrm::find_npd_header_info(bytes) {
            return Err(SceError::NpdrmUnderAppOnlyPolicy {
                content_id: npd.content_id,
                license: npd.license as u32,
            });
        }
    }
    Err(SceError::DecryptFeatureDisabled)
}

/// Owned counterpart to [`to_plaintext_elf`] that moves an already
/// plaintext image through instead of copying it.
///
/// # Errors
///
/// Same as [`to_plaintext_elf`].
pub fn into_plaintext_elf(
    bytes: Vec<u8>,
    keys: &KeyVault,
    policy: KeyPolicy<'_>,
) -> Result<Vec<u8>, SceError> {
    if !is_sce_wrapped(&bytes) {
        return Ok(bytes);
    }
    Ok(to_plaintext_elf(&bytes, keys, policy)?.into_owned())
}

#[cfg(test)]
#[path = "tests/self_image_tests.rs"]
mod tests;
