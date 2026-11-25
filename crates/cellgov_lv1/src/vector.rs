//! The answers lv1's `sc` vector gives before a handler runs, and the
//! null backend's answer for a number with no handler.
//!
//! lv1 answers these calls itself, from its hypercall table:
//! - a number past the end of the table, from either processor state;
//! - a problem-state call to an entry that is not problem-state
//!   callable;
//! - a call to an entry that holds the Invalid HV Call function.
//!
//! CellGov answers each one only at a firmware whose lv1 shows the fact
//! the answer needs, and stops the run elsewhere:
//! - The entry count is read for each firmware of
//!   [`table::HYPERCALL_TABLE_SITES`].
//! - Which entries are invalid, and which are problem-state callable,
//!   is read only from the live 4.93 table.

use cellgov_ps3_abi::lv1::dispatch::{PROBLEM_STATE_DENIED_R3, UNKNOWN_HYPERCALL_R3};
use cellgov_ps3_abi::lv1::table::{self, TableEntry, HYPERCALL_TABLE_ENTRIES};

use super::answer::{HypercallAnswer, HypercallDisposition, HypercallRefusal};
use super::request::HypercallRequest;

/// An answer of `r3` with r4 to r10 at 0, as lv1's return path gives
/// it, and no memory write.
fn answered(r3: u64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(r3, []))
}

/// The disposition the vector gives `request` from a hypervisor for the
/// firmware version `firmware`, or `None` when the call reaches its
/// table entry.
///
/// The vector's checks run in this order:
/// 1. A number at or past [`HYPERCALL_TABLE_ENTRIES`] gets
///    [`UNKNOWN_HYPERCALL_R3`], and r4 to r10 keep their values: this
///    return [OFW-4.93 lv1 a:0x0C44-0x0C58] does not load them. CellGov
///    answers it only at a firmware that shows the entry count.
/// 2. A problem-state call gets [`PROBLEM_STATE_DENIED_R3`] with r4 to
///    r10 cleared, unless its entry is problem-state callable. At a
///    firmware with no live table, CellGov refuses the call.
pub(super) fn vector_disposition(
    request: &HypercallRequest,
    firmware: Option<&str>,
) -> Option<HypercallDisposition> {
    let site = firmware.and_then(table::table_site);
    if request.number >= HYPERCALL_TABLE_ENTRIES && site.is_some() {
        return Some(HypercallDisposition::Answered(HypercallAnswer::r3_only(
            UNKNOWN_HYPERCALL_R3,
        )));
    }
    if !request.problem_state {
        return None;
    }
    match firmware.and_then(|fw| table::table_entry(fw, request.number)) {
        Some(entry) if entry.problem_state_callable() => None,
        Some(_) => Some(answered(PROBLEM_STATE_DENIED_R3)),
        None => Some(HypercallDisposition::Refused(
            HypercallRefusal::ProblemState {
                number: request.number,
                unit: request.unit,
                pc: request.pc,
            },
        )),
    }
}

/// The null backend's disposition of `request`, a call that no handler
/// answers, from a hypervisor for the firmware version `firmware`.
///
/// An entry that holds the Invalid HV Call function gets
/// [`table::INVALID_HYPERCALL_R3`] with r4 to r10 cleared. Every other
/// call stops the run: lv1 has a handler for the number that CellGov
/// does not model, or no capture gives the firmware's table.
pub(super) fn null_backend(
    request: &HypercallRequest,
    firmware: Option<&str>,
) -> HypercallDisposition {
    match firmware.and_then(|fw| table::table_entry(fw, request.number)) {
        Some(TableEntry::Invalid) => answered(table::INVALID_HYPERCALL_R3),
        Some(TableEntry::Handler(_)) | None => {
            HypercallDisposition::Refused(HypercallRefusal::Unmodelled {
                number: request.number,
                unit: request.unit,
                pc: request.pc,
            })
        }
    }
}

#[cfg(test)]
#[path = "tests/vector_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/console_problem_state_sc_tests.rs"]
mod console_tests;
