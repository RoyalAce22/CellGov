//! The incremental accumulator behind
//! [`UnitRegistry::status_hash`](super::UnitRegistry::status_hash).
//!
//! The hash reads the unit-status vector as one 64-bit lane per unit id:
//!
//! ```text
//! lane_i = status_code(effective status of unit i) + 1   when unit i is registered
//! lane_i = 0                                             otherwise
//!
//! acc  = key(S, 0) + sum over registered i of key(S, i + 1) * lane_i   (mod 2^128)
//! hash = acc >> 64
//! ```
//!
//! `key(S, k)` is [`cellgov_mem::indexed_key`] over [`STATUS_KEY_SEED`].
//! An unregistered id adds `key * 0 = 0`, so the sum is the fixed-length
//! Multilinear hash of the whole id-indexed vector.
//!
//! [LemireKaser2014 p:4 s:3] Theorem 3.1 with K = 128 and L = 64: the
//! family is strongly universal over fixed-length lane vectors.
//! [LemireKaser2014 p:2 s:1] The top 64 bits keep that property. Two
//! distinct status vectors thus give one hash with a probability of at
//! most 2^-64 over the key draw. The keys are fixed, and the bound holds
//! while no hash value steers which states the runtime produces
//! [CarterWegman1979 p:147 s:Properties of Universal Classes].
//!
//! A registered unit takes `code + 1`, never 0, so a registered Runnable
//! unit (code 0) and an unregistered id differ. Lanes combine by
//! addition, not XOR [Black1999 p:13 s:4.3].
//!
//! [WegmanCarter1981 p:277 s:5] When a value changes, delete the pair of
//! the address and the old value, and add the pair with the new value.
//! Here that is `acc += key(S, i + 1) * (new - old)`, applied only to the
//! lanes marked stale since the previous read.

use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;

/// The SplitMix64 seed of the unit-status keys, distinct from the seeds
/// of the other state hashes.
pub(super) const STATUS_KEY_SEED: u64 = 0x756e_6974_7374_6174;

/// Key `k` of the unit-status key stream.
#[inline]
pub(super) fn status_key(k: u64) -> u128 {
    cellgov_mem::indexed_key(STATUS_KEY_SEED, k)
}

/// The lane of a unit whose effective status is `status`, or of an
/// unregistered id for `None`.
#[inline]
pub(super) fn status_lane(status: Option<UnitStatus>) -> u64 {
    status.map_or(0, |s| u64::from(status_byte(s)) + 1)
}

/// Explicit `UnitStatus -> u8` mapping for the status lanes.
///
/// Exhaustive (no `_ =>`): adding a `UnitStatus` variant without updating
/// this is a compile error, not a silent hash drift.
fn status_byte(status: UnitStatus) -> u8 {
    match status {
        UnitStatus::Runnable => 0,
        UnitStatus::Blocked => 1,
        UnitStatus::Faulted => 2,
        UnitStatus::Finished => 3,
    }
}

/// The accumulator, the lane each unit last contributed, and the ids
/// whose lane can differ from its value at the previous read.
#[derive(Clone, Debug)]
pub(super) struct StatusLanes {
    acc: u128,
    /// Lane value in `acc` per unit id; an id past the end holds 0.
    lanes: Vec<u64>,
    /// Whether an id is in `stale_ids`.
    stale: Vec<bool>,
    stale_ids: Vec<UnitId>,
}

impl Default for StatusLanes {
    fn default() -> Self {
        Self {
            acc: status_key(0),
            lanes: Vec::new(),
            stale: Vec::new(),
            stale_ids: Vec::new(),
        }
    }
}

impl StatusLanes {
    /// Record that the effective status of `id` can differ from its lane.
    #[inline]
    pub(super) fn mark(&mut self, id: UnitId) {
        let i = id.raw() as usize;
        if i >= self.stale.len() {
            self.stale.resize(i + 1, false);
            self.lanes.resize(i + 1, 0);
        }
        if !self.stale[i] {
            self.stale[i] = true;
            self.stale_ids.push(id);
        }
    }

    /// Bring every stale lane up to date from `lane_of` and return the
    /// accumulator. O(stale ids).
    pub(super) fn refresh(&mut self, lane_of: impl Fn(UnitId) -> u64) -> u128 {
        for id in self.stale_ids.drain(..) {
            let i = id.raw() as usize;
            let new = lane_of(id);
            let old = self.lanes[i];
            if new != old {
                let delta = u128::from(new).wrapping_sub(u128::from(old));
                self.acc = self
                    .acc
                    .wrapping_add(status_key(id.raw() + 1).wrapping_mul(delta));
                self.lanes[i] = new;
            }
            self.stale[i] = false;
        }
        self.acc
    }
}

#[cfg(test)]
#[path = "tests/status_lanes_tests.rs"]
mod tests;
