//! The operational registers of the EHCI host controllers in a kernel
//! run, and the rule that applies a store to them.
//!
//! The run models USBCMD, USBSTS and USBINTR of each controller of
//! [`EHCI_CONTROLLERS`] as register words. No source gives their state
//! when the kernel starts, so each word starts with no known bit. The
//! rule accepts only these stores:
//! - a store of 0 to USBINTR, which disables every interrupt;
//! - a store to USBCMD that clears Run/Stop: the controller halts at the
//!   store, so USBSTS gets HCHalted = 1;
//! - a store to USBCMD that sets HCRESET while HCHalted is 1: the reset
//!   ends at the store and sets each modelled register to its initial
//!   value.
//!
//! A store to USBCMD may also hold bits that the code stores back
//! unchanged. Those bits keep their state. Only the store that a reader
//! names as the store of its loaded word passes a bit back; each 0 of
//! any other store is the code's own. Every other store is refused,
//! and so stops: a set Run/Stop, a USBSTS store, an interrupt enable, and
//! a change to a USBCMD bit that the rule does not model.
//!
//! [EHCI p:20 s:2.3.1] The controller halts within 16 micro-frames after Run/Stop clears.
//! The model has no schedule, so no transaction is in flight, and it
//! halts at once. The specification gives the reset no minimum time, so
//! HCRESET reads 0 at the first load after the store.

use cellgov_mem::{GuestMemory, MemError, RegisterPassBack, RegisterStore, RegisterWord};
use cellgov_ps3_abi::hw::ehci::{
    EHCI_CONTROLLERS, EHCI_RESET_READERS, USBCMD, USBCMD_HCRESET, USBCMD_PARK_BITS,
    USBCMD_RESET_VALUE, USBCMD_RS, USBINTR, USBINTR_RESET_VALUE, USBSTS, USBSTS_HCHALTED,
    USBSTS_RESET_VALUE,
};
use cellgov_ps3_abi::lv1::device_bus::MMIO_REAL_BASE;

/// The index of each register in a controller's register file.
const CMD: usize = 0;
const STS: usize = 1;
const INTR: usize = 2;

/// The offset of each register of the file from the operational base, in
/// the order of the indexes.
const REGISTERS: [u64; 3] = [USBCMD, USBSTS, USBINTR];

/// Declares the operational registers of each EHCI controller at
/// `firmware` as a register file of `mem`, with the readers of
/// [`EHCI_RESET_READERS`].
///
/// # Errors
///
/// Returns the refusal of [`GuestMemory::declare_registers`] or
/// [`GuestMemory::declare_register_reader`]. A register outside the
/// controller's unmodelled span is refused.
pub(super) fn declare_ehci(mem: &mut GuestMemory, firmware: &str) -> Result<(), MemError> {
    let controllers = EHCI_CONTROLLERS.iter().filter(|c| c.firmware == firmware);
    for controller in controllers {
        let base = MMIO_REAL_BASE + controller.region + controller.operational;
        let words = REGISTERS.map(|offset| (base + offset, RegisterWord::UNKNOWN));
        mem.declare_registers(&words, store_operational)?;
        let reads = EHCI_RESET_READERS.iter().filter(|r| r.firmware == firmware);
        for read in reads {
            let real = base + read.register;
            let pass_back = read.writer.map(|store| RegisterPassBack {
                store,
                bits: read.passes,
            });
            mem.declare_register_reader(real, read.reader, read.kept, pass_back)?;
        }
    }
    Ok(())
}

/// The rule of a controller's register file. The file holds the three
/// words of [`REGISTERS`]; the rule refuses every store to a file of
/// another length.
fn store_operational(words: &mut [RegisterWord], index: usize, store: RegisterStore) -> bool {
    let Ok(words) = <&mut [RegisterWord; 3]>::try_from(words) else {
        return false;
    };
    match index {
        CMD => store_usbcmd(words, store),
        INTR => store_usbintr(words, store),
        _ => false,
    }
}

/// A store to USBINTR: only a store of 0 that the code sets itself.
///
/// [EHCI p:22 s:2.3.3] A 1 in an enable bit enables an interrupt; the default value is 0.
fn store_usbintr(words: &mut [RegisterWord; 3], store: RegisterStore) -> bool {
    if store.own != u32::MAX || store.value != USBINTR_RESET_VALUE {
        return false;
    }
    words[INTR] = RegisterWord::known(USBINTR_RESET_VALUE);
    true
}

/// A store to USBCMD.
///
/// [EHCI p:20 s:2.3.1] Software should not set HCRESET while HCHalted is 0, as the result is undefined, and a write of 0 to HCRESET does not end a reset early.
fn store_usbcmd(words: &mut [RegisterWord; 3], store: RegisterStore) -> bool {
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
