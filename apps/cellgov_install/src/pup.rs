//! PUP (PlayStation Update Package) container parser.
//!
//! All multi-byte fields are big-endian. Payloads are themselves SCE-encrypted;
//! decryption is the caller's responsibility (see `sce`).

use hmac::{Hmac, Mac};
use sha1::Sha1;

use crate::crypto::PUP_KEY;

/// On-disk PUP header at file offset 0, 0x30 bytes, all fields big-endian.
#[derive(Debug)]
#[repr(C)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct PupHeader {
    /// Offset 0x00: must equal `b"SCEUF\0\0\0"`.
    pub magic: [u8; 8],
    /// Offset 0x08: PUP container format version.
    pub package_version: u64,
    /// Offset 0x10: firmware image version (e.g. 0x0004008200000000 for 4.82).
    pub image_version: u64,
    /// Offset 0x18: number of records in both the entry and hash tables.
    pub file_count: u64,
    /// Offset 0x20: byte length of the header region (header + entry table + hash table).
    pub header_length: u64,
    /// Offset 0x28: byte length of the payload region following the header.
    pub data_length: u64,
}

/// One record in the entry table; 0x20 bytes, big-endian.
#[derive(Debug)]
#[repr(C)]
pub struct PupFileEntry {
    /// Offset 0x00: stable file identifier (matches Sony's known-id table).
    pub entry_id: u64,
    /// Offset 0x08: payload start, measured from the PUP file base.
    pub data_offset: u64,
    /// Offset 0x10: payload length in bytes.
    pub data_length: u64,
    /// Offset 0x18: 8 reserved bytes, observed zero.
    pub _padding: [u8; 8],
}

/// One record in the hash table; 0x20 bytes, big-endian.
///
/// `index` is the record's own position in the hash table, not the
/// sibling entry's `entry_id`; payload-to-hash mapping is positional.
#[derive(Debug)]
#[repr(C)]
pub struct PupHashEntry {
    /// Offset 0x00: this record's positional index, must equal its slot number.
    pub index: u64,
    /// Offset 0x08: HMAC-SHA1 of the referenced payload under `PUP_KEY`.
    pub hash: [u8; 20],
    /// Offset 0x1C: 4 reserved bytes, observed zero.
    pub _padding: [u8; 4],
}

/// Parsed PUP: image version plus the entry and hash tables (payloads stay in the input buffer).
#[derive(Debug)]
pub struct Pup {
    /// Firmware image version copied from the header.
    pub image_version: u64,
    /// Entry table records in declaration order.
    pub entries: Vec<PupFileEntry>,
    /// Hash table records in declaration order; `hashes[i]` covers `entries[i]`'s payload.
    pub hashes: Vec<PupHashEntry>,
}

fn read_be_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes(
        data[offset..offset + 8]
            .try_into()
            .expect("invariant: fixed-length 8-byte slice always converts to [u8; 8]"),
    )
}

/// Why PUP parsing or hash validation failed.
#[derive(Debug, thiserror::Error)]
pub enum PupError {
    /// Input is shorter than the PUP header.
    #[error("PUP file too small for header (got {len} bytes)")]
    TooSmall {
        /// Observed input length in bytes.
        len: usize,
    },
    /// SCEUF magic mismatch; carries the first 4 observed bytes.
    #[error("bad PUP magic: {:02x}{:02x}{:02x}{:02x}", _0[0], _0[1], _0[2], _0[3])]
    BadMagic([u8; 4]),
    /// Entry / hash tables would extend past the file end.
    #[error("PUP file truncated: tables need >= 0x{required:x} bytes, file is 0x{file_len:x}")]
    TablesTruncated {
        /// Minimum byte length the header plus tables would occupy.
        required: usize,
        /// Actual byte length of the input buffer.
        file_len: usize,
    },
    /// Entry and hash table lengths disagree (a malformed PUP).
    #[error("PUP entry table ({entries}) and hash table ({hashes}) length disagree")]
    TableLengthMismatch {
        /// Number of records in the entry table.
        entries: usize,
        /// Number of records in the hash table.
        hashes: usize,
    },
    /// Hash record's `index` field disagrees with its slot position.
    #[error("PUP hash record at position {position} declares index {declared}")]
    HashIndexMismatch {
        /// Slot position in the hash table (zero-based).
        position: usize,
        /// Value the record's `index` field carries.
        declared: u64,
    },
    /// Payload referenced by an entry extends past the file end.
    #[error("entry {position} (id=0x{entry_id:x}) extends past file end")]
    EntryPastFile {
        /// Slot position of the offending entry in the entry table.
        position: usize,
        /// `entry_id` field of the offending entry.
        entry_id: u64,
    },
    /// HMAC-SHA1 initialization failed (wrong key length).
    #[error("HMAC init: {0}")]
    HmacInit(#[source] hmac::digest::InvalidLength),
    /// HMAC-SHA1 mismatch between computed and recorded hash.
    #[error("HMAC mismatch for entry {position} (id=0x{entry_id:x})")]
    HmacMismatch {
        /// Slot position of the entry whose payload failed verification.
        position: usize,
        /// `entry_id` field of the failing entry.
        entry_id: u64,
    },
}

/// Parse a PUP buffer into its header tables; does not verify payload hashes.
pub fn parse(data: &[u8]) -> Result<Pup, PupError> {
    if data.len() < 0x30 {
        return Err(PupError::TooSmall { len: data.len() });
    }
    if &data[0..8] != b"SCEUF\0\0\0" {
        return Err(PupError::BadMagic([data[0], data[1], data[2], data[3]]));
    }

    let image_version = read_be_u64(data, 0x10);
    let file_count = read_be_u64(data, 0x18) as usize;

    let entry_table_start = 0x30usize;
    let hash_table_start = entry_table_start + file_count * 0x20;
    let required = hash_table_start + file_count * 0x20;

    if required > data.len() {
        return Err(PupError::TablesTruncated {
            required,
            file_len: data.len(),
        });
    }

    let mut entries = Vec::with_capacity(file_count);
    for i in 0..file_count {
        let off = entry_table_start + i * 0x20;
        entries.push(PupFileEntry {
            entry_id: read_be_u64(data, off),
            data_offset: read_be_u64(data, off + 8),
            data_length: read_be_u64(data, off + 16),
            _padding: [0u8; 8],
        });
    }

    let mut hashes = Vec::with_capacity(file_count);
    for i in 0..file_count {
        let off = hash_table_start + i * 0x20;
        let mut hash = [0u8; 20];
        hash.copy_from_slice(&data[off + 8..off + 28]);
        hashes.push(PupHashEntry {
            index: read_be_u64(data, off),
            hash,
            _padding: [0u8; 4],
        });
    }

    Ok(Pup {
        image_version,
        entries,
        hashes,
    })
}

type HmacSha1 = Hmac<Sha1>;

/// Recompute HMAC-SHA1 of each payload under `PUP_KEY` and compare against the recorded hash.
pub fn validate_hashes(data: &[u8], pup: &Pup) -> Result<(), PupError> {
    if pup.entries.len() != pup.hashes.len() {
        return Err(PupError::TableLengthMismatch {
            entries: pup.entries.len(),
            hashes: pup.hashes.len(),
        });
    }
    for (i, entry) in pup.entries.iter().enumerate() {
        if pup.hashes[i].index != i as u64 {
            return Err(PupError::HashIndexMismatch {
                position: i,
                declared: pup.hashes[i].index,
            });
        }
        let start = entry.data_offset as usize;
        let end = start + entry.data_length as usize;
        if end > data.len() {
            return Err(PupError::EntryPastFile {
                position: i,
                entry_id: entry.entry_id,
            });
        }
        let mut mac = HmacSha1::new_from_slice(&PUP_KEY).map_err(PupError::HmacInit)?;
        mac.update(&data[start..end]);
        let result = mac.finalize().into_bytes();
        if result.as_slice() != pup.hashes[i].hash {
            return Err(PupError::HmacMismatch {
                position: i,
                entry_id: entry.entry_id,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/pup_tests.rs"]
mod tests;
