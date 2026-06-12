//! LV2 dispatch for heavy mutexes.
//!
//! `acquire_or_enqueue` is atomic: recursive-lock-by-owner (EDEADLK)
//! and contention (park on FIFO waiter list) are distinguished in
//! one call.

use cellgov_event::UnitId;
use cellgov_ps3_abi::cell_errors;

use crate::dispatch::{Lv2Dispatch, PendingResponse};
use crate::host::{Lv2Host, Lv2Runtime};
use crate::sync_primitives::MutexAttrs;

impl Lv2Host {
    pub(super) fn dispatch_mutex_create(
        &mut self,
        id_ptr: u32,
        attr_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        // `sys_mutex_attribute_t`: protocol@0 u32, recursive@4 u32,
        // pshared@8 u32 (BE). Other fields are not surfaced.
        let attrs = if attr_ptr == 0 {
            MutexAttrs::default()
        } else if let Some(bytes) = rt.read_committed(attr_ptr as u64, 12) {
            let protocol = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            let recursive_raw = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
            MutexAttrs {
                priority_policy: protocol,
                recursive: recursive_raw != 0,
                protocol,
            }
        } else {
            MutexAttrs::default()
        };
        let id = self.alloc_id();
        if self.mutexes.create_with_id(id, attrs).is_err() {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        }
        self.immediate_write_u32(id, id_ptr, requester)
    }

    pub(super) fn dispatch_mutex_destroy(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.mutexes.lookup(id) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        if entry.owner().is_some() || !entry.waiters().is_empty() {
            return Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into());
        }
        self.mutexes.destroy(id);
        Lv2Dispatch::immediate(0)
    }

    pub(super) fn dispatch_mutex_lock(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        match self.mutexes.acquire_or_enqueue(id, caller) {
            crate::sync_primitives::MutexAcquireOrEnqueue::Unknown => {
                Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into())
            }
            crate::sync_primitives::MutexAcquireOrEnqueue::Acquired => Lv2Dispatch::immediate(0),
            crate::sync_primitives::MutexAcquireOrEnqueue::WouldDeadlock => {
                Lv2Dispatch::immediate(cell_errors::CELL_EDEADLK.into())
            }
            crate::sync_primitives::MutexAcquireOrEnqueue::Enqueued => Lv2Dispatch::Block {
                reason: crate::dispatch::Lv2BlockReason::Mutex { id },
                pending: PendingResponse::ReturnCode { code: 0 },
                effects: vec![],
            },
        }
    }

    pub(super) fn dispatch_mutex_trylock(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        match self.mutexes.try_acquire(id, caller) {
            None => Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into()),
            Some(crate::sync_primitives::MutexAcquire::Acquired) => Lv2Dispatch::immediate(0),
            Some(crate::sync_primitives::MutexAcquire::Contended) => {
                Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into())
            }
        }
    }

    pub(super) fn dispatch_mutex_unlock(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        match self.mutexes.release_and_wake_next(id, caller) {
            crate::sync_primitives::MutexRelease::Unknown => {
                Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into())
            }
            crate::sync_primitives::MutexRelease::NotOwner => {
                Lv2Dispatch::immediate(cell_errors::CELL_EPERM.into())
            }
            crate::sync_primitives::MutexRelease::Freed => Lv2Dispatch::immediate(0),
            crate::sync_primitives::MutexRelease::Transferred { new_owner } => {
                match self.resolve_wake_thread(new_owner, "mutex_unlock.Transferred") {
                    Some(unit) => Lv2Dispatch::WakeAndReturn {
                        code: 0,
                        woken_unit_ids: vec![unit],
                        response_updates: vec![],
                        effects: vec![],
                    },
                    None => Lv2Dispatch::immediate(0),
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/mutex_tests.rs"]
mod tests;
