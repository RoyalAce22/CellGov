//! The event receive port of a partition:
//! - the handler of `construct_event_receive_port` (hypercall 18);
//! - the port object that lv1 constructs and the id it returns;
//! - the port fields that later interrupt calls read;
//! - the port id of each call of the GameOS kernel's boot.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and read
//! with `cellgov dev disasm`. No byte of lv1 is committed; these facts
//! are.
//!
//! The handler [OFW-4.93 lv1 a:0x2EF424] reads no input register. It
//! gives the function [OFW-4.93 lv1 a:0x2E24E8] the address of the
//! return slot of r4, HSPRG0 - 0x6A38 [OFW-4.93 lv1 a:0x2EF4A0-0x2EF4AC],
//! and returns the r3 of that function [OFW-4.93 lv1 a:0x2EF518]. The
//! function does these steps:
//! 1. It allocates a 168-byte object [OFW-4.93 lv1 a:0x2E250C-0x2E2530].
//! 2. It constructs the object as a port with the calling partition
//!    [OFW-4.93 lv1 a:0x2E2540]. See the list of port fields below.
//! 3. It puts the port in the slot of the partition's object table that
//!    the port id names [OFW-4.93 lv1 a:0x2E2610-0x2E2618].
//! 4. It stores the port id in the return slot of r4, and returns 0
//!    [OFW-4.93 lv1 a:0x2E2628-0x2E2634].
//!
//! The handler writes no other return slot, and the vector's entry
//! zeroes the slots [OFW-4.93 lv1 a:0x0C08]. So r5 to r10 return 0.
//!
//! The function returns `LV1_RESOURCE_SHORTAGE` (-2), zero-extended, in
//! these cases [OFW-4.93 lv1 a:0x2E2668-0x2E2684]:
//! - lv1 cannot allocate the object;
//! - the constructor returns a nonzero status, for example when the
//!   object table holds no free id.
//!
//! The port id is the port's id in the partition's object table. The
//! port constructor [OFW-4.93 lv1 a:0x2DF60C] calls the object
//! constructor [OFW-4.93 lv1 a:0x2CC64C] with the partition
//! [OFW-4.93 lv1 a:0x2DF644]. The object constructor gives the table's
//! allocator [OFW-4.93 lv1 a:0x2CA258] the address of +72
//! [OFW-4.93 lv1 a:0x2CC6DC-0x2CC6EC]. The allocator takes the first free
//! id of the table [OFW-4.93 lv1 a:0x2CA3DC-0x2CA42C]. The logical PPE
//! and the virtual address spaces take their ids from the same table;
//! see [`super::ppe`] and [`super::address_space`].
//!
//! The port constructor sets these port fields:
//! - +120 holds the partition [OFW-4.93 lv1 a:0x2DF670].
//! - +128 holds the port's connection, 0 for no connection
//!   [OFW-4.93 lv1 a:0x2DF688].
//! - +144 holds the bit number of the connected plug, 0 at first
//!   [OFW-4.93 lv1 a:0x2DF678].
//! - +148 is 1 when the object constructor succeeded
//!   [OFW-4.93 lv1 a:0x2DF668-0x2DF694].
//! - +149 is the pending byte, 0 at first [OFW-4.93 lv1 a:0x2DF67C].
//!
//! Later interrupt calls read the port by its id:
//! - `connect_irq_plug_ext` (hypercall 12) looks up its r6 in the object
//!   table and checks the object's class word at +48. A failed lookup
//!   returns `LV1_NO_ENTRY` (-6) with r6 marked
//!   [OFW-4.93 lv1 a:0x2EA16C-0x2EA1A8]. Its connect step sets +128 and
//!   +144; see [`super::irq_plug`].
//! - `send_event_locally` (hypercall 24) gives its r3 to the function
//!   [OFW-4.93 lv1 a:0x2E269C] [OFW-4.93 lv1 a:0x2EF5B4]. That function
//!   looks up r3 the same way, and also checks the word at +136
//!   [OFW-4.93 lv1 a:0x2E27CC-0x2E27F8].
//! - The event function [OFW-4.93 lv1 a:0x2DF808] needs +148 set, and
//!   sets the pending byte [OFW-4.93 lv1 a:0x2DF850-0x2DF87C]. For a
//!   port with a connection, it gives the bit number at +144 to the
//!   function [OFW-4.93 lv1 a:0x2C65B8] [OFW-4.93 lv1 a:0x2DF8BC-0x2DF8C8],
//!   which sets the bit in the thread's interrupt status; see
//!   [`super::interrupt`].

/// The handler of hypercall 18 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventReceivePortHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `construct_event_receive_port` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 18 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const EVENT_RECEIVE_PORT_HANDLERS: &[EventReceivePortHandler] = &[EventReceivePortHandler {
    firmware: "4.93",
    handler: 0x2EF424,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn event_receive_port_handler(firmware: &str) -> Option<&'static EventReceivePortHandler> {
    EVENT_RECEIVE_PORT_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// One `construct_event_receive_port` call of the GameOS kernel, and the
/// port id that lv1 gave it on the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsPortCall {
    /// The hardware thread that makes the call.
    pub thread: u8,
    /// The address of the call's `sc`.
    pub sc: u64,
    /// The port id in r4 of lv1's answer.
    pub id: u64,
}

/// The `construct_event_receive_port` calls of the GameOS kernel's boot at
/// one firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOsPortCalls {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// Each call, in the order of its port id.
    pub calls: &'static [GameOsPortCall],
}

/// The call in the setup of a hardware thread's record
/// [OFW-4.93 lv2_kernel a:0x8000000000007d8c].
const RECORD_SC: u64 = 0x8000_0000_0000_7d8c;
/// Call A of a hardware thread's start
/// [OFW-4.93 lv2_kernel a:0x80000000002656d0].
const START_A_SC: u64 = 0x8000_0000_0026_56d0;
/// Call B of a hardware thread's start
/// [OFW-4.93 lv2_kernel a:0x8000000000265784].
const START_B_SC: u64 = 0x8000_0000_0026_5784;
/// Call C of a hardware thread's start
/// [OFW-4.93 lv2_kernel a:0x8000000000265840].
const START_C_SC: u64 = 0x8000_0000_0026_5840;

const fn call(thread: u8, sc: u64, id: u64) -> GameOsPortCall {
    GameOsPortCall { thread, sc, id }
}

/// The port id of each `construct_event_receive_port` call of the GameOS
/// kernel's boot, for each firmware that a source gives the ids for.
///
/// The partition's other objects decide each id, so no image read gives
/// it. The capture `lv2_event_port_ids` reads the id of each 4.93 call;
/// its README cites the place where the kernel stores each id. On each
/// hardware thread the 4.93 kernel makes these calls:
/// - one call in the setup of the thread's record;
/// - calls A, B and C in the thread's start.
///
/// Thread 0 runs the kernel's subsystem init before its call A
/// [OFW-4.93 lv2_kernel a:0x80000000002656bc]
/// [OFW-4.93 lv2_kernel a:0x80000000002657a8-0x80000000002657ac]. In the
/// capture, thread 1's start calls took their ids before thread 0's.
pub const GAME_OS_PORT_CALLS: &[GameOsPortCalls] = &[GameOsPortCalls {
    firmware: "4.93",
    calls: &[
        call(0, RECORD_SC, 3),
        call(1, RECORD_SC, 4),
        call(1, START_A_SC, 5),
        call(1, START_B_SC, 6),
        call(1, START_C_SC, 7),
        call(0, START_A_SC, 8),
        call(0, START_B_SC, 9),
        call(0, START_C_SC, 10),
    ],
}];

/// The kernel's calls at the firmware version `firmware`, or `None` when
/// no source gives their ids.
pub fn game_os_port_calls(firmware: &str) -> Option<&'static [GameOsPortCall]> {
    GAME_OS_PORT_CALLS
        .iter()
        .find(|calls| calls.firmware == firmware)
        .map(|calls| calls.calls)
}

/// The port id that lv1 gave the GameOS kernel's call at the `sc` address
/// `sc` on the hardware thread `thread`.
///
/// The value is `None` when no source gives that id at the firmware
/// version `firmware`.
pub fn game_os_port_id(firmware: &str, thread: u8, sc: u64) -> Option<u64> {
    game_os_port_calls(firmware)?
        .iter()
        .find(|call| call.thread == thread && call.sc == sc)
        .map(|call| call.id)
}

#[cfg(test)]
#[path = "tests/event_port_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/event_port_console_tests.rs"]
mod console_tests;

#[cfg(test)]
#[path = "tests/event_port_ids_console_tests.rs"]
mod ids_console_tests;
