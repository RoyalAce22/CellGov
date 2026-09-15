//! PUP (PlayStation Update Package) container parser.
//!
//! All multi-byte fields are big-endian. Payloads are themselves SCE-encrypted;
//! decryption is the caller's responsibility (see `sce`). Payload HMAC
//! validation runs under the vault's PUP HMAC key, so it sits behind
//! the `decrypt` feature; parsing does not.

#![deny(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_lossless
)]

#[cfg(feature = "decrypt")]
use hmac::{Hmac, Mac};
#[cfg(feature = "decrypt")]
use sha1::Sha1;

use crate::field::{read_be_u64, usize_from_header};
#[cfg(feature = "decrypt")]
use crate::keys::KeyVault;
use cellgov_ps3_abi::format::pup::{
    parse_pup_version_txt, ENTRY_ID_VERSION_TXT, PUP_HEADER_SIZE, PUP_RECORD_SIZE,
};

/// Table bytes one `file_count` unit costs: an entry record plus its
/// hash record.
const RECORD_PAIR_LEN: usize = 2 * PUP_RECORD_SIZE;

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
    /// Offset 0x20: byte length of the header region.
    ///
    /// The region holds:
    ///
    /// - the header;
    /// - the entry and hash tables;
    /// - the 0x20 bytes a retail PUP places after the tables.
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

impl PupFileEntry {
    /// The bytes this entry declares, or `None` when its extent leaves
    /// `data`.
    ///
    /// The check forms no offset-plus-length sum, so an extent near
    /// `u64::MAX` cannot wrap. A zero-length entry at the end of the
    /// file is an empty payload.
    #[must_use]
    pub(crate) fn payload<'a>(&self, data: &'a [u8]) -> Option<&'a [u8]> {
        let start = usize_from_header(self.data_offset)?;
        let len = usize_from_header(self.data_length)?;
        data.get(start..)?.get(..len)
    }
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
    /// Offset 0x08: HMAC-SHA1 of the referenced payload under the PUP HMAC key.
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
    #[error(
        "PUP file truncated: {file_count} entries need 0x40 table bytes each past the \
         0x30-byte header, file is 0x{file_len:x}"
    )]
    TablesTruncated {
        /// `file_count` the header declares.
        file_count: u64,
        /// Actual byte length of the input buffer.
        file_len: usize,
    },
    /// The entry and hash tables run past the header region the header
    /// declares.
    ///
    /// This rule is CellGov's own. The reference implementation does
    /// not check it.
    #[error("PUP tables end at 0x{tables_end:x}, past header_length 0x{header_length:x}")]
    TablesPastHeader {
        /// Byte offset where the hash table ends.
        tables_end: usize,
        /// `header_length` the header declares.
        header_length: u64,
    },
    /// The header and payload regions the header declares run past the
    /// file end.
    #[error(
        "PUP declares 0x{header_length:x} header bytes and 0x{data_length:x} payload bytes, \
         past the 0x{file_len:x}-byte file"
    )]
    DeclaredSizePastFile {
        /// `header_length` the header declares.
        header_length: u64,
        /// `data_length` the header declares.
        data_length: u64,
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
    /// No entry in the table carries the id a reader asked for.
    #[error("PUP has no entry 0x{entry_id:x}")]
    NoEntry {
        /// The id looked up.
        entry_id: u64,
    },
    /// The `version.txt` payload does not spell a firmware version.
    #[error("PUP version.txt (entry 0x{ENTRY_ID_VERSION_TXT:x}) reads {text:?}, not a version")]
    VersionUnparseable {
        /// The payload's first line, cut at `VERSION_QUOTE_LEN` chars,
        /// with any bytes that are not UTF-8 replaced.
        text: String,
    },
    /// HMAC-SHA1 initialization failed (wrong key length).
    #[cfg(feature = "decrypt")]
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
    /// The key vault holds no PUP HMAC key.
    #[error("{0}")]
    Keys(#[from] crate::keys::KeyVaultError),
}

/// Parse a PUP buffer into its header tables; does not verify payload hashes.
///
/// # Errors
///
/// - [`PupError::TooSmall`] / [`PupError::BadMagic`] for input that is
///   not a PUP.
/// - [`PupError::TablesTruncated`] when the bytes after the header
///   cannot hold `file_count` entry and hash records.
/// - [`PupError::TablesPastHeader`] when those tables run past the
///   header region the header declares.
/// - [`PupError::DeclaredSizePastFile`] when the declared header and
///   payload regions run past the file.
pub fn parse(data: &[u8]) -> Result<Pup, PupError> {
    let Some((header, tables_and_payload)) = data.split_first_chunk::<PUP_HEADER_SIZE>() else {
        return Err(PupError::TooSmall { len: data.len() });
    };
    if &header[0..8] != b"SCEUF\0\0\0" {
        return Err(PupError::BadMagic([
            header[0], header[1], header[2], header[3],
        ]));
    }

    let image_version = read_be_u64(header, 0x10);
    let file_count = read_be_u64(header, 0x18);
    let header_length = read_be_u64(header, 0x20);
    let data_length = read_be_u64(header, 0x28);

    // The filter bounds the count by the bytes after the header before
    // any table size uses it. A crafted count then cannot overflow the
    // products below or size the table allocations.
    let truncated = || PupError::TablesTruncated {
        file_count,
        file_len: data.len(),
    };
    let count = usize_from_header(file_count)
        .filter(|&n| n <= tables_and_payload.len() / RECORD_PAIR_LEN)
        .ok_or_else(truncated)?;
    let table_len = count.checked_mul(PUP_RECORD_SIZE).ok_or_else(truncated)?;
    let tables_end = table_len
        .checked_mul(2)
        .and_then(|both| both.checked_add(PUP_HEADER_SIZE))
        .ok_or_else(truncated)?;
    // A `header_length` wider than `usize` ends past every table. The
    // declared-size check below refuses it.
    if usize_from_header(header_length).is_some_and(|len| tables_end > len) {
        return Err(PupError::TablesPastHeader {
            tables_end,
            header_length,
        });
    }
    let declared_end = header_length
        .checked_add(data_length)
        .and_then(usize_from_header);
    if declared_end.is_none_or(|end| end > data.len()) {
        return Err(PupError::DeclaredSizePastFile {
            header_length,
            data_length,
            file_len: data.len(),
        });
    }

    let (entry_table, rest) = tables_and_payload.split_at(table_len);
    let hash_table = &rest[..table_len];
    let entries = entry_table
        .chunks_exact(PUP_RECORD_SIZE)
        .map(|record| PupFileEntry {
            entry_id: read_be_u64(record, 0),
            data_offset: read_be_u64(record, 0x08),
            data_length: read_be_u64(record, 0x10),
            _padding: record[0x18..0x20]
                .try_into()
                .expect("invariant: an 8-byte slice converts to [u8; 8]"),
        })
        .collect();
    let hashes = hash_table
        .chunks_exact(PUP_RECORD_SIZE)
        .map(|record| PupHashEntry {
            index: read_be_u64(record, 0),
            hash: record[0x08..0x1C]
                .try_into()
                .expect("invariant: a 20-byte slice converts to [u8; 20]"),
            _padding: record[0x1C..0x20]
                .try_into()
                .expect("invariant: a 4-byte slice converts to [u8; 4]"),
        })
        .collect();

    Ok(Pup {
        image_version,
        entries,
        hashes,
    })
}

/// The payload the entry with `entry_id` names, bounds-checked.
///
/// # Errors
///
/// - [`PupError::NoEntry`] when the table names no such id.
/// - [`PupError::EntryPastFile`] when the extent it declares leaves the
///   buffer.
pub fn entry_payload<'a>(data: &'a [u8], pup: &Pup, entry_id: u64) -> Result<&'a [u8], PupError> {
    let (position, entry) = pup
        .entries
        .iter()
        .enumerate()
        .find(|(_, e)| e.entry_id == entry_id)
        .ok_or(PupError::NoEntry { entry_id })?;
    entry
        .payload(data)
        .ok_or(PupError::EntryPastFile { position, entry_id })
}

/// Longest prefix of an unparseable `version.txt` a refusal quotes.
const VERSION_QUOTE_LEN: usize = 32;

/// The firmware version key the PUP's `version.txt` payload names,
/// read without a key.
///
/// The header's `image_version` word also names the version; this text
/// is the spelling the console shows and the store keys an entry by.
///
/// # Errors
///
/// - [`PupError::NoEntry`] when the container carries no `version.txt`.
/// - [`PupError::EntryPastFile`] when its extent leaves the buffer.
/// - [`PupError::VersionUnparseable`] when the text is not one version.
pub fn version_key(data: &[u8], pup: &Pup) -> Result<String, PupError> {
    let payload = entry_payload(data, pup, ENTRY_ID_VERSION_TXT)?;
    let text = String::from_utf8_lossy(payload);
    parse_pup_version_txt(&text).ok_or_else(|| PupError::VersionUnparseable {
        text: text
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(VERSION_QUOTE_LEN)
            .collect(),
    })
}

#[cfg(feature = "decrypt")]
type HmacSha1 = Hmac<Sha1>;

/// Recompute HMAC-SHA1 of each payload under the vault's PUP HMAC key
/// and compare against the recorded hash.
///
/// # Errors
///
/// [`PupError::Keys`] when the vault has no PUP HMAC key, before any
/// payload is hashed.
#[cfg(feature = "decrypt")]
pub fn validate_hashes(data: &[u8], pup: &Pup, keys: &KeyVault) -> Result<(), PupError> {
    let pup_key = keys.pup_hmac()?;
    if pup.entries.len() != pup.hashes.len() {
        return Err(PupError::TableLengthMismatch {
            entries: pup.entries.len(),
            hashes: pup.hashes.len(),
        });
    }
    for (position, (entry, hash)) in pup.entries.iter().zip(&pup.hashes).enumerate() {
        if usize_from_header(hash.index) != Some(position) {
            return Err(PupError::HashIndexMismatch {
                position,
                declared: hash.index,
            });
        }
        let Some(payload) = entry.payload(data) else {
            return Err(PupError::EntryPastFile {
                position,
                entry_id: entry.entry_id,
            });
        };
        let mut mac = HmacSha1::new_from_slice(pup_key).map_err(PupError::HmacInit)?;
        mac.update(payload);
        let result = mac.finalize().into_bytes();
        if result.as_slice() != hash.hash {
            return Err(PupError::HmacMismatch {
                position,
                entry_id: entry.entry_id,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_lossless,
    reason = "fixtures lay out synthetic headers"
)]
#[path = "tests/pup_tests.rs"]
mod tests;

#[cfg(test)]
#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_lossless,
    reason = "fixtures lay out synthetic headers"
)]
#[path = "tests/pup_version_tests.rs"]
mod version_tests;

#[cfg(test)]
#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_lossless,
    reason = "fixtures lay out synthetic headers"
)]
#[path = "tests/pup_geometry_tests.rs"]
mod geometry_tests;
