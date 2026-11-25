//! The PS3 setup registers of a 4.93 kernel run: the stores that follow
//! the controller reset, and the reset that returns them to unknown.

use cellgov_mem::{
    ByteRange, GuestAddr, GuestMemory, MemError, RegisterWord, StagedWrite, StagingMemory,
};
use cellgov_ps3_abi::hw::ehci::{EHCI_CONTROLLERS, INSNREG01, INSNREG03, USBCMD};
use cellgov_ps3_abi::lv1::device_bus::MMIO_REAL_BASE;

use super::declare_device_mmio;

/// [OFW-4.93 lv2_kernel a:0x800000000023F964] stwx r6,r5,r9: the store of the register write helper.
const HELPER_STORE: u64 = 0x8000_0000_0023_F964;

/// [OFW-4.93 lv2_kernel a:0x8000000000245998-0x80000000002459AC] The kernel gives the helper the word 0x01000020.
const BURST_COUNTS: u32 = 0x0100_0020;

/// [OFW-4.93 lv2_kernel a:0x80000000002459B0-0x80000000002459C0] The kernel gives the helper the word 1.
const BURST_ENABLE: u32 = 1;

fn declared() -> GuestMemory {
    let mut mem = GuestMemory::new(0x1000);
    declare_device_mmio(&mut mem, "4.93").unwrap();
    mem
}

/// The real address of USBCMD, `insnreg01` and `insnreg03` of the first
/// 4.93 controller.
fn registers() -> [u64; 3] {
    let c = EHCI_CONTROLLERS
        .iter()
        .find(|c| c.firmware == "4.93")
        .expect("a 4.93 controller");
    let base = MMIO_REAL_BASE + c.region + c.operational;
    [base + USBCMD, base + INSNREG01, base + INSNREG03]
}

/// Commits a store of `value` to the register at `real` through the
/// staging drain, as the commit pipeline does.
pub(super) fn store(mem: &mut GuestMemory, real: u64, value: u32) -> Result<(), MemError> {
    let range = ByteRange::new(GuestAddr::new(real), 4).unwrap();
    let mut staging = StagingMemory::new();
    staging.stage(StagedWrite::new(range, &value.to_be_bytes()));
    let drained = staging.drain_into(mem).map(|_| ());
    staging.clear();
    drained
}

/// Runs the kernel's halt and reset with stores of constant words: a
/// clear of Run/Stop, then a set of HCRESET.
fn reset(mem: &mut GuestMemory) {
    let [cmd, _, _] = registers();
    store(mem, cmd, 0).unwrap();
    store(mem, cmd, 2).unwrap();
}

#[test]
fn the_helper_stores_after_the_reset_hold_the_setup_words() {
    let mut mem = declared();
    let [_, insn01, insn03] = registers();
    reset(&mut mem);
    for (real, value) in [(insn01, BURST_COUNTS), (insn03, BURST_ENABLE)] {
        assert!(mem.register_store_accepted(real, value, HELPER_STORE));
        store(&mut mem, real, value).unwrap();
    }
    assert_eq!(
        [insn01, insn03].map(|real| mem.register_word(real)),
        [
            Some(RegisterWord::known(BURST_COUNTS)),
            Some(RegisterWord::known(BURST_ENABLE)),
        ]
    );
}

#[test]
fn a_setup_store_with_no_source_is_refused() {
    let [_, insn01, insn03] = registers();
    // (what the store is, its register, its value, its instruction)
    let cases: &[(&str, u64, u32, u64)] = &[
        (
            "another instruction",
            insn01,
            BURST_COUNTS,
            HELPER_STORE + 4,
        ),
        ("another word", insn01, BURST_COUNTS | 1, HELPER_STORE),
        (
            "the word of the other register",
            insn03,
            BURST_COUNTS,
            HELPER_STORE,
        ),
        ("a store of 0", insn03, 0, HELPER_STORE),
    ];
    assert_eq!(cases.len(), 4);
    for &(what, real, value, pc) in cases {
        let mut mem = declared();
        reset(&mut mem);
        assert!(!mem.register_store_accepted(real, value, pc), "{what}");
        assert_eq!(
            mem.register_word(real),
            Some(RegisterWord::UNKNOWN),
            "{what}"
        );
    }
}

#[test]
fn a_setup_store_of_another_word_fails_at_the_commit() {
    let mut mem = declared();
    let [_, insn01, _] = registers();
    assert_eq!(
        store(&mut mem, insn01, BURST_ENABLE),
        Err(MemError::RegisterStoreRefused { real: insn01 })
    );
    assert_eq!(mem.register_word(insn01), Some(RegisterWord::UNKNOWN));
}

#[test]
fn a_host_controller_reset_returns_the_setup_registers_to_unknown() {
    let mut mem = declared();
    let [_, insn01, insn03] = registers();
    store(&mut mem, insn01, BURST_COUNTS).unwrap();
    store(&mut mem, insn03, BURST_ENABLE).unwrap();
    reset(&mut mem);
    assert_eq!(
        [insn01, insn03].map(|real| mem.register_word(real)),
        [Some(RegisterWord::UNKNOWN), Some(RegisterWord::UNKNOWN)]
    );
}

#[test]
fn a_load_of_a_known_setup_register_gets_no_word() {
    let mut mem = declared();
    let [_, insn01, _] = registers();
    store(&mut mem, insn01, BURST_COUNTS).unwrap();
    assert_eq!(
        mem.register_word(insn01),
        Some(RegisterWord::known(BURST_COUNTS))
    );
    for reader in [HELPER_STORE, HELPER_STORE - 4] {
        assert_eq!(mem.register_read(insn01, reader), None, "{reader:x}");
    }
}
