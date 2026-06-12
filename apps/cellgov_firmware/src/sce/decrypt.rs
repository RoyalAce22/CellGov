//! Envelope + section decrypt pipeline: AES-256-CBC key envelope,
//! AES-128-CTR metadata directory, per-section decrypt + decompress.

use aes::cipher::{BlockDecryptMut, KeyIvInit, StreamCipher, StreamCipherSeek};

use cellgov_ps3_abi::sce::{
    SCEPKG_ERK, SCEPKG_RIV, SCE_COMP_KIND_NONE, SCE_COMP_KIND_ZLIB, SCE_ENC_KIND_AES128_CTR,
    SCE_ENC_KIND_PLAIN,
};

use super::elf::assemble_elf_from_sections;
use super::error::SceError;
use super::raw::{
    checked_add_oob, checked_mul_oob, parse_sce_header, read_be_u32, read_be_u64,
    EncryptedSectionDescriptor, SceContainerHeader,
};

type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;
type Aes128Ctr = ctr::Ctr128BE<aes::Aes128>;

/// Decrypt an SCE package (PUP-style PKG) using the firmware-update ERK/RIV
/// and return the most-likely payload (TAR if present, else largest section).
pub fn decrypt_package(data: &[u8]) -> Result<Vec<u8>, SceError> {
    decrypt_sce(data, &SCEPKG_ERK, &SCEPKG_RIV)
}

/// Decrypt a SELF container and reconstruct a plaintext ELF64 image.
///
/// The returned ELF has section-header offsets zeroed and per-segment
/// signatures stripped; it must not be re-signed or handed to anything
/// that verifies signatures.
pub fn decrypt_self_to_elf(data: &[u8]) -> Result<Vec<u8>, SceError> {
    let hdr = parse_sce_header(data)?;
    let revision = hdr.revision_flags & 0x7FFF;
    let key =
        crate::crypto::app_key_for_revision(revision).ok_or(SceError::NoAppKey { revision })?;

    let envelope = decrypt_envelope_app_keyed(data, &hdr, &key.erk, &key.riv)?;
    let sections = decrypt_sections_from_envelope(data, &hdr, &envelope)?;
    assemble_elf_from_sections(data, &sections)
}

fn decrypt_sce(data: &[u8], erk: &[u8; 0x20], riv: &[u8; 0x10]) -> Result<Vec<u8>, SceError> {
    let sections = decrypt_sce_sections(data, erk, riv)?;

    if std::env::var("CELLGOV_FW_DEBUG").is_ok() {
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
            if std::env::var("CELLGOV_FW_DEBUG").is_ok() {
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
/// APP-keyed path: extracts the [`super::MetadataKeyEnvelope`] via
/// AES-256-CBC against ERK/RIV, then hands off to a shared
/// section-decrypt helper. The NPDRM path produces the envelope
/// differently (see [`crate::npdrm`]) but consumes the same
/// downstream helper.
pub fn decrypt_sce_sections(
    data: &[u8],
    erk: &[u8; 0x20],
    riv: &[u8; 0x10],
) -> Result<Vec<(EncryptedSectionDescriptor, Vec<u8>)>, SceError> {
    let hdr = parse_sce_header(data)?;
    let envelope = decrypt_envelope_app_keyed(data, &hdr, erk, riv)?;
    decrypt_sections_from_envelope(data, &hdr, &envelope)
}

/// Decrypt the 0x40-byte [`super::MetadataKeyEnvelope`] using the
/// AES-256-CBC ERK/RIV pair (the APP-keyed path RPCS3 takes for
/// retail SELFs). Returns the plaintext envelope, with padding
/// regions validated to be zero.
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
        // NPDRM peel runs first when present, then the APP peel.
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

    if !is_debug
        && (envelope[0x10..0x20].iter().any(|&b| b != 0)
            || envelope[0x30..0x40].iter().any(|&b| b != 0))
    {
        return Err(SceError::KeyEnvelopePadding);
    }

    Ok(envelope)
}

/// Decrypt the metadata directory and every encrypted section using
/// a plaintext [`super::MetadataKeyEnvelope`]. Shared by the
/// APP-keyed and NPDRM-keyed paths once each has produced the
/// envelope by its own route.
///
/// The envelope layout is:
///   `[0x00..0x10] = aes_key`
///   `[0x10..0x20] = zero padding`
///   `[0x20..0x30] = aes_iv`
///   `[0x30..0x40] = zero padding`
pub(crate) fn decrypt_sections_from_envelope(
    data: &[u8],
    hdr: &SceContainerHeader,
    envelope: &[u8; 0x40],
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
    if directory_end > data.len() || directory_offset >= directory_end {
        return Err(SceError::TooSmall {
            what: "SCE metadata headers",
            got: data.len(),
            need: directory_end,
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
                let mut decoder = ZlibDecoder::new(sec_data.as_slice());
                let mut decompressed = Vec::new();
                decoder
                    .read_to_end(&mut decompressed)
                    .map_err(|source| SceError::ZlibDecompress { index: i, source })?;
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
