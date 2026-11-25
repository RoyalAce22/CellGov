//! The real address of each piece of device MMIO that a kernel run
//! declares, and the refusal of a region that lv1's address function
//! gives no address CellGov holds.

use cellgov_mem::{GuestMemory, RegisterWord};
use cellgov_ps3_abi::hw::ehci::{USBCMD, USBINTR, USBSTS};
use cellgov_ps3_abi::lv1::device_bus::{
    mmio_real_address, MmioDeviceIndex, MmioRealAddress, MMIO_BUS_DOUBLEWORD_INDEX,
    MMIO_DEVICE_INDEXES, MMIO_REAL_BASE,
};

use super::*;

/// The real address that [`mmio_real_address`] gives the region at
/// `bus_address` of device `device` of bus `bus` at 4.93.
fn abi_real(bus: u64, device: u64, bus_address: u64) -> u64 {
    match mmio_real_address("4.93", bus, device, bus_address) {
        MmioRealAddress::Real(real) => real,
        other => panic!("bus {bus} device {device} 0x{bus_address:x}: {other:?}"),
    }
}

fn declared() -> GuestMemory {
    let mut mem = GuestMemory::new(0x1000);
    declare_device_mmio(&mut mem, "4.93").unwrap();
    mem
}

/// The 4.93 index row of device `device` of bus 1, with the index
/// `index`. No shipped table holds a row that this function changes.
fn index_row(device: u64, index: u32) -> MmioDeviceIndex {
    let shipped = MMIO_DEVICE_INDEXES
        .iter()
        .find(|row| (row.firmware, row.bus, row.device) == ("4.93", 1, device))
        .expect("a 4.93 index row");
    MmioDeviceIndex { index, ..*shipped }
}

/// The first 4.93 region of device `device` of bus 1, in table order.
fn first_region(device: u64) -> &'static MmioRegion {
    SHIPPED
        .regions
        .iter()
        .find(|r| (r.firmware, r.bus, r.device) == ("4.93", 1, device))
        .expect("a 4.93 region")
}

/// Asserts that `mem` holds none of the 4.93 device MMIO.
fn assert_declares_nothing(mem: &GuestMemory) {
    assert_eq!(mem.unmodelled_spans(), &[] as &[(u64, u64)]);
    for read in SHIPPED.reads {
        let real = abi_real(read.bus, read.device, read.region) + read.offset;
        assert_eq!(mem.sourced_word(real, read.reader), None, "{read:?}");
    }
    for c in SHIPPED.controllers {
        let base = abi_real(c.bus, c.device, c.region) + c.operational;
        for offset in [USBCMD, USBSTS, USBINTR] {
            assert_eq!(mem.register_word(base + offset), None, "{c:?}");
        }
    }
}

/// lv1's init gives devices 1 and 2 index 3 and index 4
/// [OFW-4.93 lv1 a:0x3A9A4C-0x3A9A5C] [OFW-4.93 lv1 a:0x3A9AA0-0x3A9AB0].
/// For those indexes lv1's address function adds only [`MMIO_REAL_BASE`]
/// [OFW-4.93 lv1 a:0x22BE1C-0x22BE44].
#[test]
fn each_4_93_span_lies_at_the_mmio_real_address_of_its_region() {
    let regions: Vec<_> = SHIPPED
        .regions
        .iter()
        .filter(|r| r.firmware == "4.93")
        .collect();
    assert_eq!(regions.len(), 6);
    let mut expected: Vec<_> = regions
        .iter()
        .map(|r| {
            let real = abi_real(r.bus, r.device, r.address);
            assert_eq!(real, MMIO_REAL_BASE + r.address, "{r:?}");
            (real, real + r.size)
        })
        .collect();
    expected.sort_unstable();
    assert_eq!(declared().unmodelled_spans(), expected.as_slice());
}

#[test]
fn each_4_93_register_word_lies_at_the_mmio_real_address_of_its_region_plus_its_offset() {
    let mem = declared();
    let words = words_of(SHIPPED, "4.93").unwrap();
    assert_eq!(words.len(), 2);
    for (real, read) in words {
        let expected = abi_real(read.bus, read.device, read.region) + read.offset;
        assert_eq!(real, expected, "{read:?}");
        let word = read.value.to_be_bytes();
        assert_eq!(
            mem.sourced_word(expected, read.reader),
            Some((expected, &word[..])),
            "{read:?}"
        );
    }
}

#[test]
fn each_4_93_ehci_controller_lies_at_the_mmio_real_address_of_its_region_plus_its_operational_offset(
) {
    let mem = declared();
    let controllers = ehci::controllers_of(SHIPPED, "4.93").unwrap();
    let rows: Vec<_> = SHIPPED
        .controllers
        .iter()
        .filter(|c| c.firmware == "4.93")
        .collect();
    assert_eq!(controllers.len(), 1);
    assert_eq!(controllers.len(), rows.len());
    for ((base, _), c) in controllers.into_iter().zip(rows) {
        let expected = abi_real(c.bus, c.device, c.region) + c.operational;
        assert_eq!(base, expected, "{c:?}");
        for offset in [USBCMD, USBSTS, USBINTR] {
            assert_eq!(
                mem.register_word(expected + offset),
                Some(RegisterWord::UNKNOWN),
                "{c:?}"
            );
        }
    }
}

#[test]
fn an_access_in_a_4_93_region_gets_the_bus_address_of_its_offset_from_the_region_real_address() {
    let regions: Vec<_> = SHIPPED
        .regions
        .iter()
        .filter(|r| r.firmware == "4.93")
        .collect();
    assert_eq!(regions.len(), 6);
    for region in regions {
        let real = abi_real(region.bus, region.device, region.address);
        for offset in [0, region.size - 4, region.size - 1] {
            assert_eq!(
                region_at(SHIPPED, "4.93", real + offset),
                Some((region, region.address + offset)),
                "{region:?} +0x{offset:x}"
            );
        }
        let past = region_at(SHIPPED, "4.93", real + region.size).map(|(r, _)| r);
        assert_ne!(past, Some(region), "{region:?}");
    }
}

/// For index 6, lv1's address function also adds a doubleword of the bus
/// object [OFW-4.93 lv1 a:0x22BE4C-0x22BE60].
#[test]
fn a_device_of_index_6_gets_the_bus_doubleword_refusal_and_declares_nothing() {
    let indexes = [index_row(1, 3), index_row(2, MMIO_BUS_DOUBLEWORD_INDEX)];
    let tables = MmioTables {
        indexes: &indexes,
        ..SHIPPED
    };
    let region = first_region(2);
    let mut mem = GuestMemory::new(0x1000);
    assert_eq!(
        declare_from(&mut mem, tables, "4.93"),
        Err(DeviceMmioError::NoRealAddress {
            firmware: "4.93".to_owned(),
            bus: 1,
            device: 2,
            bus_address: region.address,
            source: MmioRealRefusal::BusDoubleword,
        })
    );
    assert_declares_nothing(&mem);
}

#[test]
fn a_device_with_no_index_row_gets_the_no_index_refusal_and_declares_nothing() {
    let indexes = [index_row(2, 4)];
    let tables = MmioTables {
        indexes: &indexes,
        ..SHIPPED
    };
    let region = first_region(1);
    let mut mem = GuestMemory::new(0x1000);
    assert_eq!(
        declare_from(&mut mem, tables, "4.93"),
        Err(DeviceMmioError::NoRealAddress {
            firmware: "4.93".to_owned(),
            bus: 1,
            device: 1,
            bus_address: region.address,
            source: MmioRealRefusal::NoIndex,
        })
    );
    assert_declares_nothing(&mem);
}

#[test]
fn a_register_word_and_an_ehci_controller_of_an_index_6_device_get_the_bus_doubleword_refusal() {
    let indexes = [index_row(1, MMIO_BUS_DOUBLEWORD_INDEX), index_row(2, 4)];
    let tables = MmioTables {
        indexes: &indexes,
        ..SHIPPED
    };
    let read = SHIPPED.reads.first().expect("a register word");
    assert_eq!(
        words_of(tables, "4.93"),
        Err(DeviceMmioError::NoRealAddress {
            firmware: "4.93".to_owned(),
            bus: read.bus,
            device: read.device,
            bus_address: read.region,
            source: MmioRealRefusal::BusDoubleword,
        })
    );
    let c = SHIPPED.controllers.first().expect("an EHCI controller");
    assert_eq!(
        ehci::controllers_of(tables, "4.93").map(|v| v.len()),
        Err(DeviceMmioError::NoRealAddress {
            firmware: "4.93".to_owned(),
            bus: c.bus,
            device: c.device,
            bus_address: c.region,
            source: MmioRealRefusal::BusDoubleword,
        })
    );
}

#[test]
fn no_access_matches_a_region_of_an_index_6_device() {
    let indexes = [index_row(1, 3), index_row(2, MMIO_BUS_DOUBLEWORD_INDEX)];
    let tables = MmioTables {
        indexes: &indexes,
        ..SHIPPED
    };
    let regions: Vec<_> = SHIPPED
        .regions
        .iter()
        .filter(|r| (r.firmware, r.device) == ("4.93", 2))
        .collect();
    assert_eq!(regions.len(), 3);
    for region in regions {
        let sum = MMIO_REAL_BASE + region.address;
        assert_eq!(region_at(tables, "4.93", sum), None, "{region:?}");
    }
}
