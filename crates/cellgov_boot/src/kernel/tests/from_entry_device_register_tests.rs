use cellgov_ps3_abi::hw::ehci::{EHCI_CONTROLLERS, USBCMD};
use cellgov_ps3_abi::lv1::device_bus::{mmio_region, MMIO_REAL_BASE};

use super::device_mmio::program;
use super::page_table::{DATA_PAGE, ESID_EA};
use super::*;
use crate::kernel::DeviceMmioAccess;

/// The stop of the run of [`program`] with `access` at USBCMD of device
/// 1's controller, and the access the run should stop at.
fn run_at_usbcmd(access: u32) -> (KernelRunStop, KernelRunStop) {
    let controller = EHCI_CONTROLLERS[0];
    assert_eq!((controller.bus, controller.device), (1, 1));
    let offset = (controller.operational + USBCMD) as u32;
    let (code, at) = program(access, offset);
    let run = run_kernel_from_entry(&kernel_elf(&code), "4.93", 10_000).unwrap();
    let (bus, device) = (controller.bus, controller.device);
    let region = *mmio_region("4.93", bus, device, controller.region).expect("the 4.93 region");
    let bus_address = controller.region + controller.operational + USBCMD;
    let expected = KernelRunStop::DeviceMmio(DeviceMmioAccess {
        thread: 0,
        pc: Some(ENTRY + 4 * at as u64),
        region,
        bus_address,
        real: MMIO_REAL_BASE + bus_address,
        ea: u64::from((ESID_EA << 16) | DATA_PAGE | offset),
    });
    (run.stop, expected)
}

/// [EHCI p:18 s:2.3.1] Interrupt Threshold Control in bits 23:16 defaults to 08h, and 00h is reserved.
#[test]
fn a_store_of_a_constant_0_to_usbcmd_stops_at_the_word() {
    // li r10,0 then stw r10: no load of USBCMD gives the stored word, so
    // each 0 is the code's own, and the store writes 00h to the threshold.
    let (stop, expected) = run_at_usbcmd(36 << 26);
    assert_eq!(stop, expected);
}
