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
mod tests {
    use super::*;
    use crate::ppu_thread::{PpuThreadAttrs, PpuThreadState, PpuThreadTable};
    use cellgov_event::UnitId;

    fn dummy_attrs() -> PpuThreadAttrs {
        PpuThreadAttrs {
            entry: 0x10_0000,
            arg: 0,
            stack_base: 0xD000_0000,
            stack_size: 0x10000,
            priority: 1000,
            tls_base: 0x0020_0000,
        }
    }

    #[test]
    fn event_flag_wait_mode_stable_tag_is_injective() {
        let tags = [
            EventFlagWaitMode::AndNoClear.stable_tag(),
            EventFlagWaitMode::AndClear.stable_tag(),
            EventFlagWaitMode::OrNoClear.stable_tag(),
            EventFlagWaitMode::OrClear.stable_tag(),
        ];
        let mut seen = std::collections::BTreeSet::new();
        for t in tags {
            assert!(seen.insert(t), "duplicate stable_tag value: {t}");
        }
        assert_eq!(seen.len(), 4);
    }

    #[test]
    fn guest_block_reason_stable_tag_is_injective_and_nonzero() {
        let reasons = [
            GuestBlockReason::WaitingOnJoin {
                target: PpuThreadId::PRIMARY,
            },
            GuestBlockReason::WaitingOnLwMutex { id: 1 },
            GuestBlockReason::WaitingOnMutex { id: 1 },
            GuestBlockReason::WaitingOnSemaphore { id: 1 },
            GuestBlockReason::WaitingOnEventQueue { id: 1 },
            GuestBlockReason::WaitingOnEventFlag {
                id: 1,
                mask: 0,
                mode: EventFlagWaitMode::AndNoClear,
            },
            GuestBlockReason::WaitingOnCond {
                cond_id: 1,
                mutex_id: 1,
            },
        ];
        let mut seen = std::collections::BTreeSet::new();
        for r in reasons {
            let t = r.stable_tag();
            assert_ne!(t, 0, "reason tag collides with Runnable lifecycle tag");
            assert!(seen.insert(t), "duplicate reason tag {t}");
        }
        assert_eq!(seen.len(), 7);
    }

    #[test]
    fn blocked_state_carries_guest_reason() {
        let mut t = PpuThreadTable::new();
        let waiter = t.create(UnitId::new(2), dummy_attrs()).unwrap();
        let target = t.create(UnitId::new(3), dummy_attrs()).unwrap();
        t.get_mut(waiter).unwrap().state =
            PpuThreadState::Blocked(GuestBlockReason::WaitingOnJoin { target });
        match &t.get(waiter).unwrap().state {
            PpuThreadState::Blocked(GuestBlockReason::WaitingOnJoin { target: tgt }) => {
                assert_eq!(*tgt, target);
            }
            other => panic!("expected WaitingOnJoin, got {other:?}"),
        }
    }

    #[test]
    fn all_guest_block_reason_variants_round_trip_through_blocked_state() {
        let mut t = PpuThreadTable::new();
        let waiter = t.create(UnitId::new(2), dummy_attrs()).unwrap();
        let reasons = [
            GuestBlockReason::WaitingOnJoin {
                target: PpuThreadId::PRIMARY,
            },
            GuestBlockReason::WaitingOnLwMutex { id: 7 },
            GuestBlockReason::WaitingOnMutex { id: 7 },
            GuestBlockReason::WaitingOnSemaphore { id: 7 },
            GuestBlockReason::WaitingOnEventQueue { id: 7 },
            GuestBlockReason::WaitingOnEventFlag {
                id: 7,
                mask: 0xF0F0,
                mode: EventFlagWaitMode::AndClear,
            },
            GuestBlockReason::WaitingOnCond {
                cond_id: 7,
                mutex_id: 8,
            },
        ];
        for reason in reasons {
            t.get_mut(waiter).unwrap().state = PpuThreadState::Blocked(reason);
            match &t.get(waiter).unwrap().state {
                PpuThreadState::Blocked(stored) => assert_eq!(*stored, reason),
                other => panic!("expected Blocked, got {other:?}"),
            }
        }
    }
}
