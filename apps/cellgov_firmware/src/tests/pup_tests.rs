//! PUP container parsing and per-entry HMAC-SHA1 hash validation.

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
