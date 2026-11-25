//! The event that a partition sends on one of its event receive ports:
//! - the handler of `send_event_locally` (hypercall 24);
//! - the objects that it accepts, and the r3 of a failed lookup;
//! - what lv1 does to the port, and to the thread whose plug holds it.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and read
//! with `cellgov dev disasm`. The repository holds no byte of lv1, only
//! these facts.
//!
//! The handler [OFW-4.93 lv1 a:0x2EF530] reads one input, r3, the id of
//! an event receive port. It gives r3 to the send function
//! [OFW-4.93 lv1 a:0x2E269C] [OFW-4.93 lv1 a:0x2EF5B0-0x2EF5B4], and
//! returns the r3 of that function [OFW-4.93 lv1 a:0x2EF620]. It writes
//! no return slot, and the vector's entry zeroes the slots
//! [OFW-4.93 lv1 a:0x0C08]. So r4 to r10 return 0.
//!
//! # The lookup
//!
//! The send function looks up r3 in the partition's object table
//! [OFW-4.93 lv1 a:0x2E26BC-0x2E27C8]. It returns
//! [`SEND_EVENT_NO_ENTRY_R3`] when one of these checks fails:
//! - The table holds an object at r3.
//! - The object's class word at +48 is the class in the TOC slot at
//!   r2 - 7888 [OFW-4.93 lv1 a:0x2E27CC-0x2E27D8]. That slot holds the
//!   class that the port constructor gives its objects, from the slot at
//!   r2 - 8032 [OFW-4.93 lv1 a:0x358168] [OFW-4.93 lv1 a:0x3580D8].
//! - The object's doubleword at +136 is the word in the TOC slot at
//!   r2 - 7896 [OFW-4.93 lv1 a:0x2E27DC-0x2E27E8]. The function of
//!   `construct_event_receive_port` gives that word to the port
//!   constructor [OFW-4.93 lv1 a:0x2E2538], which stores it at +136
//!   [OFW-4.93 lv1 a:0x2DF664].
//! - The port's byte at +148 is not 0. The function reads the byte under
//!   the port's lock at +88 [OFW-4.93 lv1 a:0x2E2868-0x2E28CC].
//!
//! So a port of `construct_event_receive_port` passes each check; see
//! [`super::event_port`]. An irq outlet holds another word at +136, so it
//! fails; see [`super::spe`] and [`super::uart`]. A logical SPE fails the
//! class check: the SPE constructor gives the object constructor the
//! class in the TOC slot at r2 - 5008 [OFW-4.93 lv1 a:0x2FA634]
//! [OFW-4.93 lv1 a:0x2CC6A8], and that slot holds another class
//! [OFW-4.93 lv1 a:0x358CA8].
//!
//! # The event
//!
//! The send function gives the port to the event function
//! [OFW-4.93 lv1 a:0x2DF808] [OFW-4.93 lv1 a:0x2E28D8]. The event
//! function takes the port's lock and does these steps:
//! 1. It returns the status 0x96095A03 for a port whose byte at +148 is 0
//!    [OFW-4.93 lv1 a:0x2DF850-0x2DF860]. The send function maps that
//!    status to [`SEND_EVENT_NO_ENTRY_R3`] [OFW-4.93 lv1 a:0x2E28EC-0x2E28F8].
//! 2. It returns 0, and changes nothing, when the pending byte at +149 is
//!    set [OFW-4.93 lv1 a:0x2DF864-0x2DF86C] [OFW-4.93 lv1 a:0x2DF928].
//! 3. It sets the pending byte [OFW-4.93 lv1 a:0x2DF870-0x2DF87C].
//! 4. It returns 0 for a port with no connection at +128
//!    [OFW-4.93 lv1 a:0x2DF880].
//! 5. For a connected port, it takes the lock that +0 of the plug table
//!    points to. It gives the raise function [OFW-4.93 lv1 a:0x2C65B8]
//!    the thread's lv1 context at +8 of the table and the plug at +144 of
//!    the port [OFW-4.93 lv1 a:0x2DF884-0x2DF8C8]. It then returns 0
//!    [OFW-4.93 lv1 a:0x2DF8F0].
//!
//! The send function returns 0 for a status of 0
//! [OFW-4.93 lv1 a:0x2E28DC-0x2E28E8].
//!
//! # The raise
//!
//! The raise function takes the status doubleword and the bit of the
//! plug, as [`super::irq_plug::plug_status_bit`] gives them
//! [OFW-4.93 lv1 a:0x2C65C8-0x2C65F4]. It does these steps:
//! 1. It returns 1, and changes nothing, when the bit is set in the status
//!    shadow at +1696 of the context [OFW-4.93 lv1 a:0x2C65F8-0x2C661C].
//! 2. It sets the bit in the shadow, and stores the shadow doubleword in
//!    the status doubleword that +1728 names
//!    [OFW-4.93 lv1 a:0x2C6620-0x2C6634]. That doubleword is in the
//!    thread's registered bitmap; see [`super::interrupt`].
//! 3. When the pause flag at +1825 is set, it reads the mask doubleword
//!    that +1736 names [OFW-4.93 lv1 a:0x2C6628] [OFW-4.93 lv1 a:0x2C6680-0x2C668C].
//!    For a set mask bit of the plug, it clears the pause flag and runs
//!    the wake list at +1816 [OFW-4.93 lv1 a:0x2C6694-0x2C66DC]. That is
//!    the wake of [`super::pause`].
//! 4. For the context of a thread other than the caller, it sends that
//!    thread an interprocessor interrupt with the function
//!    [OFW-4.93 lv1 a:0x2C2D24] [OFW-4.93 lv1 a:0x2C663C-0x2C6650]
//!    [OFW-4.93 lv1 a:0x2C6720-0x2C672C]; see [`super::interrupt_mask`].
//! 5. It returns 0 [OFW-4.93 lv1 a:0x2C6654-0x2C6658].
//!
//! On each hypercall's return, lv1's vector raises the external interrupt
//! for a bit that is set in both the caller's status shadow and its mask;
//! see [`super::interrupt_mask`].
//!
//! # The kernel's call
//!
//! The kernel's call [OFW-4.93 lv2_kernel a:0x8000000000025a34] passes
//! the doubleword at +0x38 of a hardware thread's record in r3
//! [OFW-4.93 lv2_kernel a:0x8000000000025a28-0x8000000000025a30]. That is
//! the port of the record; see [`super::event_port::RECORD_PORT_CAPTURE`].

/// The handler of hypercall 24 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendEventHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `send_event_locally` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 24 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const SEND_EVENT_HANDLERS: &[SendEventHandler] = &[SendEventHandler {
    firmware: "4.93",
    handler: 0x2EF530,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn send_event_handler(firmware: &str) -> Option<&'static SendEventHandler> {
    SEND_EVENT_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The r3 of `send_event_locally` for an r3 that names no event receive
/// port of the partition: `LV1_NO_ENTRY` (-6), with r3 marked in the high
/// word [OFW-4.93 lv1 a:0x2E27EC-0x2E27F8].
///
/// As with `allocate_memory`, bit 32 marks r3.
pub const SEND_EVENT_NO_ENTRY_R3: u64 = 0x0000_0001_FFFF_FFFA;

#[cfg(test)]
#[path = "tests/send_event_tests.rs"]
mod tests;
