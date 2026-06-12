//! Guest block-reason vocabulary and its fixed-width payload encoding.

use super::id::PpuThreadId;

/// Why a PPU thread is currently blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestBlockReason {
    /// Waiting for `target` to call `sys_ppu_thread_exit`.
    WaitingOnJoin {
        /// Thread whose exit this waiter is parked on.
        target: PpuThreadId,
    },
    /// Waiting for `sys_lwmutex_unlock` on `id`.
    WaitingOnLwMutex {
        /// Lightweight mutex guest id.
        id: u32,
    },
    /// Waiting for `sys_mutex_unlock` on `id`.
    WaitingOnMutex {
        /// Heavy mutex guest id.
        id: u32,
    },
    /// Waiting for `sys_semaphore_post` on `id` to hand this
    /// waiter a slot.
    WaitingOnSemaphore {
        /// Semaphore guest id.
        id: u32,
    },
    /// Waiting for `sys_event_queue_send` to deliver a payload
    /// on `id`.
    WaitingOnEventQueue {
        /// Event queue guest id.
        id: u32,
    },
    /// Waiting for `sys_event_flag_set` that satisfies `mask`
    /// per `mode` on `id`.
    WaitingOnEventFlag {
        /// Event flag guest id.
        id: u32,
        /// Bit mask evaluated under `mode`.
        mask: u64,
        /// Match plus clear-on-wake policy.
        mode: EventFlagWaitMode,
    },
    /// Waiting for `sys_cond_signal` / `_signal_all` on
    /// `cond_id`; the wake path re-acquires `mutex_id` or parks
    /// on it if held.
    WaitingOnCond {
        /// Cond guest id.
        cond_id: u32,
        /// Heavy mutex released at `cond_wait` entry.
        mutex_id: u32,
    },
}

impl GuestBlockReason {
    /// Stable `u8` tag for determinism-sensitive hashing.
    ///
    /// Tags are 1..=7 so none coincide with the `Runnable`
    /// lifecycle tag 0. The match is exhaustive: a new variant
    /// that forgets to pick a tag fails to compile.
    pub fn stable_tag(&self) -> u8 {
        match self {
            GuestBlockReason::WaitingOnJoin { .. } => 1,
            GuestBlockReason::WaitingOnLwMutex { .. } => 2,
            GuestBlockReason::WaitingOnMutex { .. } => 3,
            GuestBlockReason::WaitingOnSemaphore { .. } => 4,
            GuestBlockReason::WaitingOnEventQueue { .. } => 5,
            GuestBlockReason::WaitingOnEventFlag { .. } => 6,
            GuestBlockReason::WaitingOnCond { .. } => 7,
        }
    }
}

/// Event-flag wait policy: mask-match semantics (AND/OR) crossed
/// with clear-on-wake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventFlagWaitMode {
    /// All bits in `mask` must be set; do not clear on wake.
    AndNoClear,
    /// All bits in `mask` must be set; clear the matched bits.
    AndClear,
    /// Any bit in `mask` must be set; do not clear on wake.
    OrNoClear,
    /// Any bit in `mask` must be set; clear the matched bits.
    OrClear,
}

impl EventFlagWaitMode {
    /// Stable `u8` tag for determinism-sensitive hashing.
    pub fn stable_tag(self) -> u8 {
        match self {
            EventFlagWaitMode::AndNoClear => 0,
            EventFlagWaitMode::AndClear => 1,
            EventFlagWaitMode::OrNoClear => 2,
            EventFlagWaitMode::OrClear => 3,
        }
    }
}

/// Width budget for [`block_reason_payload`].
///
/// Per-variant usage: `WaitingOnJoin` 8; lw/heavy/sem/queue 4;
/// `WaitingOnEventFlag` 4+8+1 = 13; `WaitingOnCond` 4+4 = 8. A
/// variant exceeding this panics in `copy_from_slice`; bump the
/// constant and propagate through the `[u8; N]` signature.
pub(super) const BLOCK_REASON_PAYLOAD_WIDTH: usize = 24;

/// Fixed-width payload encoding the non-tag fields of a
/// [`GuestBlockReason`].
pub(super) fn block_reason_payload(reason: &GuestBlockReason) -> [u8; BLOCK_REASON_PAYLOAD_WIDTH] {
    let mut p = [0u8; BLOCK_REASON_PAYLOAD_WIDTH];
    match *reason {
        GuestBlockReason::WaitingOnJoin { target } => {
            p[0..8].copy_from_slice(&target.raw().to_le_bytes());
        }
        GuestBlockReason::WaitingOnLwMutex { id } => {
            p[0..4].copy_from_slice(&id.to_le_bytes());
        }
        GuestBlockReason::WaitingOnMutex { id } => {
            p[0..4].copy_from_slice(&id.to_le_bytes());
        }
        GuestBlockReason::WaitingOnSemaphore { id } => {
            p[0..4].copy_from_slice(&id.to_le_bytes());
        }
        GuestBlockReason::WaitingOnEventQueue { id } => {
            p[0..4].copy_from_slice(&id.to_le_bytes());
        }
        GuestBlockReason::WaitingOnEventFlag { id, mask, mode } => {
            p[0..4].copy_from_slice(&id.to_le_bytes());
            p[4..12].copy_from_slice(&mask.to_le_bytes());
            p[12] = mode.stable_tag();
        }
        GuestBlockReason::WaitingOnCond { cond_id, mutex_id } => {
            p[0..4].copy_from_slice(&cond_id.to_le_bytes());
            p[4..8].copy_from_slice(&mutex_id.to_le_bytes());
        }
    }
    p
}

#[cfg(test)]
#[path = "tests/block_reason_tests.rs"]
mod tests;
