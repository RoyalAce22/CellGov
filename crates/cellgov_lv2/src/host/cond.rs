//! LV2 dispatch for condition variables.
//!
//! Two-hop cond-wake protocol: `cond_wait` drops the caller's mutex
//! and parks on the cond; `cond_signal*` moves the waker off the
//! cond, tries to reacquire the mutex, and either wakes with
//! `ReturnCode{0}` or re-parks on the mutex waiter list so the next
//! unlock-wake resolves it. Signals are non-sticky.

use cellgov_event::UnitId;
use cellgov_ps3_abi::cell_errors;

use crate::dispatch::{CondMutexKind, Lv2Dispatch, PendingResponse};
use crate::host::Lv2Host;
use crate::ppu_thread::PpuThreadId;

impl Lv2Host {
    pub(super) fn dispatch_cond_create(
        &mut self,
        id_ptr: u32,
        mutex_id: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        if self.mutexes.lookup(mutex_id).is_none() {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        }
        let id = self.alloc_id();
        if self
            .conds
            .create_with_id(id, mutex_id, CondMutexKind::Mutex)
            .is_err()
        {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        }
        self.immediate_write_u32(id, id_ptr, requester)
    }

    pub(super) fn dispatch_cond_destroy(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.conds.lookup(id) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        if !entry.waiters().is_empty() {
            return Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into());
        }
        self.conds.destroy(id);
        Lv2Dispatch::immediate(0)
    }

    pub(super) fn dispatch_cond_wait(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        let Some(entry) = self.conds.lookup(id) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        let mutex_id = entry.mutex_id();
        let mutex_kind = entry.mutex_kind();
        let release = match mutex_kind {
            CondMutexKind::Mutex => self.mutexes.release_and_wake_next(mutex_id, caller),
            CondMutexKind::LwMutex => {
                return Lv2Dispatch::immediate(cell_errors::CELL_EPERM.into());
            }
        };
        match release {
            crate::sync_primitives::MutexRelease::Unknown => {
                Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into())
            }
            crate::sync_primitives::MutexRelease::NotOwner => {
                Lv2Dispatch::immediate(cell_errors::CELL_EPERM.into())
            }
            crate::sync_primitives::MutexRelease::Freed => {
                match self.conds.enqueue_waiter(id, caller) {
                    Ok(()) => {}
                    Err(crate::sync_primitives::CondEnqueueError::UnknownId) => {
                        return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
                    }
                    Err(crate::sync_primitives::CondEnqueueError::DuplicateWaiter) => {
                        self.record_invariant_break(
                            "cond_wait.Freed.DuplicateWaiter",
                            format_args!("cond {id}: caller {caller:?} already on waiter list"),
                        );
                        return Lv2Dispatch::immediate(cell_errors::CELL_EDEADLK.into());
                    }
                }
                Lv2Dispatch::Block {
                    reason: crate::dispatch::Lv2BlockReason::Cond {
                        id,
                        mutex_id,
                        mutex_kind,
                    },
                    pending: PendingResponse::CondWakeReacquire {
                        mutex_id,
                        mutex_kind,
                    },
                    effects: vec![],
                }
            }
            crate::sync_primitives::MutexRelease::Transferred { new_owner } => {
                match self.conds.enqueue_waiter(id, caller) {
                    Ok(()) => {}
                    Err(crate::sync_primitives::CondEnqueueError::UnknownId) => {
                        return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
                    }
                    Err(crate::sync_primitives::CondEnqueueError::DuplicateWaiter) => {
                        self.record_invariant_break(
                            "cond_wait.Transferred.DuplicateWaiter",
                            format_args!("cond {id}: caller {caller:?} already on waiter list"),
                        );
                        return Lv2Dispatch::immediate(cell_errors::CELL_EDEADLK.into());
                    }
                }
                let woken_unit_ids =
                    match self.resolve_wake_thread(new_owner, "cond_wait.Transferred.new_owner") {
                        Some(unit) => vec![unit],
                        None => vec![],
                    };
                Lv2Dispatch::BlockAndWake {
                    reason: crate::dispatch::Lv2BlockReason::Cond {
                        id,
                        mutex_id,
                        mutex_kind,
                    },
                    pending: PendingResponse::CondWakeReacquire {
                        mutex_id,
                        mutex_kind,
                    },
                    woken_unit_ids,
                    response_updates: vec![],
                    effects: vec![],
                }
            }
        }
    }

    pub(super) fn dispatch_cond_signal_all(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.conds.lookup(id) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        let mutex_id = entry.mutex_id();
        let mutex_kind = entry.mutex_kind();
        if !matches!(mutex_kind, CondMutexKind::Mutex) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EPERM.into());
        }
        let wakers = self
            .conds
            .signal_all(id)
            .expect("cond looked up just above must still exist");
        if wakers.is_empty() {
            return Lv2Dispatch::immediate(0);
        }
        let mut woken_unit_ids: Vec<UnitId> = Vec::new();
        let mut response_updates: Vec<(UnitId, PendingResponse)> = Vec::new();
        for waker in wakers {
            let Some(unit) = self.resolve_wake_thread(waker, "cond_signal_all.waker") else {
                continue;
            };
            match self.mutexes.try_acquire(mutex_id, waker) {
                Some(crate::sync_primitives::MutexAcquire::Acquired) => {
                    wake_with(unit, 0u64, &mut woken_unit_ids, &mut response_updates);
                }
                Some(crate::sync_primitives::MutexAcquire::Contended) => {
                    match self.mutexes.enqueue_waiter(mutex_id, waker) {
                        Ok(()) => response_updates
                            .push((unit, PendingResponse::ReturnCode { code: 0u64 })),
                        Err(err) => {
                            self.record_invariant_break(
                                "cond_signal_all.Contended.enqueue",
                                format_args!(
                                    "enqueue_waiter failed for mutex {mutex_id} waker \
                                     {waker:?}: {err:?}; waking with ESRCH to avoid stranding"
                                ),
                            );
                            wake_with(
                                unit,
                                cell_errors::CELL_ESRCH.into(),
                                &mut woken_unit_ids,
                                &mut response_updates,
                            );
                        }
                    }
                }
                None => {
                    self.record_invariant_break(
                        "cond_signal_all.DestroyedMutex",
                        format_args!("cond waiter {waker:?} references destroyed mutex {mutex_id}"),
                    );
                    wake_with(
                        unit,
                        cell_errors::CELL_ESRCH.into(),
                        &mut woken_unit_ids,
                        &mut response_updates,
                    );
                }
            }
        }
        Lv2Dispatch::WakeAndReturn {
            code: 0,
            woken_unit_ids,
            response_updates,
            effects: vec![],
        }
    }

    pub(super) fn dispatch_cond_signal_to(&mut self, id: u32, target_thread: u32) -> Lv2Dispatch {
        let Some(entry) = self.conds.lookup(id) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        let mutex_id = entry.mutex_id();
        let mutex_kind = entry.mutex_kind();
        if !matches!(mutex_kind, CondMutexKind::Mutex) {
            return Lv2Dispatch::immediate(cell_errors::CELL_EPERM.into());
        }
        let target = PpuThreadId::new(target_thread as u64);
        match self.conds.signal_to(id, target) {
            Ok(()) => {}
            Err(crate::sync_primitives::CondSignalToError::UnknownId) => {
                return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
            }
            Err(crate::sync_primitives::CondSignalToError::TargetNotWaiting) => {
                return Lv2Dispatch::immediate(cell_errors::CELL_EPERM.into());
            }
        }
        self.cond_reacquire_wake(target, mutex_id, false)
    }

    pub(super) fn dispatch_cond_signal(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.conds.lookup(id) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        let mutex_id = entry.mutex_id();
        let mutex_kind = entry.mutex_kind();
        let Some(waker) = self.conds.signal_one(id) else {
            return Lv2Dispatch::immediate(0);
        };
        match mutex_kind {
            CondMutexKind::Mutex => self.cond_reacquire_wake(waker, mutex_id, false),
            CondMutexKind::LwMutex => Lv2Dispatch::immediate(cell_errors::CELL_EPERM.into()),
        }
    }

    /// Cond-wake mutex reacquire for one thread.
    fn cond_reacquire_wake(
        &mut self,
        waker: PpuThreadId,
        mutex_id: u32,
        use_lwmutex: bool,
    ) -> Lv2Dispatch {
        self.cond_reacquire_wake_calls = self.cond_reacquire_wake_calls.wrapping_add(1);
        debug_assert!(!use_lwmutex, "lwmutex cond re-acquire not wired");
        let Some(waker_unit) = self.resolve_wake_thread(waker, "cond_reacquire_wake") else {
            return Lv2Dispatch::immediate(0u64);
        };
        match self.mutexes.try_acquire(mutex_id, waker) {
            Some(crate::sync_primitives::MutexAcquire::Acquired) => {
                cond_wake_dispatch(waker_unit, 0u64, true)
            }
            Some(crate::sync_primitives::MutexAcquire::Contended) => {
                if let Err(err) = self.mutexes.enqueue_waiter(mutex_id, waker) {
                    self.record_invariant_break(
                        "cond_reacquire_wake.Contended.enqueue",
                        format_args!(
                            "enqueue_waiter failed for mutex {mutex_id} waker {waker:?}: \
                             {err:?}; waking with ESRCH to avoid stranding"
                        ),
                    );
                    return cond_wake_dispatch(waker_unit, cell_errors::CELL_ESRCH.into(), true);
                }
                cond_wake_dispatch(waker_unit, 0u64, false)
            }
            None => {
                self.record_invariant_break(
                    "cond_reacquire_wake.DestroyedMutex",
                    format_args!("cond waiter {waker:?} references destroyed mutex {mutex_id}"),
                );
                cond_wake_dispatch(waker_unit, cell_errors::CELL_ESRCH.into(), true)
            }
        }
    }
}

fn wake_with(
    unit: UnitId,
    code: u64,
    woken_unit_ids: &mut Vec<UnitId>,
    response_updates: &mut Vec<(UnitId, PendingResponse)>,
) {
    woken_unit_ids.push(unit);
    response_updates.push((unit, PendingResponse::ReturnCode { code }));
}

/// Single-target `Lv2Dispatch::WakeAndReturn` with one `ReturnCode`.
///
/// `woken=true` places `unit` in `woken_unit_ids`; `false` records
/// only a `response_updates` entry (re-parked on a mutex).
fn cond_wake_dispatch(unit: UnitId, code: u64, woken: bool) -> Lv2Dispatch {
    Lv2Dispatch::WakeAndReturn {
        code: 0u64,
        woken_unit_ids: if woken { vec![unit] } else { vec![] },
        response_updates: vec![(unit, PendingResponse::ReturnCode { code })],
        effects: vec![],
    }
}

#[cfg(test)]
#[path = "tests/cond_tests.rs"]
mod tests;
