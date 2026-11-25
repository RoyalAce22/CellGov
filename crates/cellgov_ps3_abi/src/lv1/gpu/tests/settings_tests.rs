use super::*;

/// [OFW-4.93 lv1 a:0x3B0D30] lines 12 and 13 of the default text;
/// [OFW-4.93 lv1 a:0x20E948-0x20E94C] `li r0, 0; std r0, 112(r1)`.
#[test]
fn the_default_text_values_are_its_lines() {
    assert_eq!(DEFAULT_RSX_ENABLE, 3);
    assert_eq!(DEFAULT_RSX_DEBUG, 0);
    assert_eq!(DEFAULT_BOOT_MODE, 0);
}
