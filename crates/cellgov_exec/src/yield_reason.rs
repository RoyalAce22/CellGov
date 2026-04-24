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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discriminants_are_locked() {
        assert_eq!(YieldReason::BudgetExhausted as u8, 0);
        assert_eq!(YieldReason::MailboxAccess as u8, 1);
        assert_eq!(YieldReason::DmaSubmitted as u8, 2);
        assert_eq!(YieldReason::DmaWait as u8, 3);
        assert_eq!(YieldReason::WaitingSync as u8, 4);
        assert_eq!(YieldReason::Syscall as u8, 5);
        assert_eq!(YieldReason::InterruptBoundary as u8, 6);
        assert_eq!(YieldReason::Fault as u8, 7);
        assert_eq!(YieldReason::Finished as u8, 8);
    }

    #[test]
    fn variants_are_distinct() {
        let all = [
            YieldReason::BudgetExhausted,
            YieldReason::MailboxAccess,
            YieldReason::DmaSubmitted,
            YieldReason::DmaWait,
            YieldReason::WaitingSync,
            YieldReason::Syscall,
            YieldReason::InterruptBoundary,
            YieldReason::Fault,
            YieldReason::Finished,
        ];
        let unique: std::collections::BTreeSet<u8> = all.iter().map(|y| *y as u8).collect();
        assert_eq!(unique.len(), all.len());
    }

    #[test]
    fn equality_is_reflexive_and_distinguishing() {
        assert_eq!(YieldReason::Fault, YieldReason::Fault);
        assert_ne!(YieldReason::Fault, YieldReason::Finished);
        assert_ne!(YieldReason::DmaSubmitted, YieldReason::DmaWait);
    }

    #[test]
    fn copy_semantics_hold() {
        let r = YieldReason::WaitingSync;
        let s = r;
        assert_eq!(r, s);
    }
}
