//! The FNV-1a state hash and the sync-state partial of [`Lv2Host`].
//!
//! # Cross-module contract
//!
//! Types that contribute to the host's `state_hash` must be folded
//! through FNV-1a via their `.raw()` (or `.to_le_bytes()`) accessor,
//! not via `std::hash::Hash`: the runtime's `sync_state_hash` must
//! stay stable across compiler versions and build configurations.

use cellgov_mem::lanes::{self, source};

use super::state::Lv2State;
use super::Lv2Host;

impl Lv2Host {
    /// FNV-1a of the committed LV2 host state that keeps no partial;
    /// the runtime adds it to `sync_state_hash` as the transitional lane.
    pub fn state_hash(&self) -> u64 {
        self.state.state_hash()
    }

    /// The sum of the host's sync-state partials: the sync-primitive,
    /// thread, process and identity state.
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
        let lwmutex_holds = if from_scratch {
            self.lwmutex_holds.partial_from_scratch()
        } else {
            self.lwmutex_holds.partial()
        };
        let firmware_identity = self
            .firmware_identity
            .as_ref()
            .map_or(0, |fw| lanes::value_term(source::FIRMWARE_IDENTITY, 0, fw));
        [
            partial!(self.lwmutexes),
            partial!(self.mutexes),
            partial!(self.semaphores),
            partial!(self.conds),
            partial!(self.event_queues),
            partial!(self.event_ports),
            partial!(self.event_flags),
            partial!(self.groups),
            partial!(self.ppu_threads),
            partial!(self.processes),
            self.stack_allocator.sync_term(),
            self.process_counts.sync_term(),
            lwmutex_holds,
            firmware_identity,
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
    /// - The sync-primitive, thread, process and identity state stays
    ///   out of this fold. It keeps partials of its own
    ///   ([`Self::sync_partial`]).
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
            groups: _,
            ppu_threads: _,
            stack_allocator: _,
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
            lwmutex_holds: _,
            fs_store,
            prx_registry,
            firmware_identity: _,
            processes: _,
            process_counts: _,
        } = self;
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&content.state_hash().to_le_bytes());
        hasher.write(&next_kernel_id.to_le_bytes());
        hasher.write(&mem_alloc_ptr.to_le_bytes());
        hasher.write(&mmapper_addr_cursor.to_le_bytes());
        hasher.write(&rsx_mem_alloc_ptr.to_le_bytes());
        hasher.write(&rsx_mem_handle_counter.to_le_bytes());
        hasher.write(&rsx_context.state_hash().to_le_bytes());
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
        hasher.finish()
    }
}

#[cfg(test)]
#[path = "tests/state_hash_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/sync_partial_tests.rs"]
mod sync_partial_tests;

#[cfg(test)]
#[path = "tests/thread_process_lanes_tests.rs"]
mod thread_process_lanes_tests;
