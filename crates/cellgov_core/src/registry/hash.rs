//! Wire-format hashes over the registry: runnable-queue hash and
//! full status hash. Pinned by golden tests so trace replay stays
//! byte-identical across code changes.

use cellgov_exec::UnitStatus;

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
}

#[cfg(test)]
#[path = "tests/hash_tests.rs"]
mod tests;
