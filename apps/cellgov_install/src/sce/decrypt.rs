//! Envelope + section decrypt pipeline: AES-256-CBC key envelope,
//! AES-128-CTR metadata directory, per-section decrypt + decompress.

use aes::cipher::{BlockDecryptMut, KeyIvInit, StreamCipher, StreamCipherSeek};

use cellgov_ps3_abi::sce::{
    SCE_COMP_KIND_NONE, SCE_COMP_KIND_ZLIB, SCE_ENC_KIND_AES128_CTR, SCE_ENC_KIND_PLAIN,
    SCE_SECTION_KIND_PHDR,
};

use crate::keys::{KeyVault, SelfClass, SelfKey};

use super::elf::{assemble_elf_from_sections, inner_elf_segment_file_sizes};
use super::error::SceError;
use super::raw::{
    checked_add_oob, checked_mul_oob, parse_sce_header, read_be_u32, read_be_u64,
    EncryptedSectionDescriptor, SceContainerHeader,
};

type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;
type Aes128Ctr = ctr::Ctr128BE<aes::Aes128>;

/// Decrypt an SCE package (PUP-style PKG) under the vault's SCE
/// package keysets and return the most-likely payload (TAR if present,
/// else largest section).
///
/// # Errors
///
/// [`SceError::Keys`] when the vault holds no package keyset; when it
/// holds several, the first whose envelope padding checks is used and
/// [`SceError::NoCandidateOpensEnvelope`] names the count when none
/// does.
pub fn decrypt_package(data: &[u8], keys: &KeyVault) -> Result<Vec<u8>, SceError> {
    let hdr = parse_sce_header(data)?;
    let revision = hdr.revision_flags & 0x7FFF;
    let mut tried = 0usize;
    let mut last = None;
    for key in keys.scepkg_keys()? {
        tried += 1;
        match decrypt_sce(data, &key.erk, &key.riv) {
            Ok(payload) => return Ok(payload),
            Err(e @ (SceError::KeyEnvelopePadding | SceError::AesCbcDecryptFailed)) => {
                last = Some(e);
            }
            Err(e) => return Err(e),
        }
    }
    Err(no_candidate_fits("SCE package", revision, tried, last))
}

/// Decrypt a SELF container under the vault's APP keysets and
/// reconstruct a plaintext ELF64 image.
///
/// The returned ELF carries none of the SELF's signature material; it
/// must not be handed to anything that verifies signatures.
///
/// # Errors
///
/// [`SceError::NoAppKey`] when the vault has no APP keyset for the
/// revision and no unlabeled candidate; every candidate is tried and
/// [`SceError::NoCandidateOpensEnvelope`] reports when none fits. A
/// lone candidate that does not fit is reported as its own
/// [`SceError::KeyEnvelopePadding`].
pub fn decrypt_self_to_elf(data: &[u8], keys: &KeyVault) -> Result<Vec<u8>, SceError> {
    let hdr = parse_sce_header(data)?;
    // High bit of revision_flags marks an unencrypted debug SELF;
    // `decrypt_envelope` skips the key peel for one. The NPDRM entry
    // point refuses the same shape.
    if hdr.revision_flags & 0x8000 != 0 {
        return Err(SceError::DebugSelfUnsupported {
            revision_flags: hdr.revision_flags,
        });
    }
    let revision = hdr.revision_flags & 0x7FFF;
    let envelope = open_envelope_with(
        data,
        &hdr,
        keys.self_key_candidates(SelfClass::App, revision),
        None,
        "APP",
        revision,
        || SceError::NoAppKey { revision },
    )?;
    let segment_file_sizes = inner_elf_segment_file_sizes(data)?;
    let sections =
        decrypt_sections_from_envelope(data, &hdr, &envelope, Some(&segment_file_sizes))?;
    assemble_elf_from_sections(data, &sections)
}

/// Open the key envelope with the first of `candidates` that fits.
///
/// A wrong keyset fails the envelope's zero-padding self-check, so the
/// next is tried; any other refusal is the container's and stops the
/// walk. With exactly one candidate its own refusal is returned, so a
/// labeled key that does not fit still reads as the padding failure
/// it is.
pub(crate) fn open_envelope_with<'k>(
    data: &[u8],
    hdr: &SceContainerHeader,
    candidates: impl Iterator<Item = &'k SelfKey>,
    npdrm_layer_key: Option<&[u8; 0x10]>,
    class: &'static str,
    revision: u16,
    on_none: impl FnOnce() -> SceError,
) -> Result<[u8; 0x40], SceError> {
    let mut tried = 0usize;
    let mut last = None;
    for key in candidates {
        tried += 1;
        match decrypt_envelope(data, hdr, &key.erk, &key.riv, npdrm_layer_key) {
            Ok(envelope) => return Ok(envelope),
            Err(e @ (SceError::KeyEnvelopePadding | SceError::AesCbcDecryptFailed)) => {
                last = Some(e);
            }
            Err(e) => return Err(e),
        }
    }
    if tried == 0 {
        return Err(on_none());
    }
    Err(no_candidate_fits(class, revision, tried, last))
}

fn no_candidate_fits(
    class: &'static str,
    revision: u16,
    tried: usize,
    last: Option<SceError>,
) -> SceError {
    match (tried, last) {
        (1, Some(e)) => e,
        _ => SceError::NoCandidateOpensEnvelope {
            class,
            revision,
            tried,
        },
    }
}

fn decrypt_sce(data: &[u8], erk: &[u8; 0x20], riv: &[u8; 0x10]) -> Result<Vec<u8>, SceError> {
    let sections = decrypt_sce_sections(data, erk, riv)?;

    if super::section_trace_enabled() {
        for (i, (_, s)) in sections.iter().enumerate() {
            let magic = if s.len() >= 4 {
                format!("{:02x}{:02x}{:02x}{:02x}", s[0], s[1], s[2], s[3])
            } else {
                "??".to_string()
            };
            eprintln!("    section[{i}]: {} bytes, magic={magic}", s.len());
        }
    }

    for (i, (_, s)) in sections.iter().enumerate() {
        if s.len() >= 0x107 && &s[0x101..0x106] == b"ustar" {
            if super::section_trace_enabled() {
                eprintln!("    -> using section[{i}] (ustar TAR)");
            }
            return Ok(sections
                .into_iter()
                .nth(i)
                .expect("invariant: i comes from sections.iter().enumerate() above")
                .1);
        }
    }

    if let Some((_, largest)) = sections.into_iter().max_by_key(|(_, s)| s.len()) {
        Ok(largest)
    } else {
        Err(SceError::NoUsableSection)
    }
}

/// Decrypt every section of an SCE container using the supplied AES-256
/// ERK/RIV and return each section descriptor paired with its decrypted
/// (and zlib-decompressed where applicable) payload.
///
/// APP-keyed path; the NPDRM path produces its envelope through
/// [`crate::npdrm`].
///
/// The container is not assumed to wrap an ELF -- a firmware-update
/// PKG does not -- so no per-segment inflate bound is available and a
/// zlib section is inflated to whatever length its stream produces.
pub fn decrypt_sce_sections(
    data: &[u8],
    erk: &[u8; 0x20],
    riv: &[u8; 0x10],
) -> Result<Vec<(EncryptedSectionDescriptor, Vec<u8>)>, SceError> {
    let hdr = parse_sce_header(data)?;
    let envelope = decrypt_envelope_app_keyed(data, &hdr, erk, riv)?;
    decrypt_sections_from_envelope(data, &hdr, &envelope, None)
}

/// Decrypt the 0x40-byte [`super::MetadataKeyEnvelope`] with a single
/// AES-256-CBC ERK/RIV peel and no NPDRM layer. The pair is whichever
/// keyset the caller resolved: APP for a SELF, SCE package for a
/// PUP-style PKG.
fn decrypt_envelope_app_keyed(
    data: &[u8],
    hdr: &SceContainerHeader,
    erk: &[u8; 0x20],
    riv: &[u8; 0x10],
) -> Result<[u8; 0x40], SceError> {
    decrypt_envelope(data, hdr, erk, riv, None)
}

/// Decrypt the 0x40-byte [`super::MetadataKeyEnvelope`], optionally
/// peeling an NPDRM layer first.
///
/// When `npdrm_layer_key` is `Some`, AES-128-CBC peel (IV = zeros)
/// runs before the AES-256-CBC APP peel; `None` skips straight to
/// the APP peel. Padding regions `[0x10..0x20]` and `[0x30..0x40]`
/// must decrypt to zero; non-zero indicates a wrong key.
pub(crate) fn decrypt_envelope(
    data: &[u8],
    hdr: &SceContainerHeader,
    erk: &[u8; 0x20],
    riv: &[u8; 0x10],
    npdrm_layer_key: Option<&[u8; 0x10]>,
) -> Result<[u8; 0x40], SceError> {
    let key_envelope_offset =
        checked_add_oob(hdr.metadata_offset as usize, 0x20, "SCE metadata info")?;
    let key_envelope_end = checked_add_oob(key_envelope_offset, 0x40, "SCE metadata info")?;
    if key_envelope_end > data.len() {
        return Err(SceError::TooSmall {
            what: "SCE metadata info",
            got: data.len(),
            need: key_envelope_end,
        });
    }

    let mut envelope = [0u8; 0x40];
    envelope.copy_from_slice(&data[key_envelope_offset..key_envelope_end]);

    let is_debug = (hdr.revision_flags & 0x8000) != 0;
    if !is_debug {
        if let Some(layer_key) = npdrm_layer_key {
            type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;
            let cbc_iv = [0u8; 16];
            let decryptor = Aes128CbcDec::new(
                aes::cipher::generic_array::GenericArray::from_slice(layer_key),
                aes::cipher::generic_array::GenericArray::from_slice(&cbc_iv),
            );
            decryptor
                .decrypt_padded_mut::<aes::cipher::block_padding::NoPadding>(&mut envelope)
                .map_err(|_| SceError::AesCbcDecryptFailed)?;
        }

        let decryptor = Aes256CbcDec::new(
            aes::cipher::generic_array::GenericArray::from_slice(erk),
            aes::cipher::generic_array::GenericArray::from_slice(riv),
        );
        decryptor
            .decrypt_padded_mut::<aes::cipher::block_padding::NoPadding>(&mut envelope)
            .map_err(|_| SceError::AesCbcDecryptFailed)?;
    }

    // The padding self-check runs for a debug container too, so a
    // container that claims to be plaintext but is not gets named here
    // instead of carrying a garbage AES key into the CTR pass.
    if envelope[0x10..0x20].iter().any(|&b| b != 0) || envelope[0x30..0x40].iter().any(|&b| b != 0)
    {
        return Err(SceError::KeyEnvelopePadding);
    }

    Ok(envelope)
}

/// Decrypt the metadata directory and every encrypted section using
/// a plaintext [`super::MetadataKeyEnvelope`].
///
/// The envelope layout is:
///   `[0x00..0x10] = aes_key`
///   `[0x10..0x20] = zero padding`
///   `[0x20..0x30] = aes_iv`
///   `[0x30..0x40] = zero padding`
///
/// `segment_file_sizes` is the inner ELF's `p_filesz` table, supplied
/// by callers decrypting a SELF; it bounds how far a PHDR-kind
/// section's zlib stream may inflate. `None` leaves zlib sections
/// unbounded, which is all a container with no inner ELF can offer.
pub(crate) fn decrypt_sections_from_envelope(
    data: &[u8],
    hdr: &SceContainerHeader,
    envelope: &[u8; 0x40],
    segment_file_sizes: Option<&[usize]>,
) -> Result<Vec<(EncryptedSectionDescriptor, Vec<u8>)>, SceError> {
    let key_envelope_offset =
        checked_add_oob(hdr.metadata_offset as usize, 0x20, "SCE metadata info")?;

    let aes_key: [u8; 16] = envelope[0..16]
        .try_into()
        .expect("invariant: fixed-length 16-byte slice always converts to [u8; 16]");
    let aes_iv: [u8; 16] = envelope[0x20..0x30]
        .try_into()
        .expect("invariant: fixed-length 16-byte slice always converts to [u8; 16]");

    let directory_offset = checked_add_oob(key_envelope_offset, 0x40, "SCE metadata directory")?;
    let directory_end = hdr.header_size as usize;
    if directory_end > data.len() {
        return Err(SceError::TooSmall {
            what: "SCE metadata headers",
            got: data.len(),
            need: directory_end,
        });
    }
    // `header_size` is where the metadata directory ends. A value at or
    // below where it starts describes no directory at all -- a
    // malformed header, not a short file, and reporting it as `TooSmall`
    // would print a `need` smaller than the bytes already on hand.
    if directory_offset >= directory_end {
        return Err(SceError::HeaderOffsetOutOfRange {
            what: "SCE metadata directory",
        });
    }
    let mut directory_buf = data[directory_offset..directory_end].to_vec();

    let mut ctr_cipher = Aes128Ctr::new(
        aes::cipher::generic_array::GenericArray::from_slice(&aes_key),
        aes::cipher::generic_array::GenericArray::from_slice(&aes_iv),
    );
    ctr_cipher.seek(0u64);
    ctr_cipher.apply_keystream(&mut directory_buf);

    if directory_buf.len() < 0x20 {
        return Err(SceError::MetadataTooSmall);
    }
    let section_count = read_be_u32(&directory_buf, 0x0C) as usize;
    let key_count = read_be_u32(&directory_buf, 0x10) as usize;

    let sections_start = 0x20usize;
    let sections_bytes = checked_mul_oob(section_count, 0x30, "SCE metadata sections")?;
    let keys_start = checked_add_oob(sections_start, sections_bytes, "SCE metadata sections")?;
    let keys_bytes = checked_mul_oob(key_count, 0x10, "SCE metadata keys")?;
    let keys_end = checked_add_oob(keys_start, keys_bytes, "SCE metadata keys")?;

    if keys_end > directory_buf.len() {
        return Err(SceError::MetadataHeadersTruncated {
            needed: keys_end,
            have: directory_buf.len(),
        });
    }

    let data_keys = &directory_buf[keys_start..keys_end];

    let mut sections: Vec<(EncryptedSectionDescriptor, Vec<u8>)> = Vec::new();

    for i in 0..section_count {
        let row_off = checked_mul_oob(i, 0x30, "SCE section descriptor row")?;
        let off = checked_add_oob(sections_start, row_off, "SCE section descriptor row")?;
        let sec = EncryptedSectionDescriptor {
            payload_offset: read_be_u64(&directory_buf, off),
            payload_size: read_be_u64(&directory_buf, off + 8),
            section_kind: read_be_u32(&directory_buf, off + 0x10),
            program_segment_index: read_be_u32(&directory_buf, off + 0x14),
            sha1_hashed: read_be_u32(&directory_buf, off + 0x18),
            sha1_slot: read_be_u32(&directory_buf, off + 0x1C),
            encryption_kind: read_be_u32(&directory_buf, off + 0x20),
            key_slot: read_be_u32(&directory_buf, off + 0x24),
            iv_slot: read_be_u32(&directory_buf, off + 0x28),
            compression_kind: read_be_u32(&directory_buf, off + 0x2C),
        };

        let sec_start = sec.payload_offset as usize;
        let sec_end = sec_start
            .checked_add(sec.payload_size as usize)
            .ok_or(SceError::SectionPastFile { index: i })?;
        if sec_end > data.len() {
            return Err(SceError::SectionPastFile { index: i });
        }

        let mut sec_data = data[sec_start..sec_end].to_vec();

        match sec.encryption_kind {
            SCE_ENC_KIND_PLAIN => {}
            SCE_ENC_KIND_AES128_CTR => {
                let k_off = (sec.key_slot as usize)
                    .checked_mul(0x10)
                    .ok_or(SceError::SectionKeyIvIndexOutOfRange { index: i })?;
                let iv_off = (sec.iv_slot as usize)
                    .checked_mul(0x10)
                    .ok_or(SceError::SectionKeyIvIndexOutOfRange { index: i })?;
                let k_end = k_off
                    .checked_add(0x10)
                    .ok_or(SceError::SectionKeyIvIndexOutOfRange { index: i })?;
                let iv_end = iv_off
                    .checked_add(0x10)
                    .ok_or(SceError::SectionKeyIvIndexOutOfRange { index: i })?;
                if k_end > data_keys.len() || iv_end > data_keys.len() {
                    return Err(SceError::SectionKeyIvIndexOutOfRange { index: i });
                }
                let sec_key: [u8; 16] = data_keys[k_off..k_off + 0x10]
                    .try_into()
                    .expect("invariant: fixed-length 16-byte slice always converts to [u8; 16]");
                let sec_iv: [u8; 16] = data_keys[iv_off..iv_off + 0x10]
                    .try_into()
                    .expect("invariant: fixed-length 16-byte slice always converts to [u8; 16]");

                let mut sec_cipher = Aes128Ctr::new(
                    aes::cipher::generic_array::GenericArray::from_slice(&sec_key),
                    aes::cipher::generic_array::GenericArray::from_slice(&sec_iv),
                );
                sec_cipher.seek(0u64);
                sec_cipher.apply_keystream(&mut sec_data);
            }
            other => {
                return Err(SceError::UnknownEncryptionKind {
                    index: i,
                    got: other,
                });
            }
        }

        match sec.compression_kind {
            SCE_COMP_KIND_NONE => {}
            SCE_COMP_KIND_ZLIB => {
                use flate2::read::ZlibDecoder;
                use std::io::Read;
                // A stream that inflates to more bytes than its
                // destination segment's `p_filesz` declares is not a
                // stream this container describes. Reading one byte
                // past that size separates the two without letting a
                // crafted stream drive the allocation.
                let cap = match segment_file_sizes {
                    Some(sizes) if sec.section_kind == SCE_SECTION_KIND_PHDR => {
                        let prog_idx = sec.program_segment_index as usize;
                        Some(*sizes.get(prog_idx).ok_or(
                            SceError::SectionProgramIndexOutOfRange {
                                prog_idx,
                                e_phnum: sizes.len(),
                            },
                        )?)
                    }
                    _ => None,
                };
                let mut decoder = ZlibDecoder::new(sec_data.as_slice());
                let mut decompressed = Vec::new();
                match cap {
                    Some(cap) => {
                        let inflated = decoder
                            .by_ref()
                            .take((cap as u64).saturating_add(1))
                            .read_to_end(&mut decompressed)
                            .map_err(|source| SceError::ZlibDecompress { index: i, source })?;
                        if inflated > cap {
                            return Err(SceError::SectionInflatesPastSegment {
                                index: i,
                                prog_idx: sec.program_segment_index as usize,
                                p_filesz: cap,
                            });
                        }
                    }
                    None => {
                        decoder
                            .read_to_end(&mut decompressed)
                            .map_err(|source| SceError::ZlibDecompress { index: i, source })?;
                    }
                }
                sec_data = decompressed;
            }
            other => {
                return Err(SceError::UnknownCompressionKind {
                    index: i,
                    got: other,
                });
            }
        }

        sections.push((sec, sec_data));
    }

    Ok(sections)
}
