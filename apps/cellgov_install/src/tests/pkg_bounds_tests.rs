//! The entry-table and item-data refusals at the declared data-region end.

use super::*;
use crate::test_support::{build_pkg, pkg_file, synthetic_vault};

const KLIC: [u8; 16] = [0x5A; 16];
// Header offsets of the fields these tests rewrite.
const FILE_COUNT_FIELD: std::ops::Range<usize> = 0x14..0x18;
const PKG_SIZE_FIELD: std::ops::Range<usize> = 0x18..0x20;
const DATA_SIZE_FIELD: std::ops::Range<usize> = 0x28..0x30;
/// `build_pkg` places the data region here.
const DATA_OFFSET: usize = 0x80;

#[test]
fn a_maximal_file_count_is_an_entry_table_past_the_region_not_a_wrap() {
    // 0x0800_0001 records of 0x20 bytes wrap a 32-bit product to one
    // record, which the one-item region would hold.
    for file_count in [u32::MAX, 0x0800_0001] {
        let keys = synthetic_vault();
        let mut pkg = build_pkg(&keys, &KLIC, "TEST00001", &[pkg_file("A", 3, b"a")]);
        pkg[FILE_COUNT_FIELD].copy_from_slice(&file_count.to_be_bytes());
        let err = extract(&pkg, &keys).unwrap_err();
        assert!(
            matches!(
                err,
                PkgError::EntryTableOutOfBounds { file_count: got, .. } if got == file_count
            ),
            "file_count 0x{file_count:x}: got {err:?}"
        );
    }
}

#[test]
fn an_item_past_the_declared_data_size_is_refused_though_the_file_holds_the_bytes() {
    let keys = synthetic_vault();
    let mut pkg = build_pkg(&keys, &KLIC, "TEST00001", &[pkg_file("A", 3, b"a")]);
    let data_size = pkg.len() - DATA_OFFSET;
    // A retail package's pkg_size covers a 0x60-byte trailer past the
    // data region. The trailer is inside the package, so only the
    // declared data_size bounds the item.
    pkg.extend_from_slice(&[0u8; 0x60]);
    let pkg_size = pkg.len() as u64;
    pkg[PKG_SIZE_FIELD].copy_from_slice(&pkg_size.to_be_bytes());

    // The entry table is CTR-encrypted, and CTR is its own inverse.
    let key = *keys.pkg_aes().unwrap();
    let region = &mut pkg[DATA_OFFSET..DATA_OFFSET + data_size];
    ctr_decrypt(&key, &KLIC, region);
    region[0x08..0x10].copy_from_slice(&(data_size as u64).to_be_bytes()); // file_offset
    region[0x10..0x18].copy_from_slice(&0x10u64.to_be_bytes()); // file_size
    ctr_decrypt(&key, &KLIC, region);

    let err = extract(&pkg, &keys).unwrap_err();
    assert!(
        matches!(
            &err,
            PkgError::FileDataOutOfBounds { index: 0, region, .. } if *region == data_size
        ),
        "got {err:?}"
    );
}

#[test]
fn a_declared_data_size_short_of_the_entry_table_is_named() {
    let keys = synthetic_vault();
    let mut pkg = build_pkg(
        &keys,
        &KLIC,
        "TEST00001",
        &[pkg_file("A", 3, b"a"), pkg_file("B", 3, b"b")],
    );
    // One record's worth of region for a two-record table.
    pkg[DATA_SIZE_FIELD].copy_from_slice(&0x20u64.to_be_bytes());
    let err = extract(&pkg, &keys).unwrap_err();
    assert!(
        matches!(
            err,
            PkgError::EntryTableOutOfBounds {
                file_count: 2,
                region: 0x20
            }
        ),
        "got {err:?}"
    );
}
