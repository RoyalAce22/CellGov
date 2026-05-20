//! PUP (PlayStation Update Package) container parser.
//!
//! All multi-byte fields are big-endian. Payloads are themselves SCE-encrypted;
//! decryption is the caller's responsibility (see `sce`).

use hmac::{Hmac, Mac};
use sha1::Sha1;

use crate::crypto::PUP_KEY;

#[derive(Debug)]
#[repr(C)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct PupHeader {
    pub magic: [u8; 8],
    pub package_version: u64,
    pub image_version: u64,
    pub file_count: u64,
    pub header_length: u64,
    pub data_length: u64,
}

#[derive(Debug)]
#[repr(C)]
pub struct PupFileEntry {
    pub entry_id: u64,
    pub data_offset: u64,
    pub data_length: u64,
    pub _padding: [u8; 8],
}

/// `index` is the record's own position in the hash table, not the
/// sibling entry's `entry_id`; payload-to-hash mapping is positional.
#[derive(Debug)]
#[repr(C)]
pub struct PupHashEntry {
    pub index: u64,
    pub hash: [u8; 20],
    pub _padding: [u8; 4],
}

#[derive(Debug)]
pub struct Pup {
    pub image_version: u64,
    pub entries: Vec<PupFileEntry>,
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
#[derive(Debug)]
pub enum PupError {
    /// Input is shorter than the PUP header.
    TooSmall { len: usize },
    /// SCEUF magic mismatch; carries the first 4 observed bytes.
    BadMagic([u8; 4]),
    /// Entry / hash tables would extend past the file end.
    TablesTruncated { required: usize, file_len: usize },
    /// Entry and hash table lengths disagree (a malformed PUP).
    TableLengthMismatch { entries: usize, hashes: usize },
    /// Hash record's `index` field disagrees with its slot position.
    HashIndexMismatch { position: usize, declared: u64 },
    /// Payload referenced by an entry extends past the file end.
    EntryPastFile { position: usize, entry_id: u64 },
    /// HMAC-SHA1 initialization failed (wrong key length).
    HmacInit(hmac::digest::InvalidLength),
    /// HMAC-SHA1 mismatch between computed and recorded hash.
    HmacMismatch { position: usize, entry_id: u64 },
}

impl std::fmt::Display for PupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooSmall { len } => write!(f, "PUP file too small for header (got {len} bytes)"),
            Self::BadMagic(bytes) => write!(
                f,
                "bad PUP magic: {:02x}{:02x}{:02x}{:02x}",
                bytes[0], bytes[1], bytes[2], bytes[3]
            ),
            Self::TablesTruncated { required, file_len } => write!(
                f,
                "PUP file truncated: tables need >= 0x{required:x} bytes, file is 0x{file_len:x}"
            ),
            Self::TableLengthMismatch { entries, hashes } => write!(
                f,
                "PUP entry table ({entries}) and hash table ({hashes}) length disagree"
            ),
            Self::HashIndexMismatch { position, declared } => write!(
                f,
                "PUP hash record at position {position} declares index {declared}"
            ),
            Self::EntryPastFile { position, entry_id } => write!(
                f,
                "entry {position} (id=0x{entry_id:x}) extends past file end"
            ),
            Self::HmacInit(e) => write!(f, "HMAC init: {e}"),
            Self::HmacMismatch { position, entry_id } => {
                write!(f, "HMAC mismatch for entry {position} (id=0x{entry_id:x})")
            }
        }
    }
}

impl std::error::Error for PupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::HmacInit(e) => Some(e),
            _ => None,
        }
    }
}

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
mod tests {
    use super::*;

    #[test]
    fn parse_rejects_short_data() {
        assert!(parse(&[0u8; 10]).is_err());
    }

    #[test]
    fn parse_rejects_bad_magic() {
        let mut data = [0u8; 0x30];
        data[0..8].copy_from_slice(b"NOTAPUP\0");
        assert!(matches!(parse(&data).unwrap_err(), PupError::BadMagic(_)));
    }

    #[test]
    fn parse_accepts_valid_empty_pup() {
        let mut data = [0u8; 0x30];
        data[0..8].copy_from_slice(b"SCEUF\0\0\0");
        let pup = parse(&data).unwrap();
        assert_eq!(pup.entries.len(), 0);
    }

    /// Build a single-entry PUP with a matching HMAC. Returns the
    /// assembled buffer and the computed hash.
    fn build_one_entry_pup(entry_id: u64, payload: &[u8]) -> (Vec<u8>, [u8; 20]) {
        let entry_table_start = 0x30usize;
        let hash_table_start = entry_table_start + 0x20;
        let payload_offset = hash_table_start + 0x20;
        let total = payload_offset + payload.len();

        let mut buf = vec![0u8; total];
        buf[0..8].copy_from_slice(b"SCEUF\0\0\0");
        buf[0x18..0x20].copy_from_slice(&1u64.to_be_bytes()); // file_count

        buf[entry_table_start..entry_table_start + 8].copy_from_slice(&entry_id.to_be_bytes());
        buf[entry_table_start + 8..entry_table_start + 0x10]
            .copy_from_slice(&(payload_offset as u64).to_be_bytes());
        buf[entry_table_start + 0x10..entry_table_start + 0x18]
            .copy_from_slice(&(payload.len() as u64).to_be_bytes());

        buf[payload_offset..payload_offset + payload.len()].copy_from_slice(payload);

        let mut mac = HmacSha1::new_from_slice(&PUP_KEY).expect("HMAC init");
        mac.update(payload);
        let mac_out = mac.finalize().into_bytes();
        let mut hash = [0u8; 20];
        hash.copy_from_slice(&mac_out);

        // Hash record: index (8) | hash (20) | padding (4) = 0x20 bytes.
        buf[hash_table_start..hash_table_start + 8].copy_from_slice(&0u64.to_be_bytes());
        buf[hash_table_start + 8..hash_table_start + 28].copy_from_slice(&hash);

        (buf, hash)
    }

    #[test]
    fn validate_hashes_accepts_correct_hmac() {
        let (data, _) = build_one_entry_pup(0x300, b"payload bytes here");
        let pup = parse(&data).expect("parse");
        validate_hashes(&data, &pup).expect("HMAC valid");
    }

    #[test]
    fn validate_hashes_rejects_corrupted_hash() {
        let (mut data, _) = build_one_entry_pup(0x300, b"payload bytes here");
        let hash_table_start = 0x30 + 0x20;
        data[hash_table_start + 8] ^= 0xFF;
        let pup = parse(&data).expect("parse");
        let err = validate_hashes(&data, &pup).unwrap_err();
        assert!(matches!(err, PupError::HmacMismatch { .. }));
    }

    #[test]
    fn validate_hashes_rejects_hash_record_with_wrong_index() {
        let (mut data, _) = build_one_entry_pup(0x300, b"payload bytes here");
        let hash_table_start = 0x30 + 0x20;
        data[hash_table_start..hash_table_start + 8].copy_from_slice(&5u64.to_be_bytes());
        let pup = parse(&data).expect("parse");
        let err = validate_hashes(&data, &pup).unwrap_err();
        assert!(matches!(err, PupError::HashIndexMismatch { .. }));
    }
}
