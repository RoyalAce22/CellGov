//! The operational registers of the EHCI host controllers in a kernel
//! run, and the rule that applies a store to them.
//!
//! The run models these registers of each controller of
//! [`EHCI_CONTROLLERS`] as register words:
//! - USBCMD, USBSTS and USBINTR;
//! - the PS3 setup registers `insnreg01` and `insnreg03`.
//!
//! No source gives their state when the kernel starts, so each word
//! starts with no known bit. The rule accepts only these stores, and a
//! refused store stops the run:
//! - a store of 0 to USBINTR, which disables every interrupt;
//! - a store to USBCMD that clears Run/Stop: the controller halts at the
//!   store, so USBSTS gets HCHalted = 1;
//! - a store to USBCMD that sets HCRESET while HCHalted is 1: the reset
//!   ends at the store. It sets USBCMD, USBSTS and USBINTR to their
//!   initial values, and the setup registers to unknown values;
//! - a store to a setup register of the word that a row of
//!   [`EHCI_SETUP_STORES`] at the controller's firmware gives for it.
//!   The setup register then holds that word. The run declares the store
//!   instruction of each such row as a writer of the register. It
//!   declares no reader of a setup register, so each load of one stops
//!   the run.

use cellgov_mem::{
    GuestMemory, MemError, RegisterPassBack, RegisterRule, RegisterStore, RegisterWord,
};
use cellgov_ps3_abi::hw::ehci::{
    EhciSetupStore, EHCI_CONTROLLERS, EHCI_RESET_READERS, EHCI_SETUP_STORES, INSNREG01, INSNREG03,
    USBCMD, USBCMD_HCRESET, USBCMD_PARK_BITS, USBCMD_RESET_VALUE, USBCMD_RS, USBINTR,
    USBINTR_RESET_VALUE, USBSTS, USBSTS_HCHALTED, USBSTS_RESET_VALUE,
};

use super::mmio_real::{DeviceMmioError, MmioTables};

#[cfg(test)]
#[path = "tests/ehci_firmware_rule_tests.rs"]
mod firmware_rule_tests;

/// The index of each register in a controller's register file.
const CMD: usize = 0;
const STS: usize = 1;
const INTR: usize = 2;
const INSN01: usize = 3;
const INSN03: usize = 4;

/// The number of words in a controller's register file.
const WORDS: usize = 5;

/// The offset of each register of the file from the operational base, in
/// the order of the indexes.
const REGISTERS: [u64; WORDS] = [USBCMD, USBSTS, USBINTR, INSNREG01, INSNREG03];

/// The rule of each controller of [`EHCI_CONTROLLERS`], in the order of
/// the table. The array has the length of the table, so a new controller
/// row does not compile without its own rule.
const RULES: [RegisterRule; EHCI_CONTROLLERS.len()] = [store_operational::<0>];

/// The real address of the operational registers of each EHCI controller
/// of `tables` at `firmware`, with the rule of that controller.
///
/// Each controller at `firmware` gets the rule of `firmware`
/// ([`rule_of`]).
///
/// # Errors
///
/// Returns one of these for the first controller that fails:
/// - [`DeviceMmioError::NoEhciRule`] when `firmware` has no rule;
/// - [`DeviceMmioError::NoRealAddress`] when the controller's region has
///   no real address.
pub(super) fn controllers_of(
    tables: MmioTables<'_>,
    firmware: &str,
) -> Result<Vec<(u64, RegisterRule)>, DeviceMmioError> {
    let rule = rule_of(firmware);
    tables
        .controllers
        .iter()
        .filter(|c| c.firmware == firmware)
        .map(|c| {
            let rule = rule.ok_or_else(|| DeviceMmioError::NoEhciRule {
                firmware: firmware.to_owned(),
                bus: c.bus,
                device: c.device,
            })?;
            let real = tables.real(firmware, c.bus, c.device, c.region)?;
            Ok((real + c.operational, rule))
        })
        .collect()
}

/// The rule of an EHCI controller at `firmware`, or `None` when no
/// controller of [`EHCI_CONTROLLERS`] is at `firmware`.
///
/// The rule is the entry of [`RULES`] at the index of the first such
/// controller. That rule reads the firmware of that controller, which is
/// `firmware`.
fn rule_of(firmware: &str) -> Option<RegisterRule> {
    EHCI_CONTROLLERS
        .iter()
        .zip(RULES)
        .find_map(|(c, rule)| (c.firmware == firmware).then_some(rule))
}

/// Declares the registers of each controller of `controllers` as a
/// register file of `mem`.
///
/// Each entry of `controllers` holds the real address of one controller's
/// operational registers and the rule of that controller. The function
/// also declares these:
/// - the readers of [`EHCI_RESET_READERS`];
/// - the store instruction of each row of [`EHCI_SETUP_STORES`] at
///   `firmware`, as a writer of its setup register.
///
/// # Errors
///
/// Returns the refusal of [`GuestMemory::declare_registers`],
/// [`GuestMemory::declare_register_reader`] or
/// [`GuestMemory::declare_register_writer`]. The memory refuses a
/// register outside the controller's unmodelled span.
pub(super) fn declare_ehci(
    mem: &mut GuestMemory,
    firmware: &str,
    controllers: &[(u64, RegisterRule)],
) -> Result<(), MemError> {
    for &(base, rule) in controllers {
        let words = REGISTERS.map(|offset| (base + offset, RegisterWord::UNKNOWN));
        mem.declare_registers(&words, rule)?;
        let reads = EHCI_RESET_READERS.iter().filter(|r| r.firmware == firmware);
        for read in reads {
            let real = base + read.register;
            let pass_back = read.writer.map(|store| RegisterPassBack {
                store,
                bits: read.passes,
            });
            mem.declare_register_reader(real, read.reader, read.kept, pass_back)?;
        }
        let stores = EHCI_SETUP_STORES.iter().filter(|s| s.firmware == firmware);
        for store in stores {
            mem.declare_register_writer(base + store.register, store.store)?;
        }
    }
    Ok(())
}

/// The rule of the register file of the controller at index `C` of
/// [`EHCI_CONTROLLERS`]. A setup register takes only the words of the
/// [`EHCI_SETUP_STORES`] rows at that controller's firmware.
fn store_operational<const C: usize>(
    words: &mut [RegisterWord],
    index: usize,
    store: RegisterStore,
) -> bool {
    let firmware = const { EHCI_CONTROLLERS[C].firmware };
    apply_store(firmware, EHCI_SETUP_STORES, words, index, store)
}

/// Applies `store` to the word at `index` of a controller at `firmware`,
/// whose setup registers take only the words of the `setup` rows at
/// `firmware`. The file holds the words of [`REGISTERS`]; the rule
/// refuses every store to a file of another length.
fn apply_store(
    firmware: &str,
    setup: &[EhciSetupStore],
    words: &mut [RegisterWord],
    index: usize,
    store: RegisterStore,
) -> bool {
    let Ok(words) = <&mut [RegisterWord; WORDS]>::try_from(words) else {
        return false;
    };
    match index {
        CMD => store_usbcmd(words, store),
        INTR => store_usbintr(words, store),
        INSN01 | INSN03 => store_setup(firmware, setup, words, index, store),
        _ => false,
    }
}

/// A store to a setup register: only a word that a `setup` row at
/// `firmware` stores to that register, set by the code itself. A
/// firmware with no such row has no setup word, so the rule refuses each
/// store. No source gives what the word does to the controller, so the
/// rule changes no other word.
fn store_setup(
    firmware: &str,
    setup: &[EhciSetupStore],
    words: &mut [RegisterWord; WORDS],
    index: usize,
    store: RegisterStore,
) -> bool {
    let sourced = setup.iter().any(|s| {
        s.firmware == firmware && s.register == REGISTERS[index] && s.value == store.value
    });
    if store.own != u32::MAX || !sourced {
        return false;
    }
    words[index] = RegisterWord::known(store.value);
    true
}

/// A store to USBINTR: only a store of 0 that the code sets itself.
///
/// [EHCI p:22 s:2.3.3] A 1 in an enable bit enables an interrupt; the default value is 0.
fn store_usbintr(words: &mut [RegisterWord; WORDS], store: RegisterStore) -> bool {
    if store.own != u32::MAX || store.value != USBINTR_RESET_VALUE {
        return false;
    }
    words[INTR] = RegisterWord::known(USBINTR_RESET_VALUE);
    true
}

/// A store to USBCMD.
///
/// A store may also hold bits that the code stores back unchanged. Those
/// bits keep their state. Only the store that a reader names as the store
/// of its loaded word passes a bit back; each 0 of any other store is the
/// code's own. The rule refuses a store when one of its own bits outside
/// Run/Stop and HCRESET is one of these in USBCMD:
/// - unknown;
/// - of another value.
///
/// [EHCI p:20 s:2.3.1] The controller halts within 16 micro-frames after Run/Stop clears.
/// The model has no schedule, so no transaction is in flight, and it
/// halts at once. The specification gives the reset no minimum time, so
/// HCRESET reads 0 at the first load after the store.
/// [EHCI p:20 s:2.3.1] Software should not set HCRESET while HCHalted is 0, as the result is undefined, and a write of 0 to HCRESET does not end a reset early.
fn store_usbcmd(words: &mut [RegisterWord; WORDS], store: RegisterStore) -> bool {
    let cmd = words[CMD];
    let unmodelled = store.own & !(USBCMD_RS | USBCMD_HCRESET);
    if unmodelled & !cmd.known != 0 || (store.value ^ cmd.value) & unmodelled != 0 {
        return false;
    }
    let own = |bit: u32| store.own & bit != 0;
    let set = |bit: u32| store.value & bit != 0;
    if own(USBCMD_RS) && set(USBCMD_RS) {
        return false;
    }
    if own(USBCMD_HCRESET) && set(USBCMD_HCRESET) {
        let status = words[STS];
        if status.known & status.value & USBSTS_HCHALTED == 0 {
            return false;
        }
        words[CMD] = RegisterWord {
            value: USBCMD_RESET_VALUE,
            known: !USBCMD_PARK_BITS,
        };
        words[STS] = RegisterWord::known(USBSTS_RESET_VALUE);
        words[INTR] = RegisterWord::known(USBINTR_RESET_VALUE);
        // Linux `drivers/usb/host/ehci-ps3.c`, `ps3_ehci_setup_insnreg`,
        // says that a host controller reset returns the setup registers
        // to their chip defaults. No source gives those defaults.
        words[INSN01] = RegisterWord::UNKNOWN;
        words[INSN03] = RegisterWord::UNKNOWN;
        return true;
    }
    if own(USBCMD_RS) {
        words[CMD] = RegisterWord {
            value: cmd.value & !USBCMD_RS,
            known: cmd.known | USBCMD_RS,
        };
        let status = words[STS];
        words[STS] = RegisterWord {
            value: status.value | USBSTS_HCHALTED,
            known: status.known | USBSTS_HCHALTED,
        };
    }
    true
}
