//! The CoreOS file table: what a well-formed image yields, and each
//! malformed shape by name.

use super::*;
use crate::test_support::build_core_os_image;

#[test]
fn a_well_formed_table_lists_every_entry_in_table_order_with_its_bytes() {
    let image = build_core_os_image(&[
        ("creserved_0", &[0xff; 8]),
        ("lv2_kernel.self", b"SCE\0kernel"),
        ("lv0", b"boot"),
    ]);
    let table = parse_table(&image).expect("a well-formed table");
    let names: Vec<&str> = table.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["creserved_0", "lv2_kernel.self", "lv0"]);
    let kernel = table.find("lv2_kernel.self").expect("the kernel entry");
    assert_eq!(kernel.size, 10);
    assert_eq!(kernel.payload(&image), Some(&b"SCE\0kernel"[..]));
    assert_eq!(
        table.find("lv0").and_then(|e| e.payload(&image)),
        Some(&b"boot"[..])
    );
    assert!(table.find("lv1.self").is_none());
}

#[test]
fn an_empty_table_is_a_table_of_no_entries() {
    let image = build_core_os_image(&[]);
    assert!(parse_table(&image)
        .expect("an empty table")
        .entries
        .is_empty());
}

#[test]
fn a_header_with_an_unknown_format_word_is_refused_rather_than_read_as_empty() {
    // A zero-filled buffer is the shape a wrong or blank section
    // yields: count 0, declared length 0, and no format word.
    let err = parse_table(&[0u8; 0x40]).unwrap_err();
    assert!(
        matches!(err, CoreOsTableError::UnknownFormat { format: 0 }),
        "{err}"
    );
    let mut image = build_core_os_image(&[("lv2_kernel.self", b"k")]);
    image[0..4].copy_from_slice(&2u32.to_be_bytes());
    let err = parse_table(&image).unwrap_err();
    assert!(
        matches!(err, CoreOsTableError::UnknownFormat { format: 2 }),
        "{err}"
    );
}

#[test]
fn an_image_shorter_than_the_header_is_refused_by_length() {
    let err = parse_table(&[0u8; 0x0f]).unwrap_err();
    assert!(
        matches!(err, CoreOsTableError::TooSmall { len: 0x0f }),
        "{err}"
    );
}

#[test]
fn a_declared_length_past_the_buffer_is_a_truncated_image() {
    let mut image = build_core_os_image(&[("lv2_kernel.self", b"k")]);
    image.truncate(image.len() - 1);
    let err = parse_table(&image).unwrap_err();
    assert!(
        matches!(err, CoreOsTableError::DeclaredLengthPastImage { .. }),
        "{err}"
    );
}

#[test]
fn a_count_the_image_cannot_hold_is_refused_before_any_entry_is_read() {
    let mut image = build_core_os_image(&[("lv2_kernel.self", b"k")]);
    image[4..8].copy_from_slice(&0x4000_0000u32.to_be_bytes());
    let err = parse_table(&image).unwrap_err();
    assert!(
        matches!(
            err,
            CoreOsTableError::TablePastImage {
                count: 0x4000_0000,
                ..
            }
        ),
        "{err}"
    );
}

#[test]
fn an_entry_whose_extent_leaves_the_image_is_named_by_index_and_name() {
    let mut image = build_core_os_image(&[("lv0", b"boot"), ("lv2_kernel.self", b"k")]);
    // Entry 1's size field.
    let size_field = 0x10 + 0x30 + 0x08;
    image[size_field..size_field + 8].copy_from_slice(&0x1000u64.to_be_bytes());
    let err = parse_table(&image).unwrap_err();
    let CoreOsTableError::EntryPastImage {
        index, name, size, ..
    } = &err
    else {
        panic!("expected EntryPastImage, got {err}");
    };
    assert_eq!(
        (*index, name.as_str(), *size),
        (1, "lv2_kernel.self", 0x1000)
    );
}

#[test]
fn an_offset_near_the_top_of_the_range_cannot_wrap_past_the_bound() {
    let mut image = build_core_os_image(&[("lv0", b"boot")]);
    let offset_field = 0x10;
    image[offset_field..offset_field + 8].copy_from_slice(&u64::MAX.to_be_bytes());
    let err = parse_table(&image).unwrap_err();
    assert!(
        matches!(err, CoreOsTableError::EntryPastImage { .. }),
        "{err}"
    );
}

#[test]
fn a_name_that_is_not_utf8_is_refused_by_index() {
    let mut image = build_core_os_image(&[("lv0", b"boot")]);
    let name_field = 0x10 + 0x10;
    image[name_field] = 0xff;
    let err = parse_table(&image).unwrap_err();
    assert!(
        matches!(err, CoreOsTableError::NameNotUtf8 { index: 0 }),
        "{err}"
    );
}

#[test]
fn a_name_filling_its_whole_field_is_read_without_a_terminator() {
    let long = "a".repeat(0x20);
    let image = build_core_os_image(&[(&long, b"x")]);
    let table = parse_table(&image).expect("a full-width name");
    assert_eq!(table.entries[0].name, long);
}
