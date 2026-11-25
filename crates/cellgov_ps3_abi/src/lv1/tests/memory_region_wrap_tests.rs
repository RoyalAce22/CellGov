use super::*;

/// A 1 MB region has e = 20, so its address holds 2^22 counter values.
/// lv1 ANDs the counter value with 2^22 - 1 [OFW-4.93 lv1 a:0x2CED98].
#[test]
fn a_counter_past_the_address_field_wraps_to_the_low_address() {
    assert_eq!(region_counter_values(0x10_0000), Some(1 << 22));
    assert_eq!(
        region_address(0x10_0000, (1 << 22) - 1),
        Some((20 << 42) | (((1 << 22) - 1) << 20))
    );
    assert_eq!(region_address(0x10_0000, 1 << 22), Some(20 << 42));
    assert_eq!(
        region_address(0x10_0000, (1 << 22) + 3),
        region_address(0x10_0000, 3)
    );
}

/// [OFW-4.93 lv1 a:0x2CED34-0x2CED3C] [OFW-4.93 lv1 a:0x2CEE2C-0x2CEE38]
#[test]
fn a_size_of_0xfff_bytes_or_less_takes_the_exponent_12() {
    for size in [0, 1, 0xFFF, 0x1000] {
        assert_eq!(region_function_exp(size), Some(12), "{size:#x}");
        assert_eq!(
            region_address(size, 2),
            Some((12 << 42) | (2 << 12)),
            "{size:#x}"
        );
    }
    assert_eq!(region_function_exp(0x1001), Some(13));
}

/// lv1 refuses a size above 2^42 [OFW-4.93 lv1 a:0x2CECC8-0x2CECD4]. A
/// 2^42-byte region has e = 42, and its mask is 0.
#[test]
fn a_region_of_2_pow_42_bytes_has_one_address_and_a_larger_one_none() {
    assert_eq!(region_counter_values(REGION_SIZE_MAX), Some(1));
    assert_eq!(region_address(REGION_SIZE_MAX, 0), Some(42 << 42));
    assert_eq!(region_address(REGION_SIZE_MAX, 5), Some(42 << 42));
    assert_eq!(region_function_exp(REGION_SIZE_MAX + 1), None);
    assert_eq!(region_counter_values(REGION_SIZE_MAX + 1), None);
    assert_eq!(region_address(REGION_SIZE_MAX + 1, 0), None);
}
