//! The readonly view exposed to a running execution unit.
//!
//! The shared memory view is frozen for the duration of a single
//! `run_until_yield` call; new commits become visible only on the
//! unit's next scheduled invocation. Enforced structurally via the
//! immutable borrow of `GuestMemory` held by `ExecutionContext`.

use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_sync::ReservationTable;
use cellgov_time::GuestTicks;

/// Readonly view of runtime state passed into `run_until_yield`.
///
/// Units publish changes only by emitting `Effect` packets in their
/// step result. The one write through the context is the flag
/// [`Self::inbound_mailbox`] and [`Self::mailbox_occupancy`] set, which
/// records a read and carries no guest state.
#[derive(Debug, Clone, Copy)]
pub struct ExecutionContext<'a> {
    memory: &'a GuestMemory,
    received: &'a [u32],
    syscall_return: Option<u64>,
    register_writes: &'a [(u8, u64)],
    reservations: Option<&'a ReservationTable>,
    current_tick: GuestTicks,
    /// When `true`, units that support per-instruction state
    /// fingerprinting must capture `(pc, state_hash)` on every
    /// retired instruction so the runtime can drain them via
    /// `drain_retired_state_hashes`.
    trace_per_step: bool,
    outstanding_dma_tags: u32,
    list_stall_tags: u32,
    dma_queue_occupancy: u32,
    inbound_mailbox: &'a [u32],
    /// Set when the unit reads [`Self::inbound_mailbox`], so the
    /// runtime knows the step read its mailbox. It records what the
    /// unit observed and carries no guest state.
    mailbox_read: Option<&'a core::cell::Cell<bool>>,
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
            outstanding_dma_tags: 0,
            list_stall_tags: 0,
            dma_queue_occupancy: 0,
            inbound_mailbox: &[],
            mailbox_read: None,
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
            outstanding_dma_tags: 0,
            list_stall_tags: 0,
            dma_queue_occupancy: 0,
            inbound_mailbox: &[],
            mailbox_read: None,
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
            outstanding_dma_tags: 0,
            list_stall_tags: 0,
            dma_queue_occupancy: 0,
            inbound_mailbox: &[],
            mailbox_read: None,
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
            outstanding_dma_tags: 0,
            list_stall_tags: 0,
            dma_queue_occupancy: 0,
            inbound_mailbox: &[],
            mailbox_read: None,
        }
    }

    /// Attach the runtime's current guest-tick count, replacing any
    /// prior value.
    #[inline]
    pub const fn with_current_tick(self, current_tick: GuestTicks) -> Self {
        Self {
            current_tick,
            ..self
        }
    }

    /// Attach the committed reservation table, replacing any prior
    /// reservation view.
    #[inline]
    pub const fn with_reservations(self, table: &'a ReservationTable) -> Self {
        Self {
            reservations: Some(table),
            ..self
        }
    }

    /// Set the per-instruction trace flag.
    #[inline]
    pub const fn with_trace_per_step(self, on: bool) -> Self {
        Self {
            trace_per_step: on,
            ..self
        }
    }

    /// Tag groups with a transfer of this unit's still outstanding at the
    /// start of the step, one bit per group.
    ///
    /// A transfer stays outstanding until it leaves the DMA queue.
    #[inline]
    pub const fn with_outstanding_dma_tags(self, bits: u32) -> Self {
        Self {
            outstanding_dma_tags: bits,
            ..self
        }
    }

    /// Tag groups with a queued stall-and-notify list element of this
    /// unit's, one bit per group.
    ///
    /// The element's list stalls once the element leaves the queue.
    #[inline]
    pub const fn with_list_stall_tags(self, bits: u32) -> Self {
        Self {
            list_stall_tags: bits,
            ..self
        }
    }

    /// Number of the unit's MFC commands queued and not yet complete at
    /// the start of the step.
    #[inline]
    pub const fn with_dma_queue_occupancy(self, count: u32) -> Self {
        Self {
            dma_queue_occupancy: count,
            ..self
        }
    }

    /// The messages waiting in the unit's own inbound mailbox at the
    /// start of the step, oldest first.
    #[inline]
    pub const fn with_inbound_mailbox(self, messages: &'a [u32]) -> Self {
        Self {
            inbound_mailbox: messages,
            ..self
        }
    }

    /// Attach the flag [`Self::inbound_mailbox`] sets when the unit
    /// reads it.
    #[inline]
    pub const fn with_mailbox_read_flag(self, flag: &'a core::cell::Cell<bool>) -> Self {
        Self {
            mailbox_read: Some(flag),
            ..self
        }
    }

    /// The messages waiting in the unit's own inbound mailbox at the
    /// start of the step, oldest first.
    #[inline]
    pub fn inbound_mailbox(&self) -> &'a [u32] {
        if let Some(flag) = self.mailbox_read {
            flag.set(true);
        }
        self.inbound_mailbox
    }

    /// Number of messages waiting in the unit's own inbound mailbox at
    /// the start of the step.
    #[inline]
    pub fn mailbox_occupancy(&self) -> u32 {
        u32::try_from(self.inbound_mailbox().len()).unwrap_or(u32::MAX)
    }

    /// Tag groups with a transfer of this unit's still outstanding.
    #[inline]
    pub const fn outstanding_dma_tags(&self) -> u32 {
        self.outstanding_dma_tags
    }

    /// Tag groups with a queued stall-and-notify list element of this
    /// unit's.
    #[inline]
    pub const fn list_stall_tags(&self) -> u32 {
        self.list_stall_tags
    }

    /// Number of this unit's MFC commands queued and not yet complete.
    #[inline]
    pub const fn dma_queue_occupancy(&self) -> u32 {
        self.dma_queue_occupancy
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
#[path = "tests/context_tests.rs"]
mod tests;
