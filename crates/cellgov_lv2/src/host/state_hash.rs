//! The FNV-1a state hash and the sync-state partial of [`Lv2Host`].
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
    /// FNV-1a of the committed LV2 host state that keeps no partial;
    /// the runtime adds it to `sync_state_hash` as the transitional lane.
    pub fn state_hash(&self) -> u64 {
        self.state.state_hash()
    }

    /// The sum of the sync-primitive tables' partials.
    pub fn sync_partial(&self) -> u128 {
        self.state.sync_partial(false)
    }

    /// [`Self::sync_partial`] computed from every entry.
    pub fn sync_partial_from_scratch(&self) -> u128 {
        self.state.sync_partial(true)
    }
}

impl Lv2State {
    /// [`Lv2Host::sync_partial`]; `from_scratch` rebuilds each table's
    /// partial from every entry.
    fn sync_partial(&self, from_scratch: bool) -> u128 {
        macro_rules! partial {
            ($table:expr) => {
                if from_scratch {
                    $table.sync_partial_from_scratch()
                } else {
                    $table.sync_partial()
                }
            };
        }
        [
            partial!(self.lwmutexes),
            partial!(self.mutexes),
            partial!(self.semaphores),
            partial!(self.conds),
            partial!(self.event_queues),
            partial!(self.event_ports),
            partial!(self.event_flags),
        ]
        .into_iter()
        .fold(0u128, u128::wrapping_add)
    }

    /// FNV-1a of the fields that keep no partial, via an exhaustive
    /// destructure with no rest pattern: adding a field to `Lv2State`
    /// without a fold decision here is a compile error.
    ///
    /// # Gating
    ///
    /// - The sync-primitive tables stay out of this fold. They keep
    ///   partials of their own ([`Self::sync_partial`]).
    /// - The child-stack allocator contributes only past its sentinel.
    /// - `next_kernel_id`, `mem_alloc_ptr` and `mmapper_addr_cursor`
    ///   always contribute. A primitive whose id comes from
    ///   `next_kernel_id` moves this hash even after its destroy.
    ///
    /// # Cost
    ///
    /// Linear in the entries of the folded tables; runs once per
    /// commit boundary.
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
            config,
            uart,
            usbd,
            memory_containers,
            lwmutexes: _,
            mutexes: _,
            semaphores: _,
            event_queues: _,
            event_ports: _,
            event_flags: _,
            conds: _,
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
        if !config.is_pristine() {
            hasher.write(&config.state_hash().to_le_bytes());
        }
        if !uart.is_pristine() {
            hasher.write(&uart.state_hash().to_le_bytes());
        }
        if !usbd.is_pristine() {
            hasher.write(&usbd.state_hash().to_le_bytes());
        }
        if !memory_containers.is_empty() {
            hasher.write(&(memory_containers.len() as u64).to_le_bytes());
            for cid in memory_containers {
                hasher.write(&cid.to_le_bytes());
            }
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
            if *pid == cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID {
                // Boot-entry gating preserves the pre-table byte
                // stream: a raw-ELF boot (no authid) and one set to
                // the retail-application fallback serve byte-identical
                // `sys_ss_access_control_engine` pkg-2 responses, so
                // they hash identically; only a distinct
                // system-process authid folds in. Same rationale for
                // `ctrl_flags1`: an unprivileged boot carries 0 and
                // hashes as it did before the field existed.
                if entry.authority_id
                    != cellgov_ps3_abi::format::sce::RETAIL_APP_PROGRAM_AUTHORITY_ID
                {
                    hasher.write(&entry.authority_id.to_le_bytes());
                }
                if entry.control_flags1 != 0 {
                    hasher.write(&entry.control_flags1.to_le_bytes());
                }
                // Boot ppid is fixed at construction, so only a
                // deviation folds; the tag byte keeps the 4-byte ppid
                // distinct from an untagged `control_flags1` of the
                // same value.
                if entry.ppid != cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PPID {
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

#[cfg(test)]
#[path = "tests/sync_partial_tests.rs"]
mod sync_partial_tests;
