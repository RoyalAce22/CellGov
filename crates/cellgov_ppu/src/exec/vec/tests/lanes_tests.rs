//! Lane order and the saturation flag of the element-wise maps.

use super::lanes::{map_u16, map_u16_sat, map_u32, map_u32_sat, map_u8, map_u8_sat};

const RAMP: u128 = 0x0001_0203_0405_0607_0809_0A0B_0C0D_0E0F;

#[test]
fn byte_element_zero_is_the_most_significant_byte() {
    // Each output byte is its element index, read from `a`, minus 0.
    assert_eq!(map_u8(RAMP, 0, |x, _| x), RAMP);
    // The second operand's element i pairs with the first's element i.
    assert_eq!(map_u8(RAMP, RAMP, |x, y| x.wrapping_add(y)), RAMP << 1);
}

#[test]
fn halfword_and_word_elements_pair_by_position() {
    // The second operand's elements land in the same positions.
    assert_eq!(map_u16(0, RAMP, |_, y| y), RAMP);
    assert_eq!(map_u32(0, RAMP, |_, y| y), RAMP);
    assert_eq!(map_u16_sat(0, RAMP, |_, y| (y, false)).0, RAMP);
    assert_eq!(map_u32_sat(0, RAMP, |_, y| (y, false)).0, RAMP);
    assert_eq!(
        map_u16(RAMP, 0, |x, _| x.swap_bytes()),
        0x0100_0302_0504_0706_0908_0B0A_0D0C_0F0E
    );
    assert_eq!(
        map_u32(RAMP, 0, |x, _| x.rotate_left(8)),
        0x0102_0300_0506_0704_090A_0B08_0D0E_0F0C
    );
}

#[test]
fn the_saturation_flag_is_the_or_of_the_elements() {
    let only_last = |x: u8, _: u8| (x, x == 0x0F);
    assert_eq!(map_u8_sat(RAMP, 0, only_last), (RAMP, true));
    let none = |x: u8, _: u8| (x, false);
    assert_eq!(map_u8_sat(RAMP, 0, none), (RAMP, false));
    assert!(map_u16_sat(RAMP, 0, |x, _| (x, x == 0x0001)).1);
    assert!(!map_u32_sat(RAMP, 0, |x, _| (x, x == 0xFFFF_FFFF)).1);
}
