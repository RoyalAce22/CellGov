//! The firmware floor where the two version spellings meet: major
//! widths, sub-revision digits, and strings that name no order.

use super::*;

fn key(s: &str) -> (u32, u32) {
    version_key(s).unwrap_or_else(|| panic!("{s:?} names no version"))
}

/// Whether the console orders the two spellings this way is not known;
/// the test pins CellGov's own reading.
#[test]
fn a_titles_sub_revision_digits_count_against_a_firmware_key_that_carries_none() {
    assert!(key("4.93") < key("04.9312"));
    assert_eq!(
        firmware_shortfall(GameVersion::Base, "04.9312", "4.93"),
        Some(UnderstatedFirmware {
            entry: GameVersion::Base,
            declared: "04.9312".to_string(),
            selected: "4.93".to_string(),
            incomparable: false,
        })
    );
    assert_eq!(
        firmware_shortfall(GameVersion::Base, "04.9300", "4.93"),
        None
    );
}

#[test]
fn a_two_digit_major_orders_above_every_one_digit_major() {
    assert!(key("10.0100") > key("9.99"));
    assert!(key("10.01") > key("09.9900"));
    assert_eq!(key("10.01"), key("10.0100"));
    assert_eq!(
        firmware_shortfall(GameVersion::Base, "10.0100", "9.99").map(|n| n.incomparable),
        Some(false)
    );
    assert_eq!(
        firmware_shortfall(GameVersion::Base, "09.9900", "10.01"),
        None
    );
}

#[test]
fn a_zero_major_keeps_its_order_in_both_spellings() {
    assert_eq!(key("0.31"), key("00.3100"));
    assert!(key("00.3100") < key("1.00"));
}

#[test]
fn an_empty_declared_minimum_is_reported_as_incomparable_not_dropped() {
    let note = firmware_shortfall(GameVersion::Update("02.51".to_string()), "", "4.93")
        .expect("an empty claim is not a met claim");
    assert!(note.incomparable);
    assert_eq!(note.declared, "");
    assert_eq!(note.entry, GameVersion::Update("02.51".to_string()));
}
