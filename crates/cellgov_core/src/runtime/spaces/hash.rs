//! The committed-memory hash and the schedule explorer's observable hash.

use crate::runtime::state::Runtime;

impl Runtime {
    /// Committed-memory hash over every space's content.
    ///
    /// Child spaces fold in so a cross-process divergence in one is
    /// witnessed; with no child space this is space 0's content hash.
    /// Mapping metadata stays outside it and reaches the sync-channel
    /// state hash through `metadata_hash`.
    pub fn committed_memory_hash(&self) -> u64 {
        if self.spaces.extra.is_empty() {
            return self.memory.content_hash();
        }
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&self.memory.content_hash().to_le_bytes());
        for (space, mem) in &self.spaces.extra {
            hasher.write(&space.raw().to_le_bytes());
            hasher.write(&mem.content_hash().to_le_bytes());
        }
        hasher.finish()
    }

    /// The schedule explorer's observable: [`Runtime::committed_memory_hash`]
    /// folded with every unit's private memory, in unit-id order.
    ///
    /// A unit that reports no private memory
    /// ([`cellgov_exec::ExecutionUnit::local_memory_hash`]) contributes
    /// nothing. A runtime whose units all report none hashes exactly as
    /// `committed_memory_hash` does. Each contributing unit folds its id
    /// beside its hash, so two units with exchanged local stores read as
    /// a different state.
    pub fn observable_hash(&self) -> u64 {
        let committed = self.committed_memory_hash();
        let mut contributors = self
            .registry
            .iter()
            .filter_map(|(id, unit)| unit.local_memory_hash().map(|hash| (id, hash)))
            .peekable();
        if contributors.peek().is_none() {
            return committed;
        }
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&committed.to_le_bytes());
        for (id, hash) in contributors {
            hasher.write(&id.raw().to_le_bytes());
            hasher.write(&hash.to_le_bytes());
        }
        hasher.finish()
    }
}

#[cfg(test)]
#[path = "tests/observable_hash_tests.rs"]
mod observable_hash_tests;
