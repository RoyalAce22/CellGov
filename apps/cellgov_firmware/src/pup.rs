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

pub fn parse(data: &[u8]) -> Result<Pup, String> {
    if data.len() < 0x30 {
        return Err("file too small for PUP header".into());
    }
    if &data[0..8] != b"SCEUF\0\0\0" {
        return Err(format!(
            "bad PUP magic: {:02x}{:02x}{:02x}{:02x}",
            data[0], data[1], data[2], data[3]
        ));
    }

    let image_version = read_be_u64(data, 0x10);
    let file_count = read_be_u64(data, 0x18) as usize;

    let entry_table_start = 0x30usize;
    let hash_table_start = entry_table_start + file_count * 0x20;

    if hash_table_start + file_count * 0x20 > data.len() {
        return Err("PUP file truncated (tables exceed file size)".into());
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

pub fn validate_hashes(data: &[u8], pup: &Pup) -> Result<(), String> {
    if pup.entries.len() != pup.hashes.len() {
        return Err(format!(
            "PUP entry table ({}) and hash table ({}) length disagree",
            pup.entries.len(),
            pup.hashes.len()
        ));
    }
    for (i, entry) in pup.entries.iter().enumerate() {
        if pup.hashes[i].index != i as u64 {
            return Err(format!(
                "PUP hash record at position {i} declares index {} (should be {i})",
                pup.hashes[i].index,
            ));
        }
        let start = entry.data_offset as usize;
        let end = start + entry.data_length as usize;
        if end > data.len() {
            return Err(format!(
                "entry {} (id=0x{:x}) extends past file end",
                i, entry.entry_id
            ));
        }
        let mut mac = HmacSha1::new_from_slice(&PUP_KEY).map_err(|e| format!("HMAC init: {e}"))?;
        mac.update(&data[start..end]);
        let result = mac.finalize().into_bytes();
        if result.as_slice() != pup.hashes[i].hash {
            return Err(format!(
                "HMAC mismatch for entry {} (id=0x{:x})",
                i, entry.entry_id
            ));
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
        assert!(parse(&data).unwrap_err().contains("bad PUP magic"));
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
        assert!(err.contains("HMAC mismatch"));
    }

    #[test]
    fn validate_hashes_rejects_hash_record_with_wrong_index() {
        let (mut data, _) = build_one_entry_pup(0x300, b"payload bytes here");
        let hash_table_start = 0x30 + 0x20;
        data[hash_table_start..hash_table_start + 8].copy_from_slice(&5u64.to_be_bytes());
        let pup = parse(&data).expect("parse");
        let err = validate_hashes(&data, &pup).unwrap_err();
        assert!(err.contains("declares index"));
    }
}
