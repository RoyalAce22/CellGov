//! Why an execution unit handed control back to the runtime.
//!
//! `YieldReason` is the closed set of reasons a unit's `run_until_yield`
//! call can return. There are nine variants, explicitly **not** tied
//! to any specific game or instruction mnemonic. New architectural
//! reasons (PPU-specific, SPU-specific, RSX-specific) belong in
//! architecture crates as wrapper enums on top of this set, not as
//! additional variants here.
//!
//! Discriminants are locked. The trace format is binary from day one;
//! reordering or renumbering variants would break replay against any
//! existing trace.

/// The reason an execution unit yielded control back to the runtime.
///
/// Variant order and `#[repr(u8)]` discriminants below are part of the
/// runtime's stable trace contract. Do not reorder, do not insert
/// variants in the middle, do not renumber. New variants must be
/// appended at the end with explicit discriminants strictly greater
/// than [`YieldReason::Finished`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum YieldReason {
    /// The scheduler-granted budget was exhausted before the unit could
    /// reach a more meaningful yield point. The unit is still runnable
    /// and will be re-scheduled at the next opportunity.
    BudgetExhausted = 0,
    /// The unit attempted a mailbox operation that needs runtime
    /// arbitration: a send into a full mailbox, a receive from an empty
    /// one, or any access whose visibility depends on the commit
    /// pipeline.
    MailboxAccess = 1,
    /// The unit submitted a DMA request and is yielding so the runtime
    /// can stage the request and decide its modeled completion time.
    /// The unit is not necessarily blocked -- it may continue running
    /// after the request is acknowledged.
    DmaSubmitted = 2,
    /// The unit is blocked waiting for a previously-submitted DMA
    /// transfer to reach its modeled completion time.
    DmaWait = 3,
    /// The unit is blocked on a sync primitive: a signal, a barrier, a
    /// mutex, or any state machine in `cellgov_sync` that produced a
    /// block condition.
    WaitingSync = 4,
    /// The unit invoked an abstract guest syscall and is yielding so
    /// the runtime can route the request to its handler. No concrete
    /// syscall set exists yet; this variant is the seam.
    Syscall = 5,
    /// The unit reached a clean point at which an interrupt could be
    /// delivered. The runtime may inject pending interrupts before
    /// resuming the unit.
    InterruptBoundary = 6,
    /// The unit raised a fault. The step's effects are discarded
    /// wholesale (the fault rule), the fault is recorded in the
    /// trace, and the unit transitions to its fault-handling state.
    Fault = 7,
    /// The unit finished its work and will not be scheduled again.
    /// Terminal -- the runtime should remove it from the runnable set
    /// after observing this reason.
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
        // A small set guards against accidental duplicate discriminants
        // by leveraging Hash + Eq. If two variants ever collapsed to
        // the same discriminant, this set would shrink.
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
        // Both still usable; this would not compile if Copy were absent.
        assert_eq!(r, s);
    }
}
