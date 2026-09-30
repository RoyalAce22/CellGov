//! Lane views round-trip and number their slots from the left.

use super::*;

const REG: [u8; 16] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
];

#[test]
fn halfword_slot_zero_is_the_leftmost_two_bytes() {
    let h = halfwords(REG);
    assert_eq!(h[0], 0x0001);
    assert_eq!(h[7], 0x0E0F);
    assert_eq!(from_halfwords(h), REG);
}

#[test]
fn word_slot_zero_is_the_leftmost_four_bytes() {
    let w = words(REG);
    assert_eq!(w[0], 0x0001_0203);
    assert_eq!(w[3], 0x0C0D_0E0F);
    assert_eq!(from_words(w), REG);
}
