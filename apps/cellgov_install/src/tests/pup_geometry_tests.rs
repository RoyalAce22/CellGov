//! Header geometry a crafted PUP names:
//!
//! - table counts;
//! - declared regions;
//! - entry extents whose sums would wrap.

use super::*;

/// A header that names `file_count` entries, `header_length` and
/// `data_length`, then `trailing` zero bytes.
fn header(file_count: u64, header_length: u64, data_length: u64, trailing: usize) -> Vec<u8> {
    let mut data = vec![0u8; 0x30 + trailing];
    data[0..8].copy_from_slice(b"SCEUF\0\0\0");
    data[0x18..0x20].copy_from_slice(&file_count.to_be_bytes());
    data[0x20..0x28].copy_from_slice(&header_length.to_be_bytes());
    data[0x28..0x30].copy_from_slice(&data_length.to_be_bytes());
    data
}

#[test]
fn a_file_count_whose_table_size_wraps_is_a_truncated_table_not_a_capacity_panic() {
    // Each entry costs 0x40 table bytes, so this count's table size
    // wraps u64 to 0x40. An unchecked product then ends the tables at
    // 0x70. That equals the file length and the declared
    // header_length, so every later bound passes.
    let file_count = (1u64 << 58) + 1;
    assert_eq!(file_count.wrapping_mul(0x40).wrapping_add(0x30), 0x70);
    let data = header(file_count, 0x70, 0, 0x40);
    let err = parse(&data).unwrap_err();
    assert!(
        matches!(
            err,
            PupError::TablesTruncated {
                file_count: got,
                file_len: 0x70
            } if got == file_count
        ),
        "got {err:?}"
    );
}

#[test]
fn the_largest_count_the_bytes_after_the_header_hold_is_accepted_and_one_more_is_not() {
    // Two entries' tables fill 0x80 bytes exactly.
    let data = header(2, 0xB0, 0, 0x80);
    assert_eq!(parse(&data).unwrap().entries.len(), 2);

    let data = header(3, 0xF0, 0, 0x80);
    assert!(
        matches!(
            parse(&data),
            Err(PupError::TablesTruncated {
                file_count: 3,
                file_len: 0xB0
            })
        ),
        "a third entry's 0x40 table bytes are not there"
    );
}

#[test]
fn tables_past_the_declared_header_length_are_named() {
    let data = header(1, 0x60, 0, 0x40);
    let err = parse(&data).unwrap_err();
    assert!(
        matches!(
            err,
            PupError::TablesPastHeader {
                tables_end: 0x70,
                header_length: 0x60
            }
        ),
        "got {err:?}"
    );
}

#[test]
fn a_declared_payload_past_the_file_is_named_at_parse() {
    let data = header(0, 0x30, 1, 0);
    let err = parse(&data).unwrap_err();
    assert!(
        matches!(
            err,
            PupError::DeclaredSizePastFile {
                header_length: 0x30,
                data_length: 1,
                file_len: 0x30
            }
        ),
        "got {err:?}"
    );
}

#[test]
fn declared_regions_whose_sum_wraps_are_past_the_file_not_inside_it() {
    // 0x30 + (u64::MAX - 0x2F) wraps to 0, which an unchecked sum would
    // read as regions that end inside the file.
    let data_length = u64::MAX - 0x2F;
    let data = header(0, 0x30, data_length, 0);
    let err = parse(&data).unwrap_err();
    assert!(
        matches!(
            err,
            PupError::DeclaredSizePastFile {
                header_length: 0x30,
                data_length: got,
                file_len: 0x30
            } if got == data_length
        ),
        "got {err:?}"
    );
}

/// A one-entry PUP whose entry declares `data_offset` / `data_length`
/// over a file of 0x80 bytes.
fn one_entry(data_offset: u64, data_length: u64) -> Vec<u8> {
    let mut data = header(1, 0x70, 0x10, 0x50);
    data[0x30..0x38].copy_from_slice(&0x300u64.to_be_bytes());
    data[0x38..0x40].copy_from_slice(&data_offset.to_be_bytes());
    data[0x40..0x48].copy_from_slice(&data_length.to_be_bytes());
    data
}

#[test]
fn an_entry_extent_whose_end_wraps_to_zero_is_past_the_file() {
    // 0x80 + (u64::MAX - 0x7F) wraps to 0: an unchecked end would read
    // as the empty extent at the file start and pass every bound.
    let data = one_entry(0x80, u64::MAX - 0x7F);
    let pup = parse(&data).unwrap();
    assert!(
        matches!(
            entry_payload(&data, &pup, 0x300),
            Err(PupError::EntryPastFile {
                position: 0,
                entry_id: 0x300
            })
        ),
        "the extent leaves the file"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn the_hash_gate_refuses_an_entry_extent_whose_end_wraps() {
    let keys = crate::test_support::synthetic_vault();
    let mut data = crate::test_support::build_pup(&keys, 0, &[(0x300, b"payload")]);
    // In a one-entry build, 0x38 and 0x40 start entry 0's offset and
    // length fields.
    let offset = u64::try_from(data.len()).unwrap();
    data[0x38..0x40].copy_from_slice(&offset.to_be_bytes());
    data[0x40..0x48].copy_from_slice(&(u64::MAX - (offset - 1)).to_be_bytes());
    let pup = parse(&data).unwrap();
    assert!(
        matches!(
            validate_hashes(&data, &pup, &keys),
            Err(PupError::EntryPastFile {
                position: 0,
                entry_id: 0x300
            })
        ),
        "the hash gate names the extent before hashing it"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn the_hash_gate_refuses_an_offset_of_u64_max_before_any_slice_leans_on_it() {
    let keys = crate::test_support::synthetic_vault();
    let mut data = crate::test_support::build_pup(&keys, 0, &[(0x300, b"payload")]);
    data[0x38..0x40].copy_from_slice(&u64::MAX.to_be_bytes());
    data[0x40..0x48].copy_from_slice(&2u64.to_be_bytes());
    let pup = parse(&data).unwrap();
    assert!(
        matches!(
            validate_hashes(&data, &pup, &keys),
            Err(PupError::EntryPastFile {
                position: 0,
                entry_id: 0x300
            })
        ),
        "the installer slices update_files only after this gate"
    );
}

#[test]
fn an_entry_reads_its_reserved_bytes_from_the_record() {
    let mut data = one_entry(0x70, 0x10);
    data[0x48..0x50].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
    let pup = parse(&data).unwrap();
    assert_eq!(pup.entries[0]._padding, [1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn a_hash_record_reads_its_index_hash_and_reserved_bytes_from_the_record() {
    let mut data = one_entry(0x70, 0x10);
    // The hash table starts at 0x50 in a one-entry layout.
    data[0x50..0x58].copy_from_slice(&7u64.to_be_bytes());
    let hash: [u8; 20] = std::array::from_fn(|i| 0xA0 + i as u8);
    data[0x58..0x6C].copy_from_slice(&hash);
    data[0x6C..0x70].copy_from_slice(&[0xC1, 0xC2, 0xC3, 0xC4]);
    let pup = parse(&data).unwrap();
    assert_eq!(pup.hashes[0].index, 7);
    assert_eq!(pup.hashes[0].hash, hash);
    assert_eq!(pup.hashes[0]._padding, [0xC1, 0xC2, 0xC3, 0xC4]);
}

/// A disc's update PUP sits in a file padded well past the header and
/// payload regions it declares. The reference implementation refuses
/// only a declared size past the file.
#[test]
fn a_file_longer_than_its_declared_regions_is_accepted() {
    let mut data = one_entry(0x70, 0x10);
    data.resize(0x1000, 0);
    let pup = parse(&data).unwrap();
    assert_eq!(entry_payload(&data, &pup, 0x300).unwrap().len(), 0x10);
}
