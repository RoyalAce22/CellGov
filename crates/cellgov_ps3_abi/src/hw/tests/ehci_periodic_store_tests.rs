use super::*;

#[test]
fn the_periodic_frame_list_address_store_reaches_region_offset_0x24() {
    let c = EHCI_CONTROLLERS
        .iter()
        .find(|c| c.firmware == "4.93")
        .expect("a 4.93 controller");
    let stores: Vec<(u64, u64)> = EHCI_ADDRESS_STORES
        .iter()
        .filter(|s| s.firmware == "4.93" && s.store == 0x8000_0000_0024_5E6C)
        .map(|s| (c.operational + s.register, s.store))
        .collect();
    // [OFW-4.93 lv2_kernel a:0x8000000000245E6C] stw r3,20(r29), with r29
    // the region plus the capability length 0x10.
    assert_eq!(stores, [(0x24, 0x8000_0000_0024_5E6C)]);
}

#[test]
fn the_4_93_kernel_declares_one_store_for_each_schedule_address_register() {
    let mut registers: Vec<u64> = EHCI_ADDRESS_STORES
        .iter()
        .filter(|s| s.firmware == "4.93")
        .map(|s| s.register)
        .collect();
    registers.sort_unstable();
    // [EHCI p:24 s:2.3.6] PERIODICLISTBASE is at 14h; [EHCI p:25 s:2.3.7] ASYNCLISTADDR is at 18h.
    assert_eq!(registers, [0x14, 0x18]);
}
