//! Wire-format hashes over the registry: runnable-queue hash and
//! full status hash, and the sync-state term of the queued unit inputs.
//! Pinned by golden tests so trace replay stays byte-identical across
//! code changes.

use cellgov_event::UnitId;
use cellgov_exec::{CompareResult, UnitStatus};
use cellgov_mem::lanes::{contribution, source, LaneIndex};

use super::status_lanes::{runnable_key, status_key, status_lane, LaneSums};
use super::UnitRegistry;

impl UnitRegistry {
    /// Multilinear-128 hash of the set of units whose effective status is
    /// Runnable: membership, not queue order.
    ///
    /// The construction and its collision bound are in the
    /// `status_lanes` module. The call shares the status hash's refresh,
    /// so it pays only for the units marked stale since the previous
    /// read of either hash.
    ///
    /// Wire-format contract: pinned by `runnable_queue_hash_wire_format_golden`.
    pub fn runnable_queue_hash(&self) -> u64 {
        let h = (self.refresh_lanes().runnable >> 64) as u64;
        debug_assert_eq!(
            h,
            self.runnable_queue_hash_from_scratch(),
            "incremental runnable-set hash out of date"
        );
        h
    }

    /// [`Self::runnable_queue_hash`] computed from every registered unit,
    /// without the accumulator the registry keeps.
    pub fn runnable_queue_hash_from_scratch(&self) -> u64 {
        let mut acc = runnable_key(0);
        for id in self.units.keys() {
            if self.effective_status(*id) == Some(UnitStatus::Runnable) {
                acc = acc.wrapping_add(runnable_key(id.raw() + 1));
            }
        }
        (acc >> 64) as u64
    }

    /// Multilinear-128 hash of the effective status of every unit.
    ///
    /// The construction and its collision bound are in the
    /// `status_lanes` module. The call refreshes only the lanes marked
    /// stale since the previous call, so a commit pays for the units
    /// whose status can change, not for every unit.
    ///
    /// Wire-format contract: pinned by `status_hash_wire_format_golden`.
    pub fn status_hash(&self) -> u64 {
        let h = (self.refresh_lanes().status >> 64) as u64;
        debug_assert_eq!(
            h,
            self.status_hash_from_scratch(),
            "incremental unit-status hash out of date"
        );
        h
    }

    /// [`Self::status_hash`] computed from every registered unit,
    /// without the accumulator the registry keeps.
    pub fn status_hash_from_scratch(&self) -> u64 {
        let mut acc = status_key(0);
        for id in self.units.keys() {
            let lane = status_lane(self.effective_status(*id));
            acc = acc.wrapping_add(status_key(id.raw() + 1).wrapping_mul(u128::from(lane)));
        }
        (acc >> 64) as u64
    }

    /// Bring every stale lane up to date and return both sums.
    fn refresh_lanes(&self) -> LaneSums {
        self.status_lanes
            .borrow_mut()
            .refresh(|id| status_lane(self.effective_status(id)))
    }

    /// The sync-state term of the inputs that commits queued for each
    /// unit's next step, computed on read.
    ///
    /// Each unit is object `id` of [`source::PENDING_UNIT_INPUT`], with no
    /// presence lane. Its fields are:
    ///
    /// - 1: 1 when a syscall return is queued, and 2: that return.
    /// - 3: the count of received messages, and 4, slot `i`: message `i`.
    /// - 5: the count of register writes. Slot `i` of 6 holds the
    ///   register of write `i` plus 1, and slot `i` of 7 holds its value.
    /// - 8: 1 when CR fields are queued. Slot `i` of 9 holds CR field
    ///   `i`: 0 to keep it, 1 for LT, 2 for GT, 3 for EQ.
    ///
    /// A registry with no queued input adds 0.
    pub fn pending_inputs_sync_term(&self) -> u128 {
        let lane = |id: UnitId, field: u8, slot: usize, value: u64| {
            contribution(
                LaneIndex::new(source::PENDING_UNIT_INPUT, id.raw(), field, slot as u64),
                value,
            )
        };
        let mut sum = 0u128;
        let mut add = |term: u128| sum = sum.wrapping_add(term);
        for (&id, &code) in &self.pending_syscall_returns {
            add(lane(id, 1, 0, 1));
            add(lane(id, 2, 0, code));
        }
        for (&id, messages) in &self.pending_receives {
            add(lane(id, 3, 0, messages.len() as u64));
            for (slot, &message) in messages.iter().enumerate() {
                add(lane(id, 4, slot, u64::from(message)));
            }
        }
        for (&id, writes) in &self.pending_register_writes {
            add(lane(id, 5, 0, writes.len() as u64));
            for (slot, &(reg, value)) in writes.iter().enumerate() {
                add(lane(id, 6, slot, u64::from(reg) + 1));
                add(lane(id, 7, slot, value));
            }
        }
        for (&id, fields) in &self.pending_compare_fields {
            add(lane(id, 8, 0, 1));
            for (slot, field) in fields.0.iter().enumerate() {
                add(lane(id, 9, slot, compare_lane(*field)));
            }
        }
        sum
    }
}

/// The lane value of one queued CR field.
fn compare_lane(field: Option<CompareResult>) -> u64 {
    match field {
        None => 0,
        Some(CompareResult::Less) => 1,
        Some(CompareResult::Greater) => 2,
        Some(CompareResult::Equal) => 3,
    }
}

#[cfg(test)]
#[path = "tests/hash_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/pending_hash_tests.rs"]
mod pending_hash_tests;
