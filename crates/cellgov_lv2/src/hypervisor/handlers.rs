//! The handler of each hypercall number CellGov answers.

use cellgov_ps3_abi::lv1::call::{
    GET_LOGICAL_PARTITION_ID, LOGICAL_CONSOLE_CONSTRUCT, LOGICAL_CONSOLE_FLUSH,
    LOGICAL_CONSOLE_SEND, READ_REPOSITORY_NODE,
};
use cellgov_ps3_abi::lv1::partition::LPAR_ID_GAME_OS;

use super::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use super::request::HypercallRequest;
use super::{console, repository};

/// The numbers a handler answers, ascending. Every other number takes
/// the null backend.
pub const ANSWERED: &[u64] = &[
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
/// firmware version `firmware`.
pub fn answer(request: &HypercallRequest, firmware: Option<&str>) -> HypercallDisposition {
    match request.number {
        GET_LOGICAL_PARTITION_ID => {
            HypercallDisposition::Answered(HypercallAnswer::success(vec![LPAR_ID_GAME_OS]))
        }
        READ_REPOSITORY_NODE => repository::read_node(request, firmware),
        LOGICAL_CONSOLE_CONSTRUCT => console::construct(),
        LOGICAL_CONSOLE_SEND => console::send(request),
        LOGICAL_CONSOLE_FLUSH => console::flush(),
        number => HypercallDisposition::Refused(HypercallRefusal::Unmodelled {
            number,
            unit: request.unit,
            pc: request.pc,
        }),
    }
}

#[cfg(test)]
#[path = "tests/handler_tests.rs"]
mod tests;
