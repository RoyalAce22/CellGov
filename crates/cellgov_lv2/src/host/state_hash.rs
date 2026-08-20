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
            processes,
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
        for (pid, entry) in processes.iter() {
            if *pid == cellgov_ps3_abi::sys_process::BOOT_PROCESS_PID {
                // Boot-entry gating preserves the pre-table byte
                // stream: a raw-ELF boot (no authid) and one set to
                // the retail-application fallback serve byte-identical
                // `sys_ss_access_control_engine` pkg-2 responses, so
                // they hash identically; only a distinct
                // system-process authid folds in. Same rationale for
                // `ctrl_flags1`: an unprivileged boot carries 0 and
                // hashes as it did before the field existed.
                if entry.authority_id != cellgov_ps3_abi::sce::RETAIL_APP_PROGRAM_AUTHORITY_ID {
                    hasher.write(&entry.authority_id.to_le_bytes());
                }
                if entry.control_flags1 != 0 {
                    hasher.write(&entry.control_flags1.to_le_bytes());
                }
                // Boot ppid is fixed at construction, so only a
                // deviation folds; the tag byte keeps the 4-byte ppid
                // distinct from an untagged `control_flags1` of the
                // same value.
                if entry.ppid != cellgov_ps3_abi::sys_process::BOOT_PROCESS_PPID {
                    hasher.write(&[2u8]);
                    hasher.write(&entry.ppid.to_le_bytes());
                }
                // Boot exit ends the run; gate keeps the pre-field
                // stream while it is None. The discriminant byte keeps
                // a recorded status distinct from a bare
                // `control_flags1` of the same 4-byte value.
                if let Some(status) = entry.exit_status {
                    hasher.write(&[1u8]);
                    hasher.write(&status.to_le_bytes());
                }
            } else {
                // Children have no legacy stream to preserve; every
                // identity field folds unconditionally.
                hasher.write(&pid.to_le_bytes());
                hasher.write(&entry.ppid.to_le_bytes());
                hasher.write(&entry.authority_id.to_le_bytes());
                hasher.write(&entry.control_flags1.to_le_bytes());
                match entry.exit_status {
                    Some(status) => {
                        hasher.write(&[1u8]);
                        hasher.write(&status.to_le_bytes());
                    }
                    None => hasher.write(&[0u8]),
                }
            }
        }
        // Unit->process bindings exist only once a spawn happened;
        // empty map contributes nothing. The length prefix (the same
        // shape as `lwmutex_holds` / `mmapper_ipc` above) keeps a
        // binding's 12 bytes distinct from the gated boot identity
        // fields that precede it in the stream.
        let binding_count = processes.unit_bindings().count() as u64;
        if binding_count != 0 {
            hasher.write(&binding_count.to_le_bytes());
            for (unit, pid) in processes.unit_bindings() {
                hasher.write(&unit.raw().to_le_bytes());
                hasher.write(&pid.to_le_bytes());
            }
        }
        hasher.finish()
    }
}

#[cfg(test)]
#[path = "tests/state_hash_tests.rs"]
mod tests;
