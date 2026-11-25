use super::*;

#[test]
fn the_asynchronous_list_address_store_reaches_region_offset_0x28() {
    let c = EHCI_CONTROLLERS
        .iter()
        .find(|c| c.firmware == "4.93")
        .expect("a 4.93 controller");
    let stores: Vec<(u64, u64)> = EHCI_ADDRESS_STORES
        .iter()
        .filter(|s| s.firmware == "4.93")
        .map(|s| (c.operational + s.register, s.store))
        .collect();
    // [OFW-4.93 lv2_kernel a:0x8000000000245CEC] stw r3,24(r28), with r28
    // the region plus the capability length 0x10.
    assert_eq!(stores, [(0x28, 0x8000_0000_0024_5CEC)]);
}
