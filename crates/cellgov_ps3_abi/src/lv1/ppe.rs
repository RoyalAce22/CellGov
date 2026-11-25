//! The logical PPE of a partition:
//! - the handler of `get_logical_ppe_id` (hypercall 69) and the
//!   registers it returns;
//! - where lv1 keeps the id;
//! - the logical PPE id of the GameOS partition.
//!
//! The handler facts come from the 4.93 `lv1.self`, decrypted offline
//! and read with `cellgov dev disasm`. No byte of lv1 is committed;
//! these facts are.
//!
//! The handler returns these registers:
//! - r3 is 0, `LV1_SUCCESS` [OFW-4.93 lv1 a:0x2E9024].
//! - r4 is the doubleword at +72 of the logical PPE object that the
//!   calling thread's lv1 context names at HSPRG0 - 0x67F0. The handler
//!   loads it at [OFW-4.93 lv1 a:0x2E8FA8-0x2E8FC0] and stores it in the return slot
//!   of r4, HSPRG0 - 0x6A38, at [OFW-4.93 lv1 a:0x2E8FC4].
//! - r5 to r10 are 0. The handler writes no other return slot, and the
//!   vector's entry zeroes the slots [OFW-4.93 lv1 a:0x0C08].
//!
//! The doubleword at +72 is the object's id in its partition's object
//! table:
//! - The object constructor at [OFW-4.93 lv1 a:0x2CC64C] gives the
//!   table's allocator [OFW-4.93 lv1 a:0x2CA258] the address of +72
//!   [OFW-4.93 lv1 a:0x2CC6E0] [OFW-4.93 lv1 a:0x2CC6EC]. The allocator
//!   takes the first free id of the table.
//! - The logical PPE constructor at [OFW-4.93 lv1 a:0x2E0DF8] calls it at
//!   [OFW-4.93 lv1 a:0x2E0E58].
//! - The constructor has two callers, [OFW-4.93 lv1 a:0x2E7D94] and
//!   [OFW-4.93 lv1 a:0x2E854C]. The first is in
//!   [OFW-4.93 lv1 a:0x2E7754], which takes a partition that already
//!   exists from its argument [OFW-4.93 lv1 a:0x2E796C]. The function
//!   [OFW-4.93 lv1 a:0x2994D8] calls [OFW-4.93 lv1 a:0x2E7754], and the
//!   table entry at [OFW-4.93 lv1 a:0x35F780] pairs the number 5 with its
//!   function descriptor.
//!
//! So the partition's other objects decide the id, and no image read
//! gives it. The repository gives it; see [`game_os_logical_ppe_id`].

use super::partition::LPAR_ID_GAME_OS;
use super::repository::{field, first_field, lookup, RepositoryKey};

/// The handler of hypercall 69 in one firmware's lv1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogicalPpeIdHandler {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The address of the `get_logical_ppe_id` handler.
    pub handler: u32,
}

/// Each firmware whose lv1 image shows the handler, ascending.
///
/// The address is the entry of hypercall 69 in the live 4.93 hypercall
/// table ([`super::table::LIVE_TABLE_HANDLERS`]).
pub const LOGICAL_PPE_ID_HANDLERS: &[LogicalPpeIdHandler] = &[LogicalPpeIdHandler {
    firmware: "4.93",
    handler: 0x2E8F38,
}];

/// The handler of the firmware version `firmware`, or `None` when no lv1
/// image read shows it.
pub fn logical_ppe_id_handler(firmware: &str) -> Option<&'static LogicalPpeIdHandler> {
    LOGICAL_PPE_ID_HANDLERS
        .iter()
        .find(|handler| handler.firmware == firmware)
}

/// The repository node that holds the id of the GameOS partition's first
/// logical PU: `bi.pu` at PU index 0.
///
/// Linux `arch/powerpc/platforms/ps3/repository.c`,
/// `ps3_repository_read_pu_id`, reads this node. Its first value is the
/// PU id.
pub const GAME_OS_PU_ID_NODE: RepositoryKey = RepositoryKey {
    lpar: LPAR_ID_GAME_OS,
    n1: first_field(b"bi", 0),
    n2: field(b"pu", 0),
    n3: 0,
    n4: 0,
};

/// The console capture of the 4.93 kernel's two per-thread records, on
/// the reference console, relative to the workspace root. Each record
/// holds the r4 of the kernel's boot `get_logical_ppe_id` call at +0x08.
/// Its README gives the method and the analysis.
pub const LOGICAL_PPE_RECORD_CAPTURE: &str =
    "tests/micro/lv2_logical_ppe_record/ps3/cech20-cex-493";

/// The logical PPE id that lv1 gives the GameOS partition at the
/// firmware version `firmware`, or `None` when no source gives it.
///
/// The id is the first value of [`GAME_OS_PU_ID_NODE`]. The PU id and the
/// logical PPE id are one id: Linux
/// `arch/powerpc/platforms/ps3/repository.c`,
/// `ps3_repository_read_mm_info`, passes the r4 of
/// `lv1_get_logical_ppe_id` as the PU id word of `bi.pu.<id>.rm_size`.
/// The partition has one PU, as `bi.pun` gives. At 4.93 the capture
/// [`LOGICAL_PPE_RECORD_CAPTURE`] reads the same id in the record of
/// each hardware thread.
pub fn game_os_logical_ppe_id(firmware: &str) -> Option<u64> {
    lookup(&GAME_OS_PU_ID_NODE, firmware).map(|node| node.v1)
}

#[cfg(test)]
#[path = "tests/ppe_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/ppe_console_tests.rs"]
mod console_tests;
