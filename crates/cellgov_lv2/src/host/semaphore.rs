//! LV2 dispatch for counting semaphores.
//!
//! Invariant: count never exceeds `max`. A post with a parked waiter hands
//! off directly without incrementing; over-max post with no waiter is EBUSY.

use cellgov_event::UnitId;
use cellgov_ps3_abi::cell_errors;

use crate::dispatch::{Lv2Dispatch, PendingResponse};
use crate::host::{Lv2Host, Lv2Runtime};

impl Lv2Host {
    pub(super) fn dispatch_semaphore_create(
        &mut self,
        id_ptr: u32,
        attr_ptr: u32,
        initial: i32,
        max: i32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        // EFAULT for NULL id/attr precedes bounds checks (real LV2 order).
        if id_ptr == 0 || attr_ptr == 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        // sys_semaphore_attribute_t: protocol u32 at +0, type s32 at +20
        // (shared with event_flag/mutex/cond). Memset-zero fails validation.
        let Some(attr_bytes) = rt.read_committed(attr_ptr as u64, 24) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        };
        let protocol =
            u32::from_be_bytes([attr_bytes[0], attr_bytes[1], attr_bytes[2], attr_bytes[3]]);
        use cellgov_ps3_abi::sys_sync::{SYS_SYNC_FIFO, SYS_SYNC_PRIORITY};
        if protocol != SYS_SYNC_FIFO && protocol != SYS_SYNC_PRIORITY {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        // `max == 0` is invalid (real LV2 rejects an unacquirable semaphore).
        if max <= 0 || initial < 0 || initial > max {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        let id = self.alloc_id();
        match self.semaphores.create_with_id(id, initial, max) {
            Ok(()) => {}
            Err(crate::sync_primitives::SemaphoreCreateError::IdCollision(_)) => {
                // Host-invariant break; ENOMEM is the best-effort errno
                // (no Cell OS code maps to "allocator handed me a live id").
                return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
            }
            Err(crate::sync_primitives::SemaphoreCreateError::InvalidBounds) => {
                return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
            }
        }
        self.immediate_write_u32(id, id_ptr, requester)
    }

    pub(super) fn dispatch_semaphore_destroy(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.semaphores.lookup(id) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        if !entry.waiters().is_empty() {
            return Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into());
        }
        self.semaphores.destroy(id);
        Lv2Dispatch::immediate(0)
    }

    pub(super) fn dispatch_semaphore_wait(
        &mut self,
        id: u32,
        timeout: u64,
        requester: UnitId,
    ) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        match self.semaphores.try_wait(id) {
            None => Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into()),
            Some(crate::sync_primitives::SemaphoreWait::Acquired) => Lv2Dispatch::immediate(0),
            Some(crate::sync_primitives::SemaphoreWait::Empty) => {
                // Finite timeout with no peer that could post: ETIMEDOUT
                // now, since CellGov has no guest clock and blocking the
                // only live thread would stall the schedule.
                if timeout != 0 && !self.ppu_threads.has_other_alive_thread(caller) {
                    return Lv2Dispatch::immediate(cell_errors::CELL_ETIMEDOUT.into());
                }
                match self.semaphores.enqueue_waiter(id, caller) {
                    Ok(()) => {}
                    // Both branches are host-invariant breaks (try_wait
                    // confirmed the id; a blocked caller cannot re-enter
                    // wait). ESRCH because real sys_semaphore_wait never
                    // returns EDEADLK.
                    Err(
                        crate::sync_primitives::SemaphoreEnqueueError::UnknownId
                        | crate::sync_primitives::SemaphoreEnqueueError::DuplicateWaiter,
                    ) => {
                        return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
                    }
                }
                Lv2Dispatch::Block {
                    reason: crate::dispatch::Lv2BlockReason::Semaphore { id },
                    pending: PendingResponse::ReturnCode { code: 0 },
                    effects: vec![],
                }
            }
        }
    }

    pub(super) fn dispatch_semaphore_trywait(&mut self, id: u32) -> Lv2Dispatch {
        match self.semaphores.try_wait(id) {
            None => Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into()),
            Some(crate::sync_primitives::SemaphoreWait::Acquired) => Lv2Dispatch::immediate(0),
            Some(crate::sync_primitives::SemaphoreWait::Empty) => {
                Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into())
            }
        }
    }

    pub(super) fn dispatch_semaphore_get_value(
        &mut self,
        id: u32,
        out_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        // EFAULT for NULL out precedes the id lookup (real LV2 order).
        if out_ptr == 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        let Some(entry) = self.semaphores.lookup(id) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        let count = entry.count() as u32;
        self.immediate_write_u32(count, out_ptr, requester)
    }

    pub(super) fn dispatch_semaphore_post(&mut self, id: u32, val: i32) -> Lv2Dispatch {
        // Error order (real LV2 / RPCS3 sys_semaphore_post): id lookup,
        // then val<=0, then overflow-vs-max. The overflow check folds in
        // waiters: post(N) wakes up to N waiters and only the leftover
        // counts toward `max`.
        let Some(entry) = self.semaphores.lookup(id) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        if val <= 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        let waiters_len = entry.waiters().len() as i32;
        let leftover = (val - waiters_len).max(0);
        if leftover > entry.max() - entry.count() {
            return Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into());
        }
        match self.semaphores.post_and_wake_n(id, val as u32) {
            crate::sync_primitives::SemaphorePostN::Unknown => {
                Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into())
            }
            crate::sync_primitives::SemaphorePostN::OverMax => {
                Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into())
            }
            crate::sync_primitives::SemaphorePostN::Posted { woken, .. } => {
                if woken.is_empty() {
                    return Lv2Dispatch::immediate(0);
                }
                let mut units = Vec::with_capacity(woken.len());
                for tid in woken {
                    if let Some(unit) = self.resolve_wake_thread(tid, "semaphore_post.Woke") {
                        units.push(unit);
                    }
                }
                if units.is_empty() {
                    Lv2Dispatch::immediate(0)
                } else {
                    Lv2Dispatch::WakeAndReturn {
                        code: 0,
                        woken_unit_ids: units,
                        response_updates: vec![],
                        effects: vec![],
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/semaphore_tests.rs"]
mod tests;
