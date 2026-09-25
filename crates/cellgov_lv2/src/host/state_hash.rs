//! The sync-state partial of [`Lv2Host`]: the Multilinear-128 sum of
//! the partials and terms of every `Lv2State` field.

use cellgov_mem::lanes::{self, source};

use super::state::Lv2State;
use super::Lv2Host;

impl Lv2Host {
    /// The host's partial of the runtime's sync-state sum.
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
    ///
    /// The exhaustive destructure has no rest pattern: a new field of
    /// `Lv2State` without a term here is a compile error.
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
        macro_rules! map_partial {
            ($map:expr) => {
                if from_scratch {
                    $map.partial_from_scratch()
                } else {
                    $map.partial()
                }
            };
        }
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
        let cursors = [
            next_kernel_id,
            mem_alloc_ptr,
            mmapper_addr_cursor,
            rsx_mem_alloc_ptr,
            rsx_mem_handle_counter,
        ]
        .into_iter()
        .zip(0u64..)
        .fold(0u128, |acc, (cursor, object)| {
            acc.wrapping_add(lanes::value_term(source::KERNEL_CURSORS, object, cursor))
        });
        let firmware_identity = firmware_identity
            .as_ref()
            .map_or(0, |fw| lanes::value_term(source::FIRMWARE_IDENTITY, 0, fw));
        [
            partial!(lwmutexes),
            partial!(mutexes),
            partial!(semaphores),
            partial!(conds),
            partial!(event_queues),
            partial!(event_ports),
            partial!(event_flags),
            partial!(groups),
            partial!(ppu_threads),
            partial!(processes),
            stack_allocator.sync_term(),
            process_counts.sync_term(),
            map_partial!(lwmutex_holds),
            firmware_identity,
            partial!(content),
            partial!(fs_store),
            partial!(prx_registry),
            partial!(config),
            partial!(mmapper_handles),
            map_partial!(mmapper_ipc),
            map_partial!(memory_containers),
            lanes::value_term(source::RSX_CONTEXT, 0, rsx_context),
            uart.sync_term(),
            usbd.sync_term(),
            cursors,
        ]
        .into_iter()
        .fold(0u128, u128::wrapping_add)
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

#[cfg(test)]
#[path = "tests/file_content_lanes_tests.rs"]
mod file_content_lanes_tests;

#[cfg(test)]
#[path = "tests/device_lanes_tests.rs"]
mod device_lanes_tests;
