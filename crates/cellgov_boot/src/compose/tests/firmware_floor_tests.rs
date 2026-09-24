//! The firmware floor where the two version spellings meet: a
//! sub-revision, a two-digit major, and a claim no order can be read
//! from. The ordering itself is `SystemVersion`'s, tested beside it.

use super::*;

/// Whether the console orders the two spellings this way is not known;
/// the test pins CellGov's own reading.
#[test]
fn a_titles_sub_revision_digits_count_against_a_firmware_key_that_carries_none() {
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
fn an_empty_declared_minimum_is_reported_as_incomparable_not_dropped() {
    let note = firmware_shortfall(GameVersion::Update("02.51".to_string()), "", "4.93")
        .expect("an empty claim is not a met claim");
    assert!(note.incomparable);
    assert_eq!(note.declared, "");
    assert_eq!(note.entry, GameVersion::Update("02.51".to_string()));
}
