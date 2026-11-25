use super::*;

#[test]
fn the_insnreg02_bit_set_reaches_the_offset_that_the_kernel_gives_its_helpers() {
    let c = EHCI_CONTROLLERS
        .iter()
        .find(|c| c.firmware == "4.93")
        .expect("a 4.93 controller");
    let sets: Vec<(u64, u32)> = EHCI_SETUP_BIT_SETS
        .iter()
        .filter(|s| s.firmware == "4.93")
        .map(|s| (c.operational + s.register, s.bits))
        .collect();
    // [OFW-4.93 lv2_kernel a:0x8000000000245F00-0x8000000000245F34] r5 = 152 for both helpers; ori sets 0x800.
    assert_eq!(sets, [(152, 0x800)]);
}

#[test]
fn each_bit_set_register_has_no_setup_word_and_its_store_is_a_setup_store() {
    assert_eq!(EHCI_SETUP_BIT_SETS.len(), 1);
    for set in EHCI_SETUP_BIT_SETS {
        let words = EHCI_SETUP_STORES
            .iter()
            .filter(|s| s.firmware == set.firmware && s.register == set.register);
        assert_eq!(words.count(), 0, "{set:?}");
        let helper = EHCI_SETUP_STORES
            .iter()
            .any(|s| s.firmware == set.firmware && s.store == set.store);
        assert!(helper, "{set:?}");
        assert_ne!(set.bits, 0, "{set:?}");
    }
}
