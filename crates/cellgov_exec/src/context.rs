//! The readonly view exposed to a running execution unit.
//!
//! The shared memory view is frozen for the entire duration of a
//! single `run_until_yield` call. A unit cannot observe commits made
//! by other units mid-step; new commits become visible only on the
//! unit's next scheduled invocation. The freeze is enforced
//! structurally by the borrow checker: `ExecutionContext` holds an
//! immutable borrow of `GuestMemory`, and `run_until_yield` takes
//! `&ExecutionContext`.

use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_sync::ReservationTable;
use cellgov_time::GuestTicks;

/// Readonly view of runtime state passed into `run_until_yield`.
///
/// Units publish changes only by emitting `Effect` packets in their
/// step result; there is no mutable access through the context.
/// Constructors are named (`new`, `with_received`, `with_syscall_return`,
/// `with_syscall_return_and_regs`, `with_reservations`) so that
/// adding a borrowed field is a non-breaking change.
#[derive(Debug, Clone, Copy)]
pub struct ExecutionContext<'a> {
    memory: &'a GuestMemory,
    received: &'a [u32],
    syscall_return: Option<u64>,
    register_writes: &'a [(u8, u64)],
    reservations: Option<&'a ReservationTable>,
    current_tick: GuestTicks,
    /// When `true`, units that support per-instruction state
    /// fingerprinting MUST capture `(pc, state_hash)` on every
    /// retired instruction so the runtime can drain them via
    /// `drain_retired_state_hashes`. The runtime sets this from its
    /// own `mode`; default is `false` so the FaultDriven hot path
    /// pays no hashing cost.
    trace_per_step: bool,
}

impl<'a> ExecutionContext<'a> {
    /// Context over the given committed memory with no pending inputs.
    #[inline]
    pub const fn new(memory: &'a GuestMemory) -> Self {
        Self {
            memory,
            received: &[],
            syscall_return: None,
            register_writes: &[],
            reservations: None,
            current_tick: GuestTicks::ZERO,
            trace_per_step: false,
        }
    }

    /// Context carrying messages the runtime drained for this unit
    /// during the preceding commit cycle.
    #[inline]
    pub fn with_received(memory: &'a GuestMemory, received: &'a [u32]) -> Self {
        Self {
            memory,
            received,
            syscall_return: None,
            register_writes: &[],
            reservations: None,
            current_tick: GuestTicks::ZERO,
            trace_per_step: false,
        }
    }

    /// Context for resuming a unit whose previous step yielded
    /// `YieldReason::Syscall` and was serviced with an immediate
    /// return. The unit writes `code` into its syscall-return
    /// register and advances past the syscall instruction.
    #[inline]
    pub fn with_syscall_return(memory: &'a GuestMemory, received: &'a [u32], code: u64) -> Self {
        Self {
            memory,
            received,
            syscall_return: Some(code),
            register_writes: &[],
            reservations: None,
            current_tick: GuestTicks::ZERO,
            trace_per_step: false,
        }
    }

    /// Variant of [`Self::with_syscall_return`] for HLE stubs that
    /// also need to write registers beyond the return register
    /// (e.g. TLS setup).
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
            current_tick: GuestTicks::ZERO,
            trace_per_step: false,
        }
    }

    /// Attach the runtime's current guest-tick count, replacing any
    /// prior value. Default is `GuestTicks::ZERO`; the runtime sets
    /// this to `Runtime::time()` before each step so PPU code paths
    /// that read the TB register see a monotonic counter coherent
    /// with `sys_time_get_current_time`.
    #[inline]
    pub const fn with_current_tick(self, current_tick: GuestTicks) -> Self {
        Self {
            current_tick,
            ..self
        }
    }

    /// Attach the committed reservation table, replacing any prior
    /// reservation view. Chainable because `ExecutionContext` is `Copy`.
    #[inline]
    pub const fn with_reservations(self, table: &'a ReservationTable) -> Self {
        Self {
            reservations: Some(table),
            ..self
        }
    }

    /// Set the per-instruction trace flag. Units that fingerprint
    /// state read this from the context every step, so the runtime's
    /// mode (`FullTrace` / `DeterminismCheck`) is the single source
    /// of truth and unit registration / mode flips do not need to
    /// resync any per-unit flag.
    #[inline]
    pub const fn with_trace_per_step(self, on: bool) -> Self {
        Self {
            trace_per_step: on,
            ..self
        }
    }

    /// Committed memory view, borrowed for the step's lifetime.
    #[inline]
    pub const fn memory(&self) -> &GuestMemory {
        self.memory
    }

    /// Committed-state half of the conditional-store verdict: whether
    /// `unit` currently holds a reservation per the installed table.
    /// Returns `false` when no table was attached via
    /// [`Self::with_reservations`]. The unit's own local reservation
    /// register is the other half; `stwcx` / `putllc` succeed only
    /// when both agree.
    #[inline]
    pub fn reservation_held(&self, unit: UnitId) -> bool {
        match self.reservations {
            Some(table) => table.is_held_by(unit),
            None => false,
        }
    }

    /// Messages delivered to this unit by the runtime during the
    /// preceding commit cycle, in delivery order.
    #[inline]
    pub const fn received_messages(&self) -> &[u32] {
        self.received
    }

    /// Syscall return code from the LV2 host, if the unit's prior
    /// step yielded `YieldReason::Syscall` and the runtime serviced
    /// it immediately.
    #[inline]
    pub const fn syscall_return(&self) -> Option<u64> {
        self.syscall_return
    }

    /// Extra `(gpr_index, value)` writes accompanying a syscall
    /// return, for HLE stubs that touch registers beyond the return
    /// register.
    #[inline]
    pub const fn register_writes(&self) -> &[(u8, u64)] {
        self.register_writes
    }

    /// Runtime's current guest-tick count at the start of this step.
    #[inline]
    pub const fn current_tick(&self) -> GuestTicks {
        self.current_tick
    }

    /// Whether the runtime wants per-instruction `(pc, state_hash)`
    /// fingerprints captured this step. Units that support tracing
    /// read this and gate their `per_step_hashes` push on it; units
    /// that do not just ignore it.
    #[inline]
    pub const fn trace_per_step(&self) -> bool {
        self.trace_per_step
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
