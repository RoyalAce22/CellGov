//! Per-unit table of pending syscall responses.
//!
//! When a unit blocks on a syscall, the LV2 host produces a
//! `PendingResponse` describing the wake action (return code, join
//! out-pointer writes, event payload delivery, etc.). This table owns
//! those records between block and wake, keyed by `UnitId`.

use cellgov_event::UnitId;
use cellgov_lv2::PendingResponse;
use std::collections::BTreeMap;

/// Pending-response table for blocked syscall callers.
///
/// At most one response per unit. Participates in the runtime's
/// `sync_state_hash`.
#[derive(Debug, Clone, Default)]
pub struct SyscallResponseTable {
    pending: BTreeMap<UnitId, PendingResponse>,
    /// Release-mode displacement count surfaced via
    /// [`Self::displacement_count`]; not part of `state_hash`.
    displacement_count: usize,
}

/// Debug-only runaway guard for [`SyscallResponseTable::insert`].
/// Parallel to the scheduler's runnables cap.
const MAX_PENDING_RESPONSES: usize = 65_536;

/// Wire-format version prepended to [`SyscallResponseTable::state_hash`].
/// Bumping this constant requires updating
/// `tests::state_hash_wire_format_golden`'s `EXPECTED` in the same commit.
const STATE_HASH_FORMAT_VERSION: u64 = 4;

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
    /// log the first displacement to stderr (subsequent ones bump
    /// [`Self::displacement_count`] silently) and return the displaced
    /// response; `#[must_use]` forces the caller to acknowledge the
    /// owed r3 and out-pointer writes the displaced response carries.
    #[must_use = "insert may displace an existing pending response; handle the Some case \
                  (the displaced response carries an owed r3 and possible out-pointer \
                  writes that will otherwise be silently lost)"]
    pub fn insert(&mut self, unit: UnitId, response: PendingResponse) -> Option<PendingResponse> {
        debug_assert!(
            !self.pending.contains_key(&unit),
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
        if let Some(prev) = displaced.as_ref() {
            if self.displacement_count == 0 {
                let new_response = self
                    .pending
                    .get(&unit)
                    .expect("just-inserted response must be present");
                #[allow(
                    clippy::print_stderr,
                    reason = "one-shot diagnostic for an invariant break: a pending syscall response was overwritten before the runtime read it; gated to first occurrence so a runaway loop cannot flood stderr"
                )]
                {
                    eprintln!(
                        "SyscallResponseTable::insert: displaced pending response for {unit:?}: \
                         {prev:?} (overwritten by {new_response:?}) -- original r3 and any owed \
                         out-pointer writes are lost. Further displacements in this table will be \
                         counted but not logged; inspect displacement_count() for the total."
                    );
                }
            }
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
        self.pending.remove(&unit)
    }

    /// Remove and return the pending response for `unit`.
    ///
    /// # Panics
    ///
    /// Panics if no response is present; a missing entry indicates a
    /// double-wake or a missing upstream insert.
    #[allow(dead_code)]
    pub fn take_expected(&mut self, unit: UnitId) -> PendingResponse {
        self.pending.remove(&unit).unwrap_or_else(|| {
            panic!(
                "SyscallResponseTable::take_expected: no pending response for {unit:?}; \
                 probable double-wake or missing insert"
            )
        })
    }

    /// Borrow the pending response for `unit` without removing it.
    pub fn peek(&self, unit: UnitId) -> Option<&PendingResponse> {
        self.pending.get(&unit)
    }

    /// Check whether `unit` has a pending response.
    pub fn contains(&self, unit: UnitId) -> bool {
        self.pending.contains_key(&unit)
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
        self.pending.keys().copied()
    }

    /// FNV-1a hash of the table contents.
    ///
    /// Wire format: `STATE_HASH_FORMAT_VERSION` (u64 LE), entry count
    /// (u64 LE), then for each `(UnitId, PendingResponse)` pair in
    /// ascending id order the id's u64 LE bytes, a 1-byte variant tag
    /// (0..=5), and the variant's fixed-size fields. Any variable-length
    /// field added to a variant requires a new format version.
    ///
    /// Drift is pinned by `state_hash_wire_format_golden` and by
    /// `state_hash_is_insertion_order_independent`.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&STATE_HASH_FORMAT_VERSION.to_le_bytes());
        hasher.write(&(self.pending.len() as u64).to_le_bytes());
        for (unit, response) in &self.pending {
            hasher.write(&unit.raw().to_le_bytes());
            match response {
                PendingResponse::ReturnCode { code } => {
                    hasher.write(&[0u8]);
                    hasher.write(&code.to_le_bytes());
                }
                PendingResponse::ThreadGroupJoin {
                    group_id,
                    code,
                    cause_ptr,
                    status_ptr,
                    cause,
                    status,
                } => {
                    hasher.write(&[1u8]);
                    hasher.write(&group_id.to_le_bytes());
                    hasher.write(&code.to_le_bytes());
                    hasher.write(&cause_ptr.to_le_bytes());
                    hasher.write(&status_ptr.to_le_bytes());
                    hasher.write(&cause.to_le_bytes());
                    hasher.write(&status.to_le_bytes());
                }
                PendingResponse::PpuThreadJoin {
                    target,
                    status_out_ptr,
                } => {
                    hasher.write(&[2u8]);
                    hasher.write(&target.to_le_bytes());
                    hasher.write(&status_out_ptr.to_le_bytes());
                }
                PendingResponse::EventQueueReceive { out_ptr, payload } => {
                    hasher.write(&[3u8]);
                    hasher.write(&out_ptr.to_le_bytes());
                    match payload {
                        None => hasher.write(&[0u8]),
                        Some(p) => {
                            hasher.write(&[1u8]);
                            hasher.write(&p.source.to_le_bytes());
                            hasher.write(&p.data1.to_le_bytes());
                            hasher.write(&p.data2.to_le_bytes());
                            hasher.write(&p.data3.to_le_bytes());
                        }
                    }
                }
                PendingResponse::CondWakeReacquire {
                    mutex_id,
                    mutex_kind,
                } => {
                    hasher.write(&[4u8]);
                    hasher.write(&mutex_id.to_le_bytes());
                    hasher.write(&[*mutex_kind as u8]);
                }
                PendingResponse::EventFlagWake {
                    result_ptr,
                    observed,
                } => {
                    hasher.write(&[5u8]);
                    hasher.write(&result_ptr.to_le_bytes());
                    hasher.write(&observed.to_le_bytes());
                }
                PendingResponse::LwMutexWake { mutex_ptr, caller } => {
                    hasher.write(&[6u8]);
                    hasher.write(&mutex_ptr.to_le_bytes());
                    hasher.write(&caller.to_le_bytes());
                }
            }
        }
        hasher.finish()
    }
}

#[cfg(test)]
#[path = "tests/syscall_table_tests.rs"]
mod tests;
