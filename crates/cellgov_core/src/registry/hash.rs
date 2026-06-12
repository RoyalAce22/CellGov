//! Wire-format hashes over the registry: runnable-queue hash and
//! full status hash. Pinned by golden tests so trace replay stays
//! byte-identical across code changes.

use cellgov_exec::UnitStatus;

use super::UnitRegistry;

impl UnitRegistry {
    /// FNV-1a over the `id.raw()` LE bytes of every runnable unit, in
    /// id order. Empty set hashes to the FNV-1a empty-input value.
    ///
    /// Wire-format contract: pinned by `runnable_queue_hash_wire_format_golden`;
    /// any drift invalidates every existing trace.
    pub fn runnable_queue_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for id in self.units.keys() {
            if self.effective_status(*id) == Some(UnitStatus::Runnable) {
                hasher.write(&id.raw().to_le_bytes());
            }
        }
        hasher.finish()
    }

    /// FNV-1a over (`id.raw()` LE, `status_byte(status)`) for every unit
    /// in id order. Uses effective status so overrides are hashed.
    ///
    /// Wire-format contract: pinned by `status_hash_wire_format_golden`.
    /// `status_byte` is the explicit mapping (not `as u8`) so a future
    /// `#[repr]` change cannot silently drift the hash.
    pub fn status_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for (id, unit) in self.units.iter() {
            hasher.write(&id.raw().to_le_bytes());
            let status = self
                .status_overrides
                .get(id)
                .copied()
                .unwrap_or_else(|| unit.status());
            hasher.write(&[status_byte(status)]);
        }
        hasher.finish()
    }
}

/// Explicit `UnitStatus -> u8` mapping for [`UnitRegistry::status_hash`].
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

#[cfg(test)]
#[path = "tests/hash_tests.rs"]
mod tests;
