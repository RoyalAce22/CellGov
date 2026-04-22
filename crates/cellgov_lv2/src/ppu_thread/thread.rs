//! Per-thread vocabulary: lifecycle state, creation attrs, join outcome, and record.

use super::block_reason::GuestBlockReason;
use super::id::PpuThreadId;
use cellgov_event::UnitId;

/// Lifecycle state of a PPU thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PpuThreadState {
    /// Ready to run.
    Runnable,
    /// Parked on a guest-LV2 condition.
    Blocked(GuestBlockReason),
    /// Called `sys_ppu_thread_exit`; exit value is available.
    Finished,
    /// Explicitly detached; resources released on exit without
    /// a join.
    Detached,
}

/// Attributes captured from `sys_ppu_thread_create`.
#[derive(Debug, Clone)]
pub struct PpuThreadAttrs {
    /// Guest entry-point PC resolved from the OPD's first word
    /// (what the PPC64 ABI loads into NIP).
    pub entry: u64,
    /// Argument passed in `r3` at entry.
    pub arg: u64,
    /// Lowest address of the child's stack region; stack grows
    /// downward from `stack_base + stack_size`.
    pub stack_base: u32,
    /// Size in bytes of the child's stack region.
    pub stack_size: u32,
    /// Scheduling priority. Captured but not consulted by the
    /// current round-robin scheduler.
    pub priority: u32,
    /// Base address of the child's per-thread TLS block.
    /// Host-chosen, not guest-provided.
    pub tls_base: u32,
}

/// Outcome of [`super::PpuThreadTable::add_join_waiter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddJoinWaiter {
    /// Waiter enqueued.
    Parked,
    /// Target is not in the table.
    UnknownTarget,
    /// Caller attempted to park a thread on itself.
    SelfJoin,
    /// Target is already `Finished`.
    TargetAlreadyFinished,
    /// Target is `Detached`; no joiner will ever wake.
    TargetDetached,
}

/// A single PPU thread tracked by the host.
#[derive(Debug, Clone)]
pub struct PpuThread {
    /// Guest-facing thread id.
    pub id: PpuThreadId,
    /// Runtime execution-unit id.
    pub unit_id: UnitId,
    /// Current lifecycle state.
    pub state: PpuThreadState,
    /// Creation attributes. Immutable after create.
    pub attrs: PpuThreadAttrs,
    /// Value returned via `sys_ppu_thread_exit`, if any.
    pub exit_value: Option<u64>,
    /// Drained by `mark_finished`; appended by
    /// `add_join_waiter`.
    pub join_waiters: Vec<PpuThreadId>,
}
