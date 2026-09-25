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
use cellgov_mem::lanes::{source, LaneMap, LaneValue, ObjectLanes};
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

/// A queued wake with its deadline, which is also the first half of its
/// key.
#[derive(Debug, Clone, Copy)]
struct Queued {
    deadline: GuestTicks,
    wake: TimerWake,
}

/// Lane fields of one wake:
///
/// 1. the deadline
/// 2. the unit
/// 3. the kind: 1 for a sleep, 2 for a sync wait
/// 4. and up: the block reason of a sync wait
impl LaneValue for Queued {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.deadline.raw());
        lanes.lane(2, 0, self.wake.unit.raw());
        match self.wake.kind {
            TimerWakeKind::Sleep => lanes.lane(3, 0, 1),
            TimerWakeKind::SyncWait(reason) => {
                lanes.lane(3, 0, 2);
                reason.push_lanes(lanes, 4);
            }
        }
    }
}

/// Deterministic priority queue of pending timer wakes.
///
/// At most one live entry per unit: a unit is blocked on at most one
/// wait at a time, so a second insert before a cancel is an upstream
/// wake-path bug. Participates in the runtime's `sync_state_hash`.
#[derive(Debug, Clone)]
pub struct TimerWakeQueue {
    entries: LaneMap<(GuestTicks, u64), Queued>,
    /// Reverse index for O(log n) cancel on early wake.
    by_unit: BTreeMap<UnitId, (GuestTicks, u64)>,
    next_seq: u64,
    /// Release-mode displacement count; not part of the sync-state hash.
    displacement_count: usize,
}

impl Default for TimerWakeQueue {
    fn default() -> Self {
        Self {
            entries: LaneMap::new(source::TIMER_WAKE, |(_, seq)| seq),
            by_unit: BTreeMap::new(),
            next_seq: 0,
            displacement_count: 0,
        }
    }
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

    /// Register a wake for `unit` at `deadline` and return the entry it
    /// displaced.
    ///
    /// A returned entry is an upstream wake-path bug. The caller
    /// records the invariant break; the queue only counts it.
    ///
    /// # Panics
    ///
    /// Debug builds panic when `unit` already has a pending wake.
    /// Release builds bump [`Self::displacement_count`], cancel the
    /// prior entry, and register the new one.
    pub fn insert(
        &mut self,
        deadline: GuestTicks,
        unit: UnitId,
        kind: TimerWakeKind,
    ) -> Option<TimerWake> {
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
        let mut displaced = None;
        if let Some(prior_key) = self.by_unit.remove(&unit) {
            displaced = self.entries.remove(prior_key).map(|q| q.wake);
            self.displacement_count = self.displacement_count.saturating_add(1);
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.entries.insert(
            (deadline, seq),
            Queued {
                deadline,
                wake: TimerWake { unit, kind },
            },
        );
        self.by_unit.insert(unit, (deadline, seq));
        displaced
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
                let removed = self.entries.remove(key);
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
        self.entries.first().map(|((deadline, _), _)| deadline)
    }

    /// Drain every wake with `deadline <= now`, in `(deadline,
    /// sequence)` order.
    pub fn pop_due(&mut self, now: GuestTicks) -> Vec<TimerWake> {
        let mut due = Vec::new();
        while self
            .entries
            .first()
            .is_some_and(|((deadline, _), _)| deadline <= now)
        {
            if let Some((_, queued)) = self.entries.pop_first() {
                due.push(queued.wake);
            }
        }
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

    /// The queue's partial of the sync-state sum, with the sequence
    /// number of each pending wake as its object.
    #[inline]
    pub fn sync_partial(&self) -> u128 {
        self.entries.partial()
    }

    /// [`Self::sync_partial`] computed from every entry.
    pub fn sync_partial_from_scratch(&self) -> u128 {
        self.entries.partial_from_scratch()
    }
}

#[cfg(test)]
#[path = "tests/timer_queue_tests.rs"]
mod tests;

/// A debug build panics in `insert` before a displacement can return
/// anything.
#[cfg(all(test, not(debug_assertions)))]
#[path = "tests/timer_queue_displacement_tests.rs"]
mod displacement_tests;
