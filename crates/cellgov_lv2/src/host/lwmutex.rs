//! LV2 dispatch for lightweight mutexes.
//!
//! The kernel side is a signal flag plus a FIFO sleep queue; owner,
//! recursion, and waiter count live in the user-space `sys_lwmutex_t`
//! and only invoke the kernel for contention. `lwmutex_lock` consumes
//! a pending signal or parks; `lwmutex_unlock` wakes the head of the
//! sleep queue or sets the signal for the next acquirer.

use cellgov_event::UnitId;
use cellgov_ps3_abi::cell_errors;

use crate::dispatch::{Lv2Dispatch, PendingResponse};
use crate::host::Lv2Host;

impl Lv2Host {
    pub(super) fn dispatch_lwmutex_create(
        &mut self,
        id_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        let Some(id) = self.lwmutexes.create() else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        };
        self.immediate_write_u32(id, id_ptr, requester)
    }

    pub(super) fn dispatch_lwmutex_lock(
        &mut self,
        id: u32,
        mutex_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        match self.lwmutexes.acquire_or_enqueue(id, caller) {
            crate::sync_primitives::LwMutexAcquireOrEnqueue::Unknown => {
                Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into())
            }
            crate::sync_primitives::LwMutexAcquireOrEnqueue::Acquired => Lv2Dispatch::immediate(0),
            crate::sync_primitives::LwMutexAcquireOrEnqueue::WouldDeadlock => {
                Lv2Dispatch::immediate(cell_errors::CELL_EDEADLK.into())
            }
            crate::sync_primitives::LwMutexAcquireOrEnqueue::Enqueued => Lv2Dispatch::Block {
                reason: crate::dispatch::Lv2BlockReason::LwMutex { id },
                pending: PendingResponse::LwMutexWake {
                    mutex_ptr,
                    caller: caller.raw() as u32,
                },
                effects: vec![],
            },
        }
    }

    pub(super) fn dispatch_lwmutex_trylock(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        match self.lwmutexes.try_acquire(id, caller) {
            None => Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into()),
            Some(crate::sync_primitives::LwMutexAcquire::Acquired) => Lv2Dispatch::immediate(0),
            Some(crate::sync_primitives::LwMutexAcquire::Contended) => {
                Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into())
            }
        }
    }

    pub(super) fn dispatch_lwmutex_unlock(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        match self.lwmutexes.release_and_wake_next(id, caller) {
            crate::sync_primitives::LwMutexRelease::Unknown => {
                Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into())
            }
            crate::sync_primitives::LwMutexRelease::Signaled => Lv2Dispatch::immediate(0),
            crate::sync_primitives::LwMutexRelease::Transferred { new_owner } => {
                match self.resolve_wake_thread(new_owner, "lwmutex_unlock.Transferred") {
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

    pub(super) fn dispatch_lwmutex_destroy(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.lwmutexes.lookup(id) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        // Only parked waiters block destroy. The signal flag does
        // not, since user-space ownership is invisible to us.
        // Only parked waiters block destroy; the signal flag does
        // not (user-space ownership is invisible to the kernel).
        if !entry.waiters().is_empty() {
            return Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into());
        }
        self.lwmutexes.destroy(id);
        Lv2Dispatch::immediate(0)
    }
}

#[cfg(test)]
#[path = "tests/lwmutex_tests.rs"]
mod tests;
