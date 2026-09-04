//! Hex scalars, patch-byte pairs, and fault-dump ranges.

use super::*;

#[test]
fn hex_takes_either_prefix_spelling_or_none() {
    for spelling in ["0x2a", "0X2A", "2a", " 2a "] {
        assert_eq!(parse_hex_u64_value(spelling, "addr").unwrap(), 0x2a);
    }
}

#[test]
fn hex_names_its_context_when_it_refuses() {
    let err = parse_hex_u64_value("", "--vaddr").unwrap_err();
    assert!(matches!(err, CliArgError::EmptyHexValue { .. }));
    assert!(err.to_string().contains("--vaddr"), "{err}");

    let err = parse_hex_u64_value("0x", "--vaddr").unwrap_err();
    assert!(matches!(err, CliArgError::HexPrefixNoDigits { .. }));

    let err = parse_hex_u64_value("0xzz", "--vaddr").unwrap_err();
    assert!(matches!(err, CliArgError::CannotParseHexU64 { .. }));
}

#[test]
fn the_prefix_stripper_leaves_a_bare_value_alone() {
    assert_eq!(strip_hex_prefix("0x10"), "10");
    assert_eq!(strip_hex_prefix("0X10"), "10");
    assert_eq!(strip_hex_prefix("10"), "10");
    assert_eq!(strip_hex_prefix(""), "");
}

#[test]
fn a_patch_byte_pair_takes_two_hex_fields() {
    assert_eq!(
        parse_patch_byte_pair_value("0x1000=ff").unwrap(),
        (0x1000, 0xff)
    );
    assert_eq!(parse_patch_byte_pair_value("1000=0").unwrap(), (0x1000, 0));
}

#[test]
fn a_patch_byte_pair_names_which_half_is_wrong() {
    for (pair, expected) in [
        ("", "empty argument"),
        ("0x1000", "missing '='"),
        ("=ff", "empty address"),
        ("0x1000=", "empty value"),
        ("0x1000=f=f", "extra '='"),
    ] {
        let err = parse_patch_byte_pair_value(pair).unwrap_err();
        assert!(
            err.to_string().contains(expected),
            "{pair:?} should name {expected:?}, got {err}"
        );
    }
}

#[test]
fn a_patch_byte_value_is_at_most_two_hex_digits() {
    let err = parse_patch_byte_pair_value("0x1000=fff").unwrap_err();
    assert!(
        matches!(err, CliArgError::HexU8TooLong { digits: 3, .. }),
        "{err}"
    );
}

#[test]
fn a_fault_dump_range_without_a_length_takes_the_default() {
    assert_eq!(
        parse_dump_mem_fault_spec("0x10000").unwrap(),
        (0x10000, 0x40)
    );
}

#[test]
fn a_fault_dump_range_reads_both_fields_as_hex() {
    assert_eq!(
        parse_dump_mem_fault_spec("0x10000:20").unwrap(),
        (0x10000, 0x20)
    );
}

#[test]
fn a_fault_dump_range_is_bounded_at_both_ends() {
    for (spec, expected) in [
        ("0x1000:0", "zero-byte length"),
        ("0x1000:20000", "exceeds maximum"),
        ("0x1000:8:8", "extra ':'"),
        ("0xffffffffffffffff:8", "overflows u64"),
    ] {
        let err = parse_dump_mem_fault_spec(spec).unwrap_err();
        assert!(
            err.to_string().contains(expected),
            "{spec:?} should name {expected:?}, got {err}"
        );
    }
}

#[test]
fn a_fault_dump_range_ending_on_the_final_address_is_accepted() {
    let last = u64::MAX - 7;
    assert_eq!(
        parse_dump_mem_fault_spec(&format!("{last:x}:8")).unwrap(),
        (last, 8)
    );
}
