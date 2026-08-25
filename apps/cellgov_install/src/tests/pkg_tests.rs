//! Retail PKG header validation, CTR extraction, and rejection of
//! debug / non-PS3 / out-of-bounds packages.

use super::*;

const TEST_KLIC: [u8; 16] = [
    0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x00,
];

fn align16(n: usize) -> usize {
    n.div_ceil(16) * 16
}

/// Wrap an already-laid-out plaintext data region into a full retail
/// PKG: build the header, CTR-encrypt the region, and append it at
/// `data_offset` (0x80). `pkg_size`/`data_size` are derived to match.
fn wrap_region(klic: &[u8; 16], title_id: &str, file_count: u32, plaintext: &[u8]) -> Vec<u8> {
    let data_offset: u64 = 0x80;
    let mut region = plaintext.to_vec();
    super::ctr_decrypt(klic, &mut region); // symmetric: encrypts here
    let data_size = region.len() as u64;
    let pkg_size = data_offset + data_size;

    let mut buf = vec![0u8; data_offset as usize];
    buf[0..4].copy_from_slice(&[0x7F, b'P', b'K', b'G']);
    buf[0x04..0x06].copy_from_slice(&PKG_TYPE_RELEASE.to_be_bytes());
    buf[0x06..0x08].copy_from_slice(&PKG_PLATFORM_PS3.to_be_bytes());
    buf[0x14..0x18].copy_from_slice(&file_count.to_be_bytes());
    buf[0x18..0x20].copy_from_slice(&pkg_size.to_be_bytes());
    buf[0x20..0x28].copy_from_slice(&data_offset.to_be_bytes());
    buf[0x28..0x30].copy_from_slice(&data_size.to_be_bytes());
    let tid = title_id.as_bytes();
    buf[0x30..0x30 + tid.len()].copy_from_slice(tid);
    buf[0x70..0x80].copy_from_slice(klic);

    buf.extend_from_slice(&region);
    buf
}

/// A spec for one item in a synthetic package.
struct ItemSpec {
    name: &'static str,
    raw_type: u32,
    data: Vec<u8>,
}

fn file_item(name: &'static str, raw_type: u32, data: &[u8]) -> ItemSpec {
    ItemSpec {
        name,
        raw_type,
        data: data.to_vec(),
    }
}

fn dir_item(name: &'static str) -> ItemSpec {
    ItemSpec {
        name,
        raw_type: 4,
        data: Vec::new(),
    }
}

/// Lay out a plaintext data region for `items` (entry table, then
/// 16-aligned name and data blobs) and wrap it into a full PKG.
fn build_pkg(klic: &[u8; 16], title_id: &str, items: &[ItemSpec]) -> Vec<u8> {
    let n = items.len();
    let table_len = n * 0x20;
    // First pass: place names and data, recording offsets.
    let mut blob = Vec::new();
    let mut placed: Vec<(u32, u32, u64, u64)> = Vec::new(); // name_off, name_size, file_off, file_size
    for it in items {
        let name_off = (table_len + blob.len()) as u32;
        let name_bytes = it.name.as_bytes();
        blob.extend_from_slice(name_bytes);
        blob.resize(align16(blob.len()), 0);
        let (file_off, file_size) = if it.data.is_empty() && (it.raw_type & 0xFF) == 4 {
            (0u64, 0u64)
        } else {
            let off = (table_len + blob.len()) as u64;
            blob.extend_from_slice(&it.data);
            blob.resize(align16(blob.len()), 0);
            (off, it.data.len() as u64)
        };
        placed.push((name_off, name_bytes.len() as u32, file_off, file_size));
    }

    let mut region = vec![0u8; table_len];
    for (i, it) in items.iter().enumerate() {
        let (name_off, name_size, file_off, file_size) = placed[i];
        let rec = i * 0x20;
        region[rec..rec + 4].copy_from_slice(&name_off.to_be_bytes());
        region[rec + 4..rec + 8].copy_from_slice(&name_size.to_be_bytes());
        region[rec + 8..rec + 16].copy_from_slice(&file_off.to_be_bytes());
        region[rec + 16..rec + 24].copy_from_slice(&file_size.to_be_bytes());
        region[rec + 24..rec + 28].copy_from_slice(&it.raw_type.to_be_bytes());
    }
    region.extend_from_slice(&blob);

    wrap_region(klic, title_id, n as u32, &region)
}

fn find<'a>(archive: &'a PkgArchive, name: &str) -> Option<&'a PkgFile> {
    archive.files.iter().find(|f| f.name == name)
}

#[test]
fn extract_round_trips_files_and_dirs() {
    let eboot = b"\x53\x43\x45\x00 fake encrypted eboot bytes ........";
    let sfo = b"\x00PSF fake param sfo blob";
    let pkg = build_pkg(
        &TEST_KLIC,
        "NPUA80001",
        &[
            file_item("PARAM.SFO", 3, sfo),
            dir_item("USRDIR"),
            file_item("USRDIR/EBOOT.BIN", 1, eboot),
        ],
    );

    let archive = extract(&pkg).expect("well-formed PKG extracts");
    assert_eq!(archive.header.content_id, "NPUA80001");
    assert_eq!(archive.header.klicensee, TEST_KLIC);

    let sfo_file = find(&archive, "PARAM.SFO").expect("PARAM.SFO present");
    assert_eq!(sfo_file.kind, PkgEntryKind::File);
    assert_eq!(archive.file_data(sfo_file), sfo);

    let dir = find(&archive, "USRDIR").expect("USRDIR present");
    assert_eq!(dir.kind, PkgEntryKind::Directory);
    assert!(archive.file_data(dir).is_empty());

    let eboot_file = find(&archive, "USRDIR/EBOOT.BIN").expect("EBOOT present");
    assert_eq!(eboot_file.kind, PkgEntryKind::File);
    assert_eq!(
        archive.file_data(eboot_file),
        eboot,
        "EBOOT bytes survive CTR round-trip"
    );
}

#[test]
fn skips_edat_and_sdat_items() {
    let pkg = build_pkg(
        &TEST_KLIC,
        "NPUA80001",
        &[
            file_item("PARAM.SFO", 3, b"sfo"),
            file_item("USRDIR/DATA.EDAT", 2, b"edat-bytes"),
            file_item("USRDIR/SAVE.SDAT", 9, b"sdat-bytes"),
        ],
    );
    let archive = extract(&pkg).expect("extract");
    assert!(find(&archive, "PARAM.SFO").is_some());
    assert!(find(&archive, "USRDIR/DATA.EDAT").is_none(), "EDAT skipped");
    assert!(find(&archive, "USRDIR/SAVE.SDAT").is_none(), "SDAT skipped");
}

#[test]
fn rejects_bad_magic() {
    let mut pkg = build_pkg(&TEST_KLIC, "NPUA80001", &[file_item("PARAM.SFO", 3, b"x")]);
    pkg[1] ^= 0xFF;
    assert!(matches!(extract(&pkg).unwrap_err(), PkgError::BadMagic(_)));
}

#[test]
fn rejects_debug_package() {
    let mut pkg = build_pkg(&TEST_KLIC, "NPUA80001", &[file_item("PARAM.SFO", 3, b"x")]);
    pkg[0x04..0x06].copy_from_slice(&PKG_TYPE_DEBUG.to_be_bytes());
    assert!(matches!(extract(&pkg).unwrap_err(), PkgError::DebugPackage));
}

#[test]
fn rejects_non_ps3_platform() {
    let mut pkg = build_pkg(&TEST_KLIC, "NPUA80001", &[file_item("PARAM.SFO", 3, b"x")]);
    pkg[0x06..0x08].copy_from_slice(&2u16.to_be_bytes());
    assert!(matches!(
        extract(&pkg).unwrap_err(),
        PkgError::NonPs3Platform(2)
    ));
}

#[test]
fn rejects_pkg_size_over_file_length() {
    let mut pkg = build_pkg(&TEST_KLIC, "NPUA80001", &[file_item("PARAM.SFO", 3, b"x")]);
    let bogus = (pkg.len() as u64) + 0x1000;
    pkg[0x18..0x20].copy_from_slice(&bogus.to_be_bytes());
    assert!(matches!(
        extract(&pkg).unwrap_err(),
        PkgError::SizeMismatch { .. }
    ));
}

#[test]
fn rejects_data_region_escaping_pkg_size() {
    let mut pkg = build_pkg(&TEST_KLIC, "NPUA80001", &[file_item("PARAM.SFO", 3, b"x")]);
    // data_size far larger than pkg_size permits.
    pkg[0x28..0x30].copy_from_slice(&0xFFFF_FFFFu64.to_be_bytes());
    assert!(matches!(
        extract(&pkg).unwrap_err(),
        PkgError::DataRegionOutOfBounds { .. }
    ));
}

#[test]
fn rejects_name_too_long() {
    // Hand-build a one-entry region with name_size > MAX_NAME_LEN.
    let mut region = vec![0u8; 0x20];
    let name_off = 0x20u32;
    let name_size = MAX_NAME_LEN + 1;
    region[0..4].copy_from_slice(&name_off.to_be_bytes());
    region[4..8].copy_from_slice(&name_size.to_be_bytes());
    region[24..28].copy_from_slice(&3u32.to_be_bytes());
    region.resize(0x20 + name_size as usize, b'A');
    let pkg = wrap_region(&TEST_KLIC, "NPUA80001", 1, &region);
    assert!(matches!(
        extract(&pkg).unwrap_err(),
        PkgError::NameTooLong { .. }
    ));
}

#[test]
fn rejects_name_escaping_region() {
    let mut region = vec![0u8; 0x20];
    region[0..4].copy_from_slice(&0xFFFFu32.to_be_bytes()); // name_offset
    region[4..8].copy_from_slice(&4u32.to_be_bytes()); // name_size
    region[24..28].copy_from_slice(&3u32.to_be_bytes());
    let pkg = wrap_region(&TEST_KLIC, "NPUA80001", 1, &region);
    assert!(matches!(
        extract(&pkg).unwrap_err(),
        PkgError::NameOutOfBounds { .. }
    ));
}

#[test]
fn rejects_file_data_escaping_region() {
    // One entry: a valid name, but file_offset/size run off the region.
    let name = b"A.BIN";
    let mut region = vec![0u8; 0x20];
    let name_off = 0x20u32;
    region[0..4].copy_from_slice(&name_off.to_be_bytes());
    region[4..8].copy_from_slice(&(name.len() as u32).to_be_bytes());
    region[8..16].copy_from_slice(&0xFFFFu64.to_be_bytes()); // file_offset
    region[16..24].copy_from_slice(&0x100u64.to_be_bytes()); // file_size
    region[24..28].copy_from_slice(&3u32.to_be_bytes());
    region.extend_from_slice(name);
    region.resize(align16(region.len()), 0);
    let pkg = wrap_region(&TEST_KLIC, "NPUA80001", 1, &region);
    assert!(matches!(
        extract(&pkg).unwrap_err(),
        PkgError::FileDataOutOfBounds { .. }
    ));
}

#[test]
fn rejects_unsafe_traversal_name() {
    let pkg = build_pkg(
        &TEST_KLIC,
        "NPUA80001",
        &[file_item("../escape.bin", 3, b"x")],
    );
    assert!(matches!(
        extract(&pkg).unwrap_err(),
        PkgError::UnsafePath { .. }
    ));
}

#[test]
fn rejects_too_small() {
    assert!(matches!(
        parse_header(&[0u8; 0x10]).unwrap_err(),
        PkgError::TooSmall { .. }
    ));
}

/// Hand-build a one-file region: entry table, 16-aligned name, then
/// `data` placed at `file_offset` with the given `file_size` and NO
/// trailing alignment pad, so the region ends where the data does.
fn one_file_region(name: &[u8], data: &[u8], file_size: u64) -> Vec<u8> {
    let mut region = vec![0u8; 0x20];
    let name_off = 0x20u32;
    region[0..4].copy_from_slice(&name_off.to_be_bytes());
    region[4..8].copy_from_slice(&(name.len() as u32).to_be_bytes());
    region[24..28].copy_from_slice(&3u32.to_be_bytes());
    region.extend_from_slice(name);
    region.resize(align16(region.len()), 0);
    let file_off = region.len() as u64;
    region[8..16].copy_from_slice(&file_off.to_be_bytes());
    region[16..24].copy_from_slice(&file_size.to_be_bytes());
    region.extend_from_slice(data);
    region
}

// RPCS3 `Crypto/unpkg.cpp` `package_reader::read_entries` rejects an
// item only when `fsz - file_size < file_offset`, so an item whose
// last byte is the last byte of the region is accepted; the range
// must resolve it without an off-by-one at the region end.
#[test]
fn file_ending_exactly_at_region_end_resolves() {
    let data = b"tailend";
    let region = one_file_region(b"T.BIN", data, data.len() as u64);
    assert_eq!(region.len() % 16, 7, "region end is left unaligned");
    let pkg = wrap_region(&TEST_KLIC, "NPUA80001", 1, &region);

    let archive = extract(&pkg).expect("file ending at region end extracts");
    let file = find(&archive, "T.BIN").expect("T.BIN present");
    assert_eq!(archive.file_data(file), data);
}

// RPCS3 `read_entries` skips the data bound when `file_size == 0`; the
// range for such an item is empty and must resolve even when its
// offset is the region length itself.
#[test]
fn zero_length_file_at_region_end_resolves_empty() {
    let region = one_file_region(b"Z.BIN", b"", 0);
    let pkg = wrap_region(&TEST_KLIC, "NPUA80001", 1, &region);

    let archive = extract(&pkg).expect("zero-length file extracts");
    let file = find(&archive, "Z.BIN").expect("Z.BIN present");
    assert_eq!(file.kind, PkgEntryKind::File);
    assert!(archive.file_data(file).is_empty());
}

#[test]
#[should_panic(expected = "invariant: extract bounds-proved every item range")]
fn file_data_from_a_foreign_archive_that_escapes_the_region_panics() {
    let big = build_pkg(
        &TEST_KLIC,
        "NPUA80001",
        &[file_item("BIG.BIN", 3, &[0xAAu8; 64])],
    );
    let small = build_pkg(&TEST_KLIC, "NPUA80002", &[file_item("S", 3, b"s")]);
    let big_archive = extract(&big).expect("big extracts");
    let small_archive = extract(&small).expect("small extracts");
    let foreign = find(&big_archive, "BIG.BIN").expect("BIG.BIN present");

    let _ = small_archive.file_data(foreign);
}
