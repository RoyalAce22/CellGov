//! SCE/SELF package decrypter for PS3 firmware and game binaries.
//!
//! All SCE/SELF headers are big-endian. [`decrypt_self_to_elf`] emits a
//! plaintext ELF with both per-segment and outer SCE signatures
//! stripped; the result must not be re-signed or fed to anything that
//! verifies signatures.

use aes::cipher::{BlockDecryptMut, KeyIvInit, StreamCipher, StreamCipherSeek};

use cellgov_ps3_abi::sce::{
    SCEPKG_ERK, SCEPKG_RIV, SCE_COMP_KIND_NONE as COMP_KIND_NONE,
    SCE_COMP_KIND_ZLIB as COMP_KIND_ZLIB, SCE_ENC_KIND_AES128_CTR as ENC_KIND_AES128_CTR,
    SCE_ENC_KIND_PLAIN as ENC_KIND_PLAIN, SCE_SECTION_KIND_PHDR as SECTION_KIND_PHDR,
};

#[derive(Debug)]
#[repr(C)]
pub struct SceContainerHeader {
    pub magic: u32,
    pub header_version: u32,
    pub revision_flags: u16,
    pub category: u16,
    pub metadata_offset: u32,
    pub header_size: u64,
    pub encrypted_payload_size: u64,
}

#[derive(Debug)]
#[repr(C)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct MetadataKeyEnvelope {
    pub aes_key: [u8; 16],
    pub aes_key_padding: [u8; 16],
    pub aes_iv: [u8; 16],
    pub aes_iv_padding: [u8; 16],
}

#[derive(Debug)]
#[repr(C)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct EncryptedMetadataDirectory {
    pub signed_region_length: u64,
    pub reserved_a: u32,
    pub section_count: u32,
    pub key_count: u32,
    pub auxiliary_header_size: u32,
    pub reserved_b: u32,
    pub reserved_c: u32,
}

#[derive(Debug)]
#[repr(C)]
pub struct EncryptedSectionDescriptor {
    pub payload_offset: u64,
    pub payload_size: u64,
    pub section_kind: u32,
    pub program_segment_index: u32,
    pub sha1_hashed: u32,
    pub sha1_slot: u32,
    pub encryption_kind: u32,
    pub key_slot: u32,
    pub iv_slot: u32,
    pub compression_kind: u32,
}

fn read_be_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes(
        data[offset..offset + 8]
            .try_into()
            .expect("invariant: fixed-length 8-byte slice always converts to [u8; 8]"),
    )
}

fn read_be_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(
        data[offset..offset + 4]
            .try_into()
            .expect("invariant: fixed-length 4-byte slice always converts to [u8; 4]"),
    )
}

fn read_be_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes(
        data[offset..offset + 2]
            .try_into()
            .expect("invariant: fixed-length 2-byte slice always converts to [u8; 2]"),
    )
}

/// Why SCE/SELF parsing or decryption failed.
#[derive(Debug)]
pub enum SceError {
    /// Buffer is too small for a fixed-size structure.
    TooSmall {
        what: &'static str,
        got: usize,
        need: usize,
    },
    /// SCE container magic mismatch.
    BadMagic { got: u32 },
    /// No APP key registered for the SELF revision.
    NoAppKey { revision: u16 },
    /// SELF's ELF header offset is outside the buffer.
    HeaderOffsetOutOfRange { what: &'static str },
    /// ELF EI_CLASS is not ELFCLASS64.
    BadElfClass { got: u8 },
    /// AES-256-CBC key-envelope decrypt failed.
    AesCbcDecryptFailed,
    /// Key-envelope padding did not decrypt to zero (likely wrong ERK/RIV).
    KeyEnvelopePadding,
    /// Decrypted metadata directory is shorter than its header.
    MetadataTooSmall,
    /// Metadata directory headers extend past the directory buffer.
    MetadataHeadersTruncated { needed: usize, have: usize },
    /// Section's encrypted payload extends past the file.
    SectionPastFile { index: usize },
    /// Section's key/iv slot index is outside the data-keys table.
    SectionKeyIvIndexOutOfRange { index: usize },
    /// Unknown encryption_kind in section header.
    UnknownEncryptionKind { index: usize, got: u32 },
    /// zlib decompress failed for a section.
    ZlibDecompress {
        index: usize,
        source: std::io::Error,
    },
    /// Unknown compression_kind in section header.
    UnknownCompressionKind { index: usize, got: u32 },
    /// Section's program_segment_index >= e_phnum.
    SectionProgramIndexOutOfRange { prog_idx: usize, e_phnum: usize },
    /// Section size disagrees with phdr p_filesz.
    SectionSizeMismatch {
        prog_idx: usize,
        got: usize,
        expected: usize,
    },
    /// Section would write past the reconstructed ELF buffer.
    SectionPastReconstructedElf {
        prog_idx: usize,
        offset: usize,
        size: usize,
        elf_size: usize,
    },
    /// Reconstructed ELF has bad magic.
    ReconstructedBadMagic { got: u32 },
    /// Decrypted package contained no usable section.
    NoUsableSection,
}

impl std::fmt::Display for SceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooSmall { what, got, need } => write!(
                f,
                "SCE: {what} too small ({got} bytes, need {need})"
            ),
            Self::BadMagic { got } => write!(f, "SCE: bad magic 0x{got:08x}"),
            Self::NoAppKey { revision } => {
                write!(f, "SCE: no APP key for SELF revision 0x{revision:04x}")
            }
            Self::HeaderOffsetOutOfRange { what } => {
                write!(f, "SCE: {what} offset out of range")
            }
            Self::BadElfClass { got } => write!(
                f,
                "SCE: SELF ELF header is not ELFCLASS64 (EI_CLASS=0x{got:02x})"
            ),
            Self::AesCbcDecryptFailed => f.write_str("SCE: AES-256-CBC decrypt failed"),
            Self::KeyEnvelopePadding => f.write_str(
                "SCE: MetadataKeyEnvelope padding validation failed (wrong key?)",
            ),
            Self::MetadataTooSmall => f.write_str("SCE: decrypted metadata too small for header"),
            Self::MetadataHeadersTruncated { needed, have } => write!(
                f,
                "SCE: metadata headers truncated: need {needed} bytes, have {have}"
            ),
            Self::SectionPastFile { index } => {
                write!(f, "SCE: section {index} extends past file end")
            }
            Self::SectionKeyIvIndexOutOfRange { index } => {
                write!(f, "SCE: section {index} key/iv index out of range")
            }
            Self::UnknownEncryptionKind { index, got } => write!(
                f,
                "SCE: section {index} has unknown encryption_kind {got} (expected 1=plain or 3=aes128-ctr)"
            ),
            Self::ZlibDecompress { index, source } => write!(
                f,
                "SCE: zlib decompress failed for section {index}: {source}"
            ),
            Self::UnknownCompressionKind { index, got } => write!(
                f,
                "SCE: section {index} has unknown compression_kind {got} (expected 1=none or 2=zlib)"
            ),
            Self::SectionProgramIndexOutOfRange {
                prog_idx,
                e_phnum,
            } => write!(
                f,
                "SCE: section program_segment_index {prog_idx} >= e_phnum {e_phnum}"
            ),
            Self::SectionSizeMismatch {
                prog_idx,
                got,
                expected,
            } => write!(
                f,
                "SCE: section for program segment {prog_idx} has {got} bytes but phdr p_filesz is {expected}"
            ),
            Self::SectionPastReconstructedElf {
                prog_idx,
                offset,
                size,
                elf_size,
            } => write!(
                f,
                "SCE: section for program segment {prog_idx} (offset 0x{offset:x}, size 0x{size:x}) exceeds reconstructed ELF size 0x{elf_size:x}"
            ),
            Self::ReconstructedBadMagic { got } => {
                write!(f, "SCE: reconstructed ELF has bad magic 0x{got:08x}")
            }
            Self::NoUsableSection => f.write_str("SCE: no usable section found in decrypted package"),
        }
    }
}

impl std::error::Error for SceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ZlibDecompress { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub fn parse_sce_header(data: &[u8]) -> Result<SceContainerHeader, SceError> {
    if data.len() < 0x20 {
        return Err(SceError::TooSmall {
            what: "SCE header",
            got: data.len(),
            need: 0x20,
        });
    }
    let magic = read_be_u32(data, 0);
    if magic != 0x53434500 {
        return Err(SceError::BadMagic { got: magic });
    }
    Ok(SceContainerHeader {
        magic,
        header_version: read_be_u32(data, 4),
        revision_flags: read_be_u16(data, 8),
        category: read_be_u16(data, 10),
        metadata_offset: read_be_u32(data, 12),
        header_size: read_be_u64(data, 16),
        encrypted_payload_size: read_be_u64(data, 24),
    })
}

type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;
type Aes128Ctr = ctr::Ctr128BE<aes::Aes128>;

pub fn decrypt_package(data: &[u8]) -> Result<Vec<u8>, SceError> {
    decrypt_sce(data, &SCEPKG_ERK, &SCEPKG_RIV)
}

pub fn decrypt_self_to_elf(data: &[u8]) -> Result<Vec<u8>, SceError> {
    let hdr = parse_sce_header(data)?;
    let revision = hdr.revision_flags & 0x7FFF;
    let key =
        crate::crypto::app_key_for_revision(revision).ok_or(SceError::NoAppKey { revision })?;

    if data.len() < 0x40 {
        return Err(SceError::TooSmall {
            what: "SELF extended header",
            got: data.len(),
            need: 0x40,
        });
    }
    let ehdr_offset = read_be_u64(data, 0x30) as usize;
    let phdr_offset = read_be_u64(data, 0x38) as usize;

    if ehdr_offset + 0x40 > data.len() {
        return Err(SceError::HeaderOffsetOutOfRange {
            what: "SELF ELF header",
        });
    }
    // Field offsets below assume ELFCLASS64. EI_CLASS == 2 is the
    // only value PS3 SELFs use.
    let ei_class = data[ehdr_offset + 4];
    if ei_class != 2 {
        return Err(SceError::BadElfClass { got: ei_class });
    }
    let e_phnum = read_be_u16(data, ehdr_offset + 0x38) as usize;
    let e_phentsize = read_be_u16(data, ehdr_offset + 0x36) as usize;
    if phdr_offset + e_phnum * e_phentsize > data.len() {
        return Err(SceError::HeaderOffsetOutOfRange {
            what: "SELF program headers",
        });
    }

    let sections = decrypt_sce_sections(data, &key.erk, &key.riv)?;

    let mut elf_size: usize = 0x40 + e_phnum * e_phentsize;
    for i in 0..e_phnum {
        let ph_off = phdr_offset + i * e_phentsize;
        let p_offset = read_be_u64(data, ph_off + 0x08) as usize;
        let p_filesz = read_be_u64(data, ph_off + 0x20) as usize;
        let end = p_offset + p_filesz;
        if end > elf_size {
            elf_size = end;
        }
    }

    let mut elf = vec![0u8; elf_size];
    elf[..0x40].copy_from_slice(&data[ehdr_offset..ehdr_offset + 0x40]);
    let phdr_dst = 0x40usize;
    elf[phdr_dst..phdr_dst + e_phnum * e_phentsize]
        .copy_from_slice(&data[phdr_offset..phdr_offset + e_phnum * e_phentsize]);
    rewrite_elf_header_offsets(&mut elf, phdr_dst as u64);

    for (sec, sec_data) in &sections {
        if sec.section_kind != SECTION_KIND_PHDR {
            continue;
        }
        if sec_data.is_empty() {
            continue;
        }
        let prog_idx = sec.program_segment_index as usize;
        if prog_idx >= e_phnum {
            return Err(SceError::SectionProgramIndexOutOfRange { prog_idx, e_phnum });
        }
        let ph_off = phdr_offset + prog_idx * e_phentsize;
        let p_offset = read_be_u64(data, ph_off + 0x08) as usize;
        let p_filesz = read_be_u64(data, ph_off + 0x20) as usize;
        if sec_data.len() != p_filesz {
            return Err(SceError::SectionSizeMismatch {
                prog_idx,
                got: sec_data.len(),
                expected: p_filesz,
            });
        }
        if p_offset + p_filesz > elf.len() {
            return Err(SceError::SectionPastReconstructedElf {
                prog_idx,
                offset: p_offset,
                size: p_filesz,
                elf_size: elf.len(),
            });
        }
        elf[p_offset..p_offset + p_filesz].copy_from_slice(sec_data);
    }

    let magic = u32::from_be_bytes([elf[0], elf[1], elf[2], elf[3]]);
    if magic != 0x7F454C46 {
        return Err(SceError::ReconstructedBadMagic { got: magic });
    }

    Ok(elf)
}

/// Relocate `e_phoff` to `phdr_dst` and zero `e_shoff` / `e_shnum` /
/// `e_shstrndx`. The original section-header offsets point into the
/// still-encrypted SELF and would dereference into garbage.
fn rewrite_elf_header_offsets(elf: &mut [u8], phdr_dst: u64) {
    elf[0x20..0x28].copy_from_slice(&phdr_dst.to_be_bytes());
    elf[0x28..0x30].copy_from_slice(&0u64.to_be_bytes());
    elf[0x3C..0x3E].copy_from_slice(&0u16.to_be_bytes());
    elf[0x3E..0x40].copy_from_slice(&0u16.to_be_bytes());
}

/// Zero the section-header fields CellGov does not preserve but a
/// third-party decrypt (e.g. RPCS3) might. Apply to both sides before
/// bit-comparing two reconstructed ELFs.
pub fn mask_non_semantic_elf_bytes(elf: &mut [u8]) {
    if elf.len() < 0x40 {
        return;
    }
    elf[0x28..0x30].copy_from_slice(&0u64.to_be_bytes());
    elf[0x3C..0x3E].copy_from_slice(&0u16.to_be_bytes());
    elf[0x3E..0x40].copy_from_slice(&0u16.to_be_bytes());
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

pub fn decrypt_sce_sections(
    data: &[u8],
    erk: &[u8; 0x20],
    riv: &[u8; 0x10],
) -> Result<Vec<(EncryptedSectionDescriptor, Vec<u8>)>, SceError> {
    let hdr = parse_sce_header(data)?;

    let key_envelope_offset = hdr.metadata_offset as usize + 0x20;
    if key_envelope_offset + 0x40 > data.len() {
        return Err(SceError::TooSmall {
            what: "SCE metadata info",
            got: data.len(),
            need: key_envelope_offset + 0x40,
        });
    }

    let mut key_envelope_buf = [0u8; 0x40];
    key_envelope_buf.copy_from_slice(&data[key_envelope_offset..key_envelope_offset + 0x40]);

    // Debug SELFs ship the key envelope in cleartext; retail uses AES-256-CBC.
    let is_debug = (hdr.revision_flags & 0x8000) != 0;
    if !is_debug {
        let decryptor = Aes256CbcDec::new(
            aes::cipher::generic_array::GenericArray::from_slice(erk),
            aes::cipher::generic_array::GenericArray::from_slice(riv),
        );
        decryptor
            .decrypt_padded_mut::<aes::cipher::block_padding::NoPadding>(&mut key_envelope_buf)
            .map_err(|_| SceError::AesCbcDecryptFailed)?;
    }

    let aes_key: [u8; 16] = key_envelope_buf[0..16]
        .try_into()
        .expect("invariant: fixed-length 16-byte slice always converts to [u8; 16]");
    let aes_iv: [u8; 16] = key_envelope_buf[0x20..0x30]
        .try_into()
        .expect("invariant: fixed-length 16-byte slice always converts to [u8; 16]");

    // Padding regions must decrypt to zero; non-zero means a wrong ERK/RIV.
    if !is_debug
        && (key_envelope_buf[0x10..0x20].iter().any(|&b| b != 0)
            || key_envelope_buf[0x30..0x40].iter().any(|&b| b != 0))
    {
        return Err(SceError::KeyEnvelopePadding);
    }

    let directory_offset = key_envelope_offset + 0x40;
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
    let keys_start = sections_start + section_count * 0x30;
    let keys_end = keys_start + key_count * 0x10;

    if keys_end > directory_buf.len() {
        return Err(SceError::MetadataHeadersTruncated {
            needed: keys_end,
            have: directory_buf.len(),
        });
    }

    let data_keys = &directory_buf[keys_start..keys_end];

    let mut sections: Vec<(EncryptedSectionDescriptor, Vec<u8>)> = Vec::new();

    for i in 0..section_count {
        let off = sections_start + i * 0x30;
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
        let sec_end = sec_start + sec.payload_size as usize;
        if sec_end > data.len() {
            return Err(SceError::SectionPastFile { index: i });
        }

        let mut sec_data = data[sec_start..sec_end].to_vec();

        match sec.encryption_kind {
            ENC_KIND_PLAIN => {}
            ENC_KIND_AES128_CTR => {
                let k_off = sec.key_slot as usize * 0x10;
                let iv_off = sec.iv_slot as usize * 0x10;
                if k_off + 0x10 > data_keys.len() || iv_off + 0x10 > data_keys.len() {
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
            COMP_KIND_NONE => {}
            COMP_KIND_ZLIB => {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sce_header_rejects_short() {
        assert!(parse_sce_header(&[0u8; 16]).is_err());
    }

    #[test]
    fn parse_sce_header_rejects_bad_magic() {
        let mut data = [0u8; 0x20];
        data[0..4].copy_from_slice(&0xDEADBEEFu32.to_be_bytes());
        assert!(matches!(
            parse_sce_header(&data).unwrap_err(),
            SceError::BadMagic { .. }
        ));
    }

    #[test]
    fn parse_sce_header_accepts_valid() {
        let mut data = [0u8; 0x20];
        data[0..4].copy_from_slice(&0x53434500u32.to_be_bytes());
        data[16..24].copy_from_slice(&256u64.to_be_bytes());
        let hdr = parse_sce_header(&data).unwrap();
        assert_eq!(hdr.magic, 0x53434500);
        assert_eq!(hdr.header_size, 256);
    }

    #[test]
    fn decrypt_package_rejects_truncated() {
        assert!(decrypt_package(&[0u8; 8]).is_err());
    }

    /// Foundation SPRXes already have zero `e_shoff`/`e_shnum`/
    /// `e_shstrndx` in the source, so this exercises the zeroing
    /// against non-zero inputs.
    #[test]
    fn rewrite_elf_header_offsets_zeroes_section_header_fields() {
        let mut elf = vec![0u8; 0x80];
        elf[0x28..0x30].copy_from_slice(&0xDEADBEEFCAFEBABEu64.to_be_bytes());
        elf[0x3C..0x3E].copy_from_slice(&0x4242u16.to_be_bytes());
        elf[0x3E..0x40].copy_from_slice(&0x1234u16.to_be_bytes());

        rewrite_elf_header_offsets(&mut elf, 0x40);

        assert_eq!(&elf[0x20..0x28], &0x40u64.to_be_bytes(), "e_phoff");
        assert_eq!(&elf[0x28..0x30], &[0u8; 8], "e_shoff");
        assert_eq!(&elf[0x3C..0x3E], &[0u8; 2], "e_shnum");
        assert_eq!(&elf[0x3E..0x40], &[0u8; 2], "e_shstrndx");
    }
}
