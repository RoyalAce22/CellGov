use super::*;

#[test]
fn param_sfo_floors_translate_to_their_store_keys() {
    for (sfo, key) in [
        ("01.5000", "1.50"),
        ("01.9400", "1.94"),
        ("02.7600", "2.76"),
        ("03.7000", "3.70"),
        ("04.9300", "4.93"),
    ] {
        assert_eq!(firmware_version_key(sfo).as_deref(), Ok(key), "{sfo}");
    }
}

#[test]
fn a_two_digit_major_keeps_both_digits() {
    assert_eq!(firmware_version_key("10.0100").as_deref(), Ok("10.01"));
}

#[test]
fn the_minor_part_is_truncated_not_rounded() {
    assert_eq!(firmware_version_key("04.9399").as_deref(), Ok("4.93"));
}

fn version(s: &str) -> SystemVersion {
    SystemVersion::parse(s).unwrap_or_else(|| panic!("{s:?} names no version"))
}

/// The firmware floor and the store key read one ordering: a
/// sub-revision orders above the version it revises, and still names
/// that version's store entry.
#[test]
fn one_ordering_reads_both_spellings_and_keeps_the_sub_revision() {
    assert_eq!(version("4.93"), version("04.9300"));
    assert!(version("4.93") < version("04.9312"));
    assert!(version("04.9300") < version("04.9312"));
    assert!(version("4.9") < version("4.93"));
    assert_eq!(firmware_version_key("04.9312").as_deref(), Ok("4.93"));
    assert_eq!(version("04.9312").store_key(), "4.93");
    assert_eq!(version("4.93").store_key(), "4.93");
}

#[test]
fn a_two_digit_major_orders_above_every_one_digit_major() {
    assert!(version("10.0100") > version("9.99"));
    assert!(version("10.01") > version("09.9900"));
    assert_eq!(version("10.01"), version("10.0100"));
}

#[test]
fn a_zero_major_keeps_its_order_in_both_spellings() {
    assert_eq!(version("0.31"), version("00.3100"));
    assert!(version("00.3100") < version("1.00"));
    assert_eq!(firmware_version_key("00.3100").as_deref(), Ok("0.31"));
}

#[test]
fn a_string_no_order_can_be_read_from_is_no_version() {
    for bad in [
        "",
        "4",
        "4.",
        ".93",
        "4.93000",
        "4.930000",
        "4.9a",
        "x.93",
        "4,93",
        "latest",
        "493",
        "99999999999.9300",
    ] {
        assert_eq!(SystemVersion::parse(bad), None, "{bad:?}");
    }
}

#[test]
fn a_value_outside_the_mm_mmmm_shape_is_refused_naming_it() {
    for bad in [
        "",
        "1.50",
        "01.50",
        "01.500",
        "01.50000",
        "1.5000",
        "001.5000",
        "01-5000",
        "01.5000\n",
        "0a.5000",
        "01.5o00",
        ".5000",
        "01.",
    ] {
        let err = firmware_version_key(bad).expect_err(bad);
        assert_eq!(err.value, bad);
        assert!(err.to_string().contains(&format!("{bad:?}")), "{err}");
    }
}
