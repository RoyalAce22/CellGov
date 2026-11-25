//! The handler of each hypercall number CellGov answers.

use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::call::{
    ALLOCATE_MEMORY, CONFIGURE_IRQ_STATE_BITMAP, CONSTRUCT_VIRTUAL_ADDRESS_SPACE,
    GET_LOGICAL_PARTITION_ID, GET_LOGICAL_PPE_ID, LOGICAL_CONSOLE_CONSTRUCT, LOGICAL_CONSOLE_FLUSH,
    LOGICAL_CONSOLE_SEND, MAP_HTAB, READ_HTAB_ENTRIES, READ_REPOSITORY_NODE,
    SELECT_VIRTUAL_ADDRESS_SPACE, WRITE_HTAB_ENTRY,
};
use cellgov_ps3_abi::lv1::partition::LPAR_ID_GAME_OS;

use super::address_space::AddressSpaceState;
use super::answer::{HypercallAnswer, HypercallDisposition};
use super::interrupt::IrqStateBitmapState;
use super::memory::MemoryState;
use super::request::HypercallRequest;
use super::{address_space, console, interrupt, memory, page_table, ppe, repository, vector};

/// The numbers a handler answers, ascending. Every other number takes
/// the null backend, unless lv1's vector answers it first.
pub const ANSWERED: &[u64] = &[
    ALLOCATE_MEMORY,
    WRITE_HTAB_ENTRY,
    CONSTRUCT_VIRTUAL_ADDRESS_SPACE,
    SELECT_VIRTUAL_ADDRESS_SPACE,
    CONFIGURE_IRQ_STATE_BITMAP,
    GET_LOGICAL_PPE_ID,
    GET_LOGICAL_PARTITION_ID,
    READ_REPOSITORY_NODE,
    READ_HTAB_ENTRIES,
    LOGICAL_CONSOLE_CONSTRUCT,
    LOGICAL_CONSOLE_SEND,
    LOGICAL_CONSOLE_FLUSH,
    MAP_HTAB,
];

/// Whether a handler answers hypercall `number`.
pub fn answers(number: u64) -> bool {
    ANSWERED.contains(&number)
}

/// The disposition of `request` from a hypervisor that answers for the
/// firmware version `firmware` and holds these states:
/// - the memory regions `memory`;
/// - the virtual address spaces `spaces`;
/// - the interrupt state bitmaps `bitmaps`.
///
/// `storage` is the caller's real address space. The page table calls
/// read the page table there.
///
/// lv1's vector answers first: a number past the table, and a
/// problem-state call. A handler answers the call next. The null backend
/// answers a number with no handler.
pub fn answer(
    request: &HypercallRequest,
    firmware: Option<&str>,
    memory: &MemoryState,
    spaces: &AddressSpaceState,
    bitmaps: &IrqStateBitmapState,
    storage: &GuestMemory,
) -> HypercallDisposition {
    if let Some(disposition) = vector::vector_disposition(request, firmware) {
        return disposition;
    }
    match request.number {
        ALLOCATE_MEMORY => memory::allocate(request, firmware, memory),
        WRITE_HTAB_ENTRY => page_table::write_entry(request, firmware, spaces, storage),
        CONSTRUCT_VIRTUAL_ADDRESS_SPACE => address_space::construct(request, firmware, spaces),
        SELECT_VIRTUAL_ADDRESS_SPACE => address_space::select(request, firmware, spaces, storage),
        CONFIGURE_IRQ_STATE_BITMAP => {
            interrupt::configure_bitmap(request, firmware, bitmaps, storage)
        }
        GET_LOGICAL_PPE_ID => ppe::logical_ppe_id(request, firmware),
        GET_LOGICAL_PARTITION_ID => {
            HypercallDisposition::Answered(HypercallAnswer::success(vec![LPAR_ID_GAME_OS]))
        }
        READ_REPOSITORY_NODE => repository::read_node(request, firmware),
        READ_HTAB_ENTRIES => page_table::read_entries(request, firmware, spaces, storage),
        LOGICAL_CONSOLE_CONSTRUCT => console::construct(),
        LOGICAL_CONSOLE_SEND => console::send(request),
        LOGICAL_CONSOLE_FLUSH => console::flush(),
        MAP_HTAB => page_table::map_table(request, firmware, spaces, memory),
        _ => vector::null_backend(request, firmware),
    }
}

#[cfg(test)]
#[path = "tests/handler_tests.rs"]
mod tests;
