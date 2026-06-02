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
    SCE_SUPPLEMENTAL_KIND_NPDRM,
};

/// Outer SCE container header at file offset 0 (big-endian, 0x20 bytes).
#[derive(Debug)]
#[repr(C)]
pub struct SceContainerHeader {
    /// Magic at offset 0x00: `0x53434500` ("SCE\0").
    pub magic: u32,
    /// SCE header layout version at offset 0x04.
    pub header_version: u32,
    /// Revision + flag bits at offset 0x08; low 15 bits index the APP key, high bit set means debug SELF.
    pub revision_flags: u16,
    /// Container category at offset 0x0A (SELF, PKG, etc.).
    pub category: u16,
    /// Offset 0x0C: byte offset of the metadata info block from file start.
    pub metadata_offset: u32,
    /// Offset 0x10: total size in bytes of all SCE headers (where encrypted payload begins).
    pub header_size: u64,
    /// Offset 0x18: size of the encrypted payload that follows the headers.
    pub encrypted_payload_size: u64,
}

/// 0x40-byte AES-256-CBC-encrypted envelope holding the per-file data key + IV
/// used to decrypt the metadata directory.
#[derive(Debug)]
#[repr(C)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct MetadataKeyEnvelope {
    /// AES-128 data key at envelope offset 0x00.
    pub aes_key: [u8; 16],
    /// Offset 0x10: padding; must decrypt to all zero for a correct ERK/RIV.
    pub aes_key_padding: [u8; 16],
    /// AES-128 data IV at envelope offset 0x20.
    pub aes_iv: [u8; 16],
    /// Offset 0x30: padding; must decrypt to all zero for a correct ERK/RIV.
    pub aes_iv_padding: [u8; 16],
}

/// 0x20-byte header at the start of the AES-128-CTR-encrypted metadata
/// directory; section descriptors and the data-key table follow it.
#[derive(Debug)]
#[repr(C)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct EncryptedMetadataDirectory {
    /// Offset 0x00: length in bytes of the region covered by the outer signature.
    pub signed_region_length: u64,
    /// Offset 0x08: reserved per SCE header layout; not validated.
    pub reserved_a: u32,
    /// Offset 0x0C: number of `EncryptedSectionDescriptor` entries that follow.
    pub section_count: u32,
    /// Offset 0x10: number of 16-byte key/IV slots in the data-keys table.
    pub key_count: u32,
    /// Offset 0x14: size in bytes of optional capability/auxiliary header that may trail the table.
    pub auxiliary_header_size: u32,
    /// Offset 0x18: reserved per SCE header layout; not validated.
    pub reserved_b: u32,
    /// Offset 0x1C: reserved per SCE header layout; not validated.
    pub reserved_c: u32,
}

/// 0x30-byte descriptor for one encrypted payload section in the SCE file.
#[derive(Debug)]
#[repr(C)]
pub struct EncryptedSectionDescriptor {
    /// Offset 0x00: byte offset of this section's payload from file start.
    pub payload_offset: u64,
    /// Offset 0x08: payload size in bytes (pre-decompression).
    pub payload_size: u64,
    /// Offset 0x10: section kind; `SCE_SECTION_KIND_PHDR == 2` for program-segment payloads.
    pub section_kind: u32,
    /// Offset 0x14: index into the ELF program-header table for `PHDR`-kind sections.
    pub program_segment_index: u32,
    /// Offset 0x18: non-zero if this section's payload has a SHA-1 entry in the hash table.
    pub sha1_hashed: u32,
    /// Offset 0x1C: index into the SHA-1 hash table when `sha1_hashed != 0`.
    pub sha1_slot: u32,
    /// Offset 0x20: encryption kind; 1=plain, 3=AES-128-CTR.
    pub encryption_kind: u32,
    /// Offset 0x24: index into the data-keys table for the section AES key.
    pub key_slot: u32,
    /// Offset 0x28: index into the data-keys table for the section AES IV.
    pub iv_slot: u32,
    /// Offset 0x2C: compression kind; 1=none, 2=zlib.
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

/// Checked addition routed to [`SceError::HeaderOffsetOutOfRange`].
/// Used on file-derived offsets / sizes where overflow would
/// wrap the bounds check and let downstream indexing panic.
fn checked_add_oob(a: usize, b: usize, what: &'static str) -> Result<usize, SceError> {
    a.checked_add(b)
        .ok_or(SceError::HeaderOffsetOutOfRange { what })
}

/// Checked multiplication routed to [`SceError::HeaderOffsetOutOfRange`].
/// Used on counts-times-element-size products derived from file
/// bytes (e.g. `e_phnum * e_phentsize`, `section_count * 0x30`).
fn checked_mul_oob(a: usize, b: usize, what: &'static str) -> Result<usize, SceError> {
    a.checked_mul(b)
        .ok_or(SceError::HeaderOffsetOutOfRange { what })
}

/// Why SCE/SELF parsing or decryption failed.
#[derive(Debug, thiserror::Error)]
pub enum SceError {
    /// Buffer is too small for a fixed-size structure.
    #[error("SCE: {what} too small ({got} bytes, need {need})")]
    TooSmall {
        /// Name of the structure that was being read.
        what: &'static str,
        /// Bytes available in the input buffer.
        got: usize,
        /// Bytes required to read the structure.
        need: usize,
    },
    /// SCE container magic mismatch.
    #[error("SCE: bad magic 0x{got:08x}")]
    BadMagic {
        /// Magic word actually read at file offset 0.
        got: u32,
    },
    /// No APP key registered for the SELF revision.
    #[error("SCE: no APP key for SELF revision 0x{revision:04x}")]
    NoAppKey {
        /// SELF revision (low 15 bits of `revision_flags`) for which no APP key is known.
        revision: u16,
    },
    /// SELF's ELF header offset is outside the buffer.
    #[error("SCE: {what} offset out of range")]
    HeaderOffsetOutOfRange {
        /// Name of the SELF sub-header whose offset escaped the buffer.
        what: &'static str,
    },
    /// Inner ELF header magic word is not `\x7fELF`.
    #[error("SCE: inner ELF header has bad magic 0x{got:08x}")]
    InnerElfBadMagic {
        /// Magic word read at the inner ELF header offset.
        got: u32,
    },
    /// ELF EI_CLASS is not ELFCLASS64.
    #[error("SCE: SELF ELF header is not ELFCLASS64 (EI_CLASS=0x{got:02x})")]
    BadElfClass {
        /// `EI_CLASS` byte read from the inner ELF header.
        got: u8,
    },
    /// ELF64 header field size disagrees with the architectural constant.
    /// `e_phentsize` must be 0x38 (size of `Elf64_Phdr`); `e_shentsize`
    /// must be 0x40 (size of `Elf64_Shdr`).
    #[error("SCE: inner ELF {what} = 0x{got:04x}, expected 0x{expected:04x}")]
    BadElfEntSize {
        /// Name of the entsize field that disagreed (`e_phentsize` /
        /// `e_shentsize`).
        what: &'static str,
        /// Value read from the inner ELF header.
        got: u16,
        /// Architectural constant for ELF64.
        expected: u16,
    },
    /// AES-256-CBC key-envelope decrypt failed.
    #[error("SCE: AES-256-CBC decrypt failed")]
    AesCbcDecryptFailed,
    /// Key-envelope padding did not decrypt to zero (likely wrong ERK/RIV).
    #[error("SCE: MetadataKeyEnvelope padding validation failed (wrong key?)")]
    KeyEnvelopePadding,
    /// Decrypted metadata directory is shorter than its header.
    #[error("SCE: decrypted metadata too small for header")]
    MetadataTooSmall,
    /// Metadata directory headers extend past the directory buffer.
    #[error("SCE: metadata headers truncated: need {needed} bytes, have {have}")]
    MetadataHeadersTruncated {
        /// Bytes required by the section + key-table layout the directory header claims.
        needed: usize,
        /// Bytes actually present in the decrypted directory buffer.
        have: usize,
    },
    /// Section's encrypted payload extends past the file.
    #[error("SCE: section {index} extends past file end")]
    SectionPastFile {
        /// Zero-based section index that escapes the input buffer.
        index: usize,
    },
    /// Section's key/iv slot index is outside the data-keys table.
    #[error("SCE: section {index} key/iv index out of range")]
    SectionKeyIvIndexOutOfRange {
        /// Zero-based section index with the bad slot index.
        index: usize,
    },
    /// Unknown encryption_kind in section header.
    #[error(
        "SCE: section {index} has unknown encryption_kind {got} (expected 1=plain or 3=aes128-ctr)"
    )]
    UnknownEncryptionKind {
        /// Zero-based section index whose `encryption_kind` was not recognized.
        index: usize,
        /// Raw `encryption_kind` value read from the section descriptor.
        got: u32,
    },
    /// zlib decompress failed for a section.
    #[error("SCE: zlib decompress failed for section {index}: {source}")]
    ZlibDecompress {
        /// Zero-based section index whose decompression failed.
        index: usize,
        /// Underlying `flate2` error.
        #[source]
        source: std::io::Error,
    },
    /// Unknown compression_kind in section header.
    #[error("SCE: section {index} has unknown compression_kind {got} (expected 1=none or 2=zlib)")]
    UnknownCompressionKind {
        /// Zero-based section index whose `compression_kind` was not recognized.
        index: usize,
        /// Raw `compression_kind` value read from the section descriptor.
        got: u32,
    },
    /// Section's program_segment_index >= e_phnum.
    #[error("SCE: section program_segment_index {prog_idx} >= e_phnum {e_phnum}")]
    SectionProgramIndexOutOfRange {
        /// `program_segment_index` claimed by the section descriptor.
        prog_idx: usize,
        /// ELF program-header count read from the inner ELF.
        e_phnum: usize,
    },
    /// Section size disagrees with phdr p_filesz.
    #[error("SCE: section for program segment {prog_idx} has {got} bytes but phdr p_filesz is {expected}")]
    SectionSizeMismatch {
        /// Program-header index the mismatched section targets.
        prog_idx: usize,
        /// Decrypted+decompressed section length.
        got: usize,
        /// `p_filesz` declared by the program header.
        expected: usize,
    },
    /// Section would write past the reconstructed ELF buffer.
    #[error("SCE: section for program segment {prog_idx} (offset 0x{offset:x}, size 0x{size:x}) exceeds reconstructed ELF size 0x{elf_size:x}")]
    SectionPastReconstructedElf {
        /// Program-header index the offending section targets.
        prog_idx: usize,
        /// `p_offset` of the destination program segment.
        offset: usize,
        /// `p_filesz` of the destination program segment.
        size: usize,
        /// Total size of the reconstructed ELF buffer.
        elf_size: usize,
    },
    /// Reconstructed ELF has bad magic.
    #[error("SCE: reconstructed ELF has bad magic 0x{got:08x}")]
    ReconstructedBadMagic {
        /// Magic word read from the reconstructed ELF at offset 0.
        got: u32,
    },
    /// Decrypted package contained no usable section.
    #[error("SCE: no usable section found in decrypted package")]
    NoUsableSection,
    /// SELF is NPDRM-wrapped but no klicensee was supplied. Callers
    /// should use [`crate::npdrm::decrypt_self_to_elf_npdrm`] (or `_auto`).
    #[error("SCE: SELF is NPDRM-wrapped (content_id={content_id}); needs a klicensee")]
    NeedsNpdrmKlic {
        /// `content_id` from the NPD supplemental header.
        content_id: String,
    },
    /// NPDRM klicensee lookup returned `None` for the named title.
    #[error("SCE: no RAP/klicensee for NPDRM title {content_id}")]
    NoRapForNpdrmTitle {
        /// `content_id` from the NPD supplemental header.
        content_id: String,
    },
    /// NPDRM license value is not 1, 2, or 3.
    #[error("SCE: NPDRM license value {got} is not 1, 2, or 3")]
    NpdrmBadLicense {
        /// Raw `license` field (u32 BE on the wire).
        got: u32,
    },
    /// SELF carries the debug/fself flag (high bit of `revision_flags`);
    /// the NPDRM decrypt path does not handle unencrypted SELFs.
    #[error("SCE: SELF is flagged debug/fself (revision_flags=0x{revision_flags:04x}); unencrypted SELFs are not in scope for the NPDRM decrypt path")]
    DebugSelfUnsupported {
        /// Raw `revision_flags` field from the SCE container header.
        revision_flags: u16,
    },
}

/// Validated NPDRM license type. Wire field is u32 BE; only 1 /
/// 2 / 3 are accepted, with any other wire value surfacing as
/// [`SceError::NpdrmBadLicense`] at the parse site. Discriminants
/// match the wire encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum NpdLicense {
    /// License type 1: network license, klicensee derived from a
    /// per-account RAP file.
    Network = 1,
    /// License type 2: local license, klicensee derived from a
    /// per-account RAP file.
    Local = 2,
    /// License type 3: free license; klicensee defaults to
    /// `NP_KLIC_FREE` when no RAP is supplied.
    Free = 3,
}

// Hand-rolled so the error path lands directly on
// `SceError::NpdrmBadLicense { got }` without a `From` wrapper for
// `num_enum::TryFromPrimitiveError`.
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
    /// Validated license type. Out-of-range wire values are
    /// rejected by [`find_npd_header_info`] with
    /// [`SceError::NpdrmBadLicense`]; consumers receive a value
    /// already in the 1 / 2 / 3 set.
    pub license: NpdLicense,
}

/// Walk an SELF's supplemental-header chain and return the NPDRM
/// (type 3) entry's payload if present.
///
/// The extended header at file offset 0x20 carries
/// `supplemental_hdr_offset` (u64 BE at offset 0x58) and
/// `supplemental_hdr_size` (u64 BE at offset 0x60). The chain is a
/// sequence of `{type:u32, size:u32, next:u64, body}` records
/// totalling `supplemental_hdr_size` bytes.
///
/// Returns `Ok(None)` for SELFs that have no NPDRM supplemental
/// (APP-keyed retail / disc binaries take this path).
pub fn find_npd_header_info(data: &[u8]) -> Result<Option<NpdHeaderInfo>, SceError> {
    if data.len() < 0x68 {
        return Err(SceError::TooSmall {
            what: "SELF extended header",
            got: data.len(),
            need: 0x68,
        });
    }
    let supplemental_offset = read_be_u64(data, 0x58) as usize;
    let supplemental_size = read_be_u64(data, 0x60) as usize;
    if supplemental_size == 0 {
        return Ok(None);
    }
    let supplemental_end = supplemental_offset.checked_add(supplemental_size).ok_or(
        SceError::HeaderOffsetOutOfRange {
            what: "SELF supplemental headers",
        },
    )?;
    if supplemental_end > data.len() {
        return Err(SceError::HeaderOffsetOutOfRange {
            what: "SELF supplemental headers",
        });
    }

    let mut cursor = supplemental_offset;
    while cursor < supplemental_end {
        let record_header_end = checked_add_oob(cursor, 0x10, "SELF supplemental header record")?;
        if record_header_end > supplemental_end {
            return Err(SceError::HeaderOffsetOutOfRange {
                what: "SELF supplemental header record",
            });
        }
        let kind = read_be_u32(data, cursor);
        let record_size = read_be_u32(data, cursor + 4) as usize;
        let record_end =
            checked_add_oob(cursor, record_size, "SELF supplemental header record body")?;
        if record_size < 0x10 || record_end > supplemental_end {
            return Err(SceError::HeaderOffsetOutOfRange {
                what: "SELF supplemental header record body",
            });
        }
        if kind == SCE_SUPPLEMENTAL_KIND_NPDRM {
            // Type-3 body is the 0x80-byte NPD_HEADER at +0x10:
            // content_id is 48 bytes at NPD+0x10 (NUL-trimmed),
            // license is u32 BE at NPD+0x08.
            let npd_off = checked_add_oob(cursor, 0x10, "NPDRM supplemental NPD body")?;
            let npd_end = checked_add_oob(npd_off, 0x80, "NPDRM supplemental NPD body")?;
            if npd_end > supplemental_end {
                return Err(SceError::HeaderOffsetOutOfRange {
                    what: "NPDRM supplemental NPD body",
                });
            }
            let cid_off = npd_off + 0x10;
            let cid_bytes = &data[cid_off..cid_off + 0x30];
            let cid_end = cid_bytes.iter().position(|&b| b == 0).unwrap_or(0x30);
            let content_id = String::from_utf8_lossy(&cid_bytes[..cid_end]).into_owned();
            let license_raw = u32::from_be_bytes(
                data[npd_off + 0x08..npd_off + 0x0C]
                    .try_into()
                    .expect("invariant: fixed 4-byte slice always converts to [u8; 4]"),
            );
            let license = NpdLicense::try_from(license_raw)?;
            return Ok(Some(NpdHeaderInfo {
                content_id,
                license,
            }));
        }
        cursor = checked_add_oob(cursor, record_size, "SELF supplemental header walk")?;
    }
    Ok(None)
}

/// Parse the 0x20-byte outer SCE header from the start of `data`, validating the magic.
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

/// Reassemble a plaintext ELF from decrypted SCE sections.
///
/// Layout: ehdr as-is, program headers packed immediately after,
/// each PHDR-kind section copied to its declared `p_offset`, then
/// -- if the SELF's `shdr_offset` is non-zero -- the original
/// section-header table copied to the ELF's declared `e_shoff`.
/// Shared by [`decrypt_self_to_elf`] (APP-keyed) and
/// [`crate::npdrm::decrypt_self_to_elf_npdrm`] (NPDRM-keyed); only
/// the envelope path upstream differs.
pub(crate) fn assemble_elf_from_sections(
    data: &[u8],
    sections: &[(EncryptedSectionDescriptor, Vec<u8>)],
) -> Result<Vec<u8>, SceError> {
    if data.len() < 0x68 {
        return Err(SceError::TooSmall {
            what: "SELF extended header",
            got: data.len(),
            need: 0x68,
        });
    }
    let ehdr_offset = read_be_u64(data, 0x30) as usize;
    let phdr_offset = read_be_u64(data, 0x38) as usize;
    let shdr_offset_in_self = read_be_u64(data, 0x40) as usize;

    let ehdr_end = checked_add_oob(ehdr_offset, 0x40, "SELF ELF header")?;
    if ehdr_end > data.len() {
        return Err(SceError::HeaderOffsetOutOfRange {
            what: "SELF ELF header",
        });
    }
    let inner_magic = read_be_u32(data, ehdr_offset);
    if inner_magic != 0x7F45_4C46 {
        return Err(SceError::InnerElfBadMagic { got: inner_magic });
    }
    // Field offsets below assume ELFCLASS64 (the only value PS3 SELFs use).
    let ei_class = data[ehdr_offset + 4];
    if ei_class != 2 {
        return Err(SceError::BadElfClass { got: ei_class });
    }
    let e_shoff = read_be_u64(data, ehdr_offset + 0x28) as usize;
    let e_phnum = read_be_u16(data, ehdr_offset + 0x38) as usize;
    let e_shnum = read_be_u16(data, ehdr_offset + 0x3C) as usize;
    let e_phentsize_raw = read_be_u16(data, ehdr_offset + 0x36);
    let e_shentsize_raw = read_be_u16(data, ehdr_offset + 0x3A);
    // Per ELF, entsize is "size of one entry" and only meaningful
    // when there are entries. Firmware SPRXes ship with e_shnum = 0
    // and e_shentsize = 0; only validate when the count is non-zero,
    // matching the architectural constants for ELF64
    // (Elf64_Phdr = 0x38, Elf64_Shdr = 0x40).
    if e_phnum > 0 && e_phentsize_raw != 0x38 {
        return Err(SceError::BadElfEntSize {
            what: "e_phentsize",
            got: e_phentsize_raw,
            expected: 0x38,
        });
    }
    if e_shnum > 0 && e_shentsize_raw != 0x40 {
        return Err(SceError::BadElfEntSize {
            what: "e_shentsize",
            got: e_shentsize_raw,
            expected: 0x40,
        });
    }
    let e_phentsize = e_phentsize_raw as usize;
    let e_shentsize = e_shentsize_raw as usize;
    let phdr_table_bytes = checked_mul_oob(e_phnum, e_phentsize, "SELF program headers")?;
    let phdr_end = checked_add_oob(phdr_offset, phdr_table_bytes, "SELF program headers")?;
    if phdr_end > data.len() {
        return Err(SceError::HeaderOffsetOutOfRange {
            what: "SELF program headers",
        });
    }

    let mut elf_size: usize = checked_add_oob(0x40, phdr_table_bytes, "reconstructed ELF size")?;
    for i in 0..e_phnum {
        let row_off = checked_mul_oob(i, e_phentsize, "SELF program header row")?;
        let ph_off = checked_add_oob(phdr_offset, row_off, "SELF program header row")?;
        let p_offset = read_be_u64(data, ph_off + 0x08) as usize;
        let p_filesz = read_be_u64(data, ph_off + 0x20) as usize;
        let end = checked_add_oob(p_offset, p_filesz, "SELF program segment extent")?;
        if end > elf_size {
            elf_size = end;
        }
    }
    let shdr_table_bytes = checked_mul_oob(e_shnum, e_shentsize, "SELF section headers")?;
    if shdr_offset_in_self != 0 && e_shnum > 0 {
        let shdr_end = checked_add_oob(e_shoff, shdr_table_bytes, "SELF section headers")?;
        if shdr_end > elf_size {
            elf_size = shdr_end;
        }
        let shdr_end_in_self = checked_add_oob(
            shdr_offset_in_self,
            shdr_table_bytes,
            "SELF section headers",
        )?;
        if shdr_end_in_self > data.len() {
            return Err(SceError::HeaderOffsetOutOfRange {
                what: "SELF section headers",
            });
        }
    }

    let mut elf = vec![0u8; elf_size];
    elf[..0x40].copy_from_slice(&data[ehdr_offset..ehdr_offset + 0x40]);
    let phdr_dst = 0x40usize;
    elf[phdr_dst..phdr_dst + phdr_table_bytes]
        .copy_from_slice(&data[phdr_offset..phdr_offset + phdr_table_bytes]);
    // Rewrite e_phoff to the packed phdr position; the inner ELF's
    // original value may differ from 0x40.
    elf[0x20..0x28].copy_from_slice(&(phdr_dst as u64).to_be_bytes());

    if shdr_offset_in_self != 0 && e_shnum > 0 {
        elf[e_shoff..e_shoff + shdr_table_bytes]
            .copy_from_slice(&data[shdr_offset_in_self..shdr_offset_in_self + shdr_table_bytes]);
    }

    for (sec, sec_data) in sections {
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
        let row_off = checked_mul_oob(prog_idx, e_phentsize, "SELF program header row")?;
        let ph_off = checked_add_oob(phdr_offset, row_off, "SELF program header row")?;
        let p_offset = read_be_u64(data, ph_off + 0x08) as usize;
        let p_filesz = read_be_u64(data, ph_off + 0x20) as usize;
        if sec_data.len() != p_filesz {
            return Err(SceError::SectionSizeMismatch {
                prog_idx,
                got: sec_data.len(),
                expected: p_filesz,
            });
        }
        let write_end =
            p_offset
                .checked_add(p_filesz)
                .ok_or(SceError::SectionPastReconstructedElf {
                    prog_idx,
                    offset: p_offset,
                    size: p_filesz,
                    elf_size: elf.len(),
                })?;
        if write_end > elf.len() {
            return Err(SceError::SectionPastReconstructedElf {
                prog_idx,
                offset: p_offset,
                size: p_filesz,
                elf_size: elf.len(),
            });
        }
        elf[p_offset..write_end].copy_from_slice(sec_data);
    }

    let magic = u32::from_be_bytes([elf[0], elf[1], elf[2], elf[3]]);
    if magic != 0x7F454C46 {
        return Err(SceError::ReconstructedBadMagic { got: magic });
    }

    Ok(elf)
}

/// Zero `e_shoff`, `e_shnum`, and `e_shstrndx` so a reconstructed
/// ELF can be byte-compared modulo section-header layout.
///
/// The contract is "identical run image," not "identical ELF
/// file": `e_shoff` / `e_shnum` / `e_shstrndx` describe the
/// section / link view, not part of the run image. `e_phoff` and
/// the program-header table describe the segment / execution
/// view, which the loader consumes to build the run image, and
/// stay unmasked.
///
/// The three-field set is empirically sufficient for the current
/// title corpus (flOw / SSHD / WipEout plus the firmware-PRX
/// parity gate); not proven-minimal against arbitrary PS3 SELFs.
/// Widen when a corpus addition surfaces a fourth non-semantic
/// ELF64 header field.
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

/// Decrypt every section of an SCE container using the supplied AES-256
/// ERK/RIV and return each section descriptor paired with its decrypted
/// (and zlib-decompressed where applicable) payload.
///
/// APP-keyed path: extracts the [`MetadataKeyEnvelope`] via AES-256-CBC
/// against ERK/RIV, then hands off to a shared section-decrypt helper.
/// The NPDRM path produces the envelope differently (see
/// [`crate::npdrm`]) but consumes the same downstream helper.
pub fn decrypt_sce_sections(
    data: &[u8],
    erk: &[u8; 0x20],
    riv: &[u8; 0x10],
) -> Result<Vec<(EncryptedSectionDescriptor, Vec<u8>)>, SceError> {
    let hdr = parse_sce_header(data)?;
    let envelope = decrypt_envelope_app_keyed(data, &hdr, erk, riv)?;
    decrypt_sections_from_envelope(data, &hdr, &envelope)
}

/// Decrypt the 0x40-byte [`MetadataKeyEnvelope`] using the
/// AES-256-CBC ERK/RIV pair (the APP-keyed path RPCS3 takes for
/// retail SELFs). Returns the plaintext envelope, with padding
/// regions validated to be zero.
pub(crate) fn decrypt_envelope_app_keyed(
    data: &[u8],
    hdr: &SceContainerHeader,
    erk: &[u8; 0x20],
    riv: &[u8; 0x10],
) -> Result<[u8; 0x40], SceError> {
    decrypt_envelope(data, hdr, erk, riv, None)
}

/// Decrypt the 0x40-byte [`MetadataKeyEnvelope`], optionally peeling
/// an NPDRM layer first.
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
/// a plaintext [`MetadataKeyEnvelope`]. Shared by the APP-keyed and
/// NPDRM-keyed paths once each has produced the envelope by its own
/// route.
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
            ENC_KIND_PLAIN => {}
            ENC_KIND_AES128_CTR => {
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

    #[test]
    fn mask_non_semantic_elf_bytes_zeroes_section_header_fields_and_moves_nothing_else() {
        // The {e_shoff, e_shnum, e_shstrndx} set is empirically
        // sufficient for the current title corpus (flOw / SSHD /
        // WipEout + the firmware-PRX byte parity). Not proven-minimal
        // against arbitrary PS3 SELFs; widen only when a corpus
        // addition surfaces a fourth non-semantic ELF64 header field.
        let mut elf: Vec<u8> = (0u8..=0xFFu8).cycle().take(0x80).collect();
        elf[0x28..0x30].copy_from_slice(&0xDEADBEEFCAFEBABEu64.to_be_bytes());
        elf[0x3C..0x3E].copy_from_slice(&0x4242u16.to_be_bytes());
        elf[0x3E..0x40].copy_from_slice(&0x1234u16.to_be_bytes());
        let before = elf.clone();

        mask_non_semantic_elf_bytes(&mut elf);

        assert_eq!(&elf[0x28..0x30], &[0u8; 8], "e_shoff");
        assert_eq!(&elf[0x3C..0x3E], &[0u8; 2], "e_shnum");
        assert_eq!(&elf[0x3E..0x40], &[0u8; 2], "e_shstrndx");

        // Nothing-else-moved: every byte outside the three masked
        // ranges must equal its pre-mask value.
        for (i, (b_before, b_after)) in before.iter().zip(elf.iter()).enumerate() {
            let in_shoff = (0x28..0x30).contains(&i);
            let in_shnum = (0x3C..0x3E).contains(&i);
            let in_shstrndx = (0x3E..0x40).contains(&i);
            if in_shoff || in_shnum || in_shstrndx {
                continue;
            }
            assert_eq!(
                b_before, b_after,
                "byte at 0x{i:02x} changed: 0x{b_before:02x} -> 0x{b_after:02x}",
            );
        }
    }

    #[test]
    fn mask_non_semantic_elf_bytes_is_noop_on_short_input() {
        let mut elf = vec![0xABu8; 0x3F];
        let before = elf.clone();
        mask_non_semantic_elf_bytes(&mut elf);
        assert_eq!(elf, before);
    }

    /// Craft a minimal SELF buffer that satisfies the early
    /// fixed-position bounds checks in `assemble_elf_from_sections`:
    /// ehdr at 0x100 with valid magic + ELFCLASS64 + ELF64 entsize
    /// values, phdr at 0x200, no section-header table. Per-field
    /// perturbations on top of this are the per-overflow tests below.
    fn build_synthetic_self() -> Vec<u8> {
        let mut data = vec![0u8; 0x400];
        let ehdr_offset: u64 = 0x100;
        let phdr_offset: u64 = 0x200;
        data[0x30..0x38].copy_from_slice(&ehdr_offset.to_be_bytes());
        data[0x38..0x40].copy_from_slice(&phdr_offset.to_be_bytes());
        data[0x40..0x48].copy_from_slice(&0u64.to_be_bytes());
        // Inner ELF64 header at ehdr_offset.
        data[0x100..0x104].copy_from_slice(&0x7F45_4C46u32.to_be_bytes());
        data[0x104] = 2;
        // e_phentsize at +0x36, e_phnum at +0x38, e_shentsize at +0x3A, e_shnum at +0x3C.
        data[0x136..0x138].copy_from_slice(&0x38u16.to_be_bytes());
        data[0x138..0x13A].copy_from_slice(&0u16.to_be_bytes());
        data[0x13A..0x13C].copy_from_slice(&0x40u16.to_be_bytes());
        data[0x13C..0x13E].copy_from_slice(&0u16.to_be_bytes());
        data
    }

    #[test]
    fn assemble_ehdr_offset_overflow_returns_typed_error() {
        let mut data = vec![0u8; 0x100];
        data[0x30..0x38].copy_from_slice(&(u64::MAX).to_be_bytes());
        let err = assemble_elf_from_sections(&data, &[]).unwrap_err();
        assert!(matches!(err, SceError::HeaderOffsetOutOfRange { .. }));
    }

    #[test]
    fn assemble_phdr_table_extent_overflow_returns_typed_error() {
        let mut data = build_synthetic_self();
        // Push phdr_offset to near usize::MAX so phdr_offset + 0x38 wraps.
        data[0x38..0x40].copy_from_slice(&u64::MAX.to_be_bytes());
        // e_phnum = 1 with entsize 0x38: addition wraps.
        data[0x138..0x13A].copy_from_slice(&1u16.to_be_bytes());
        let err = assemble_elf_from_sections(&data, &[]).unwrap_err();
        assert!(matches!(err, SceError::HeaderOffsetOutOfRange { .. }));
    }

    #[test]
    fn assemble_inner_elf_bad_magic_returns_typed_error() {
        let mut data = build_synthetic_self();
        data[0x100..0x104].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
        let err = assemble_elf_from_sections(&data, &[]).unwrap_err();
        assert!(matches!(
            err,
            SceError::InnerElfBadMagic { got: 0xDEAD_BEEF }
        ));
    }

    #[test]
    fn assemble_bad_phentsize_returns_typed_error() {
        let mut data = build_synthetic_self();
        // e_phnum > 0 so the entsize validation fires; e_phentsize = 0
        // would otherwise be permissible when no program headers exist.
        data[0x138..0x13A].copy_from_slice(&1u16.to_be_bytes());
        data[0x136..0x138].copy_from_slice(&0u16.to_be_bytes());
        let err = assemble_elf_from_sections(&data, &[]).unwrap_err();
        assert!(matches!(
            err,
            SceError::BadElfEntSize {
                what: "e_phentsize",
                got: 0,
                expected: 0x38,
            }
        ));
    }

    #[test]
    fn assemble_bad_shentsize_returns_typed_error() {
        let mut data = build_synthetic_self();
        // e_shnum > 0 + shdr_offset_in_self > 0 so the entsize and
        // section-table extent checks both engage.
        data[0x40..0x48].copy_from_slice(&0x40u64.to_be_bytes());
        data[0x13C..0x13E].copy_from_slice(&1u16.to_be_bytes());
        data[0x13A..0x13C].copy_from_slice(&0x80u16.to_be_bytes());
        let err = assemble_elf_from_sections(&data, &[]).unwrap_err();
        assert!(matches!(
            err,
            SceError::BadElfEntSize {
                what: "e_shentsize",
                got: 0x80,
                expected: 0x40,
            }
        ));
    }

    #[test]
    fn assemble_zero_phnum_with_zero_phentsize_is_accepted() {
        // SPRX shape: e_phnum = e_shnum = 0, entsize fields zero.
        // Must clear the entsize gate; downstream failures are
        // out of scope for this assertion.
        let data = build_synthetic_self();
        let result = assemble_elf_from_sections(&data, &[]);
        if let Err(SceError::BadElfEntSize { .. }) = result {
            panic!("unexpected BadElfEntSize for SPRX-shape input");
        }
    }

    /// Build a minimal supplemental-header chain at offset 0x68.
    /// Returns the data buffer; caller perturbs records in-place.
    fn build_synthetic_supplemental_chain(records_bytes: &[u8]) -> Vec<u8> {
        let mut data = vec![0u8; 0x200];
        let supp_off: u64 = 0x68;
        let supp_size: u64 = records_bytes.len() as u64;
        data[0x58..0x60].copy_from_slice(&supp_off.to_be_bytes());
        data[0x60..0x68].copy_from_slice(&supp_size.to_be_bytes());
        let start = supp_off as usize;
        let end = start + records_bytes.len();
        data[start..end].copy_from_slice(records_bytes);
        data
    }

    #[test]
    fn find_npd_no_npdrm_record_returns_ok_none() {
        // Two non-NPDRM records, both well-formed at the minimum 0x10
        // size. The disc / APP-keyed path takes this branch.
        let mut records = vec![0u8; 0x20];
        records[0..4].copy_from_slice(&1u32.to_be_bytes());
        records[4..8].copy_from_slice(&0x10u32.to_be_bytes());
        records[0x10..0x14].copy_from_slice(&2u32.to_be_bytes());
        records[0x14..0x18].copy_from_slice(&0x10u32.to_be_bytes());
        let data = build_synthetic_supplemental_chain(&records);
        assert!(find_npd_header_info(&data).unwrap().is_none());
    }

    #[test]
    fn find_npd_empty_supplemental_returns_ok_none() {
        let mut data = vec![0u8; 0x80];
        data[0x58..0x60].copy_from_slice(&0u64.to_be_bytes());
        data[0x60..0x68].copy_from_slice(&0u64.to_be_bytes());
        assert!(find_npd_header_info(&data).unwrap().is_none());
    }

    #[test]
    fn find_npd_record_size_under_minimum_returns_typed_error() {
        let mut records = vec![0u8; 0x10];
        records[0..4].copy_from_slice(&1u32.to_be_bytes());
        records[4..8].copy_from_slice(&0x0Fu32.to_be_bytes());
        let data = build_synthetic_supplemental_chain(&records);
        let err = find_npd_header_info(&data).unwrap_err();
        assert!(matches!(err, SceError::HeaderOffsetOutOfRange { .. }));
    }

    #[test]
    fn find_npd_record_size_overflow_returns_typed_error() {
        let mut records = vec![0u8; 0x10];
        records[0..4].copy_from_slice(&1u32.to_be_bytes());
        records[4..8].copy_from_slice(&u32::MAX.to_be_bytes());
        let data = build_synthetic_supplemental_chain(&records);
        let err = find_npd_header_info(&data).unwrap_err();
        assert!(matches!(err, SceError::HeaderOffsetOutOfRange { .. }));
    }

    #[test]
    fn find_npd_npd_body_overflow_returns_typed_error() {
        // Record with kind=NPDRM and size=0x10 (just the record header,
        // no body). NPD body needs 0x80 bytes past cursor+0x10, but
        // supplemental_size = 0x10 means npd_end > supplemental_end.
        let mut records = vec![0u8; 0x10];
        records[0..4].copy_from_slice(&SCE_SUPPLEMENTAL_KIND_NPDRM.to_be_bytes());
        records[4..8].copy_from_slice(&0x10u32.to_be_bytes());
        let data = build_synthetic_supplemental_chain(&records);
        let err = find_npd_header_info(&data).unwrap_err();
        assert!(matches!(err, SceError::HeaderOffsetOutOfRange { .. }));
    }

    /// Build a single NPDRM record of size 0x90 (record header +
    /// 0x80 NPD body), license set, content_id filled with `cid_fill`.
    /// Returned buffer is ready for `find_npd_header_info`.
    fn build_synthetic_npdrm_record(license_wire: u32, cid_fill: u8) -> Vec<u8> {
        let mut records = vec![0u8; 0x90];
        records[0..4].copy_from_slice(&SCE_SUPPLEMENTAL_KIND_NPDRM.to_be_bytes());
        records[4..8].copy_from_slice(&0x90u32.to_be_bytes());
        // NPD body at +0x10; license at NPD+0x08 = record offset 0x18.
        records[0x18..0x1C].copy_from_slice(&license_wire.to_be_bytes());
        // content_id at NPD+0x10 = record offset 0x20, 0x30 bytes.
        records[0x20..0x50].fill(cid_fill);
        build_synthetic_supplemental_chain(&records)
    }

    #[test]
    fn find_npd_content_id_no_nul_returns_full_48_bytes() {
        let data = build_synthetic_npdrm_record(1, b'X');
        let info = find_npd_header_info(&data).unwrap().unwrap();
        assert_eq!(info.content_id.len(), 0x30);
        assert!(info.content_id.chars().all(|c| c == 'X'));
    }

    #[test]
    fn find_npd_valid_license_values_parse_to_enum_variants() {
        for (wire, expected) in [
            (1u32, NpdLicense::Network),
            (2u32, NpdLicense::Local),
            (3u32, NpdLicense::Free),
        ] {
            let data = build_synthetic_npdrm_record(wire, 0);
            let info = find_npd_header_info(&data).unwrap().unwrap();
            assert_eq!(info.license, expected, "wire value 0x{wire:x}");
        }
    }

    #[test]
    fn find_npd_license_zero_returns_npdrm_bad_license() {
        let data = build_synthetic_npdrm_record(0, 0);
        let err = find_npd_header_info(&data).unwrap_err();
        assert!(matches!(err, SceError::NpdrmBadLicense { got: 0 }));
    }

    #[test]
    fn find_npd_license_four_returns_npdrm_bad_license() {
        let data = build_synthetic_npdrm_record(4, 0);
        let err = find_npd_header_info(&data).unwrap_err();
        assert!(matches!(err, SceError::NpdrmBadLicense { got: 4 }));
    }

    #[test]
    fn find_npd_license_u32_max_returns_npdrm_bad_license() {
        let data = build_synthetic_npdrm_record(u32::MAX, 0);
        let err = find_npd_header_info(&data).unwrap_err();
        assert!(matches!(err, SceError::NpdrmBadLicense { got: u32::MAX }));
    }
}
