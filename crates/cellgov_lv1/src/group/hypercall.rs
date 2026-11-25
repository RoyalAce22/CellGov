//! One hypercall's registration, and the input its handler reads.

use cellgov_mem::GuestMemory;
use cellgov_time::GuestTicks;

use super::state::GroupStates;
use crate::answer::HypercallDisposition;
use crate::request::HypercallRequest;

/// What a handler reads besides the request.
#[derive(Debug, Clone, Copy)]
pub(crate) struct HandlerInput<'a> {
    /// The firmware version the hypervisor answers for.
    pub(crate) firmware: Option<&'a str>,
    /// The state of each hypercall group.
    pub(crate) groups: &'a GroupStates,
    /// The caller's real address space, which holds the page table.
    pub(crate) storage: &'a GuestMemory,
}

/// A handler: the disposition of a request from its input.
pub(crate) type Handler = fn(&HypercallRequest, &HandlerInput<'_>) -> HypercallDisposition;

/// One hypercall that a group answers.
///
/// A registration makes its number answered. The dispatch, the answered
/// numbers and the census's answered column all read the registrations.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Hypercall {
    /// The number the caller puts in r11.
    pub(crate) number: u64,
    /// The name of the call. It is the call table's name for the number
    /// when the table gives one that is not `undocumented_function_N`;
    /// otherwise it is the name CellGov gives the call.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the registration guard reads the name")
    )]
    pub(crate) name: &'static str,
    /// The guest time the call charges, or `None` when no capture times
    /// it.
    pub(crate) cost: Option<GuestTicks>,
    /// The handler that answers the call.
    pub(crate) handler: Handler,
}
