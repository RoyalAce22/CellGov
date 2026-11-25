//! The dispatch of a hypercall to the handler that each group registers
//! for its number.

use cellgov_mem::GuestMemory;

use super::answer::HypercallDisposition;
use super::group::hypercall::HandlerInput;
use super::group::state::GroupStates;
use super::group::table;
use super::request::HypercallRequest;
use super::vector;

/// The numbers a handler answers, ascending. Every other number takes
/// the null backend, unless lv1's vector answers it first.
///
/// Each group registers its own numbers, so this list is built from the
/// registrations and no file lists them.
pub fn answered_numbers() -> Vec<u64> {
    let mut numbers: Vec<u64> = table::registrations().map(|h| h.number).collect();
    numbers.sort_unstable();
    numbers
}

/// Whether a handler answers hypercall `number`.
pub fn answers(number: u64) -> bool {
    table::registration(number).is_some()
}

/// The disposition of `request` from a hypervisor that answers for the
/// firmware version `firmware` and holds the group states `groups`.
///
/// `storage` is the caller's real address space. The page table calls
/// read the page table there, and the bitmap and plug calls write the
/// bitmaps there.
///
/// lv1's vector answers first: a number past the table, and a
/// problem-state call. The handler that a group registers for the number
/// answers the call next. The null backend answers a number with no
/// handler.
pub fn answer(
    request: &HypercallRequest,
    firmware: Option<&str>,
    groups: &GroupStates,
    storage: &GuestMemory,
) -> HypercallDisposition {
    if let Some(disposition) = vector::vector_disposition(request, firmware) {
        return disposition;
    }
    match table::registration(request.number) {
        Some(hypercall) => (hypercall.handler)(
            request,
            &HandlerInput {
                firmware,
                groups,
                storage,
            },
        ),
        None => vector::null_backend(request, firmware),
    }
}

#[cfg(test)]
#[path = "tests/handler_tests.rs"]
mod tests;
