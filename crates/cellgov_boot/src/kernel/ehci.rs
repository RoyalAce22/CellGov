//! The operational registers of the EHCI host controllers in a kernel
//! run, and the rule that applies a store to them.
//!
//! The run models these registers of each controller of
//! [`EHCI_CONTROLLERS`] as register words:
//! - USBCMD, USBSTS and USBINTR;
//! - PERIODICLISTBASE and ASYNCLISTADDR;
//! - the PS3 setup registers `insnreg01`, `insnreg02` and `insnreg03`.
//!
//! No source gives their state when the kernel starts, so each word
//! starts with no known bit. The rule accepts only these stores, and a
//! refused store stops the run:
//! - a store of 0 to USBINTR, which disables every interrupt;
//! - a store to USBCMD that clears Run/Stop: the controller halts at the
//!   store, so USBSTS gets HCHalted = 1;
//! - a store to USBCMD that sets HCRESET while HCHalted is 1: the reset
//!   ends at the store. It sets USBCMD, USBSTS and USBINTR to their
//!   initial values. It sets PERIODICLISTBASE, ASYNCLISTADDR and the
//!   setup registers to unknown values;
//! - a store to PERIODICLISTBASE by the instruction of a row of
//!   [`EHCI_ADDRESS_STORES`], while the periodic schedule is known to be
//!   disabled. The register then holds bits 31:12 of the stored word;
//! - a store to ASYNCLISTADDR by the instruction of a row of
//!   [`EHCI_ADDRESS_STORES`], while the asynchronous schedule is known
//!   to be disabled. The register then holds the stored word;
//! - a store to a setup register of the word that a row of
//!   [`EHCI_SETUP_STORES`] at the controller's firmware gives for it.
//!   The setup register then holds that word. The run declares the store
//!   instruction of each such row as a writer of the register;
//! - a store to a setup register that writes back the word that a row of
//!   [`EHCI_SETUP_BIT_SETS`] loads, with the row's bits set. This rule
//!   rests on an assumption that no source supports.
//!
//! The run declares the load of each [`EHCI_SETUP_BIT_SETS`] row as a
//! reader that keeps no bit. Each other load of a setup register stops
//! the run.

use cellgov_mem::{
    GuestMemory, MemError, RegisterPassBack, RegisterRule, RegisterStore, RegisterWord,
};
use cellgov_ps3_abi::hw::ehci::{
    EhciSetupBitSet, EhciSetupStore, ASYNCLISTADDR, ASYNCLISTADDR_ZERO_BITS, EHCI_ADDRESS_STORES,
    EHCI_CONTROLLERS, EHCI_RESET_READERS, EHCI_SETUP_BIT_SETS, EHCI_SETUP_STORES, INSNREG01,
    INSNREG02, INSNREG03, PERIODICLISTBASE, PERIODICLISTBASE_RESERVED_BITS, USBCMD, USBCMD_ASE,
    USBCMD_HCRESET, USBCMD_PARK_BITS, USBCMD_PSE, USBCMD_RESET_VALUE, USBCMD_RS, USBINTR,
    USBINTR_RESET_VALUE, USBSTS, USBSTS_ASS, USBSTS_HCHALTED, USBSTS_PSS, USBSTS_RESET_VALUE,
};

use super::mmio_real::{DeviceMmioError, MmioTables};

#[cfg(test)]
#[path = "tests/ehci_firmware_rule_tests.rs"]
mod firmware_rule_tests;

#[cfg(test)]
#[path = "tests/ehci_bit_set_rule_tests.rs"]
mod bit_set_rule_tests;

#[cfg(test)]
#[path = "tests/ehci_async_list_rule_tests.rs"]
mod async_list_rule_tests;

#[cfg(test)]
#[path = "tests/ehci_periodic_list_rule_tests.rs"]
mod periodic_list_rule_tests;

/// The index of each register in a controller's register file.
const CMD: usize = 0;
const STS: usize = 1;
const INTR: usize = 2;
const INSN01: usize = 3;
const INSN03: usize = 4;
const INSN02: usize = 5;
const ASYNC: usize = 6;
const PERIODIC: usize = 7;

/// The number of words in a controller's register file.
const WORDS: usize = 8;

/// The offset of each register of the file from the operational base, in
/// the order of the indexes.
const REGISTERS: [u64; WORDS] = [
    USBCMD,
    USBSTS,
    USBINTR,
    INSNREG01,
    INSNREG03,
    INSNREG02,
    ASYNCLISTADDR,
    PERIODICLISTBASE,
];

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
///   `firmware`, as a writer of its setup register;
/// - for each row of [`EHCI_SETUP_BIT_SETS`] at `firmware`, its load as
///   a reader that keeps no bit, and its store as the writer that passes
///   back each bit outside the row's bits;
/// - the store instruction of each row of [`EHCI_ADDRESS_STORES`] at
///   `firmware`, as a writer of its register.
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
        let sets = EHCI_SETUP_BIT_SETS
            .iter()
            .filter(|s| s.firmware == firmware);
        for set in sets {
            let real = base + set.register;
            let pass_back = RegisterPassBack {
                store: set.store,
                bits: !set.bits,
            };
            mem.declare_register_reader(real, set.reader, 0, Some(pass_back))?;
            mem.declare_register_writer(real, set.store)?;
        }
        let addresses = EHCI_ADDRESS_STORES
            .iter()
            .filter(|s| s.firmware == firmware);
        for address in addresses {
            mem.declare_register_writer(base + address.register, address.store)?;
        }
    }
    Ok(())
}

/// The rule of the register file of the controller at index `C` of
/// [`EHCI_CONTROLLERS`]. A setup register takes only the stores of the
/// [`EHCI_SETUP_STORES`] and [`EHCI_SETUP_BIT_SETS`] rows at that
/// controller's firmware.
fn store_operational<const C: usize>(
    words: &mut [RegisterWord],
    index: usize,
    store: RegisterStore,
) -> bool {
    let firmware = const { EHCI_CONTROLLERS[C].firmware };
    apply_store(
        firmware,
        EHCI_SETUP_STORES,
        EHCI_SETUP_BIT_SETS,
        words,
        index,
        store,
    )
}

/// Applies `store` to the word at `index` of a controller at `firmware`.
/// Its setup registers take only the stores of the `setup` and
/// `bit_sets` rows at `firmware`. The file holds the words of
/// [`REGISTERS`]; the rule refuses every store to a file of another
/// length.
fn apply_store(
    firmware: &str,
    setup: &[EhciSetupStore],
    bit_sets: &[EhciSetupBitSet],
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
        INSN02 => store_setup_bits(firmware, bit_sets, words, index, store),
        ASYNC => store_async_list(words, store),
        PERIODIC => store_periodic_list(words, store),
        _ => false,
    }
}

/// Returns true when `bit` of `word` is known and 0.
fn known_clear(word: RegisterWord, bit: u32) -> bool {
    word.known & bit != 0 && word.value & bit == 0
}

/// A store to ASYNCLISTADDR: only while USBCMD holds a known 0
/// Asynchronous Schedule Enable bit and USBSTS a known 0 Asynchronous
/// Schedule Status bit.
///
/// The register then holds the stored word with bits 4:0 at 0. The
/// controller does not read the schedule while it is disabled. So the
/// store changes no other word, and the run starts no transfer. The rule
/// refuses a store with a bit that is not the code's own, because the
/// register would then hold an unknown bit as known.
///
/// [EHCI p:71 s:4.8] Software may write this register with defined results only when the schedule is disabled: Asynchronous Schedule Enable and Asynchronous Schedule Status are zero.
/// [EHCI p:71 s:4.8] When Asynchronous Schedule Enable is zero, the host controller does not access the asynchronous schedule through ASYNCLISTADDR.
fn store_async_list(words: &mut [RegisterWord; WORDS], store: RegisterStore) -> bool {
    if store.own != u32::MAX
        || !known_clear(words[CMD], USBCMD_ASE)
        || !known_clear(words[STS], USBSTS_ASS)
    {
        return false;
    }
    words[ASYNC] = RegisterWord::known(store.value & !ASYNCLISTADDR_ZERO_BITS);
    true
}

/// A store to PERIODICLISTBASE: only while USBCMD holds a known 0
/// Periodic Schedule Enable bit and USBSTS a known 0 Periodic Schedule
/// Status bit.
///
/// The rule also refuses these stores:
/// - a store with a bit that is not the code's own;
/// - a store with a bit of 11:0 set, which software must write as 0.
///
/// The register then holds bits 31:12 of the stored word. Bits 11:0 stay
/// unknown, because the specification gives no value for them. The
/// controller does not read the frame list while the schedule is
/// disabled. So the store changes no other word, and the run starts no
/// transfer. The specification does not limit the stores to this
/// register to a disabled schedule, as it does for ASYNCLISTADDR. The
/// rule refuses a store while the schedule can run, because the run does
/// not model a controller that reads the frame list.
///
/// [EHCI p:25 s:2.3.6] Software loads this register before the host controller starts the schedule. Bits 11:0 must be written as 0s, and during runtime their values are undefined.
/// [EHCI p:66 s:4.6] When Periodic Schedule Enable is zero, the host controller does not access the periodic frame list through PERIODICLISTBASE.
/// [EHCI p:21 s:2.3.2] When Periodic Schedule Status and Periodic Schedule Enable have the same value, the periodic schedule is enabled (1) or disabled (0).
fn store_periodic_list(words: &mut [RegisterWord; WORDS], store: RegisterStore) -> bool {
    if store.own != u32::MAX
        || store.value & PERIODICLISTBASE_RESERVED_BITS != 0
        || !known_clear(words[CMD], USBCMD_PSE)
        || !known_clear(words[STS], USBSTS_PSS)
    {
        return false;
    }
    words[PERIODIC] = RegisterWord {
        value: store.value,
        known: !PERIODICLISTBASE_RESERVED_BITS,
    };
    true
}

/// A store to a setup register that writes the word back with the bits
/// of a `bit_sets` row at `firmware` set.
///
/// The rule accepts the store when all of these hold:
/// - the store sets each bit of the row;
/// - each other bit that the code sets itself is known, and the store
///   does not change it.
///
/// Each other bit is a bit that the store passes back. It keeps its
/// state, so an unknown bit stays unknown, and the rule invents no value
/// for it. The rule assumes that the store changes nothing else. No
/// source gives the effect of the row's bits on the controller. The run
/// models no such effect, so no other word and no transfer changes.
fn store_setup_bits(
    firmware: &str,
    bit_sets: &[EhciSetupBitSet],
    words: &mut [RegisterWord; WORDS],
    index: usize,
    store: RegisterStore,
) -> bool {
    let Some(set) = bit_sets
        .iter()
        .find(|s| s.firmware == firmware && s.register == REGISTERS[index])
    else {
        return false;
    };
    let word = words[index];
    let others = store.own & !set.bits;
    let sets_bits = store.own & store.value & set.bits == set.bits;
    if !sets_bits || others & !word.known != 0 || (store.value ^ word.value) & others != 0 {
        return false;
    }
    words[index] = RegisterWord {
        value: word.value | set.bits,
        known: word.known | set.bits,
    };
    true
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
        // [EHCI p:71 s:4.8] The default value of ASYNCLISTADDR after reset is undefined.
        words[ASYNC] = RegisterWord::UNKNOWN;
        // [EHCI p:53 s:4.1] Table 4-1 gives PERIODICLISTBASE an undefined value after reset.
        words[PERIODIC] = RegisterWord::UNKNOWN;
        // Linux `drivers/usb/host/ehci-ps3.c`, `ps3_ehci_setup_insnreg`,
        // says that a host controller reset returns the setup registers
        // to their chip defaults. No source gives those defaults.
        words[INSN01] = RegisterWord::UNKNOWN;
        words[INSN02] = RegisterWord::UNKNOWN;
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
