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

use cellgov_mem::GuestMemory;

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
        }
    }

    /// Borrow the committed memory view. The returned reference shares
    /// this context's lifetime, so it cannot outlive the step.
    #[inline]
    pub const fn memory(&self) -> &GuestMemory {
        self.memory
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
}
