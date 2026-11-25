//! The pause of a logical PPE thread:
//! - the handler of `pause` (hypercall 9);
//! - its mode check and the r3 of a failed check;
//! - when lv1 pauses the calling thread, and what wakes it.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and read
//! with `cellgov dev disasm`. The repository holds no byte of lv1, only
//! these facts.
//!
//! The handler [OFW-4.93 lv1 a:0x2EB618] reads one input, r3, the mode:
//! - [`PAUSE_MODE_DECREMENTER_WAKES`] lets a decrementer interrupt wake
//!   the thread;
//! - [`PAUSE_MODE_DECREMENTER_IGNORED`] does not.
//!
//! The handler returns the r3 that it builds in r31
//! [OFW-4.93 lv1 a:0x2EB76C]. It writes no return slot, and the vector's
//! entry zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r4 to r10 return 0.
//!
//! # The pause
//!
//! For a mode that passes the check, the handler does these steps:
//! 1. It stores the wake flag at +1829 of the caller's lv1 context: 1 for
//!    mode 0, else 0 [OFW-4.93 lv1 a:0x2EB6DC-0x2EB700].
//! 2. It calls the pause step with that context [OFW-4.93 lv1 a:0x2EB704].
//! 3. It returns 0 [OFW-4.93 lv1 a:0x2EB6F8].
//!
//! The pause step [OFW-4.93 lv1 a:0x2C68C4] takes the lock of the PPE
//! [OFW-4.93 lv1 a:0x2C68D4-0x2C6920]. It ends with no pause in these cases:
//! - The byte at +1830 and the wake flag are both nonzero
//!   [OFW-4.93 lv1 a:0x2C6924-0x2C692C] [OFW-4.93 lv1 a:0x2C6A5C-0x2C6A64].
//!   lv1 sets the byte at +1830 when it finds the thread's decrementer
//!   negative [OFW-4.93 lv1 a:0x2C632C-0x2C6374]. It clears the byte as it
//!   returns to the thread with the decrementer at 0
//!   [OFW-4.93 lv1 a:0x2C425C-0x2C4280]. So a mode 0 pause ends at once
//!   while a decrementer interrupt of the thread is pending.
//! - A status shadow doubleword at +1696, ANDed with the mask doubleword
//!   that +1736 names, is nonzero [OFW-4.93 lv1 a:0x2C6930-0x2C695C]. That
//!   is the test of lv1's return path for the external interrupt; see
//!   [`super::interrupt_mask`].
//!
//! Otherwise, the step pauses the thread:
//! 1. It sets the pause flag, the byte at +1825, to 1
//!    [OFW-4.93 lv1 a:0x2C6960-0x2C696C].
//! 2. It runs the pause list at +1816 when the flag was 0
//!    [OFW-4.93 lv1 a:0x2C6970-0x2C69CC].
//! 3. It releases the lock [OFW-4.93 lv1 a:0x2C69D0-0x2C69F0].
//!
//! # The wake
//!
//! The wake check [OFW-4.93 lv1 a:0x2C6AEC] clears the pause flag when a
//! status shadow doubleword, ANDed with its mask doubleword, is nonzero
//! [OFW-4.93 lv1 a:0x2C6B40-0x2C6B84]. So an interrupt on an unmasked plug
//! wakes the thread.
//!
//! A suspended PPE thread also resumes on a decrementer interrupt when
//! TSCR enables that wake for the thread [CBE-Handbook p:321 s:10.8.2.1].
//! The wake flag selects this wake for mode 0, as Linux's
//! `ps3_power_save` describes the modes.
//!
//! # The kernel's call
//!
//! The 4.93 kernel's idle thread calls `pause` with r3 = 0 in a loop
//! [OFW-4.93 lv2_kernel a:0x800000000026517C-0x8000000000265188].

/// The handler of hypercall 9 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PauseHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `pause` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 9 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const PAUSE_HANDLERS: &[PauseHandler] = &[PauseHandler {
    firmware: "4.93",
    handler: 0x2EB618,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn pause_handler(firmware: &str) -> Option<&'static PauseHandler> {
    PAUSE_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The mode in which a decrementer interrupt wakes the paused thread.
pub const PAUSE_MODE_DECREMENTER_WAKES: u64 = 0;

/// The mode in which a decrementer interrupt does not wake the paused
/// thread.
pub const PAUSE_MODE_DECREMENTER_IGNORED: u64 = 1;

/// The r3 of `pause` for a mode other than 0 and 1:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17), with r3 marked in the high word
/// [OFW-4.93 lv1 a:0x2EB784-0x2EB794].
///
/// The handler accepts r3 only when r3 equals its own bit 63
/// [OFW-4.93 lv1 a:0x2EB6B4-0x2EB6BC]. As with `allocate_memory`, bit 32
/// marks r3.
pub const PAUSE_MODE_R3: u64 = 0x0000_0001_FFFF_FFEF;

#[cfg(test)]
#[path = "tests/pause_tests.rs"]
mod tests;
