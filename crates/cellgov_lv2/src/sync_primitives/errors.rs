//! Shared error types raised by the sync-primitive tables.

use crate::ppu_thread::PpuThreadId;

/// [`super::WaiterList::enqueue`] rejection: `id` was already parked.
///
/// Callers must route this to `record_invariant_break`; ignoring
/// it drops the second wait's `PendingResponse`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("thread {id:?} already parked")]
pub struct DuplicateEnqueue {
    /// Thread id that was already parked.
    pub id: PpuThreadId,
}

/// Shared `create_with_id` rejection across [`super::mutex`],
/// [`super::event_flag`], [`super::semaphore`], and [`super::cond`].
/// Signals an allocator bug: the dispatch layer handed out an id
/// that was already live.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("id 0x{id:08x} already allocated")]
pub struct IdCollision {
    /// The id the dispatch layer attempted to (re-)allocate.
    pub id: u32,
}
