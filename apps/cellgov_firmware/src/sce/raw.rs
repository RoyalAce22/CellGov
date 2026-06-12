//! Byte-level readers, SCE header layouts, and the container /
//! supplemental-chain parses over them.

use super::error::SceError;

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

pub(super) fn read_be_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes(
        data[offset..offset + 8]
            .try_into()
            .expect("invariant: fixed-length 8-byte slice always converts to [u8; 8]"),
    )
}

pub(super) fn read_be_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(
        data[offset..offset + 4]
            .try_into()
            .expect("invariant: fixed-length 4-byte slice always converts to [u8; 4]"),
    )
}

pub(super) fn read_be_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes(
        data[offset..offset + 2]
            .try_into()
            .expect("invariant: fixed-length 2-byte slice always converts to [u8; 2]"),
    )
}

/// Checked addition routed to [`SceError::HeaderOffsetOutOfRange`].
/// Used on file-derived offsets / sizes where overflow would
/// wrap the bounds check and let downstream indexing panic.
pub(super) fn checked_add_oob(a: usize, b: usize, what: &'static str) -> Result<usize, SceError> {
    a.checked_add(b)
        .ok_or(SceError::HeaderOffsetOutOfRange { what })
}

/// Checked multiplication routed to [`SceError::HeaderOffsetOutOfRange`].
/// Used on counts-times-element-size products derived from file
/// bytes (e.g. `e_phnum * e_phentsize`, `section_count * 0x30`).
pub(super) fn checked_mul_oob(a: usize, b: usize, what: &'static str) -> Result<usize, SceError> {
    a.checked_mul(b)
        .ok_or(SceError::HeaderOffsetOutOfRange { what })
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

/// Program authority id from a SELF's plaintext program
/// identification header.
///
/// The extended header at file offset 0x20 carries
/// `program_identification_hdr_offset` (u64 BE at offset 0x28); the
/// authority id is the first u64 of that header. Field layout per
/// RPCS3 `unself.h` / `unself.cpp`. Both
/// headers are plaintext -- no decryption is involved, so the id is
/// readable from NPDRM-wrapped SELFs without RAP material.
///
/// # Errors
///
/// [`SceError::BadMagic`] / [`SceError::TooSmall`] for non-SCE input
/// (callers with possibly-raw-ELF bytes treat that as "no authority
/// id"), [`SceError::HeaderOffsetOutOfRange`] when the recorded
/// offset escapes the buffer.
pub fn parse_program_authority_id(data: &[u8]) -> Result<u64, SceError> {
    parse_sce_header(data)?;
    if data.len() < 0x30 {
        return Err(SceError::TooSmall {
            what: "SELF extended header",
            got: data.len(),
            need: 0x30,
        });
    }
    let pid_off = read_be_u64(data, 0x28);
    let Ok(pid_off) = usize::try_from(pid_off) else {
        return Err(SceError::HeaderOffsetOutOfRange {
            what: "program identification header",
        });
    };
    let end = checked_add_oob(pid_off, 8, "program identification header")?;
    if end > data.len() {
        return Err(SceError::HeaderOffsetOutOfRange {
            what: "program identification header",
        });
    }
    Ok(read_be_u64(data, pid_off))
}

/// Walk an SELF's supplemental-header chain and return the body of
/// the first record of `kind`, if present.
///
/// The extended header at file offset 0x20 carries
/// `supplemental_hdr_offset` (u64 BE at offset 0x58) and
/// `supplemental_hdr_size` (u64 BE at offset 0x60). The chain is a
/// sequence of `{type:u32, size:u32, next:u64, body}` records
/// totalling `supplemental_hdr_size` bytes; the returned body is the
/// `size - 0x10` bytes after the record header.
///
/// Returns `Ok(None)` when the chain is absent or carries no record
/// of `kind`.
pub(crate) fn find_supplemental_body(data: &[u8], kind: u32) -> Result<Option<&[u8]>, SceError> {
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
        let record_kind = read_be_u32(data, cursor);
        let record_size = read_be_u32(data, cursor + 4) as usize;
        let record_end =
            checked_add_oob(cursor, record_size, "SELF supplemental header record body")?;
        if record_size < 0x10 || record_end > supplemental_end {
            return Err(SceError::HeaderOffsetOutOfRange {
                what: "SELF supplemental header record body",
            });
        }
        if record_kind == kind {
            return Ok(Some(&data[record_header_end..record_end]));
        }
        cursor = record_end;
    }
    Ok(None)
}
