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
