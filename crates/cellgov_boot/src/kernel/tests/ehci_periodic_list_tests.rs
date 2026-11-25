//! PERIODICLISTBASE of a 4.93 kernel run: the store of the periodic frame
//! list's address after the controller reset.

use cellgov_mem::{GuestMemory, MemError, RegisterWord};
use cellgov_ps3_abi::hw::ehci::{
    EHCI_CONTROLLERS, PERIODICLISTBASE, USBCMD, USBCMD_HCRESET, USBCMD_RESET_VALUE,
};
use cellgov_ps3_abi::lv1::device_bus::MMIO_REAL_BASE;

use super::declare_device_mmio;
use super::ehci_setup_tests::store;

/// [OFW-4.93 lv2_kernel a:0x8000000000245E6C] stw r3,20(r29): the store of the periodic frame list's address.
const LIST_STORE: u64 = 0x8000_0000_0024_5E6C;

/// A 4-Kbyte aligned bus address of a frame list.
const FRAME_LIST: u32 = 0x0012_3000;

/// [EHCI p:25 s:2.3.6] Bits 31:12 hold the base address; bits 11:0 are undefined during runtime.
const HELD: RegisterWord = RegisterWord {
    value: FRAME_LIST,
    known: 0xFFFF_F000,
};

fn declared() -> GuestMemory {
    let mut mem = GuestMemory::new(0x1000);
    declare_device_mmio(&mut mem, "4.93").unwrap();
    mem
}

/// The real address of USBCMD and PERIODICLISTBASE of the first 4.93
/// controller.
fn registers() -> [u64; 2] {
    let c = EHCI_CONTROLLERS
        .iter()
        .find(|c| c.firmware == "4.93")
        .expect("a 4.93 controller");
    let base = MMIO_REAL_BASE + c.region + c.operational;
    [base + USBCMD, base + PERIODICLISTBASE]
}

/// Runs the kernel's halt and reset: a clear of Run/Stop, then a set of
/// HCRESET.
fn reset(mem: &mut GuestMemory) {
    let [cmd, _] = registers();
    store(mem, cmd, 0).unwrap();
    store(mem, cmd, USBCMD_HCRESET).unwrap();
}

#[test]
fn the_kernel_store_after_the_reset_holds_the_frame_list_address() {
    let mut mem = declared();
    let [_, list] = registers();
    assert_eq!(mem.register_word(list), Some(RegisterWord::UNKNOWN));
    reset(&mut mem);
    assert!(mem.register_store_accepted(list, FRAME_LIST, LIST_STORE));
    store(&mut mem, list, FRAME_LIST).unwrap();
    assert_eq!(mem.register_word(list), Some(HELD));
}

#[test]
fn a_periodiclistbase_store_by_another_instruction_or_before_the_reset_is_refused() {
    let [_, list] = registers();
    let mut mem = declared();
    assert!(!mem.register_store_accepted(list, FRAME_LIST, LIST_STORE));
    assert_eq!(
        store(&mut mem, list, FRAME_LIST),
        Err(MemError::RegisterStoreRefused { real: list })
    );
    reset(&mut mem);
    assert!(!mem.register_store_accepted(list, FRAME_LIST, LIST_STORE + 4));
    assert_eq!(mem.register_word(list), Some(RegisterWord::UNKNOWN));
}

#[test]
fn a_host_controller_reset_returns_periodiclistbase_to_unknown() {
    let mut mem = declared();
    let [cmd, list] = registers();
    reset(&mut mem);
    store(&mut mem, list, FRAME_LIST).unwrap();
    // A second reset keeps the known bits of USBCMD that the first one set.
    store(&mut mem, cmd, USBCMD_RESET_VALUE | USBCMD_HCRESET).unwrap();
    assert_eq!(mem.register_word(list), Some(RegisterWord::UNKNOWN));
}

#[test]
fn a_load_of_periodiclistbase_gets_no_word() {
    let mut mem = declared();
    let [_, list] = registers();
    reset(&mut mem);
    store(&mut mem, list, FRAME_LIST).unwrap();
    assert_eq!(mem.register_read(list, LIST_STORE), None);
}
