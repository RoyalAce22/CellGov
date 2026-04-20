//! `ExecutionContext` -- the readonly view exposed to a running unit.
//!
//! An execution context exposes only:
//!
//! - unit-local readable state needed to continue (lives in the unit, not here)
//! - readonly committed shared memory view
//! - readonly runtime-facing handles for querying abstract device state
//!
//! The determinism rule: the shared committed-memory view
//! must be **frozen** for the entire duration of a single `run_until_yield`
//! call. A unit cannot observe commits made by other units mid-step. Any
//! new commits become visible only on the unit's next scheduled
//! invocation. This eliminates read/observe nondeterminism by
//! construction.
//!
//! This crate enforces the freeze rule structurally: `ExecutionContext`
//! holds an immutable borrow of `GuestMemory`, and `run_until_yield`
//! takes `&ExecutionContext`. Rust's borrow checker ensures the runtime
//! cannot mutate the underlying memory while the context is alive,
//! which is exactly the duration of the unit's step.

use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_sync::ReservationTable;

/// The readonly view of runtime state exposed to an execution unit
/// during a single `run_until_yield` call.
///
/// `ExecutionContext` is narrow by construction. Everything it carries is
/// shared-borrowed from the runtime. There is no mutable access to
/// anything; units publish changes only by emitting `Effect` packets in
/// their step result, never by reaching through the context.
///
/// Exposes the committed shared memory view, any messages the
/// runtime delivered on the unit's behalf during the preceding
/// commit (e.g. from `MailboxReceiveAttempt`), an optional syscall
/// return code from a previous `YieldReason::Syscall` yield that
/// the runtime serviced, and any register writes injected by HLE
/// stubs alongside that return. Readonly runtime-facing handles
/// for querying abstract device state can be added as additional
/// borrowed fields when abstract devices exist; that is a
/// non-breaking addition because context construction goes through
/// named constructors (`new`, `with_received`, `with_syscall_return`,
/// `with_syscall_return_and_regs`) rather than exposing the struct
/// literal.
#[derive(Debug, Clone, Copy)]
pub struct ExecutionContext<'a> {
    memory: &'a GuestMemory,
    received: &'a [u32],
    syscall_return: Option<u64>,
    /// Register writes injected by HLE stubs alongside a syscall
    /// return. Each entry is `(gpr_index, value)`; the unit applies
    /// these in addition to writing the syscall return into r3.
    /// Empty unless the context was built with
    /// `with_syscall_return_and_regs`.
    register_writes: &'a [(u8, u64)],
    /// Read-only view of the committed atomic reservation table.
    /// `None` when the runtime has not installed a table (e.g. in
    /// microtests or contexts built before the reservation model is
    /// wired); in that case [`Self::reservation_held`] returns
    /// `false` for every unit. Set via
    /// [`Self::with_reservations`].
    reservations: Option<&'a ReservationTable>,
}

impl<'a> ExecutionContext<'a> {
    /// Construct an `ExecutionContext` over the given committed memory
    /// with no pending received messages and no syscall return.
    #[inline]
    pub const fn new(memory: &'a GuestMemory) -> Self {
        Self {
            memory,
            received: &[],
            syscall_return: None,
            register_writes: &[],
            reservations: None,
        }
    }

    /// Construct an `ExecutionContext` with pending received messages.
    /// The runtime calls this when `drain_receives` returned a
    /// non-empty vec for the unit about to run.
    #[inline]
    pub fn with_received(memory: &'a GuestMemory, received: &'a [u32]) -> Self {
        Self {
            memory,
            received,
            syscall_return: None,
            register_writes: &[],
            reservations: None,
        }
    }

    /// Construct an `ExecutionContext` with a syscall return code.
    /// The runtime calls this when the unit previously yielded with
    /// `YieldReason::Syscall` and the LV2 host produced an immediate
    /// response. The unit reads the return code via
    /// `syscall_return()` and writes it into its own r3.
    #[inline]
    pub fn with_syscall_return(memory: &'a GuestMemory, received: &'a [u32], code: u64) -> Self {
        Self {
            memory,
            received,
            syscall_return: Some(code),
            register_writes: &[],
            reservations: None,
        }
    }

    /// Construct an `ExecutionContext` with a syscall return code and
    /// additional register writes. Used by HLE stubs that need to set
    /// registers beyond r3 (e.g., r13 for TLS initialization).
    #[inline]
    pub fn with_syscall_return_and_regs(
        memory: &'a GuestMemory,
        received: &'a [u32],
        code: u64,
        register_writes: &'a [(u8, u64)],
    ) -> Self {
        Self {
            memory,
            received,
            syscall_return: Some(code),
            register_writes,
            reservations: None,
        }
    }

    /// Return a copy of this context with the atomic reservation
    /// table attached. Any prior reservation view is replaced.
    /// Chainable after any of the other constructors because
    /// `ExecutionContext` is `Copy`.
    #[inline]
    pub const fn with_reservations(self, table: &'a ReservationTable) -> Self {
        Self {
            memory: self.memory,
            received: self.received,
            syscall_return: self.syscall_return,
            register_writes: self.register_writes,
            reservations: Some(table),
        }
    }

    /// Borrow the committed memory view. The returned reference shares
    /// this context's lifetime, so it cannot outlive the step.
    #[inline]
    pub const fn memory(&self) -> &GuestMemory {
        self.memory
    }

    /// Whether the committed reservation table currently lists
    /// `unit` as holding an atomic reservation. Returns `false`
    /// when no reservation view has been attached to this context
    /// (`Self::with_reservations` was not called).
    ///
    /// The committed-state half of the conditional-store verdict.
    /// The unit's own local reservation register is the other
    /// half; a `stwcx` / `putllc` succeeds only when both signals
    /// say the reservation is held.
    #[inline]
    pub fn reservation_held(&self, unit: UnitId) -> bool {
        match self.reservations {
            Some(table) => table.is_held_by(unit),
            None => false,
        }
    }

    /// Messages delivered to this unit by the runtime during the
    /// preceding commit cycle (e.g. popped from a mailbox via
    /// `MailboxReceiveAttempt`). Empty if no messages were delivered.
    /// The slice is in delivery order.
    #[inline]
    pub const fn received_messages(&self) -> &[u32] {
        self.received
    }

    /// Syscall return code from the LV2 host, if the unit previously
    /// yielded with `YieldReason::Syscall` and the runtime serviced
    /// it with an `Lv2Dispatch::Immediate`. The unit should write
    /// this value into its r3 and advance PC past the `sc`.
    #[inline]
    pub const fn syscall_return(&self) -> Option<u64> {
        self.syscall_return
    }

    /// Register writes injected by HLE stubs. Each entry is
    /// (gpr_index, value). The unit applies these alongside the
    /// syscall return.
    #[inline]
    pub const fn register_writes(&self) -> &[(u8, u64)] {
        self.register_writes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
    use cellgov_sync::{ReservationTable, ReservedLine};

    fn range(start: u64, length: u64) -> ByteRange {
        ByteRange::new(GuestAddr::new(start), length).unwrap()
    }

    #[test]
    fn context_exposes_committed_memory() {
        let mut mem = GuestMemory::new(16);
        mem.apply_commit(range(0, 4), &[1, 2, 3, 4]).unwrap();
        let ctx = ExecutionContext::new(&mem);
        let bytes = ctx.memory().read(range(0, 4)).unwrap();
        assert_eq!(bytes, &[1, 2, 3, 4]);
    }

    #[test]
    fn context_is_copy() {
        let mem = GuestMemory::new(8);
        let ctx = ExecutionContext::new(&mem);
        let copy = ctx;
        // Both still usable; would not compile if Copy were absent.
        assert_eq!(ctx.memory().size(), copy.memory().size());
    }

    #[test]
    fn reservation_held_is_false_without_view() {
        let mem = GuestMemory::new(8);
        let ctx = ExecutionContext::new(&mem);
        assert!(!ctx.reservation_held(UnitId::new(0)));
        assert!(!ctx.reservation_held(UnitId::new(7)));
    }

    #[test]
    fn reservation_held_reads_installed_view() {
        let mem = GuestMemory::new(8);
        let mut table = ReservationTable::new();
        table.insert_or_replace(UnitId::new(3), ReservedLine::containing(0x1000));
        let ctx = ExecutionContext::new(&mem).with_reservations(&table);
        assert!(ctx.reservation_held(UnitId::new(3)));
        assert!(!ctx.reservation_held(UnitId::new(4)));
    }

    #[test]
    fn with_reservations_preserves_other_fields() {
        let mem = GuestMemory::new(8);
        let received = [7u32, 9];
        let writes: [(u8, u64); 1] = [(13, 0xfeedface)];
        let ctx = ExecutionContext::with_syscall_return_and_regs(&mem, &received, 42, &writes);
        let table = ReservationTable::new();
        let ctx = ctx.with_reservations(&table);
        assert_eq!(ctx.received_messages(), &[7, 9]);
        assert_eq!(ctx.syscall_return(), Some(42));
        assert_eq!(ctx.register_writes(), &[(13, 0xfeedface)]);
    }
}
