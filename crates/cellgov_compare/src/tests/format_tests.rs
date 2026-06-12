//! Comma grouping in format_with_commas across magnitudes up to u64::MAX.

use super::*;

#[test]
fn under_1000_has_no_comma() {
    assert_eq!(format_with_commas(0), "0");
    assert_eq!(format_with_commas(1), "1");
    assert_eq!(format_with_commas(42), "42");
    assert_eq!(format_with_commas(999), "999");
}

#[test]
fn thousand_boundary_inserts_comma() {
    assert_eq!(format_with_commas(1_000), "1,000");
    assert_eq!(format_with_commas(1_234), "1,234");
    assert_eq!(format_with_commas(12_345), "12,345");
    assert_eq!(format_with_commas(123_456), "123,456");
}

#[test]
fn million_and_billion_get_grouped() {
    assert_eq!(format_with_commas(1_000_000), "1,000,000");
    assert_eq!(format_with_commas(14_352_588), "14,352,588");
    assert_eq!(format_with_commas(3_674_262_528), "3,674,262,528");
}

#[test]
fn pins_billion_count() {
    assert_eq!(format_with_commas(14_352_588), "14,352,588");
    assert_eq!(format_with_commas(14_352_588u64 * 256), "3,674,262,528");
}

#[test]
fn trillion_and_higher_get_grouped() {
    assert_eq!(format_with_commas(1_000_000_000_000), "1,000,000,000,000");
    assert_eq!(
        format_with_commas(999_999_999_999_999_999),
        "999,999,999,999,999,999",
    );
    assert_eq!(
        format_with_commas(10_000_000_000_000_000_000),
        "10,000,000,000,000,000,000",
    );
}

#[test]
fn u64_max_renders_with_full_grouping() {
    assert_eq!(format_with_commas(u64::MAX), "18,446,744,073,709,551,615");
}
