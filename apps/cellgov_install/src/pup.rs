//! PUP (PlayStation Update Package) container parser.
//!
//! All multi-byte fields are big-endian. Payloads are themselves SCE-encrypted;
//! decryption is the caller's responsibility (see `sce`). Payload HMAC
//! validation runs under the vault's PUP HMAC key, so it sits behind
//! the `decrypt` feature; parsing does not.

#[cfg(feature = "decrypt")]
use hmac::{Hmac, Mac};
#[cfg(feature = "decrypt")]
use sha1::Sha1;

#[cfg(feature = "decrypt")]
use crate::keys::KeyVault;
use cellgov_ps3_abi::pup::{parse_pup_version_txt, ENTRY_ID_VERSION_TXT};

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
pub fn parse(data: &[u8]) -> Result<Pup, PupError> {
    if data.len() < 0x30 {
        return Err(PupError::TooSmall { len: data.len() });
    }
    if &data[0..8] != b"SCEUF\0\0\0" {
        return Err(PupError::BadMagic([data[0], data[1], data[2], data[3]]));
    }

    let image_version = read_be_u64(data, 0x10);
    // Saturating throughout, so a count near u64::MAX names tables past
    // the file instead of wrapping into ones that fit.
    let file_count = usize::try_from(read_be_u64(data, 0x18)).unwrap_or(usize::MAX);
    let table_len = file_count.saturating_mul(0x20);

    let entry_table_start = 0x30usize;
    let hash_table_start = entry_table_start.saturating_add(table_len);
    let required = hash_table_start.saturating_add(table_len);

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
    entry_extent(data, entry).ok_or(PupError::EntryPastFile { position, entry_id })
}

/// The bytes `entry` declares, or `None` when its extent leaves `data`.
///
/// Slicing checks the extent without an offset-plus-length sum, so an
/// extent near `u64::MAX` cannot wrap. A zero-length entry at the end
/// of the file is an empty payload.
fn entry_extent<'a>(data: &'a [u8], entry: &PupFileEntry) -> Option<&'a [u8]> {
    let start = usize::try_from(entry.data_offset).ok()?;
    let len = usize::try_from(entry.data_length).ok()?;
    data.get(start..)?.get(..len)
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
    for (i, entry) in pup.entries.iter().enumerate() {
        if pup.hashes[i].index != i as u64 {
            return Err(PupError::HashIndexMismatch {
                position: i,
                declared: pup.hashes[i].index,
            });
        }
        let Some(payload) = entry_extent(data, entry) else {
            return Err(PupError::EntryPastFile {
                position: i,
                entry_id: entry.entry_id,
            });
        };
        let mut mac = HmacSha1::new_from_slice(pup_key).map_err(PupError::HmacInit)?;
        mac.update(payload);
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

#[cfg(test)]
#[path = "tests/pup_version_tests.rs"]
mod version_tests;
