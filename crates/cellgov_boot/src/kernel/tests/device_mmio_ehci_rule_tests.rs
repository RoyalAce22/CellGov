//! The register rule of each EHCI controller row: a row past the length
//! of the rule array gets the rule of its firmware, and a row at a
//! firmware with no rule gets a refusal.

use cellgov_mem::{GuestMemory, RegisterWord};
use cellgov_ps3_abi::hw::ehci::{EhciController, USBCMD, USBINTR, USBSTS};
use cellgov_ps3_abi::lv1::device_bus::{MmioDeviceIndex, MMIO_REAL_BASE};

use super::ehci_setup_tests::store;
use super::*;

/// A firmware at which no shipped EHCI controller is.
const OTHER: &str = "another firmware";

/// The 4.93 EHCI controller of the shipped table.
fn shipped_controller() -> EhciController {
    let at_4_93: Vec<_> = SHIPPED
        .controllers
        .iter()
        .filter(|c| c.firmware == "4.93")
        .collect();
    assert_eq!(at_4_93.len(), 1);
    *at_4_93[0]
}

#[test]
fn a_4_93_controller_row_past_the_rule_array_gets_the_4_93_rule() {
    let first = shipped_controller();
    let second = EhciController {
        operational: first.operational + 0x1000,
        ..first
    };
    let controllers = [first, second];
    assert!(controllers.len() > SHIPPED.controllers.len());
    let tables = MmioTables {
        controllers: &controllers,
        ..SHIPPED
    };
    let mut mem = GuestMemory::new(0x1000);
    declare_from(&mut mem, tables, "4.93").unwrap();
    for c in controllers {
        let base = MMIO_REAL_BASE + c.region + c.operational;
        for offset in [USBCMD, USBSTS] {
            assert_eq!(
                mem.register_word(base + offset),
                Some(RegisterWord::UNKNOWN),
                "{c:?} 0x{offset:x}"
            );
        }
        store(&mut mem, base + USBINTR, 0).unwrap();
        assert_eq!(
            mem.register_word(base + USBINTR),
            Some(RegisterWord::known(0)),
            "{c:?}"
        );
    }
}

#[test]
fn a_controller_at_a_firmware_with_no_rule_gets_the_no_rule_refusal_and_declares_nothing() {
    let regions: Vec<MmioRegion> = SHIPPED
        .regions
        .iter()
        .filter(|r| r.firmware == "4.93")
        .map(|r| MmioRegion {
            firmware: OTHER,
            ..*r
        })
        .collect();
    let indexes: Vec<MmioDeviceIndex> = SHIPPED
        .indexes
        .iter()
        .filter(|i| i.firmware == "4.93")
        .map(|i| MmioDeviceIndex {
            firmware: OTHER,
            ..*i
        })
        .collect();
    let shipped = shipped_controller();
    let controllers = [EhciController {
        firmware: OTHER,
        ..shipped
    }];
    let tables = MmioTables {
        regions: &regions,
        indexes: &indexes,
        reads: &[],
        controllers: &controllers,
    };
    assert!(!regions.is_empty());
    assert!(regions_of(tables, OTHER).all(|r| r.is_ok()));
    let mut mem = GuestMemory::new(0x1000);
    assert_eq!(
        declare_from(&mut mem, tables, OTHER),
        Err(DeviceMmioError::NoEhciRule {
            firmware: OTHER.to_owned(),
            bus: shipped.bus,
            device: shipped.device,
        })
    );
    assert_eq!(mem.unmodelled_spans(), &[] as &[(u64, u64)]);
}
