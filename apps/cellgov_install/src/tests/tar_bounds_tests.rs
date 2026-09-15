//! Scan bounds of the USTAR parser:
//!
//! - where the scan stops when an archive omits the padding or the
//!   terminating block that POSIX gives it;
//! - the offset that a refusal past the first record names.

use super::*;

fn regular(name: &str, size: usize) -> [u8; BLOCK] {
    let mut h = [0u8; BLOCK];
    h[..name.len()].copy_from_slice(name.as_bytes());
    let size = format!("{size:011o}\0");
    h[0x7C..0x7C + size.len()].copy_from_slice(size.as_bytes());
    h[0x9C] = TYPE_REGULAR;
    h[MAGIC_FIELD_OFFSET..MAGIC_FIELD_OFFSET + USTAR_MAGIC.len()].copy_from_slice(USTAR_MAGIC);
    h
}

#[test]
fn a_last_payload_the_archive_leaves_unpadded_is_kept_and_ends_the_scan() {
    let mut data = regular("tail.bin", 5).to_vec();
    data.extend_from_slice(b"hello");
    let entries = parse(&data).expect("parse");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].data, b"hello");
}

#[test]
fn a_short_block_after_the_last_record_ends_the_scan() {
    let mut data = regular("a.bin", 1).to_vec();
    data.extend_from_slice(&[b'A'; BLOCK]);
    // Fewer bytes than one header block: no record can start here.
    data.extend_from_slice(&[0xFF; BLOCK - 1]);
    let entries = parse(&data).expect("parse");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].data, b"A");
}

#[test]
fn a_refusal_past_the_first_record_names_its_own_header_offset() {
    // Layout:
    // - 0x000 header;
    // - 0x200 one padded payload block;
    // - 0x400 directory header with no payload;
    // - 0x600 the refused header.
    let mut data = regular("a.bin", 1).to_vec();
    data.extend_from_slice(&[b'A'; BLOCK]);
    let mut dir = regular("d/", 0);
    dir[0x9C] = TYPE_DIRECTORY;
    data.extend_from_slice(&dir);
    let mut link = regular("link", 0);
    link[0x9C] = b'2';
    data.extend_from_slice(&link);
    let err = parse(&data).unwrap_err();
    assert!(
        matches!(
            err,
            TarParseError::UnsupportedFileType {
                offset: 0x600,
                filetype: b'2',
                ..
            }
        ),
        "got {err:?}"
    );

    data.truncate(0x600);
    data.extend_from_slice(&[0xFF; BLOCK]);
    let err = parse(&data).unwrap_err();
    assert!(
        matches!(err, TarParseError::NotUstarHeader { offset: 0x600 }),
        "got {err:?}"
    );
}

#[test]
fn a_payload_past_the_archive_names_where_it_would_start() {
    let mut data = regular("first.bin", 0).to_vec();
    data.extend_from_slice(&regular("second.bin", 0o7777));
    let err = parse(&data).unwrap_err();
    assert!(
        matches!(
            &err,
            TarParseError::PayloadPastArchive {
                name,
                offset: 0x400,
                size: 0o7777,
                archive_size: 0x400,
            } if name == "second.bin"
        ),
        "got {err:?}"
    );
}
