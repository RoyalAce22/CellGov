//! The EHCI operational registers of a 4.93 kernel run: the kernel's
//! reset order and the stores that the model refuses.

use cellgov_mem::{
    ByteRange, GuestAddr, GuestMemory, MemError, RegisterWord, StagedWrite, StagingMemory,
};
use cellgov_ps3_abi::hw::ehci::{
    EHCI_CONTROLLERS, USBCMD, USBCMD_PARK_BITS, USBCMD_RESET_VALUE, USBINTR, USBSTS,
    USBSTS_RESET_VALUE,
};
use cellgov_ps3_abi::lv1::device_bus::MMIO_REAL_BASE;

use super::declare_device_mmio;

// The four readers of the reset, in the order of the kernel's loads.

/// [OFW-4.93 lv2_kernel a:0x80000000002422E0] lwz r0,0(r11): the load of USBCMD whose Run/Stop the code clears.
const CMD_CLEAR: u64 = 0x8000_0000_0024_22E0;

/// [OFW-4.93 lv2_kernel a:0x80000000002422F4] lwz r9,4(r11): the first load of the HCHalted poll.
const STS_POLL: u64 = 0x8000_0000_0024_22F4;

/// [OFW-4.93 lv2_kernel a:0x8000000000242338] lwz r0,0(r11): the load of USBCMD whose HCRESET the code sets.
const CMD_RESET: u64 = 0x8000_0000_0024_2338;

/// [OFW-4.93 lv2_kernel a:0x8000000000242348] lwz r9,0(r11): the first load of the HCRESET poll.
const CMD_POLL: u64 = 0x8000_0000_0024_2348;

/// [OFW-4.93 lv2_kernel a:0x80000000002422F0] stw r0,0(r11): the store that writes back the word of [`CMD_CLEAR`].
const CMD_CLEAR_STORE: u64 = 0x8000_0000_0024_22F0;

fn declared() -> GuestMemory {
    let mut mem = GuestMemory::new(0x1000);
    declare_device_mmio(&mut mem, "4.93").unwrap();
    mem
}

/// The real address of each operational register of the first 4.93
/// controller: USBCMD, USBSTS and USBINTR.
fn registers() -> [u64; 3] {
    let c = EHCI_CONTROLLERS
        .iter()
        .find(|c| c.firmware == "4.93")
        .expect("a 4.93 controller");
    let base = MMIO_REAL_BASE + c.region + c.operational;
    [base + USBCMD, base + USBSTS, base + USBINTR]
}

/// Commits a store of `value` to the register at `real` through the
/// staging drain, as the commit pipeline does.
fn store(mem: &mut GuestMemory, real: u64, value: u32) -> Result<(), MemError> {
    let range = ByteRange::new(GuestAddr::new(real), 4).unwrap();
    let mut staging = StagingMemory::new();
    staging.stage(StagedWrite::new(range, &value.to_be_bytes()));
    let drained = staging.drain_into(mem).map(|_| ());
    staging.clear();
    drained
}

fn read(mem: &GuestMemory, real: u64, reader: u64) -> Option<u32> {
    let (at, bytes) = mem.register_read(real, reader)?;
    assert_eq!(at, real);
    Some(u32::from_be_bytes(bytes.try_into().unwrap()))
}

#[test]
fn the_kernel_reset_halts_and_resets_the_controller() {
    let mut mem = declared();
    let [cmd, sts, intr] = registers();
    store(&mut mem, intr, 0).unwrap();
    let word = read(&mem, cmd, CMD_CLEAR).unwrap();
    store(&mut mem, cmd, word & !1).unwrap();
    assert_eq!(read(&mem, sts, STS_POLL).map(|w| w & 0x1000), Some(0x1000));
    let word = read(&mem, cmd, CMD_RESET).unwrap();
    store(&mut mem, cmd, word | 2).unwrap();
    assert_eq!(read(&mem, cmd, CMD_POLL).map(|w| w & 2), Some(0));
    let after = [cmd, sts, intr].map(|real| mem.register_word(real));
    let reset_cmd = RegisterWord {
        value: USBCMD_RESET_VALUE,
        known: !USBCMD_PARK_BITS,
    };
    assert_eq!(
        after,
        [
            Some(reset_cmd),
            Some(RegisterWord::known(USBSTS_RESET_VALUE)),
            Some(RegisterWord::known(0)),
        ]
    );
}

#[test]
fn a_clear_of_run_stop_halts_and_leaves_each_bit_the_code_stored_back_unknown() {
    let mut mem = declared();
    let [cmd, sts, _] = registers();
    store(&mut mem, cmd, 0).unwrap();
    assert_eq!(
        mem.register_word(cmd),
        Some(RegisterWord { value: 0, known: 1 })
    );
    assert_eq!(
        mem.register_word(sts),
        Some(RegisterWord {
            value: 0x1000,
            known: 0x1000
        })
    );
}

#[test]
fn after_the_reset_a_store_that_changes_a_known_unmodelled_bit_is_refused() {
    let mut mem = declared();
    let [cmd, _, _] = registers();
    store(&mut mem, cmd, 0).unwrap();
    store(&mut mem, cmd, 2).unwrap();
    let before = mem.register_word(cmd);
    // [EHCI p:18 s:2.3.1] Interrupt Threshold Control in bits 23:16 defaults to 08h, and 00h is reserved.
    assert_eq!(
        store(&mut mem, cmd, 0),
        Err(MemError::RegisterStoreRefused { real: cmd })
    );
    assert_eq!(mem.register_word(cmd), before);
    assert!(mem.register_store_accepted(cmd, USBCMD_RESET_VALUE, CMD_CLEAR_STORE));
}

#[test]
fn a_poll_gets_no_word_before_the_bit_it_keeps_is_known() {
    let mem = declared();
    let [cmd, sts, _] = registers();
    assert_eq!(read(&mem, cmd, CMD_CLEAR), Some(0));
    assert_eq!(read(&mem, sts, STS_POLL), None);
    assert_eq!(read(&mem, cmd, CMD_RESET), None);
    assert_eq!(read(&mem, cmd, CMD_POLL), None);
    assert_eq!(read(&mem, cmd, CMD_POLL + 4), None);
}

#[test]
fn the_model_refuses_each_store_it_has_no_source_for() {
    let [cmd, sts, intr] = registers();
    // (what the store is, its register, its value)
    let cases: &[(&str, u64, u32)] = &[
        ("set Run/Stop", cmd, 1),
        ("reset before the halt", cmd, 2),
        ("set a schedule enable", cmd, 0x10),
        ("enable an interrupt", intr, 1),
        ("clear a status bit", sts, 0x1000),
    ];
    assert_eq!(cases.len(), 5);
    for &(what, real, value) in cases {
        let mut mem = declared();
        let before = mem.register_word(real);
        assert!(
            !mem.register_store_accepted(real, value, CMD_CLEAR_STORE),
            "{what}"
        );
        assert_eq!(
            store(&mut mem, real, value),
            Err(MemError::RegisterStoreRefused { real }),
            "{what}"
        );
        assert_eq!(mem.register_word(real), before, "{what}");
    }
}
