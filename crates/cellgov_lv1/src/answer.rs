//! What a handler gives back: an answer the runtime applies, or a typed
//! refusal that stops the run.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{LOGICAL_CONSOLE_SEND_MAX, MAX_OUTPUTS};
use cellgov_ps3_abi::lv1::repository::RepositoryKey;
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};

use crate::group::state::StateChange;
use crate::request::HypercallRequest;
use crate::return_path::{self, ExternalRaise, ReturnRegisters};

/// The values for r4 to r10, in register order.
pub type OutputRegisters = [u64; MAX_OUTPUTS as usize];

/// Bytes a logical console send gave the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsoleWrite {
    /// The console id the send named, from r3.
    pub console: u64,
    /// The bytes, of which the first `len` count.
    pub data: [u8; LOGICAL_CONSOLE_SEND_MAX],
    /// How many bytes of `data` the send carried.
    pub len: u8,
}

impl ConsoleWrite {
    /// The bytes the send carried.
    pub fn bytes(&self) -> &[u8] {
        &self.data[..usize::from(self.len)]
    }
}

/// The answer to one hypercall, which the runtime applies as effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HypercallAnswer {
    /// The exact value for r3. Its low word holds the LV1 status code.
    /// Its high word is what lv1 puts there on that path: 0 for a
    /// zero-extended status, or the argument mark of an
    /// `allocate_memory` refusal.
    pub r3: u64,
    /// How lv1 returns to the caller.
    pub path: ReturnPath,
    /// Memory writes, at real addresses.
    pub effects: Vec<Effect>,
    /// Bytes for the guest TTY sink.
    pub console: Option<ConsoleWrite>,
    /// The page table and large page sizes that the calling thread
    /// loads, which the runtime gives the thread.
    pub page_table: Option<PageTableLoad>,
    /// The changes to the hypercall groups' state, which the hypervisor
    /// applies in order.
    pub changes: Vec<StateChange>,
    /// Whether lv1 pauses the calling thread at the call.
    ///
    /// When it is set, the runtime parks the caller. The caller gets the
    /// return only when a wake makes it runnable again.
    pub pauses_caller: bool,
    /// The external interrupt that lv1's return raises for the caller,
    /// or `None` when it raises none. The hypervisor sets it as it
    /// dispatches the call.
    pub external: Option<ExternalRaise>,
    /// Each paused thread that the call woke, which returns from its
    /// `pause`. The hypervisor sets it as it dispatches the call.
    pub resumed: Vec<ThreadResume>,
    /// Each running thread that the call sent an interprocessor
    /// interrupt. The hypervisor sets it as it dispatches the call.
    pub interrupted: Vec<ThreadInterrupt>,
}

/// A running thread that a call sent an interprocessor interrupt, and
/// the external interrupt that lv1's handler then raises for it.
///
/// The thread takes the interprocessor interrupt at its next instruction
/// boundary; see [`cellgov_ps3_abi::lv1::interrupt_mask`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadInterrupt {
    /// The PPE hardware thread.
    pub thread: u8,
    /// The external interrupt that the handler raises, or `None` when its
    /// shadow test finds no bit.
    pub external: Option<ExternalRaise>,
}

/// A paused thread that a call woke, and the external interrupt that its
/// return from `pause` raises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadResume {
    /// The PPE hardware thread.
    pub thread: u8,
    /// The external interrupt of its return, or `None`.
    pub external: Option<ExternalRaise>,
}

/// How lv1's vector returns to the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnPath {
    /// The `hrfid` that every number in the table takes, which loads r4
    /// to r10 with these values [OFW-4.93 lv1 a:0x1040-0x1068].
    Table(OutputRegisters),
    /// The `rfid` for a number past the end of the table, which leaves r4
    /// to r10 as the caller had them [OFW-4.93 lv1 a:0x0C44-0x0C58].
    PastTheTable(ReturnRegisters),
}

/// The translation registers that lv1 loads on the calling thread when
/// it selects an address space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageTableLoad {
    /// SDR1: the page table's real address and size.
    pub sdr1: u64,
    /// HID6\[LB\] as a 4-bit value, LB bit 0 most significant.
    pub lb: u8,
}

impl HypercallAnswer {
    /// `LV1_SUCCESS` with `outputs` from r4, as [`Self::lv1_status`]
    /// returns them.
    pub fn success<const N: usize>(outputs: [u64; N]) -> Self {
        Self::lv1_status(zero_extended(LV1_SUCCESS), outputs)
    }

    /// The exact `r3` and the `outputs` from r4 that lv1 returns, with no
    /// memory write.
    ///
    /// Each register after `outputs`, up to r10, gets 0:
    /// - The vector's entry `dcbz` [OFW-4.93 lv1 a:0x0C00-0x0C08] zeroes
    ///   the 128-byte block at HSPRG0 - 0x6A80, which holds the return
    ///   slots of r3 to r10 where HSPRG0 is 128-byte aligned.
    /// - The return path [OFW-4.93 lv1 a:0x1040-0x1068] loads r3 to r10
    ///   from those slots.
    /// - A handler that lv1 calls directly writes only the slots of its
    ///   outputs, as hypercall 74 does [OFW-4.93 lv1 a:0x2EBDA8-0x2EBDC4].
    /// - The `lv1_problem_state_sc` capture shows r4 to r10 at 0 after an
    ///   answer that writes only the r3 slot.
    ///
    /// lv1 sends some numbers to a service through its dispatcher. The
    /// dispatcher copies seven reply words into the slots
    /// [OFW-4.93 lv1 a:0x2C8B14-0x2C8B34]. A service that writes fewer
    /// words leaves lv1's own stack data in the rest, and CellGov does not
    /// model that data.
    pub fn lv1_status<const N: usize>(r3: u64, outputs: [u64; N]) -> Self {
        const { assert!(N <= MAX_OUTPUTS as usize, "more outputs than r4 to r10") };
        let mut registers = OutputRegisters::default();
        registers[..N].copy_from_slice(&outputs);
        Self::with_path(r3, ReturnPath::Table(registers))
    }

    /// The exact `r3` of the past-the-table return and the `registers` it
    /// leaves, with r4 to r10 as the caller had them and no memory write.
    pub(crate) fn past_the_table(r3: u64, registers: ReturnRegisters) -> Self {
        Self::with_path(r3, ReturnPath::PastTheTable(registers))
    }

    /// The values for r4 to r10 that lv1's return loads, or `None` where
    /// lv1's return leaves r4 to r10 as the caller had them.
    pub fn outputs(&self) -> Option<OutputRegisters> {
        match self.path {
            ReturnPath::Table(outputs) => Some(outputs),
            ReturnPath::PastTheTable(_) => None,
        }
    }

    /// What lv1's return leaves in r0, r11, r12 and CR for `request`,
    /// from a hypervisor for the firmware version `firmware`.
    ///
    /// A return that enters the vector leaves the vector in r11 and the new
    /// MSR in r12; see [`ExternalRaise::Deliver`]. A return that sets
    /// LPCR\[MER\] changes no saved register [OFW-4.93 lv1 a:0x191C-0x1928].
    pub fn return_registers(
        &self,
        request: &HypercallRequest,
        firmware: Option<&str>,
    ) -> ReturnRegisters {
        let delivery = self.external.and_then(ExternalRaise::delivery);
        match (self.path, delivery) {
            (ReturnPath::Table(_), None) => return_path::table(request.problem_state, firmware),
            (ReturnPath::Table(_), Some(delivery)) => {
                return_path::delivering_table(request.problem_state, firmware, delivery)
            }
            (ReturnPath::PastTheTable(registers), _) => registers,
        }
    }

    fn with_path(r3: u64, path: ReturnPath) -> Self {
        Self {
            r3,
            path,
            effects: Vec::new(),
            console: None,
            page_table: None,
            changes: Vec::new(),
            pauses_caller: false,
            external: None,
            resumed: Vec::new(),
            interrupted: Vec::new(),
        }
    }

    /// Adds `change` to the changes the hypervisor applies.
    pub fn push_change(&mut self, change: impl Into<StateChange>) {
        self.changes.push(change.into());
    }
}

/// Why CellGov's hypervisor stops the run at a hypercall, or at a
/// thread's take of a mediated external interrupt.
///
/// A hypercall refusal answers nothing: the caller sees no r3 and does
/// not step past `sc`. A take refusal stops the run after the thread
/// entered the vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HypercallRefusal {
    /// No handler answers the number: the null backend.
    #[error("hypercall {number} from {unit:?} at pc 0x{pc:016x} has no handler")]
    Unmodelled {
        /// The hypercall number.
        number: u64,
        /// The unit that made the call.
        unit: UnitId,
        /// The address of its `sc`.
        pc: u64,
    },
    /// A problem-state call at a firmware whose live hypercall table no
    /// console capture gives, so whether its entry is problem-state
    /// callable is not known.
    #[error(
        "problem-state hypercall {number} from {unit:?} at pc 0x{pc:016x}: no capture gives \
         lv1's hypercall table at this firmware"
    )]
    ProblemState {
        /// The hypercall number.
        number: u64,
        /// The unit that made the call.
        unit: UnitId,
        /// The address of its `sc`.
        pc: u64,
    },
    /// A handler exists, and the model does not cover these inputs.
    #[error("hypercall {number} from {unit:?} at pc 0x{pc:016x}: CellGov does not model {what}")]
    UnmodelledInput {
        /// The hypercall number.
        number: u64,
        /// The unit that made the call.
        unit: UnitId,
        /// The address of its `sc`.
        pc: u64,
        /// The inputs the model does not cover.
        what: &'static str,
    },
    /// A repository read named a node that no source gives values for.
    #[error(
        "repository read from {unit:?} at pc 0x{pc:016x}: no source gives the node \
         lpar {} key 0x{:016x} 0x{:016x} 0x{:016x} 0x{:016x}",
        key.lpar, key.n1, key.n2, key.n3, key.n4
    )]
    RepositoryNode {
        /// The partition and the raw key words of the read.
        key: RepositoryKey,
        /// The unit that made the call.
        unit: UnitId,
        /// The address of its `sc`.
        pc: u64,
    },
    /// A repository read named a node whose source does not observe the
    /// second value, from a site that no analysis shows discards it.
    #[error(
        "repository read from {unit:?} at pc 0x{pc:016x}, lr 0x{lr:016x}: the source of the node \
         lpar {} key 0x{:016x} 0x{:016x} 0x{:016x} 0x{:016x} does not observe its second value, \
         and no analysis shows that this read site discards it",
        key.lpar, key.n1, key.n2, key.n3, key.n4
    )]
    RepositorySecondValueUnobserved {
        /// The partition and the raw key words of the read.
        key: RepositoryKey,
        /// The unit that made the call.
        unit: UnitId,
        /// The address of its `sc`.
        pc: u64,
        /// The link register at the `sc`.
        lr: u64,
    },
    /// A `write_virtual_uart` call completes a request on the system
    /// manager's port, and CellGov does not hold each input of the reply.
    #[error(
        "hypercall {} from {unit:?} at pc 0x{pc:016x}: CellGov does not hold the inputs of the \
         system manager's reply to a request of service id 0x{service:04x}",
        cellgov_ps3_abi::lv1::call::WRITE_VIRTUAL_UART
    )]
    SystemManagerRequest {
        /// The unit that made the call.
        unit: UnitId,
        /// The address of its `sc`.
        pc: u64,
        /// The service id that the request header gives.
        service: u16,
    },
    /// A `write_virtual_uart` call completes a request on the dispatcher
    /// manager's port, and no source gives a part of the reply.
    #[error(
        "hypercall {} from {unit:?} at pc 0x{pc:016x}: CellGov does not model the reply to {} of \
         service 0x{service:x} on the dispatcher manager's port: no source gives {part}",
        cellgov_ps3_abi::lv1::call::WRITE_VIRTUAL_UART,
        RequestId(*request)
    )]
    DispatcherRequest {
        /// The unit that made the call.
        unit: UnitId,
        /// The address of its `sc`.
        pc: u64,
        /// The service id that the request header gives.
        service: u32,
        /// The request id that the request bytes start with. The value is
        /// `None` when the header counts fewer than 8 request bytes.
        request: Option<u64>,
        /// The part of the reply that no source gives.
        part: &'static str,
    },
    /// A thread took a mediated external interrupt in a case where lv1's
    /// handler enters no vector, or enters it with other registers.
    #[error(
        "the mediated external interrupt that {unit:?} took at pc 0x{pc:016x}: CellGov does not \
         model {what}"
    )]
    MediatedExternal {
        /// The unit that took the interrupt.
        unit: UnitId,
        /// The address that the interrupt saved in SRR0.
        pc: u64,
        /// The case that the model does not cover.
        what: &'static str,
    },
}

impl HypercallRefusal {
    /// The hypercall number the refusal names, or `None` for the take of
    /// a mediated external interrupt.
    pub fn number(&self) -> Option<u64> {
        match self {
            Self::Unmodelled { number, .. }
            | Self::ProblemState { number, .. }
            | Self::UnmodelledInput { number, .. } => Some(*number),
            Self::RepositoryNode { .. } | Self::RepositorySecondValueUnobserved { .. } => {
                Some(cellgov_ps3_abi::lv1::call::READ_REPOSITORY_NODE)
            }
            Self::SystemManagerRequest { .. } | Self::DispatcherRequest { .. } => {
                Some(cellgov_ps3_abi::lv1::call::WRITE_VIRTUAL_UART)
            }
            Self::MediatedExternal { .. } => None,
        }
    }
}

/// The request that a [`HypercallRefusal::DispatcherRequest`] names.
struct RequestId(Option<u64>);

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(id) => write!(f, "request 0x{id:x}"),
            None => f.write_str("a request with no request id"),
        }
    }
}

/// A case that CellGov does not model, which a call or a take finds.
///
/// The refusal of a call is [`HypercallRefusal::UnmodelledInput`]. The
/// refusal of a take is [`HypercallRefusal::MediatedExternal`]. Each
/// refusal names `what`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the model does not cover {what}")]
pub(crate) struct UnmodelledError {
    /// The case, as the refusal names it.
    pub(crate) what: &'static str,
}

impl UnmodelledError {
    /// The error for the case `what`.
    pub(crate) const fn new(what: &'static str) -> Self {
        Self { what }
    }

    /// The refusal of `request` for this case.
    pub(crate) fn refusal(self, request: &HypercallRequest) -> HypercallDisposition {
        HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
            number: request.number,
            unit: request.unit,
            pc: request.pc,
            what: self.what,
        })
    }
}

/// What a handler gives back.
#[allow(
    clippy::large_enum_variant,
    reason = "a disposition lives for one dispatch, and a boxed answer costs an allocation per call"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HypercallDisposition {
    /// The runtime applies the answer.
    Answered(HypercallAnswer),
    /// The run stops at the call.
    Refused(HypercallRefusal),
}

#[cfg(test)]
#[path = "tests/answer_tests.rs"]
mod tests;
