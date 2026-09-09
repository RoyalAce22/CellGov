//! The keyless read of a PUP's `version.txt` payload.

use super::*;

/// A PUP whose table names `entries`, each payload laid out after the
/// tables. The hash table is zero: nothing here validates it.
fn pup_with(entries: &[(u64, &[u8])]) -> Vec<u8> {
    let header_len = 0x30 + entries.len() * 0x40;
    let mut out = Vec::new();
    out.extend_from_slice(b"SCEUF\0\0\0");
    out.extend_from_slice(&1u64.to_be_bytes());
    out.extend_from_slice(&0u64.to_be_bytes());
    out.extend_from_slice(&(entries.len() as u64).to_be_bytes());
    out.extend_from_slice(&(header_len as u64).to_be_bytes());
    let payload_len: usize = entries.iter().map(|(_, d)| d.len()).sum();
    out.extend_from_slice(&(payload_len as u64).to_be_bytes());
    let mut offset = header_len;
    for (id, data) in entries {
        out.extend_from_slice(&id.to_be_bytes());
        out.extend_from_slice(&(offset as u64).to_be_bytes());
        out.extend_from_slice(&(data.len() as u64).to_be_bytes());
        out.extend_from_slice(&[0u8; 8]);
        offset += data.len();
    }
    for i in 0..entries.len() {
        out.extend_from_slice(&(i as u64).to_be_bytes());
        out.extend_from_slice(&[0u8; 24]);
    }
    for (_, data) in entries {
        out.extend_from_slice(data);
    }
    out
}

#[test]
fn the_version_payload_is_read_without_a_key() {
    let data = pup_with(&[(0x101, b"<license/>"), (ENTRY_ID_VERSION_TXT, b"2.76\n")]);
    let pup = parse(&data).unwrap();
    assert_eq!(version_key(&data, &pup).unwrap(), "2.76");
}

#[test]
fn a_pup_without_a_version_payload_is_refused_by_entry_id() {
    let data = pup_with(&[(0x101, b"<license/>")]);
    let pup = parse(&data).unwrap();
    assert!(matches!(
        version_key(&data, &pup),
        Err(PupError::NoEntry {
            entry_id: ENTRY_ID_VERSION_TXT
        })
    ));
}

#[test]
fn a_version_payload_that_is_not_a_version_is_quoted_in_the_refusal() {
    let data = pup_with(&[(ENTRY_ID_VERSION_TXT, b"release:04.9300:\nmore")]);
    let pup = parse(&data).unwrap();
    let err = version_key(&data, &pup).unwrap_err();
    let PupError::VersionUnparseable { text } = &err else {
        panic!("expected VersionUnparseable, got {err}");
    };
    assert_eq!(text, "release:04.9300:");
    assert!(err.to_string().contains("0x100"), "{err}");
}

#[test]
fn a_version_extent_past_the_file_is_not_an_absent_entry() {
    let mut data = pup_with(&[(ENTRY_ID_VERSION_TXT, b"4.93\n")]);
    data.truncate(data.len() - 2);
    let pup = parse(&data).unwrap();
    assert!(matches!(
        version_key(&data, &pup),
        Err(PupError::EntryPastFile {
            position: 0,
            entry_id: ENTRY_ID_VERSION_TXT
        })
    ));
}

/// Byte range of entry 0's `data_offset` field in a `pup_with` buffer.
const ENTRY0_OFFSET_FIELD: std::ops::Range<usize> = 0x38..0x40;
/// Byte range of entry 0's `data_length` field in a `pup_with` buffer.
const ENTRY0_LENGTH_FIELD: std::ops::Range<usize> = 0x40..0x48;

#[test]
fn an_offset_near_u64_max_is_past_the_file_not_wrapped_into_it() {
    let mut data = pup_with(&[(ENTRY_ID_VERSION_TXT, b"4.93\n")]);
    data[ENTRY0_OFFSET_FIELD].copy_from_slice(&u64::MAX.to_be_bytes());
    let pup = parse(&data).unwrap();
    assert!(matches!(
        entry_payload(&data, &pup, ENTRY_ID_VERSION_TXT),
        Err(PupError::EntryPastFile {
            position: 0,
            entry_id: ENTRY_ID_VERSION_TXT
        })
    ));
}

#[test]
fn a_length_near_u64_max_is_past_the_file_not_wrapped_into_it() {
    let mut data = pup_with(&[(ENTRY_ID_VERSION_TXT, b"4.93\n")]);
    data[ENTRY0_LENGTH_FIELD].copy_from_slice(&u64::MAX.to_be_bytes());
    let pup = parse(&data).unwrap();
    assert!(matches!(
        entry_payload(&data, &pup, ENTRY_ID_VERSION_TXT),
        Err(PupError::EntryPastFile {
            position: 0,
            entry_id: ENTRY_ID_VERSION_TXT
        })
    ));
}

/// The hash gate runs before the version read on every caller's path,
/// so it refuses the same extents the same way.
#[cfg(feature = "decrypt")]
#[test]
fn the_hash_gate_refuses_a_length_near_u64_max_by_name() {
    let keys = crate::test_support::synthetic_vault();
    let mut data = crate::test_support::build_pup(&keys, 0, &[(ENTRY_ID_VERSION_TXT, b"4.93\n")]);
    data[ENTRY0_LENGTH_FIELD].copy_from_slice(&u64::MAX.to_be_bytes());
    let pup = parse(&data).unwrap();
    assert!(matches!(
        validate_hashes(&data, &pup, &keys),
        Err(PupError::EntryPastFile {
            position: 0,
            entry_id: ENTRY_ID_VERSION_TXT
        })
    ));
}

#[test]
fn a_file_count_near_u64_max_is_a_truncated_table_not_a_wrap() {
    let mut data = vec![0u8; 0x30];
    data[0..8].copy_from_slice(b"SCEUF\0\0\0");
    data[0x18..0x20].copy_from_slice(&u64::MAX.to_be_bytes());
    assert!(matches!(
        parse(&data),
        Err(PupError::TablesTruncated { file_len: 0x30, .. })
    ));
}

#[test]
fn a_zero_length_entry_at_the_end_of_the_file_is_an_empty_payload() {
    let data = pup_with(&[(ENTRY_ID_VERSION_TXT, b"")]);
    let pup = parse(&data).unwrap();
    assert_eq!(
        entry_payload(&data, &pup, ENTRY_ID_VERSION_TXT).unwrap(),
        b""
    );
    assert!(matches!(
        version_key(&data, &pup),
        Err(PupError::VersionUnparseable { text }) if text.is_empty()
    ));
}

/// Matches the reference implementation's read order.
#[test]
fn the_first_of_two_entries_with_one_id_is_the_one_read() {
    let data = pup_with(&[
        (ENTRY_ID_VERSION_TXT, b"1.94\n"),
        (ENTRY_ID_VERSION_TXT, b"2.76\n"),
    ]);
    let pup = parse(&data).unwrap();
    assert_eq!(version_key(&data, &pup).unwrap(), "1.94");
}

#[test]
fn a_version_line_is_read_through_a_carriage_return_and_padding() {
    let cases: [(&[u8], &str); 3] = [
        (b"4.93\r\n", "4.93"),
        (b" 2.76 \n", "2.76"),
        (b"1.94", "1.94"),
    ];
    for (payload, want) in cases {
        let data = pup_with(&[(ENTRY_ID_VERSION_TXT, payload)]);
        let pup = parse(&data).unwrap();
        assert_eq!(version_key(&data, &pup).unwrap(), want, "{payload:?}");
    }
}

#[test]
fn entry_payload_finds_an_entry_by_id_not_position() {
    let data = pup_with(&[(0x200, b"self"), (0x300, b"tar"), (0x100, b"4.93\n")]);
    let pup = parse(&data).unwrap();
    assert_eq!(entry_payload(&data, &pup, 0x300).unwrap(), b"tar");
    assert_eq!(entry_payload(&data, &pup, 0x100).unwrap(), b"4.93\n");
    assert!(matches!(
        entry_payload(&data, &pup, 0x400),
        Err(PupError::NoEntry { entry_id: 0x400 })
    ));
}
