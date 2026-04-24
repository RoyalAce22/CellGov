//! Per-thread record: lifecycle state, creation attrs, join outcome.

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
    /// Called `sys_ppu_thread_exit`; `exit_value` is populated.
    Finished,
    /// Detached; resources release on exit without a join.
    Detached,
}

/// Attributes captured from `sys_ppu_thread_create`.
#[derive(Debug, Clone)]
pub struct PpuThreadAttrs {
    /// Entry-point PC resolved from the OPD's first word.
    pub entry: u64,
    /// Argument loaded into `r3` at entry.
    pub arg: u64,
    /// Lowest address of the child's stack region; stack grows
    /// down from `stack_base + stack_size`.
    pub stack_base: u32,
    /// Size in bytes of the child's stack region.
    pub stack_size: u32,
    /// Scheduling priority; not consulted by the round-robin
    /// scheduler.
    pub priority: u32,
    /// Host-chosen base of the child's per-thread TLS block.
    pub tls_base: u32,
}

/// Outcome of [`super::PpuThreadTable::add_join_waiter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddJoinWaiter {
    /// Waiter enqueued on the target.
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
    /// Creation attributes; immutable after create.
    pub attrs: PpuThreadAttrs,
    /// Exit value from `sys_ppu_thread_exit`.
    pub exit_value: Option<u64>,
    /// Appended by `add_join_waiter`; drained by `mark_finished`.
    pub join_waiters: Vec<PpuThreadId>,
}
