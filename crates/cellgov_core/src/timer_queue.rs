//! Deterministic queue of guest-tick wake deadlines for parked units.
//!
//! Entries are keyed by `(deadline, queue-assigned sequence)`, giving a
//! total order that preserves registration order among equal deadlines.
//! The runtime registers an entry when a unit parks with a finite
//! deadline, cancels it if the unit wakes early, and fires due entries
//! as guest time reaches their deadline -- either naturally as other
//! units retire instructions, or via the all-blocked time-warp in
//! `Runtime::step`.

use cellgov_event::UnitId;
use cellgov_lv2::Lv2BlockReason;
use cellgov_time::GuestTicks;
use std::collections::BTreeMap;

/// What to do with a unit when its deadline arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerWakeKind {
    /// `sys_timer_usleep` / `sys_timer_sleep`: wake with r3 = 0.
    Sleep,
    /// Timed sync-primitive wait: expire via `Lv2Host::expire_wait`,
    /// which unqueues the waiter and stages `CELL_ETIMEDOUT`.
    SyncWait(Lv2BlockReason),
}

/// A registered wake: the parked unit and its expiry action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimerWake {
    /// The parked unit to wake.
    pub unit: UnitId,
    /// Expiry action applied at fire time.
    pub kind: TimerWakeKind,
}

/// Debug-only runaway guard mirroring the syscall-response table's cap.
const MAX_TIMER_WAKES: usize = 65_536;

/// Wire-format version prepended to [`TimerWakeQueue::state_hash`].
/// Bump whenever the per-entry serialization in `state_hash` changes
/// shape, so old and new formats can never collide.
const STATE_HASH_FORMAT_VERSION: u64 = 2;

/// Deterministic priority queue of pending timer wakes.
///
/// At most one live entry per unit: a unit is blocked on at most one
/// wait at a time, so a second insert before a cancel is an upstream
/// wake-path bug. Participates in the runtime's `sync_state_hash`.
#[derive(Debug, Clone, Default)]
pub struct TimerWakeQueue {
    entries: BTreeMap<(GuestTicks, u64), TimerWake>,
    /// Reverse index for O(log n) cancel on early wake.
    by_unit: BTreeMap<UnitId, (GuestTicks, u64)>,
    next_seq: u64,
    /// Release-mode displacement count; not part of `state_hash`.
    displacement_count: usize,
}

impl TimerWakeQueue {
    /// Construct an empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of pending wakes.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the queue holds any pending wakes.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether `unit` has a pending wake.
    pub fn contains(&self, unit: UnitId) -> bool {
        self.by_unit.contains_key(&unit)
    }

    /// Register a wake for `unit` at `deadline`.
    ///
    /// # Panics
    ///
    /// Debug builds panic when `unit` already has a pending wake.
    /// Release builds log the first displacement to stderr (subsequent
    /// ones bump [`Self::displacement_count`] silently), cancel the
    /// prior entry, and register the new one.
    pub fn insert(&mut self, deadline: GuestTicks, unit: UnitId, kind: TimerWakeKind) {
        debug_assert!(
            !self.by_unit.contains_key(&unit),
            "TimerWakeQueue::insert: unit {unit:?} already has a pending wake; \
             the wake path that resolved its previous wait failed to cancel."
        );
        debug_assert!(
            self.entries.len() < MAX_TIMER_WAKES,
            "TimerWakeQueue::insert: pending-wake count exceeded {MAX_TIMER_WAKES}; \
             fire or cancel path is likely not running"
        );
        if let Some(prior_key) = self.by_unit.remove(&unit) {
            let prior = self.entries.remove(&prior_key);
            if self.displacement_count == 0 {
                #[allow(
                    clippy::print_stderr,
                    reason = "one-shot diagnostic for an invariant break: a unit re-parked with a live timer entry, meaning its previous wake never cancelled; gated to first occurrence so a runaway loop cannot flood stderr"
                )]
                {
                    eprintln!(
                        "TimerWakeQueue::insert: displaced pending wake for {unit:?}: \
                         {prior:?} (superseded by deadline={deadline:?} kind={kind:?}). \
                         Further displacements will be counted but not logged; inspect \
                         displacement_count() for the total."
                    );
                }
            }
            self.displacement_count = self.displacement_count.saturating_add(1);
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.entries
            .insert((deadline, seq), TimerWake { unit, kind });
        self.by_unit.insert(unit, (deadline, seq));
    }

    /// Total release-mode displacements observed by [`Self::insert`].
    pub fn displacement_count(&self) -> usize {
        self.displacement_count
    }

    /// Remove `unit`'s pending wake, returning whether one existed.
    ///
    /// Idempotent: most woken units never had a deadline, so a `false`
    /// return is the common case, not an error.
    pub fn cancel(&mut self, unit: UnitId) -> bool {
        match self.by_unit.remove(&unit) {
            Some(key) => {
                let removed = self.entries.remove(&key);
                debug_assert!(
                    removed.is_some(),
                    "TimerWakeQueue::cancel: by_unit index pointed at a missing entry \
                     for {unit:?}; the two maps diverged"
                );
                true
            }
            None => false,
        }
    }

    /// Earliest pending deadline, if any.
    pub fn peek_deadline(&self) -> Option<GuestTicks> {
        self.entries.keys().next().map(|(deadline, _)| *deadline)
    }

    /// Drain every wake with `deadline <= now`, in `(deadline,
    /// sequence)` order.
    pub fn pop_due(&mut self, now: GuestTicks) -> Vec<TimerWake> {
        let due: Vec<TimerWake> = match now.raw().checked_add(1) {
            Some(split_time) => {
                let after = self.entries.split_off(&(GuestTicks::new(split_time), 0));
                let due = std::mem::replace(&mut self.entries, after);
                due.into_values().collect()
            }
            None => {
                let all = std::mem::take(&mut self.entries);
                all.into_values().collect()
            }
        };
        for wake in &due {
            let removed = self.by_unit.remove(&wake.unit);
            debug_assert!(
                removed.is_some(),
                "TimerWakeQueue::pop_due: no by_unit index for popped {:?}; \
                 the two maps diverged",
                wake.unit
            );
        }
        due
    }

    /// FNV-1a hash of the queue contents.
    ///
    /// Wire format: `STATE_HASH_FORMAT_VERSION` (u64 LE), entry count
    /// (u64 LE), then for each entry in `(deadline, sequence)` order
    /// the deadline's u64 LE bytes, the sequence's u64 LE bytes, the
    /// unit id's u64 LE bytes, a 1-byte kind tag, and for `SyncWait`
    /// a 1-byte reason tag plus the reason's fixed-size fields. Any
    /// variable-length field added to a kind requires a new format
    /// version.
    ///
    /// Drift is pinned by `state_hash_wire_format_golden`.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&STATE_HASH_FORMAT_VERSION.to_le_bytes());
        hasher.write(&(self.entries.len() as u64).to_le_bytes());
        for ((deadline, seq), wake) in &self.entries {
            hasher.write(&deadline.raw().to_le_bytes());
            hasher.write(&seq.to_le_bytes());
            hasher.write(&wake.unit.raw().to_le_bytes());
            match wake.kind {
                TimerWakeKind::Sleep => hasher.write(&[0u8]),
                TimerWakeKind::SyncWait(reason) => {
                    hasher.write(&[1u8]);
                    hash_block_reason(&mut hasher, reason);
                }
            }
        }
        hasher.finish()
    }
}

/// Serialize a block reason for the state hash: 1-byte variant tag
/// plus the variant's fixed-size fields, all LE.
fn hash_block_reason(hasher: &mut cellgov_mem::Fnv1aHasher, reason: Lv2BlockReason) {
    match reason {
        Lv2BlockReason::ThreadGroupJoin { group_id } => {
            hasher.write(&[0u8]);
            hasher.write(&group_id.to_le_bytes());
        }
        Lv2BlockReason::PpuThreadJoin { target } => {
            hasher.write(&[1u8]);
            hasher.write(&target.to_le_bytes());
        }
        Lv2BlockReason::LwMutex { id } => {
            hasher.write(&[2u8]);
            hasher.write(&id.to_le_bytes());
        }
        Lv2BlockReason::Mutex { id } => {
            hasher.write(&[3u8]);
            hasher.write(&id.to_le_bytes());
        }
        Lv2BlockReason::Semaphore { id } => {
            hasher.write(&[4u8]);
            hasher.write(&id.to_le_bytes());
        }
        Lv2BlockReason::EventQueue { id } => {
            hasher.write(&[5u8]);
            hasher.write(&id.to_le_bytes());
        }
        Lv2BlockReason::EventFlag { id } => {
            hasher.write(&[6u8]);
            hasher.write(&id.to_le_bytes());
        }
        Lv2BlockReason::Cond {
            id,
            mutex_id,
            mutex_kind,
        } => {
            hasher.write(&[7u8]);
            hasher.write(&id.to_le_bytes());
            hasher.write(&mutex_id.to_le_bytes());
            hasher.write(&[mutex_kind as u8]);
        }
    }
}

#[cfg(test)]
#[path = "tests/timer_queue_tests.rs"]
mod tests;
