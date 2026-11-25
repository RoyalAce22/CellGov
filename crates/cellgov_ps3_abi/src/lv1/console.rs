//! The logical console calls of lv1:
//! - the worker of `lv1_comm_dev_logical_console_destruct` (hypercall
//!   106);
//! - the console gate that the workers of the family read;
//! - the gate on the reference console.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and read
//! with `cellgov dev disasm`. No byte of lv1 is committed; these facts
//! are.
//!
//! Hypercall 106 reaches lv1's memory call thunk [OFW-4.93 lv1 a:0x2C89CC].
//! The console family's array gives 106 its descriptor
//! [OFW-4.93 lv1 a:0x363990-0x36399F], and the descriptor names the
//! worker [OFW-4.93 lv1 a:0x350D48].
//!
//! The worker reads the console id from the thunk's frame, then reads
//! the gate byte [OFW-4.93 lv1 a:0x312FE8-0x313010]. The address of the
//! byte is in the TOC slot [OFW-4.93 lv1 a:0x359C48]. The worker takes
//! one of these paths:
//! - Gate 1: it destructs the console [OFW-4.93 lv1 a:0x3123D8] and puts
//!   that status in r3 [OFW-4.93 lv1 a:0x313040-0x31304C].
//! - Gate 2: it puts 0 in r3 and does nothing else
//!   [OFW-4.93 lv1 a:0x313070-0x313090].
//! - Any other value: it looks up the name in the TOC slot
//!   [OFW-4.93 lv1 a:0x359C50], `sys.dbgcard.dgbe`
//!   [OFW-4.93 lv1 a:0x326F20]. When the lookup leaves its out word at 6,
//!   the worker sets the gate to 2 and takes that path
//!   [OFW-4.93 lv1 a:0x313014-0x31306C]. Otherwise it sets the gate to 1.
//!
//! The workers of 105, 107 and 109 read the same byte and take the same
//! paths. With the gate at 2:
//! - 105 puts 0 in r3 [OFW-4.93 lv1 a:0x312800-0x312814].
//! - 107 puts 0 in r3 and its input r4 in r4
//!   [OFW-4.93 lv1 a:0x312FA8-0x312FB4].
//! - 109 puts 0 in r3 [OFW-4.93 lv1 a:0x313314-0x313320].
//!
//! The thunk copies r4 to r10 from its frame
//! [OFW-4.93 lv1 a:0x2C8B14-0x2C8B34]. The 106 worker writes no word
//! there, so r4 to r10 hold data that lv1 does not set.

/// The hypercall 106 worker in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsoleDestructWorker {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The memory call descriptor of hypercall 106.
    pub descriptor: u32,
    /// The address of the worker that the descriptor names.
    pub worker: u32,
    /// The address of the console gate byte.
    pub gate: u32,
}

/// Each firmware whose lv1 image shows the worker, ascending.
///
/// The descriptor is the entry of hypercall 106 in the live 4.93 memory
/// call table ([`super::table::LIVE_TABLE_CAPTURE`]).
pub const CONSOLE_DESTRUCT_WORKERS: &[ConsoleDestructWorker] = &[ConsoleDestructWorker {
    firmware: "4.93",
    descriptor: 0x350D48,
    worker: 0x312FDC,
    gate: 0x374000,
}];

/// The worker of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn console_destruct_worker(firmware: &str) -> Option<&'static ConsoleDestructWorker> {
    CONSOLE_DESTRUCT_WORKERS
        .iter()
        .find(|worker| worker.firmware == firmware)
}

/// The gate value that makes each console call a no-op: the value the
/// worker stores when the name lookup fails.
pub const CONSOLE_GATE_OFF: u8 = 2;

/// The console capture of the gate byte on the reference console,
/// relative to the workspace root. Its README gives the method.
pub const CONSOLE_GATE_CAPTURE: &str = "tests/micro/lv1_console_gate";

/// Each firmware at which the capture [`CONSOLE_GATE_CAPTURE`] reads the
/// gate as [`CONSOLE_GATE_OFF`], ascending.
///
/// The capture reads the gate at the address that the worker gives, so
/// [`CONSOLE_DESTRUCT_WORKERS`] holds each firmware listed here.
pub const CONSOLE_GATE_OFF_FIRMWARE: &[&str] = &["4.93"];

/// Whether a console capture shows the gate off at the firmware version
/// `firmware`.
pub fn console_gate_off(firmware: &str) -> bool {
    CONSOLE_GATE_OFF_FIRMWARE.contains(&firmware)
}

#[cfg(test)]
#[path = "tests/console_tests.rs"]
mod tests;
