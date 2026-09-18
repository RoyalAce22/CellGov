use super::*;

const V3_55: u64 = 0x0003_0055_0000_0000;
const V3_60: u64 = 0x0003_0060_0000_0000;
const V3_61: u64 = 0x0003_0061_0000_0000;
const V4_93: u64 = 0x0004_0093_0000_0000;

#[test]
fn a_dotted_version_is_the_major_and_two_bcd_digits() {
    assert_eq!(Lv2Versions::parse("3.55"), Some(Lv2Versions::single(V3_55)));
    assert_eq!(
        Lv2Versions::parse(" 4.93 "),
        Some(Lv2Versions::single(V4_93))
    );
    assert_eq!(
        Lv2Versions::parse("0003005500000000"),
        Some(Lv2Versions::single(V3_55))
    );
}

#[test]
fn a_range_reads_both_ends_in_either_spelling() {
    let range = Lv2Versions {
        lo: V3_60,
        hi: V3_61,
    };
    assert_eq!(Lv2Versions::parse("3.60-3.61"), Some(range));
    assert_eq!(Lv2Versions::parse("3.60~3.61"), Some(range));
    assert_eq!(Lv2Versions::parse("3-60-3-61"), Some(range));
    assert!(range.contains(V3_60));
    assert!(range.contains(V3_61));
    assert!(!range.contains(V3_55));
    assert!(!range.contains(V4_93));
}

#[test]
fn a_label_that_is_not_a_version_is_refused() {
    for bad in [
        "",
        "0A",
        "3",
        "3.5",
        "3.555",
        "3.60-",
        "3.61-3.60",
        "x.55",
        "3.5a",
        "rev0A",
        "00030055000000000",
    ] {
        assert_eq!(Lv2Versions::parse(bad), None, "{bad:?}");
    }
}

#[test]
fn a_range_renders_as_its_ends_and_a_single_version_as_itself() {
    assert_eq!(Lv2Versions::single(V3_55).to_string(), "3.55");
    assert_eq!(
        Lv2Versions {
            lo: V3_60,
            hi: V4_93
        }
        .to_string(),
        "3.60-4.93"
    );
}

#[test]
fn a_version_word_outside_the_dotted_shape_renders_as_hex() {
    assert_eq!(version_label(V3_55), "3.55");
    assert_eq!(version_label(0x0003_5500_0000_0000), "0003550000000000");
    assert_eq!(version_label(0x0003_0055_0000_0001), "0003005500000001");
    assert_eq!(version_label(0x0003_00AB_0000_0000), "000300ab00000000");
    // A three-digit major is no dotted label `parse` reads back.
    assert_eq!(version_label(0x0064_0055_0000_0000), "0064005500000000");
}

#[test]
fn every_label_round_trips_through_parse() {
    for range in [
        Lv2Versions::single(V3_55),
        Lv2Versions {
            lo: V3_60,
            hi: V3_61,
        },
        Lv2Versions::single(0x0003_5500_0000_0000),
        Lv2Versions::single(0x0064_0055_0000_0000),
    ] {
        assert_eq!(Lv2Versions::parse(&range.to_string()), Some(range));
    }
}
