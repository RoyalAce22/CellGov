use super::*;

/// [OFW-4.93 lv1 a:0x244E14-0x244E64] with the cases at
/// [OFW-4.93 lv1 a:0x244E88-0x244F30].
#[test]
fn each_selector_with_a_table_case_gets_its_id() {
    let cases = [
        (1, 292),
        (2, 294),
        (3, 295),
        (4, 291),
        (16, 288),
        (17, 289),
        (18, 290),
        (19, 261),
        (20, 256),
    ];
    assert_eq!(cases.len(), 9);
    for (selector, id) in cases {
        assert_eq!(
            net_control_vlan_selector(selector),
            VlanSelector::Fixed(id),
            "selector {selector}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x244EC8-0x244EE0] reads the byte at +28.
#[test]
fn selector_5_depends_on_the_flag_byte() {
    assert_eq!(net_control_vlan_selector(5), VlanSelector::Flagged);
}

/// The table entries for selectors 0 and 6 to 15 are 0xFFFFFD10, the
/// offset of the failure at 0x244B24
/// [OFW-4.93 lv1 a:0x244E14] [OFW-4.93 lv1 a:0x244E2C-0x244E50].
/// The range check [OFW-4.93 lv1 a:0x244DF0-0x244DF4] is unsigned.
#[test]
fn a_selector_without_a_table_case_is_illegal() {
    let mut selectors: Vec<u64> = vec![0];
    selectors.extend(6..=15);
    selectors.extend([
        NET_CONTROL_VLAN_SELECTOR_MAX + 1,
        1 << 32,
        (1 << 32) | 1,
        u64::MAX,
    ]);
    assert_eq!(selectors.len(), 15);
    for selector in selectors {
        assert_eq!(
            net_control_vlan_selector(selector),
            VlanSelector::Illegal,
            "selector {selector:#x}"
        );
    }
}

#[test]
fn the_failure_statuses_are_the_zero_extended_lv1_codes() {
    assert_eq!(NET_CONTROL_VLAN_SELECTOR_R3, 0x0000_0000_FFFF_FFEF);
    assert_eq!(NET_CONTROL_VLAN_NONE_R3, 0x0000_0000_FFFF_FFFA);
}
