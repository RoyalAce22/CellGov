//! The refusal of a register word or an EHCI controller whose region the
//! region table does not hold, after every region of the table resolves.

use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::device_register::DeviceRegisterRead;
use cellgov_ps3_abi::hw::ehci::EhciController;

use super::real_tests::assert_declares_nothing;
use super::*;

/// A bus address of device `device` of bus `bus` at 4.93 near
/// `region`, at which the region table holds no region.
fn lacking_region(bus: u64, device: u64, region: u64) -> u64 {
    let lacking = region + 4;
    assert!(
        regions_of(SHIPPED, "4.93").all(|r| r.is_ok()),
        "every 4.93 region resolves"
    );
    assert!(
        !SHIPPED
            .regions
            .iter()
            .any(|r| (r.firmware, r.bus, r.device, r.address) == ("4.93", bus, device, lacking)),
        "bus {bus} device {device} has a region at 0x{lacking:x}"
    );
    lacking
}

#[test]
fn a_register_word_whose_region_the_table_lacks_gets_the_no_region_refusal_and_declares_nothing() {
    let shipped = SHIPPED
        .reads
        .iter()
        .find(|r| r.firmware == "4.93")
        .expect("a 4.93 register word");
    let region = lacking_region(shipped.bus, shipped.device, shipped.region);
    let reads = [DeviceRegisterRead { region, ..*shipped }];
    let tables = MmioTables {
        reads: &reads,
        ..SHIPPED
    };
    let mut mem = GuestMemory::new(0x1000);
    assert_eq!(
        declare_from(&mut mem, tables, "4.93"),
        Err(DeviceMmioError::NoRealAddress {
            firmware: "4.93".to_owned(),
            bus: shipped.bus,
            device: shipped.device,
            bus_address: region,
            source: MmioRealRefusal::NoRegion,
        })
    );
    assert_declares_nothing(&mem);
}

#[test]
fn an_ehci_controller_whose_region_the_table_lacks_gets_the_no_region_refusal_and_declares_nothing()
{
    let shipped = SHIPPED
        .controllers
        .iter()
        .find(|c| c.firmware == "4.93")
        .expect("a 4.93 EHCI controller");
    let region = lacking_region(shipped.bus, shipped.device, shipped.region);
    let controllers = [EhciController { region, ..*shipped }];
    let tables = MmioTables {
        controllers: &controllers,
        ..SHIPPED
    };
    let mut mem = GuestMemory::new(0x1000);
    assert_eq!(
        declare_from(&mut mem, tables, "4.93"),
        Err(DeviceMmioError::NoRealAddress {
            firmware: "4.93".to_owned(),
            bus: shipped.bus,
            device: shipped.device,
            bus_address: region,
            source: MmioRealRefusal::NoRegion,
        })
    );
    assert_declares_nothing(&mem);
}
