//! The closed set of reasons a unit's `run_until_yield` can return.
//!
//! Architecture-specific yield vocabularies (PPU-specific,
//! SPU-specific, RSX-specific) belong in arch crates as wrapper
//! enums on top of this set, not as additional variants here.

/// Why an execution unit yielded control back to the runtime.
///
/// Discriminants and variant order are part of the binary trace
/// contract: do not reorder, do not insert in the middle, do not
/// renumber. New variants append at the end with discriminants
/// strictly greater than [`YieldReason::Finished`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::VariantArray)]
#[repr(u8)]
pub enum YieldReason {
    /// Scheduler-granted budget exhausted before a more meaningful
    /// yield point. The unit stays runnable.
    BudgetExhausted = 0,
    /// Mailbox operation that needs runtime arbitration: send into
    /// a full mailbox, receive from an empty one, or any access
    /// whose visibility depends on the commit pipeline.
    MailboxAccess = 1,
    /// DMA request submitted; yielding so the runtime can stage it
    /// and decide its modeled completion time. The unit may continue
    /// after acknowledgement -- this does not imply blocking.
    DmaSubmitted = 2,
    /// Blocked on a previously-submitted DMA transfer reaching its
    /// modeled completion time.
    DmaWait = 3,
    /// Blocked on a `cellgov_sync` primitive: signal, barrier, mutex,
    /// or any state machine there that produced a block condition.
    WaitingSync = 4,
    /// Abstract guest syscall invoked; yielding so the runtime can
    /// route through LV2 dispatch. `syscall_args` on the step
    /// result carries the syscall number and argument registers.
    Syscall = 5,
    /// Clean point at which the runtime may inject pending
    /// interrupts before resuming.
    InterruptBoundary = 6,
    /// Fault raised. The step's effects are discarded wholesale,
    /// the fault is recorded in the trace, and the unit moves to
    /// its fault-handling state.
    Fault = 7,
    /// Terminal: the runtime should remove the unit from the
    /// runnable set after observing this.
    Finished = 8,
}

impl YieldReason {
    /// Whether this yield reason BREAKS a critical section the unit
    /// was holding. The scheduler uses this to decide whether to
    /// keep the unit sticky (continue scheduling it before its peers)
    /// or release stickiness so peers can run.
    pub fn breaks_critical_section(&self) -> bool {
        match self {
            YieldReason::WaitingSync
            | YieldReason::DmaWait
            | YieldReason::Finished
            | YieldReason::Fault => true,
            YieldReason::BudgetExhausted
            | YieldReason::MailboxAccess
            | YieldReason::DmaSubmitted
            | YieldReason::Syscall
            | YieldReason::InterruptBoundary => false,
        }
    }

    /// Whether the commit pipeline's trivial-step fast path is
    /// eligible for this yield. The fast path skips per-step LV2
    /// drain / syscall-response arbitration; a yield reason that
    /// implies runtime arbitration (`Syscall`, `Finished`) must NOT
    /// take the fast path.
    pub fn allows_trivial_fast_path(&self) -> bool {
        match self {
            YieldReason::Syscall | YieldReason::Finished => false,
            YieldReason::BudgetExhausted
            | YieldReason::MailboxAccess
            | YieldReason::DmaSubmitted
            | YieldReason::DmaWait
            | YieldReason::WaitingSync
            | YieldReason::InterruptBoundary
            | YieldReason::Fault => true,
        }
    }
}

#[cfg(test)]
#[path = "tests/yield_reason_tests.rs"]
mod tests;
