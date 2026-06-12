//! FNV-1a state-hash contribution for [`Lv2Host`].
//!
//! # Cross-module contract
//!
//! Types that contribute to the host's `state_hash` must be folded
//! through FNV-1a via their `.raw()` (or `.to_le_bytes()`) accessor,
//! not via `std::hash::Hash`. The runtime's `sync_state_hash` is
//! cross-build stable, so any hasher whose output depends on
//! compiler version / build configuration (`DefaultHasher`,
//! `RandomState`) is forbidden. New fields land their contribution
//! in [`Lv2Host::state_hash`]; gating on `!is_empty()` keeps the
//! hash stable at boot until a primitive is actually used.

use crate::ppu_thread::ThreadStackAllocator;

use super::Lv2Host;

impl Lv2Host {
    /// FNV-1a of all committed LV2 host state; folded into the
    /// runtime's `sync_state_hash` at every commit boundary.
    ///
    /// # Gating
    ///
    /// Per-primitive tables and the child-stack allocator contribute
    /// only when non-empty / past their sentinel. `next_kernel_id`
    /// and `mem_alloc_ptr` always contribute, so a
    /// created-then-destroyed primitive still advances the hash via
    /// allocator state once the table empties again.
    ///
    /// # Cost
    ///
    /// Linear in the number of live primitives plus per-thread
    /// lwmutex-hold and callback-parent map sizes; runs once per
    /// commit boundary.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for source in [self.content.state_hash(), self.groups.state_hash()] {
            hasher.write(&source.to_le_bytes());
        }
        hasher.write(&self.next_kernel_id.to_le_bytes());
        hasher.write(&self.mem_alloc_ptr.to_le_bytes());
        hasher.write(&self.rsx_mem_alloc_ptr.to_le_bytes());
        hasher.write(&self.rsx_mem_handle_counter.to_le_bytes());
        hasher.write(&self.rsx_context.state_hash().to_le_bytes());
        if !self.ppu_threads.is_empty() {
            hasher.write(&self.ppu_threads.state_hash().to_le_bytes());
        }
        if !self.tls_template.is_empty() {
            hasher.write(&self.tls_template.state_hash().to_le_bytes());
        }
        if let Some(peek) = self.stack_allocator.peek_next(0x10) {
            if peek != ThreadStackAllocator::CHILD_STACK_BASE {
                hasher.write(&peek.to_le_bytes());
            }
        }
        if !self.lwmutexes.is_empty() {
            hasher.write(&self.lwmutexes.state_hash().to_le_bytes());
        }
        if !self.mutexes.is_empty() {
            hasher.write(&self.mutexes.state_hash().to_le_bytes());
        }
        if !self.semaphores.is_empty() {
            hasher.write(&self.semaphores.state_hash().to_le_bytes());
        }
        if !self.event_queues.is_empty() {
            hasher.write(&self.event_queues.state_hash().to_le_bytes());
        }
        if !self.event_flags.is_empty() {
            hasher.write(&self.event_flags.state_hash().to_le_bytes());
        }
        if !self.conds.is_empty() {
            hasher.write(&self.conds.state_hash().to_le_bytes());
        }
        if !self.lwmutex_holds.is_empty() {
            hasher.write(&(self.lwmutex_holds.len() as u64).to_le_bytes());
            for (tid, count) in &self.lwmutex_holds {
                hasher.write(&tid.raw().to_le_bytes());
                hasher.write(&count.to_le_bytes());
            }
        }
        if !self.fs_store.is_empty() {
            hasher.write(&self.fs_store.state_hash().to_le_bytes());
        }
        if let Some(fw) = self.firmware_identity() {
            hasher.write(&fw.image_version_hash.to_le_bytes());
            hasher.write(&fw.pup_sha256_bytes);
        }
        // A raw-ELF boot (no authid) and one set to the
        // retail-application fallback serve byte-identical
        // `sys_ss_access_control_engine` pkg-2 responses, so they hash
        // identically; only a distinct system-process authid folds in.
        if self.program_authority_id != cellgov_ps3_abi::sce::RETAIL_APP_PROGRAM_AUTHORITY_ID {
            hasher.write(&self.program_authority_id.to_le_bytes());
        }
        hasher.finish()
    }
}

#[cfg(test)]
#[path = "tests/state_hash_tests.rs"]
mod tests;
