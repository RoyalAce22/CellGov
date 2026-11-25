use super::*;
use crate::hw::device_register::DEVICE_REGISTER_READS;

/// The low 4 bits of the captured word at +4 of `c`'s region: the port
/// count.
fn port_count(c: &EhciController) -> u64 {
    let read = DEVICE_REGISTER_READS
        .iter()
        .find(|r| {
            (r.firmware, r.bus, r.device, r.region, r.offset)
                == (c.firmware, c.bus, c.device, c.region, 4)
        })
        .unwrap_or_else(|| panic!("no port count read for {c:?}"));
    assert_eq!(read.known, 0xF, "{c:?}");
    u64::from(read.value)
}

#[test]
fn each_setup_store_reaches_the_offset_that_the_kernel_gives_its_write_helper() {
    let c = EHCI_CONTROLLERS
        .iter()
        .find(|c| c.firmware == "4.93")
        .expect("a 4.93 controller");
    let stores: Vec<(u64, u32)> = EHCI_SETUP_STORES
        .iter()
        .filter(|s| s.firmware == "4.93")
        .map(|s| (c.operational + s.register, s.value))
        .collect();
    // [OFW-4.93 lv2_kernel a:0x8000000000245998-0x80000000002459AC] r5 = 148 and r6 = 0x01000020.
    // [OFW-4.93 lv2_kernel a:0x80000000002459B0-0x80000000002459C0] r5 = 156 and r6 = 1.
    assert_eq!(stores, [(148, 0x0100_0020), (156, 1)]);
}

#[test]
fn each_setup_register_is_past_the_operational_registers_of_its_controller() {
    assert_eq!(EHCI_SETUP_STORES.len(), 2);
    for s in EHCI_SETUP_STORES {
        let c = EHCI_CONTROLLERS
            .iter()
            .find(|c| c.firmware == s.firmware)
            .unwrap_or_else(|| panic!("no controller for {s:?}"));
        // [EHCI p:17 s:2.3] Table 2-8 puts PORTSC(1) to PORTSC(N_PORTS) at 44h, after the last register before it.
        let end = 0x44 + 4 * port_count(c);
        assert!(s.register >= end && s.register % 4 == 0, "{s:?}");
    }
}
