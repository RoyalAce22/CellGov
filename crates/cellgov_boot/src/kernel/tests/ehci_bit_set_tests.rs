//! The PS3 setup register `insnreg02` of a 4.93 kernel run: the load that
//! gets a word with no known bit, and the store that sets bit 0x800.

use cellgov_mem::{GuestMemory, MemError, RegisterWord};
use cellgov_ps3_abi::hw::ehci::{EHCI_CONTROLLERS, INSNREG02, USBCMD};
use cellgov_ps3_abi::lv1::device_bus::MMIO_REAL_BASE;

use super::declare_device_mmio;
use super::ehci_setup_tests::store;

/// [OFW-4.93 lv2_kernel a:0x800000000023F9E8] lwzx r0,r5,r11: the load of the register read helper.
const HELPER_LOAD: u64 = 0x8000_0000_0023_F9E8;

/// [OFW-4.93 lv2_kernel a:0x800000000023F964] stwx r6,r5,r9: the store of the register write helper.
const HELPER_STORE: u64 = 0x8000_0000_0023_F964;

/// [OFW-4.93 lv2_kernel a:0x8000000000245F18] ori r0,r0,0x800: the bit that the kernel sets.
const BIT: u32 = 0x800;

fn declared() -> GuestMemory {
    let mut mem = GuestMemory::new(0x1000);
    declare_device_mmio(&mut mem, "4.93").unwrap();
    mem
}

/// The real address of USBCMD and `insnreg02` of the first 4.93
/// controller.
fn registers() -> [u64; 2] {
    let c = EHCI_CONTROLLERS
        .iter()
        .find(|c| c.firmware == "4.93")
        .expect("a 4.93 controller");
    let base = MMIO_REAL_BASE + c.region + c.operational;
    [base + USBCMD, base + INSNREG02]
}

/// Runs the kernel's read, set and write of `insnreg02`: the helper load,
/// then the helper store of the loaded word with [`BIT`] set.
fn read_set_write(mem: &mut GuestMemory, real: u64) {
    let (at, bytes) = mem.register_read(real, HELPER_LOAD).expect("a word");
    assert_eq!(at, real);
    let loaded = u32::from_be_bytes(bytes.try_into().unwrap());
    assert!(mem.register_store_accepted(real, loaded | BIT, HELPER_STORE));
    store(mem, real, loaded | BIT).unwrap();
}

#[test]
fn the_kernel_read_set_write_leaves_only_bit_0x800_known() {
    let mut mem = declared();
    let [_, insn02] = registers();
    assert_eq!(mem.register_word(insn02), Some(RegisterWord::UNKNOWN));
    let set = RegisterWord {
        value: BIT,
        known: BIT,
    };
    for _ in 0..2 {
        read_set_write(&mut mem, insn02);
        assert_eq!(mem.register_word(insn02), Some(set));
    }
}

#[test]
fn an_insnreg02_access_outside_the_kernel_read_set_write_is_refused() {
    let [_, insn02] = registers();
    // (what the store is, its value, its instruction)
    let cases: &[(&str, u32, u64)] = &[
        ("another instruction", BIT, HELPER_STORE + 4),
        ("a store that does not set the bit", 0, HELPER_STORE),
        ("a store that sets an unknown bit", BIT | 1, HELPER_STORE),
    ];
    assert_eq!(cases.len(), 3);
    for &(what, value, pc) in cases {
        let mem = declared();
        assert!(!mem.register_store_accepted(insn02, value, pc), "{what}");
        assert_eq!(
            mem.register_word(insn02),
            Some(RegisterWord::UNKNOWN),
            "{what}"
        );
    }
    let mem = declared();
    assert_eq!(mem.register_read(insn02, HELPER_LOAD + 4), None);
}

#[test]
fn an_insnreg02_store_that_clears_the_set_bit_fails_at_the_commit() {
    let mut mem = declared();
    let [_, insn02] = registers();
    read_set_write(&mut mem, insn02);
    assert_eq!(
        store(&mut mem, insn02, 0),
        Err(MemError::RegisterStoreRefused { real: insn02 })
    );
}

#[test]
fn a_host_controller_reset_returns_insnreg02_to_unknown() {
    let mut mem = declared();
    let [cmd, insn02] = registers();
    read_set_write(&mut mem, insn02);
    store(&mut mem, cmd, 0).unwrap();
    store(&mut mem, cmd, 2).unwrap();
    assert_eq!(mem.register_word(insn02), Some(RegisterWord::UNKNOWN));
}
