//! The bound on each section's output: its segment's `p_filesz` for a
//! SELF's PHDR section, and the header's plaintext size for every other
//! section.

use aes::cipher::{KeyIvInit, StreamCipher};

use super::*;

const PHDR: u32 = 2;
const SHDR: u32 = 1;
const PLAIN: u32 = 1;
const ZLIB: u32 = 2;

fn zlib(plain: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(plain).unwrap();
    encoder.finish().unwrap()
}

/// SCE container that declares `plaintext_size` and holds `sections`.
///
/// Each entry is `(section_kind, compression_kind, stored bytes)` for
/// one plain section that targets program-header row 0. The fixture
/// applies the CTR pass of an all-zero envelope to the metadata
/// directory. The decrypt pass applies the same keystream, which
/// restores the directory.
fn container(plaintext_size: u64, sections: &[(u32, u32, &[u8])]) -> Vec<u8> {
    const DIRECTORY_OFFSET: usize = 0x80;
    let directory_len = 0x20 + 0x30 * sections.len();
    let header_size = DIRECTORY_OFFSET + directory_len;

    let mut directory = vec![0u8; directory_len];
    directory[0x0C..0x10].copy_from_slice(&(sections.len() as u32).to_be_bytes());
    let mut payloads = Vec::new();
    for (i, (kind, compression, stored)) in sections.iter().enumerate() {
        let row = 0x20 + 0x30 * i;
        let offset = header_size + payloads.len();
        directory[row..row + 8].copy_from_slice(&(offset as u64).to_be_bytes());
        directory[row + 8..row + 0x10].copy_from_slice(&(stored.len() as u64).to_be_bytes());
        directory[row + 0x10..row + 0x14].copy_from_slice(&kind.to_be_bytes());
        directory[row + 0x20..row + 0x24].copy_from_slice(&PLAIN.to_be_bytes());
        directory[row + 0x2C..row + 0x30].copy_from_slice(&compression.to_be_bytes());
        payloads.extend_from_slice(stored);
    }
    ctr::Ctr128BE::<aes::Aes128>::new(&[0u8; 16].into(), &[0u8; 16].into())
        .apply_keystream(&mut directory);

    let mut data = vec![0u8; header_size];
    data[0..4].copy_from_slice(&0x5343_4500u32.to_be_bytes());
    data[12..16].copy_from_slice(&0x20u32.to_be_bytes()); // metadata_offset
    data[16..24].copy_from_slice(&(header_size as u64).to_be_bytes());
    data[24..32].copy_from_slice(&plaintext_size.to_be_bytes());
    data[DIRECTORY_OFFSET..].copy_from_slice(&directory);
    data.extend_from_slice(&payloads);
    data
}

fn decrypt(
    data: &[u8],
    segment_file_sizes: Option<&[usize]>,
) -> Result<Vec<(EncryptedSectionDescriptor, Vec<u8>)>, SceError> {
    let hdr = parse_sce_header(data).unwrap();
    decrypt_sections_from_envelope(data, &hdr, &[0u8; 0x40], segment_file_sizes)
}

#[test]
fn a_package_section_inflating_to_exactly_the_declared_plaintext_size_is_accepted() {
    let stream = zlib(&[0xAA; 0x400]);
    let sections = decrypt(&container(0x400, &[(SHDR, ZLIB, &stream)]), None).unwrap();
    assert_eq!(sections[0].1, vec![0xAA; 0x400]);
}

#[test]
fn a_package_section_inflating_past_the_declared_plaintext_size_is_named() {
    let stream = zlib(&[0xAA; 0x400]);
    let err = decrypt(&container(0x3FF, &[(SHDR, ZLIB, &stream)]), None).unwrap_err();
    assert!(
        matches!(
            err,
            SceError::SectionsPastPlaintextSize {
                index: 0,
                plaintext_size: 0x3FF
            }
        ),
        "got {err:?}"
    );
}

#[test]
fn the_plaintext_budget_spans_every_section_not_each_one_alone() {
    // A plain 0x40-byte info block leaves 0x3C0 of a 0x400 budget, short
    // of the 0x400 the second section inflates to.
    let stream = zlib(&[0xAA; 0x400]);
    let data = container(0x400, &[(SHDR, PLAIN, &[0u8; 0x40]), (SHDR, ZLIB, &stream)]);
    let err = decrypt(&data, None).unwrap_err();
    assert!(
        matches!(
            err,
            SceError::SectionsPastPlaintextSize {
                index: 1,
                plaintext_size: 0x400
            }
        ),
        "got {err:?}"
    );
}

#[test]
fn a_plain_section_past_the_plaintext_budget_is_named_too() {
    let err = decrypt(&container(0x3F, &[(SHDR, PLAIN, &[0u8; 0x40])]), None).unwrap_err();
    assert!(
        matches!(err, SceError::SectionsPastPlaintextSize { index: 0, .. }),
        "got {err:?}"
    );
}

#[test]
fn a_small_stream_declaring_an_unallocatable_output_is_refused_before_it_inflates() {
    // The size is past `isize::MAX`, so the reserve refuses it on every
    // host without a call to the allocator.
    let stream = zlib(&[0xAA; 0x10]);
    let data = container(0x8000_0000_0000_0000, &[(SHDR, ZLIB, &stream)]);
    let err = decrypt(&data, None).unwrap_err();
    assert!(
        matches!(
            err,
            SceError::SectionOutputTooLarge {
                index: 0,
                size: 0x8000_0000_0000_0000
            }
        ),
        "got {err:?}"
    );
}

#[test]
fn a_selfs_non_phdr_section_draws_on_the_plaintext_budget() {
    let stream = zlib(&[0xAA; 0x400]);
    let data = container(0x100, &[(SHDR, ZLIB, &stream)]);
    let err = decrypt(&data, Some(&[0x1000])).unwrap_err();
    assert!(
        matches!(err, SceError::SectionsPastPlaintextSize { index: 0, .. }),
        "a segment table sizes PHDR sections only, got {err:?}"
    );
}

#[test]
fn a_selfs_phdr_section_is_held_to_its_segment_and_not_to_the_budget() {
    // A SELF's plaintext size counts its ELF image, which can differ
    // from the sum of its sections. Where a segment sizes a section, the
    // budget does not apply.
    let stream = zlib(&[0xAA; 0x400]);
    let data = container(0, &[(PHDR, ZLIB, &stream)]);
    let sections = decrypt(&data, Some(&[0x400])).unwrap();
    assert_eq!(sections[0].1.len(), 0x400);
}

#[test]
fn a_stream_cut_before_its_end_marker_is_refused_rather_than_inflated_short() {
    // The cut removes only the adler32 trailer. Every data byte stays,
    // so the stream inflates fully and still lacks its end.
    let whole = zlib(&[0xAA; 0x400]);
    let cut = &whole[..whole.len() - 4];
    let err = decrypt(&container(0x400, &[(SHDR, ZLIB, cut)]), None).unwrap_err();
    assert!(
        matches!(
            &err,
            SceError::ZlibDecompress { index: 0, source }
                if source.kind() == std::io::ErrorKind::UnexpectedEof
        ),
        "got {err:?}"
    );
}

#[test]
fn a_stream_cut_mid_data_is_refused_too() {
    let whole = zlib(&(0..0x4000u32).map(|n| n as u8 ^ 0x5A).collect::<Vec<_>>());
    let cut = &whole[..whole.len() / 2];
    let err = decrypt(&container(0x4000, &[(SHDR, ZLIB, cut)]), None).unwrap_err();
    assert!(
        matches!(err, SceError::ZlibDecompress { index: 0, .. }),
        "got {err:?}"
    );
}

#[test]
fn a_budgeted_section_keeps_none_of_the_budget_it_did_not_spend() {
    let stream = zlib(&[0xAA; 0x400]);
    let sections = decrypt(&container(0x10_0000, &[(SHDR, ZLIB, &stream)]), None).unwrap();
    assert_eq!(sections[0].1.len(), 0x400);
    assert!(
        sections[0].1.capacity() < 0x10_0000,
        "a 0x400-byte output held {} bytes of capacity",
        sections[0].1.capacity()
    );
}

#[test]
fn a_segment_too_large_to_allocate_is_named_rather_than_aborting() {
    let stream = zlib(&[0xAA; 0x400]);
    let data = container(0, &[(PHDR, ZLIB, &stream)]);
    let err = decrypt(&data, Some(&[usize::MAX])).unwrap_err();
    assert!(
        matches!(
            err,
            SceError::SectionOutputTooLarge {
                index: 0,
                size: usize::MAX
            }
        ),
        "got {err:?}"
    );
}
