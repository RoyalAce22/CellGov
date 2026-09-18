//! Envelope + section decrypt pipeline: AES-256-CBC key envelope,
//! AES-128-CTR metadata directory, per-section decrypt + decompress.

#![deny(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_lossless
)]

use aes::cipher::{BlockDecryptMut, KeyIvInit, StreamCipher, StreamCipherSeek};

use cellgov_ps3_abi::format::sce::{
    SCE_COMP_KIND_NONE, SCE_COMP_KIND_ZLIB, SCE_DATA_KEY_SIZE, SCE_ENC_KIND_AES128_CTR,
    SCE_ENC_KIND_PLAIN, SCE_SECTION_DESCRIPTOR_SIZE, SCE_SECTION_KIND_PHDR, SELF_PROGRAM_TYPE_LV2,
};

use crate::field::{read_be_u32, read_be_u64, usize_from_header, usize_from_u32};
use crate::keys::{KeyVault, SelfClass, SelfKey};

use super::elf::{assemble_elf_from_sections, inner_elf_segment_file_sizes};
use super::error::SceError;
use super::raw::{
    checked_add_oob, checked_mul_oob, parse_program_identification, parse_sce_header,
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
        tried = tried.saturating_add(1);
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

/// Decrypt a SELF container under the vault's keysets for its program
/// type and reconstruct a plaintext ELF64 image.
///
/// The program identification header's type selects the key class:
///
/// - an LV2 kernel opens under the LV2 keysets, in
///   [`KeyVault::lv2_key_candidates`] order;
/// - every other type opens under the APP keysets for its key
///   revision.
///
/// NPDRM-wrapped SELFs enter through [`crate::npdrm`] instead.
///
/// The returned ELF carries none of the SELF's signature material; it
/// must not be handed to anything that verifies signatures.
///
/// # Errors
///
/// - [`SceError::NoAppKey`]: the vault has no APP keyset for the
///   revision and no unlabeled candidate.
/// - [`SceError::NoLv2Key`]: a kernel meets a vault with no LV2
///   keyset.
/// - [`SceError::NoCandidateOpensEnvelope`]: the walk tried every
///   candidate and none fits.
/// - [`SceError::KeyEnvelopePadding`]: the one candidate does not fit,
///   so the walk returns its own refusal.
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
    let program = parse_program_identification(data)?;
    let envelope = if program.program_type == SELF_PROGRAM_TYPE_LV2 {
        open_envelope_with(
            data,
            &hdr,
            keys.lv2_key_candidates(program.version),
            None,
            "LV2",
            revision,
            || SceError::NoLv2Key {
                version: program.version,
            },
        )?
    } else {
        open_envelope_with(
            data,
            &hdr,
            keys.self_key_candidates(SelfClass::App, revision),
            None,
            "APP",
            revision,
            || SceError::NoAppKey { revision },
        )?
    };
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
        tried = tried.saturating_add(1);
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
/// A firmware-update PKG wraps no ELF, so no program header sizes its
/// sections. The header's [`SceContainerHeader::plaintext_size`] bounds
/// their total output.
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
    let key_envelope_offset = checked_add_oob(
        usize_from_u32(hdr.metadata_offset),
        0x20,
        "SCE metadata info",
    )?;
    let key_envelope_end = checked_add_oob(key_envelope_offset, 0x40, "SCE metadata info")?;
    let Some(envelope_bytes) = data.get(key_envelope_offset..key_envelope_end) else {
        return Err(SceError::TooSmall {
            what: "SCE metadata info",
            got: data.len(),
            need: key_envelope_end,
        });
    };

    let mut envelope = [0u8; 0x40];
    envelope.copy_from_slice(envelope_bytes);

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
/// A size that the container declares bounds the output of each
/// section, and [`inflate_bounded`] reserves a zlib section's buffer at
/// that bound. The bound is:
///
/// - for a PHDR-kind section, when the caller passes a SELF's
///   `p_filesz` table as `segment_file_sizes`: the `p_filesz` of its
///   destination segment;
/// - for every other section: the part of the header's
///   [`SceContainerHeader::plaintext_size`] that the earlier sections
///   left.
pub(crate) fn decrypt_sections_from_envelope(
    data: &[u8],
    hdr: &SceContainerHeader,
    envelope: &[u8; 0x40],
    segment_file_sizes: Option<&[usize]>,
) -> Result<Vec<(EncryptedSectionDescriptor, Vec<u8>)>, SceError> {
    let key_envelope_offset = checked_add_oob(
        usize_from_u32(hdr.metadata_offset),
        0x20,
        "SCE metadata info",
    )?;

    let aes_key: [u8; 16] = envelope[0..16]
        .try_into()
        .expect("invariant: fixed-length 16-byte slice always converts to [u8; 16]");
    let aes_iv: [u8; 16] = envelope[0x20..0x30]
        .try_into()
        .expect("invariant: fixed-length 16-byte slice always converts to [u8; 16]");

    let directory_offset = checked_add_oob(key_envelope_offset, 0x40, "SCE metadata directory")?;
    let Some(directory_end) = usize_from_header(hdr.header_size) else {
        return Err(SceError::HeaderOffsetOutOfRange {
            what: "SCE metadata headers",
        });
    };
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
    let section_count = usize_from_u32(read_be_u32(&directory_buf, 0x0C));
    let key_count = usize_from_u32(read_be_u32(&directory_buf, 0x10));

    let sections_start = 0x20usize;
    let sections_bytes = checked_mul_oob(
        section_count,
        SCE_SECTION_DESCRIPTOR_SIZE,
        "SCE metadata sections",
    )?;
    let keys_start = checked_add_oob(sections_start, sections_bytes, "SCE metadata sections")?;
    let keys_bytes = checked_mul_oob(key_count, SCE_DATA_KEY_SIZE, "SCE metadata keys")?;
    let keys_end = checked_add_oob(keys_start, keys_bytes, "SCE metadata keys")?;

    if keys_end > directory_buf.len() {
        return Err(SceError::MetadataHeadersTruncated {
            needed: keys_end,
            have: directory_buf.len(),
        });
    }

    let descriptors = &directory_buf[sections_start..keys_start];
    let data_keys = &directory_buf[keys_start..keys_end];

    // A declared size past the host `usize` bounds nothing the host
    // could allocate, so the budget saturates at the host limit.
    let mut budget = usize_from_header(hdr.plaintext_size).unwrap_or(usize::MAX);
    let mut sections: Vec<(EncryptedSectionDescriptor, Vec<u8>)> = Vec::new();

    for (i, row) in descriptors
        .chunks_exact(SCE_SECTION_DESCRIPTOR_SIZE)
        .enumerate()
    {
        let sec = EncryptedSectionDescriptor {
            payload_offset: read_be_u64(row, 0),
            payload_size: read_be_u64(row, 0x08),
            section_kind: read_be_u32(row, 0x10),
            program_segment_index: read_be_u32(row, 0x14),
            sha1_hashed: read_be_u32(row, 0x18),
            sha1_slot: read_be_u32(row, 0x1C),
            encryption_kind: read_be_u32(row, 0x20),
            key_slot: read_be_u32(row, 0x24),
            iv_slot: read_be_u32(row, 0x28),
            compression_kind: read_be_u32(row, 0x2C),
        };

        let Some(payload) = usize_from_header(sec.payload_offset)
            .zip(usize_from_header(sec.payload_size))
            .and_then(|(start, len)| data.get(start..)?.get(..len))
        else {
            return Err(SceError::SectionPastFile { index: i });
        };
        let mut sec_data = payload.to_vec();

        match sec.encryption_kind {
            SCE_ENC_KIND_PLAIN => {}
            SCE_ENC_KIND_AES128_CTR => {
                let (Some(sec_key), Some(sec_iv)) = (
                    data_key(data_keys, sec.key_slot),
                    data_key(data_keys, sec.iv_slot),
                ) else {
                    return Err(SceError::SectionKeyIvIndexOutOfRange { index: i });
                };

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

        let segment = match segment_file_sizes {
            Some(sizes) if sec.section_kind == SCE_SECTION_KIND_PHDR => {
                let prog_idx = usize_from_u32(sec.program_segment_index);
                let p_filesz =
                    *sizes
                        .get(prog_idx)
                        .ok_or(SceError::SectionProgramIndexOutOfRange {
                            prog_idx,
                            e_phnum: sizes.len(),
                        })?;
                Some((prog_idx, p_filesz))
            }
            _ => None,
        };
        let past_bound = || match segment {
            Some((prog_idx, p_filesz)) => SceError::SectionInflatesPastSegment {
                index: i,
                prog_idx,
                p_filesz,
            },
            None => SceError::SectionsPastPlaintextSize {
                index: i,
                plaintext_size: hdr.plaintext_size,
            },
        };
        let bound = segment.map_or(budget, |(_, p_filesz)| p_filesz);

        match sec.compression_kind {
            SCE_COMP_KIND_NONE => {}
            SCE_COMP_KIND_ZLIB => {
                sec_data = inflate_bounded(&sec_data, bound).map_err(|refusal| match refusal {
                    InflateRefusal::PastBound => past_bound(),
                    InflateRefusal::Unallocatable => SceError::SectionOutputTooLarge {
                        index: i,
                        size: bound,
                    },
                    InflateRefusal::Stream(source) => SceError::ZlibDecompress { index: i, source },
                })?;
            }
            other => {
                return Err(SceError::UnknownCompressionKind {
                    index: i,
                    got: other,
                });
            }
        }
        // `assemble_elf_from_sections` checks each PHDR section against
        // its segment's `p_filesz`, and names a short section as well
        // as a long one.
        if segment.is_none() {
            budget = budget.checked_sub(sec_data.len()).ok_or_else(past_bound)?;
        }

        sections.push((sec, sec_data));
    }

    Ok(sections)
}

/// The 16 bytes of data-key table slot `slot`, or `None` when the slot
/// is past the table.
fn data_key(table: &[u8], slot: u32) -> Option<[u8; SCE_DATA_KEY_SIZE]> {
    let start = usize_from_u32(slot).checked_mul(SCE_DATA_KEY_SIZE)?;
    table
        .get(start..)?
        .get(..SCE_DATA_KEY_SIZE)?
        .try_into()
        .ok()
}

/// Why [`inflate_bounded`] produced no output.
enum InflateRefusal {
    /// The stream inflates past the bound.
    PastBound,
    /// The host cannot allocate a buffer of the bound's size.
    Unallocatable,
    /// The stream is not valid zlib, or ends before its end marker.
    Stream(std::io::Error),
}

/// Inflate the zlib `stream`, whose output the container bounds at
/// `bound` bytes.
///
/// The function reserves `bound + 1` bytes before it inflates. The
/// extra byte separates a stream that ends at the bound from one that
/// runs past it. The stream cannot grow the buffer. Before it returns,
/// the function shrinks the buffer to the output, so a small output
/// does not keep a large reservation.
fn inflate_bounded(stream: &[u8], bound: usize) -> Result<Vec<u8>, InflateRefusal> {
    use flate2::{Decompress, FlushDecompress, Status};
    use std::io::{Error, ErrorKind};

    let room = bound.checked_add(1).ok_or(InflateRefusal::Unallocatable)?;
    let mut out = Vec::new();
    out.try_reserve_exact(room)
        .map_err(|_| InflateRefusal::Unallocatable)?;
    let mut inflater = Decompress::new(true);
    loop {
        let progress = (inflater.total_in(), inflater.total_out());
        let input = usize::try_from(inflater.total_in())
            .ok()
            .and_then(|consumed| stream.get(consumed..))
            .unwrap_or_default();
        let status = inflater
            .decompress_vec(input, &mut out, FlushDecompress::Finish)
            .map_err(|e| InflateRefusal::Stream(Error::new(ErrorKind::InvalidData, e)))?;
        if out.len() > bound {
            return Err(InflateRefusal::PastBound);
        }
        if status == Status::StreamEnd {
            break;
        }
        // No progress means the input ended before the end marker. The
        // stream is truncated, whatever its inflated length.
        if (inflater.total_in(), inflater.total_out()) == progress {
            return Err(InflateRefusal::Stream(Error::new(
                ErrorKind::UnexpectedEof,
                "zlib stream ends before its end marker",
            )));
        }
    }
    out.shrink_to_fit();
    Ok(out)
}
