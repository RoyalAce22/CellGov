use super::*;

/// The fields with bit 63 clear and the flag byte `flag`.
const fn clear(flag: u8) -> VlanFields {
    VlanFields {
        capability: 0,
        flag,
    }
}

/// Selectors of each case kind:
/// - selectors with a fixed id;
/// - selector 5;
/// - selectors without a table case;
/// - selectors above the range check.
const SELECTORS: [u64; 9] = [0, 1, 4, 5, 6, 16, 20, 21, u64::MAX];

/// [OFW-4.93 lv1 a:0x244DE0-0x244DEC] tests bit 63 before the range check
/// [OFW-4.93 lv1 a:0x244DF0-0x244DF4].
#[test]
fn bit_63_set_fails_every_selector_with_minus_6() {
    let capabilities = [1, 3, u64::MAX];
    assert_eq!(capabilities.len() * SELECTORS.len(), 27);
    for capability in capabilities {
        for selector in SELECTORS {
            for flag in [0, 1] {
                assert_eq!(
                    net_control_vlan_id(selector, VlanFields { capability, flag }),
                    VlanAnswer::Failed(0x0000_0000_FFFF_FFFA),
                    "capability {capability:#x}, selector {selector:#x}, flag {flag}"
                );
            }
        }
    }
}

/// `clrlwi r0, r0, 31` at [OFW-4.93 lv1 a:0x244DE4] keeps bit 63 alone.
/// Thus bits 60 and 61, which a wireless path tests
/// [OFW-4.93 lv1 a:0x24DB84-0x24DB94], leave the table.
#[test]
fn a_capability_with_bit_63_clear_leaves_the_table() {
    let capabilities = [0, 0xC, !1];
    assert_eq!(capabilities.len(), 3);
    for capability in capabilities {
        let fields = VlanFields {
            capability,
            flag: 0,
        };
        assert_eq!(
            net_control_vlan_id(1, fields),
            VlanAnswer::Id(292),
            "{capability:#x}"
        );
        assert_eq!(
            net_control_vlan_id(21, fields),
            VlanAnswer::Failed(0x0000_0000_FFFF_FFEF),
            "{capability:#x}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x244EC8-0x244EE0] with the case at
/// [OFW-4.93 lv1 a:0x244E88-0x244E8C].
#[test]
fn selector_5_gives_294_for_a_flag_byte_of_0_and_296_otherwise() {
    assert_eq!(net_control_vlan_id(5, clear(0)), VlanAnswer::Id(294));
    let set = [1, 0x80, 0xFF];
    assert_eq!(set.len(), 3);
    for flag in set {
        assert_eq!(
            net_control_vlan_id(5, clear(flag)),
            VlanAnswer::Id(296),
            "flag {flag:#x}"
        );
    }
}

#[test]
fn with_bit_63_clear_each_other_selector_gets_its_table_case() {
    let others: Vec<u64> = SELECTORS.into_iter().filter(|&s| s != 5).collect();
    assert_eq!(others.len(), 8);
    for selector in others {
        let expected = match net_control_vlan_selector(selector) {
            VlanSelector::Fixed(id) => VlanAnswer::Id(id),
            VlanSelector::Illegal => VlanAnswer::Failed(NET_CONTROL_VLAN_SELECTOR_R3),
            VlanSelector::Flagged => panic!("selector {selector} is flagged"),
        };
        for flag in [0, 1] {
            assert_eq!(
                net_control_vlan_id(selector, clear(flag)),
                expected,
                "selector {selector:#x}, flag {flag}"
            );
        }
    }
}
