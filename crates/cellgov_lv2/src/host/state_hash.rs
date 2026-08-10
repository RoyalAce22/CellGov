//! FNV-1a state-hash contribution for [`Lv2Host`].
//!
//! # Cross-module contract
//!
//! Types that contribute to the host's `state_hash` must be folded
//! through FNV-1a via their `.raw()` (or `.to_le_bytes()`) accessor,
//! not via `std::hash::Hash`: the runtime's `sync_state_hash` must
//! stay stable across compiler versions and build configurations.

use crate::ppu_thread::ThreadStackAllocator;

use super::state::Lv2State;
use super::Lv2Host;

impl Lv2Host {
    /// FNV-1a of all committed LV2 host state; folded into the
    /// runtime's `sync_state_hash` at every commit boundary.
    pub fn state_hash(&self) -> u64 {
        self.state.state_hash()
    }
}

impl Lv2State {
    /// FNV-1a of every field, via an exhaustive destructure with no
    /// rest pattern: adding a field to `Lv2State` without a fold
    /// decision here is a compile error.
    ///
    /// # Gating
    ///
    /// Per-primitive tables and the child-stack allocator contribute
    /// only when non-empty / past their sentinel. `next_kernel_id`,
    /// `mem_alloc_ptr`, and `mmapper_addr_cursor` always contribute,
    /// so a created-then-destroyed primitive still advances the hash
    /// via allocator state once the table empties again.
    ///
    /// # Cost
    ///
    /// Linear in the number of live primitives plus the per-thread
    /// lwmutex-hold map size; runs once per commit boundary.
    pub(in crate::host) fn state_hash(&self) -> u64 {
        let Self {
            content,
            groups,
            ppu_threads,
            tls_template,
            stack_allocator,
            next_kernel_id,
            mem_alloc_ptr,
            mmapper_addr_cursor,
            rsx_mem_alloc_ptr,
            rsx_mem_handle_counter,
            rsx_context,
            mmapper_handles,
            mmapper_ipc,
            lwmutexes,
            mutexes,
            semaphores,
            event_queues,
            event_ports,
            event_flags,
            conds,
            lwmutex_holds,
            fs_store,
            prx_registry,
            firmware_identity,
            program_authority_id,
            control_flags1,
            process_counts,
        } = self;
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for source in [content.state_hash(), groups.state_hash()] {
            hasher.write(&source.to_le_bytes());
        }
        hasher.write(&next_kernel_id.to_le_bytes());
        hasher.write(&mem_alloc_ptr.to_le_bytes());
        hasher.write(&mmapper_addr_cursor.to_le_bytes());
        hasher.write(&rsx_mem_alloc_ptr.to_le_bytes());
        hasher.write(&rsx_mem_handle_counter.to_le_bytes());
        hasher.write(&rsx_context.state_hash().to_le_bytes());
        if !ppu_threads.is_empty() {
            hasher.write(&ppu_threads.state_hash().to_le_bytes());
        }
        if !tls_template.is_empty() {
            hasher.write(&tls_template.state_hash().to_le_bytes());
        }
        if let Some(peek) = stack_allocator.peek_next(0x10) {
            if peek != ThreadStackAllocator::CHILD_STACK_BASE {
                hasher.write(&peek.to_le_bytes());
            }
        }
        if !lwmutexes.is_empty() {
            hasher.write(&lwmutexes.state_hash().to_le_bytes());
        }
        if !mutexes.is_empty() {
            hasher.write(&mutexes.state_hash().to_le_bytes());
        }
        if !semaphores.is_empty() {
            hasher.write(&semaphores.state_hash().to_le_bytes());
        }
        if !event_queues.is_empty() {
            hasher.write(&event_queues.state_hash().to_le_bytes());
        }
        if !event_ports.is_empty() {
            hasher.write(&event_ports.state_hash().to_le_bytes());
        }
        if !event_flags.is_empty() {
            hasher.write(&event_flags.state_hash().to_le_bytes());
        }
        if !conds.is_empty() {
            hasher.write(&conds.state_hash().to_le_bytes());
        }
        if !lwmutex_holds.is_empty() {
            hasher.write(&(lwmutex_holds.len() as u64).to_le_bytes());
            for (tid, count) in lwmutex_holds {
                hasher.write(&tid.raw().to_le_bytes());
                hasher.write(&count.to_le_bytes());
            }
        }
        if !fs_store.is_empty() {
            hasher.write(&fs_store.state_hash().to_le_bytes());
        }
        if !mmapper_handles.is_empty() {
            hasher.write(&mmapper_handles.state_hash().to_le_bytes());
        }
        // The key -> mem_id association steers which id a keyed 332
        // answers with, and it is recorded nowhere else.
        if !mmapper_ipc.is_empty() {
            hasher.write(&(mmapper_ipc.len() as u64).to_le_bytes());
            for (key, mem_id) in mmapper_ipc {
                hasher.write(&key.to_le_bytes());
                hasher.write(&mem_id.to_le_bytes());
            }
        }
        if !process_counts.is_empty() {
            hasher.write(&process_counts.state_hash().to_le_bytes());
        }
        // Guest-mutable (sc 480 mints miss stubs): two hosts that
        // disagree on which modules a guest loaded (or unloaded)
        // must not hash equal, or the divergence hides until a later
        // sc 494 module-list walk touches memory.
        if !prx_registry.is_empty() {
            hasher.write(&(prx_registry.len() as u64).to_le_bytes());
            for id in prx_registry.ids() {
                hasher.write(&id.to_le_bytes());
                let entry = prx_registry
                    .lookup_by_id(id)
                    .expect("ids() yields present entries");
                hasher.write(&[entry.state() as u8]);
                hasher.write(entry.stem().as_bytes());
                hasher.write(&[0u8]);
            }
        }
        if let Some(fw) = firmware_identity {
            hasher.write(&fw.image_version_hash.to_le_bytes());
            hasher.write(&fw.pup_sha256_bytes);
        }
        // A raw-ELF boot (no authid) and one set to the
        // retail-application fallback serve byte-identical
        // `sys_ss_access_control_engine` pkg-2 responses, so they hash
        // identically; only a distinct system-process authid folds in.
        if *program_authority_id != cellgov_ps3_abi::sce::RETAIL_APP_PROGRAM_AUTHORITY_ID {
            hasher.write(&program_authority_id.to_le_bytes());
        }
        // Same gating rationale: an unprivileged boot carries 0 and
        // hashes as it did before the field existed.
        if *control_flags1 != 0 {
            hasher.write(&control_flags1.to_le_bytes());
        }
        hasher.finish()
    }
}

#[cfg(test)]
#[path = "tests/state_hash_tests.rs"]
mod tests;
