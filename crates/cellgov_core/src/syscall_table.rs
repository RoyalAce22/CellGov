//! Per-unit table of pending syscall responses.
//!
//! When a unit blocks on a syscall, the LV2 host produces a
//! `PendingResponse` describing the wake action (return code, join
//! out-pointer writes, event payload delivery, etc.). This table owns
//! those records between block and wake, keyed by `UnitId`.

use cellgov_event::UnitId;
use cellgov_lv2::PendingResponse;
use cellgov_mem::lanes::{source, LaneMap};

/// Pending-response table for blocked syscall callers.
///
/// At most one response per unit. Participates in the runtime's
/// `sync_state_hash`.
#[derive(Debug, Clone)]
pub struct SyscallResponseTable {
    pending: LaneMap<UnitId, PendingResponse>,
    /// Release-mode displacement count, read through
    /// [`Self::displacement_count`]. It is outside the sync-state hash.
    displacement_count: usize,
}

impl Default for SyscallResponseTable {
    fn default() -> Self {
        Self {
            pending: LaneMap::new(source::SYSCALL_RESPONSE, UnitId::raw),
            displacement_count: 0,
        }
    }
}

/// Debug-only runaway guard for [`SyscallResponseTable::insert`].
/// Parallel to the scheduler's runnables cap.
const MAX_PENDING_RESPONSES: usize = 65_536;

impl SyscallResponseTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a pending response for `unit`, returning any displaced entry.
    ///
    /// Contract: a unit is blocked on at most one syscall at a time,
    /// so `unit` must not already have a pending response.
    ///
    /// # Panics
    ///
    /// Debug builds panic when a prior entry exists. Release builds
    /// bump [`Self::displacement_count`] and return the displaced
    /// response. The caller records the invariant break; the table
    /// only counts it.
    #[must_use = "insert may displace an existing pending response; handle the Some case \
                  (the displaced response carries an owed r3 and possible out-pointer \
                  writes that will otherwise be silently lost)"]
    pub fn insert(&mut self, unit: UnitId, response: PendingResponse) -> Option<PendingResponse> {
        debug_assert!(
            !self.pending.contains_key(unit),
            "SyscallResponseTable::insert: unit {unit:?} already has a pending response; \
             a silent overwrite would lose the original r3 and any owed out-pointer writes. \
             Call try_take() first if this replacement is intentional."
        );
        debug_assert!(
            self.pending.len() < MAX_PENDING_RESPONSES,
            "SyscallResponseTable::insert: pending-response count exceeded {MAX_PENDING_RESPONSES}; \
             wake path is likely not firing"
        );
        let displaced = self.pending.insert(unit, response);
        if displaced.is_some() {
            self.displacement_count = self.displacement_count.saturating_add(1);
        }
        displaced
    }

    /// Total release-mode displacements observed by [`Self::insert`].
    #[inline]
    pub fn displacement_count(&self) -> usize {
        self.displacement_count
    }

    /// Remove and return the pending response for `unit`, if any.
    ///
    /// `None` is ambiguous (never blocked vs already drained); prefer
    /// [`Self::take_expected`] at call sites where presence is a
    /// runtime contract.
    pub fn try_take(&mut self, unit: UnitId) -> Option<PendingResponse> {
        self.pending.remove(unit)
    }

    /// Remove and return the pending response for `unit`.
    ///
    /// # Panics
    ///
    /// Panics if no response is present; a missing entry indicates a
    /// double-wake or a missing upstream insert.
    pub fn take_expected(&mut self, unit: UnitId) -> PendingResponse {
        self.pending.remove(unit).unwrap_or_else(|| {
            panic!(
                "SyscallResponseTable::take_expected: no pending response for {unit:?}; \
                 probable double-wake or missing insert"
            )
        })
    }

    /// Borrow the pending response for `unit` without removing it.
    pub fn peek(&self, unit: UnitId) -> Option<&PendingResponse> {
        self.pending.get(unit)
    }

    /// Check whether `unit` has a pending response.
    pub fn contains(&self, unit: UnitId) -> bool {
        self.pending.contains_key(unit)
    }

    /// Number of pending responses.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Iterate pending unit ids in ascending order.
    pub fn pending_ids(&self) -> impl Iterator<Item = UnitId> + '_ {
        self.pending.iter().map(|(unit, _)| unit)
    }

    /// The table's partial of the sync-state sum, with the unit of each
    /// pending response as its object.
    #[inline]
    pub fn sync_partial(&self) -> u128 {
        self.pending.partial()
    }

    /// [`Self::sync_partial`] computed from every entry.
    pub fn sync_partial_from_scratch(&self) -> u128 {
        self.pending.partial_from_scratch()
    }
}

#[cfg(test)]
#[path = "tests/syscall_table_tests.rs"]
mod tests;
