//! The handler of each hypercall number CellGov answers.

use cellgov_ps3_abi::lv1::call::{
    ALLOCATE_MEMORY, CONSTRUCT_VIRTUAL_ADDRESS_SPACE, GET_LOGICAL_PARTITION_ID,
    LOGICAL_CONSOLE_CONSTRUCT, LOGICAL_CONSOLE_FLUSH, LOGICAL_CONSOLE_SEND, READ_REPOSITORY_NODE,
    SELECT_VIRTUAL_ADDRESS_SPACE,
};
use cellgov_ps3_abi::lv1::partition::LPAR_ID_GAME_OS;

use super::address_space::AddressSpaceState;
use super::answer::{HypercallAnswer, HypercallDisposition};
use super::memory::MemoryState;
use super::request::HypercallRequest;
use super::{address_space, console, memory, repository, vector};

/// The numbers a handler answers, ascending. Every other number takes
/// the null backend, unless lv1's vector answers it first.
pub const ANSWERED: &[u64] = &[
    ALLOCATE_MEMORY,
    CONSTRUCT_VIRTUAL_ADDRESS_SPACE,
    SELECT_VIRTUAL_ADDRESS_SPACE,
    GET_LOGICAL_PARTITION_ID,
    READ_REPOSITORY_NODE,
    LOGICAL_CONSOLE_CONSTRUCT,
    LOGICAL_CONSOLE_SEND,
    LOGICAL_CONSOLE_FLUSH,
];

/// Whether a handler answers hypercall `number`.
pub fn answers(number: u64) -> bool {
    ANSWERED.contains(&number)
}

/// The disposition of `request` from a hypervisor that answers for the
/// firmware version `firmware` and holds the memory regions `memory` and
/// the virtual address spaces `spaces`.
///
/// lv1's vector answers first: a number past the table, and a
/// problem-state call. A handler answers the call next. The null backend
/// answers a number with no handler.
pub fn answer(
    request: &HypercallRequest,
    firmware: Option<&str>,
    memory: &MemoryState,
    spaces: &AddressSpaceState,
) -> HypercallDisposition {
    if let Some(disposition) = vector::vector_disposition(request, firmware) {
        return disposition;
    }
    match request.number {
        ALLOCATE_MEMORY => memory::allocate(request, firmware, memory),
        CONSTRUCT_VIRTUAL_ADDRESS_SPACE => address_space::construct(request, firmware, spaces),
        SELECT_VIRTUAL_ADDRESS_SPACE => address_space::select(request, firmware, spaces),
        GET_LOGICAL_PARTITION_ID => {
            HypercallDisposition::Answered(HypercallAnswer::success(vec![LPAR_ID_GAME_OS]))
        }
        READ_REPOSITORY_NODE => repository::read_node(request, firmware),
        LOGICAL_CONSOLE_CONSTRUCT => console::construct(),
        LOGICAL_CONSOLE_SEND => console::send(request),
        LOGICAL_CONSOLE_FLUSH => console::flush(),
        _ => vector::null_backend(request, firmware),
    }
}

#[cfg(test)]
#[path = "tests/handler_tests.rs"]
mod tests;
