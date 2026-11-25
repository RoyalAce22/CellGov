//! lv1's `storage_send_device_command` (hypercall 248): the statuses of
//! the commands it refuses, and the controller query that it starts on
//! the storage device of type 5.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and read
//! with `cellgov dev disasm`, and from the console captures
//! [`super::storage_registration::REGISTRATION_CAPTURE`] and
//! [`super::device_bus::STORAGE_DEVICE_OPEN_CAPTURE`]. The
//! repository holds no byte of lv1, only these facts. The path of a
//! command:
//! 1. The handler, at [OFW-4.93 lv1 a:0x3094E4] in the live table,
//!    zeroes the return slot of r4, HSPRG0 - 0x6A38. It calls the
//!    command core with the caller's partition, r3 to r8 and the slot's
//!    address [OFW-4.93 lv1 a:0x309590-0x3095CC]. It returns the core's
//!    r3 zero-extended [OFW-4.93 lv1 a:0x309634].
//! 2. The core [OFW-4.93 lv1 a:0x251BE0] returns
//!    [`SEND_COMMAND_NO_DEVICE_STATUS`] for a device id above 6 other
//!    than -1, and for a device whose object pointer at +3816 of the
//!    storage bus object is null [OFW-4.93 lv1 a:0x251BF0-0x251C54]
//!    [OFW-4.93 lv1 a:0x251CD4-0x251CD8]. For device -1 it uses the
//!    storage bus object [OFW-4.93 lv1 a:0x251D68-0x251D6C].
//! 3. The core asks the device's ready method, slot 6 of its virtual
//!    table. A false answer gives [`SEND_COMMAND_NOT_READY_STATUS`]
//!    [OFW-4.93 lv1 a:0x251C58-0x251CA0].
//! 4. Otherwise the core calls slot 34 of the virtual table, and returns
//!    that method's r3 [OFW-4.93 lv1 a:0x251D10-0x251D3C].
//!
//! # The device of type 5
//!
//! The capture [`super::device_bus::STORAGE_DEVICE_OPEN_CAPTURE`]
//! reads the method table 0x35E370 at +0 of device 2's object. In that
//! table:
//! - slot 6 holds the descriptor 0x338778
//!   [OFW-4.93 lv1 a:0x35E3A0-0x35E3A4], whose entry is the ready method
//!   of the storage bus [OFW-4.93 lv1 a:0x338778-0x33877C]. The method
//!   tests the two open slots of the object for the caller's partition;
//!   see [`super::storage`];
//! - slot 34 holds the descriptor 0x334DB8
//!   [OFW-4.93 lv1 a:0x35E480-0x35E484], whose entry is the request
//!   method 0x256CD4 [OFW-4.93 lv1 a:0x334DB8-0x334DBC];
//! - slot 48 holds the descriptor 0x337998
//!   [OFW-4.93 lv1 a:0x35E4F0-0x35E4F4], whose entry is the command
//!   method 0x27302C [OFW-4.93 lv1 a:0x337998-0x33799C].
//!
//! The request method does these steps under the lock at +2672 of the
//! object:
//! 1. It gives the request a tag through the request function of the
//!    storage write [OFW-4.93 lv1 a:0x256D2C-0x256D6C]. That function
//!    keeps one bit for each tag at +2680 of the object. It returns
//!    [`SEND_COMMAND_TAGS_FULL_STATUS`] when each of the
//!    [`SEND_COMMAND_TAGS`] tags is held. Otherwise it stores the lowest
//!    free tag in the return slot of r4 [OFW-4.93 lv1 a:0x25398C-0x253A04].
//!    The method returns the function's failure status
//!    [OFW-4.93 lv1 a:0x256D84-0x256DC8].
//!
//!    Before it gives the tag, for a full r8 other than 0, the function
//!    checks the data buffer [OFW-4.93 lv1 a:0x2539D0-0x2539D4]
//!    [OFW-4.93 lv1 a:0x251D14-0x251D18]. When the list that
//!    [OFW-4.93 lv1 a:0x2284C8] walks for the device does not hold the
//!    address [OFW-4.93 lv1 a:0x253AB8-0x253AE0], it finds the memory
//!    region with [OFW-4.93 lv1 a:0x2CE74C] and returns these statuses:
//!    - `LV1_NO_ENTRY` (-6) for no region, or for a region with too few
//!      bytes [OFW-4.93 lv1 a:0x253B18-0x253B1C]
//!      [OFW-4.93 lv1 a:0x253B34-0x253B48] [OFW-4.93 lv1 a:0x253BC0];
//!    - `LV1_TYPE_MISMATCH` (-8) for a region whose class at +72 is not
//!      the class of the TOC slot 0x354580 [OFW-4.93 lv1 a:0x253B20-0x253B30].
//!      That slot holds the same class as the slot that
//!      [`super::network`] checks [OFW-4.93 lv1 a:0x354580]
//!      [OFW-4.93 lv1 a:0x359920].
//! 2. It calls the command method with the low words of the command id,
//!    the command length and the data length
//!    [OFW-4.93 lv1 a:0x256DCC-0x256E00].
//! 3. When the command method returns 0, the request method clears the
//!    top bit of the return slot and returns 0
//!    [OFW-4.93 lv1 a:0x256E10-0x256E58]. Otherwise it frees the tag and
//!    returns the command method's status, sign-extended
//!    [OFW-4.93 lv1 a:0x256E5C-0x256EF0]. It does not write the return
//!    slot, and the vector loads r4 from the slot
//!    [OFW-4.93 lv1 a:0x1044]. So r4 holds the freed tag.
//!
//! The command method passes the tag and the mode 0 to the command
//! dispatcher 0x272BE4 [OFW-4.93 lv1 a:0x273038-0x273044]. The
//! dispatcher does these steps, and the first failure is the answer:
//! 1. It returns [`SEND_COMMAND_BAD_REQUEST_STATUS`] for a command id
//!    that [`command_accepted`] refuses
//!    [OFW-4.93 lv1 a:0x272C7C-0x272CAC].
//! 2. For a command length other than 0, it translates the command
//!    buffer through the storage write's translation. A failure gives
//!    [`SEND_COMMAND_BAD_REQUEST_STATUS`]
//!    [OFW-4.93 lv1 a:0x272DD0-0x272DF4].
//! 3. For a data length other than 0, it translates the data buffer. A
//!    failure leaves the data buffer's real address 0
//!    [OFW-4.93 lv1 a:0x272DF8-0x272E1C].
//! 4. For mode 0 it asks lv1's policy whether the partition may send the
//!    command. Command 1 adds the byte at +0 of the command buffer, and
//!    command 2 adds the byte at +11. A refusal gives
//!    [`SEND_COMMAND_DENIED_STATUS`] [OFW-4.93 lv1 a:0x272F84-0x273000].
//! 5. While the current slot of the object's ring of 16 command slots at
//!    +6272 is free and the controller at +5688 is idle, it starts the
//!    command and returns 0 [OFW-4.93 lv1 a:0x272D20-0x272D44]
//!    [OFW-4.93 lv1 a:0x272E20-0x272E80]. Otherwise it queues the
//!    command in a free slot, or returns [`SEND_COMMAND_TAGS_FULL_STATUS`]
//!    when no slot is free [OFW-4.93 lv1 a:0x272D48-0x272DCC].
//!
//! The start keeps the tag at +5744 of the object and the partition at
//! +7752 [OFW-4.93 lv1 a:0x272158] [OFW-4.93 lv1 a:0x272194]. It then
//! selects the command's case through the table at 0x2721B8
//! [OFW-4.93 lv1 a:0x272198-0x2721B4]. The table's entry for
//! [`CONTROLLER_QUERY_COMMAND`] [OFW-4.93 lv1 a:0x272220] selects the
//! case at 0x272990.
//!
//! # The controller query
//!
//! The case does these steps:
//! 1. For a data length below [`CONTROLLER_QUERY_DATA_BYTES`], or a data
//!    buffer that the translation refused, it completes the request with
//!    [`CONTROLLER_QUERY_BAD_DATA_COMPLETION`]
//!    [OFW-4.93 lv1 a:0x272990-0x27299C] [OFW-4.93 lv1 a:0x27227C-0x2722A8].
//! 2. Otherwise it calls the controller's query function with the real
//!    addresses of the three words at +0, +4 and +8 of the data buffer
//!    [OFW-4.93 lv1 a:0x2729A0-0x2729BC].
//! 3. The function returns [`CONTROLLER_BUSY_COMPLETION`] while the
//!    word at +16 of the controller is 0. The case then completes the
//!    request with that status [OFW-4.93 lv1 a:0x25F8B0-0x25F8BC]
//!    [OFW-4.93 lv1 a:0x272580-0x272584].
//! 4. Otherwise the function stores 0 at +16 of the controller
//!    [OFW-4.93 lv1 a:0x25F90C-0x25F918]. It keeps the reply function
//!    0x27305C at +136 [OFW-4.93 lv1 a:0x2729A4]
//!    [OFW-4.93 lv1 a:0x3379B0-0x3379B4] [OFW-4.93 lv1 a:0x25F964].
//! 5. It stores 0 in each of the three words
//!    [OFW-4.93 lv1 a:0x25F984-0x25F98C].
//! 6. It sends the query to the controller [OFW-4.93 lv1 a:0x25F990].
//!    One path of the send stores to the controller's registers through
//!    caching-inhibited real addresses [OFW-4.93 lv1 a:0x26A0DC-0x26A108].
//!
//! The controller's reply calls the reply function, which completes the
//! request under the tag at +5744 with the reply's status
//! [OFW-4.93 lv1 a:0x273080-0x2730A0]. The completion function sends the
//! event on the port of the partition's open slot; see
//! [`super::storage`]. The reply's status and the three words come from
//! the controller. No lv1 image read gives them.
//!
//! The 4.93 kernel's call [OFW-4.93 lv2_kernel a:0x80000000000ab390]
//! passes these inputs
//! [OFW-4.93 lv2_kernel a:0x80000000000ab36c-0x80000000000ab38c]:
//! - r3 = the device id at +152 of the kernel's device object;
//! - r4 = the command id, zero-extended;
//! - r5 = the command buffer's address, which the kernel's translation
//!   gives [OFW-4.93 lv2_kernel a:0x80000000000ab53c-0x80000000000ab544];
//! - r6 = the command length, zero-extended;
//! - r7 = the data buffer's address, which the same translation gives
//!   [OFW-4.93 lv2_kernel a:0x80000000000ab438-0x80000000000ab440];
//! - r8 = the data length, zero-extended.
//!
//! On r3 = 0 it keeps r4 as the request's tag
//! [OFW-4.93 lv2_kernel a:0x80000000000ab4d8-0x80000000000ab4e0]
//! [OFW-4.93 lv2_kernel a:0x80000000000ab3a8-0x80000000000ab3c4]. The
//! call at [OFW-4.93 lv2_kernel a:0x80000000000ab528] passes the same
//! inputs [OFW-4.93 lv2_kernel a:0x80000000000ab50c-0x80000000000ab524].

use super::status::{
    LV1_BUSY, LV1_DENIED_BY_POLICY, LV1_ILLEGAL_PARAMETER_VALUE, LV1_NO_ENTRY, LV1_WRONG_STATE,
};

/// The handler of hypercall 248 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageCommandHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `storage_send_device_command` handler.
    pub handler: u32,
    /// The id of the storage device whose command method a read covers.
    pub read_device: u64,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 248 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]). Device 2 holds the
/// method table whose command method the read covers.
pub const STORAGE_COMMAND_HANDLERS: &[StorageCommandHandler] = &[StorageCommandHandler {
    firmware: "4.93",
    handler: 0x30_94E4,
    read_device: 2,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn storage_command_handler(firmware: &str) -> Option<&'static StorageCommandHandler> {
    STORAGE_COMMAND_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The status of a command to a device that does not exist:
/// `LV1_NO_ENTRY` (-6) [OFW-4.93 lv1 a:0x251CD8].
///
/// At 4.93, `devices.tsv` of the registration capture reads a null
/// object pointer for devices 5 and 6.
pub const SEND_COMMAND_NO_DEVICE_STATUS: i64 = LV1_NO_ENTRY;

/// The status of a command to a device that the partition did not open:
/// `LV1_WRONG_STATE` (-11) [OFW-4.93 lv1 a:0x251C98].
pub const SEND_COMMAND_NOT_READY_STATUS: i64 = LV1_WRONG_STATE;

/// The number of request tags of a storage device object, one bit for
/// each tag at +2680 [OFW-4.93 lv1 a:0x25398C-0x2539CC].
pub const SEND_COMMAND_TAGS: u8 = 16;

/// The status of a command while requests hold each tag of the device,
/// or while commands fill each slot of its command ring: `LV1_BUSY` (-9)
/// [OFW-4.93 lv1 a:0x2539C8] [OFW-4.93 lv1 a:0x272DA8].
pub const SEND_COMMAND_TAGS_FULL_STATUS: i64 = LV1_BUSY;

/// The largest command id that the dispatcher accepts
/// [OFW-4.93 lv1 a:0x272C7C].
pub const SEND_COMMAND_ID_MAX: u32 = 0x30;

/// The command ids that the dispatcher accepts, one bit for each id
/// [OFW-4.93 lv1 a:0x272C84-0x272CA8].
pub const SEND_COMMAND_ACCEPTED: u64 = 0x0001_02D2_1E1B_0006;

/// Whether the dispatcher accepts the command id `command`, the low
/// word of the call's r4.
pub const fn command_accepted(command: u32) -> bool {
    command <= SEND_COMMAND_ID_MAX && SEND_COMMAND_ACCEPTED & (1 << command) != 0
}

/// The status of a command id that the dispatcher refuses, or of a
/// command buffer that the translation refuses:
/// `LV1_ILLEGAL_PARAMETER_VALUE` (-17) [OFW-4.93 lv1 a:0x272CAC].
pub const SEND_COMMAND_BAD_REQUEST_STATUS: i64 = LV1_ILLEGAL_PARAMETER_VALUE;

/// The status of a command that lv1's policy refuses to the partition:
/// `LV1_DENIED_BY_POLICY` (-4) [OFW-4.93 lv1 a:0x272FB4].
pub const SEND_COMMAND_DENIED_STATUS: i64 = LV1_DENIED_BY_POLICY;

/// The command id of the controller query, from its case-table entry
/// [OFW-4.93 lv1 a:0x272220].
pub const CONTROLLER_QUERY_COMMAND: u32 = 0x1A;

/// The smallest data length of a controller query: three 32-bit words
/// [OFW-4.93 lv1 a:0x272990-0x272994].
pub const CONTROLLER_QUERY_DATA_BYTES: u64 = 12;

/// The status with which lv1 completes a controller query whose data
/// buffer is short or untranslated [OFW-4.93 lv1 a:0x27227C].
pub const CONTROLLER_QUERY_BAD_DATA_COMPLETION: u64 = 0x0205_0000;

/// The status with which lv1 completes a controller query while the
/// controller is busy [OFW-4.93 lv1 a:0x25F8B4].
pub const CONTROLLER_BUSY_COMPLETION: u64 = 0x0200_0000;

#[cfg(test)]
#[path = "tests/storage_command_tests.rs"]
mod tests;
