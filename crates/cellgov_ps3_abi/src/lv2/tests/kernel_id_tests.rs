//! The kernel-id form splits back into the parts it was composed from.

use super::*;

#[test]
fn compose_places_class_generation_and_slot_in_their_bytes() {
    assert_eq!(compose(0x88, 1, 0x26), 0x8801_2600);
    assert_eq!(compose(0x85, 0, 0x26), 0x8500_2600);
    let id = compose(0x20, 5, 0x26);
    assert_eq!(
        (class_of(id), generation_of(id), slot_of(id)),
        (0x20, 5, 0x26)
    );
}

#[test]
fn compose_wraps_the_generation_and_the_slot_in_their_bytes() {
    assert_eq!(
        compose(0x85, GENERATION_COUNT + 2, SLOT_COUNT + 3),
        0x8502_0300
    );
}
