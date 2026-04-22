//! Condition variable LV2 dispatch: create, destroy, wait, signal,
//! signal_all, signal_to.

use cellgov_event::UnitId;

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
        // Cond binds to an existing heavy mutex -- reject at
        // create time (matches RPCS3 lv2_obj::idm_get).
        if self.mutexes.lookup(mutex_id).is_none() {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        }
        let id = self.alloc_id();
        if self
            .conds
            .create_with_id(id, mutex_id, CondMutexKind::Mutex)
            .is_err()
        {
            // Same-binding duplicate OR different-binding (the
            // latter fires the cond-table debug_assert). Either
            // way the guest cannot use the id.
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ENOMEM.into(),
                effects: vec![],
            };
        }
        self.immediate_write_u32(id, id_ptr, requester)
    }

    pub(super) fn dispatch_cond_destroy(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.conds.lookup(id) else {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        if !entry.waiters().is_empty() {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_EBUSY.into(),
                effects: vec![],
            };
        }
        self.conds.destroy(id);
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    }

    pub(super) fn dispatch_cond_wait(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        let Some(entry) = self.conds.lookup(id) else {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        let mutex_id = entry.mutex_id();
        let mutex_kind = entry.mutex_kind();
        // Release the cond's mutex on the caller's behalf. Any
        // parked mutex waiter inherits ownership and wakes in the
        // same dispatch as the cond-wait block.
        let release = match mutex_kind {
            CondMutexKind::Mutex => self.mutexes.release_and_wake_next(mutex_id, caller),
            CondMutexKind::LwMutex => {
                // Defensive forward-compat for sys_lwcond;
                // sys_cond itself is heavy-only.
                return Lv2Dispatch::Immediate {
                    code: crate::errno::CELL_EPERM.into(),
                    effects: vec![],
                };
            }
        };
        match release {
            crate::sync_primitives::MutexRelease::Unknown => Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            },
            crate::sync_primitives::MutexRelease::NotOwner => Lv2Dispatch::Immediate {
                code: crate::errno::CELL_EPERM.into(),
                effects: vec![],
            },
            crate::sync_primitives::MutexRelease::Freed => {
                // Mutex had no waiter; park the caller on the
                // cond. DuplicateWaiter is a host-invariant break
                // (a cond waiter is Blocked and cannot re-enter).
                match self.conds.enqueue_waiter(id, caller) {
                    Ok(()) => {}
                    Err(crate::sync_primitives::CondEnqueueError::UnknownId) => {
                        return Lv2Dispatch::Immediate {
                            code: crate::errno::CELL_ESRCH.into(),
                            effects: vec![],
                        };
                    }
                    Err(crate::sync_primitives::CondEnqueueError::DuplicateWaiter) => {
                        self.record_invariant_break(
                            "cond_wait.Freed.DuplicateWaiter",
                            format_args!("cond {id}: caller {caller:?} already on waiter list"),
                        );
                        return Lv2Dispatch::Immediate {
                            code: crate::errno::CELL_EDEADLK.into(),
                            effects: vec![],
                        };
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
                // Mutex waiter inherited ownership and wakes in
                // the same dispatch as the cond park. See Freed
                // for DuplicateWaiter rationale.
                match self.conds.enqueue_waiter(id, caller) {
                    Ok(()) => {}
                    Err(crate::sync_primitives::CondEnqueueError::UnknownId) => {
                        return Lv2Dispatch::Immediate {
                            code: crate::errno::CELL_ESRCH.into(),
                            effects: vec![],
                        };
                    }
                    Err(crate::sync_primitives::CondEnqueueError::DuplicateWaiter) => {
                        self.record_invariant_break(
                            "cond_wait.Transferred.DuplicateWaiter",
                            format_args!("cond {id}: caller {caller:?} already on waiter list"),
                        );
                        return Lv2Dispatch::Immediate {
                            code: crate::errno::CELL_EDEADLK.into(),
                            effects: vec![],
                        };
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
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        let mutex_id = entry.mutex_id();
        let mutex_kind = entry.mutex_kind();
        if !matches!(mutex_kind, CondMutexKind::Mutex) {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_EPERM.into(),
                effects: vec![],
            };
        }
        // expect() is load-bearing: a future refactor that
        // slipped a destroy between `lookup` and `signal_all`
        // would silently wake zero threads under unwrap_or_default.
        let wakers = self
            .conds
            .signal_all(id)
            .expect("cond looked up just above must still exist: no intervening destroy");
        if wakers.is_empty() {
            return Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            };
        }
        // FIFO: first acquirer wakes cleanly if the mutex is
        // free; the rest re-park on the mutex with their pending
        // response swapped to ReturnCode{0} so unlock-wake
        // resolves them as CELL_OK.
        let mut woken_unit_ids: Vec<UnitId> = Vec::new();
        let mut response_updates: Vec<(UnitId, PendingResponse)> = Vec::new();
        for waker in wakers {
            let Some(unit) = self.resolve_wake_thread(waker, "cond_signal_all.waker") else {
                continue;
            };
            match self.mutexes.try_acquire(mutex_id, waker) {
                Some(crate::sync_primitives::MutexAcquire::Acquired) => {
                    woken_unit_ids.push(unit);
                    response_updates.push((unit, PendingResponse::ReturnCode { code: 0u64 }));
                }
                Some(crate::sync_primitives::MutexAcquire::Contended) => {
                    // DuplicateWaiter / WaiterIsOwner / UnknownId
                    // are all state corruption under the
                    // single-threaded commit model: record and
                    // wake with ESRCH rather than strand the
                    // waker off the mutex queue.
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
                            woken_unit_ids.push(unit);
                            response_updates.push((
                                unit,
                                PendingResponse::ReturnCode {
                                    code: crate::errno::CELL_ESRCH.into(),
                                },
                            ));
                        }
                    }
                }
                None => {
                    // Destroyed mutex -- mutex_destroy should
                    // reject EBUSY while cond waiters exist; this
                    // branch is a host-invariant break.
                    self.record_invariant_break(
                        "cond_signal_all.DestroyedMutex",
                        format_args!("cond waiter {waker:?} references destroyed mutex {mutex_id}"),
                    );
                    woken_unit_ids.push(unit);
                    response_updates.push((
                        unit,
                        PendingResponse::ReturnCode {
                            code: crate::errno::CELL_ESRCH.into(),
                        },
                    ));
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
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        let mutex_id = entry.mutex_id();
        let mutex_kind = entry.mutex_kind();
        if !matches!(mutex_kind, CondMutexKind::Mutex) {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_EPERM.into(),
                effects: vec![],
            };
        }
        let target = PpuThreadId::new(target_thread as u64);
        // Unknown cond is ESRCH; target-not-parked is EPERM
        // (RPCS3 `rpcs3/Emu/Cell/lv2/sys_cond.cpp:391-396`).
        // Collapsing both would leak waiter state through the
        // signaler's errno.
        match self.conds.signal_to(id, target) {
            Ok(()) => {}
            Err(crate::sync_primitives::CondSignalToError::UnknownId) => {
                return Lv2Dispatch::Immediate {
                    code: crate::errno::CELL_ESRCH.into(),
                    effects: vec![],
                };
            }
            Err(crate::sync_primitives::CondSignalToError::TargetNotWaiting) => {
                return Lv2Dispatch::Immediate {
                    code: crate::errno::CELL_EPERM.into(),
                    effects: vec![],
                };
            }
        }
        self.cond_reacquire_wake(target, mutex_id, false)
    }

    pub(super) fn dispatch_cond_signal(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.conds.lookup(id) else {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        let mutex_id = entry.mutex_id();
        let mutex_kind = entry.mutex_kind();
        // Non-sticky: a signal with no parked waiter is lost.
        let Some(waker) = self.conds.signal_one(id) else {
            return Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            };
        };
        match mutex_kind {
            CondMutexKind::Mutex => self.cond_reacquire_wake(waker, mutex_id, false),
            CondMutexKind::LwMutex => Lv2Dispatch::Immediate {
                // sys_cond is heavy-only; lwmutex is EPERM.
                code: crate::errno::CELL_EPERM.into(),
                effects: vec![],
            },
        }
    }

    /// Cond-wake re-acquire for one thread. The waker holds a
    /// `PendingResponse::CondWakeReacquire` from the wait side;
    /// this helper either acquires the mutex on its behalf and
    /// wakes it with `ReturnCode{0}`, or re-parks it on the mutex
    /// waiter list so the next unlock-wake resolves it.
    /// `use_lwmutex` is forward-compat for sys_lwcond.
    fn cond_reacquire_wake(
        &mut self,
        waker: PpuThreadId,
        mutex_id: u32,
        use_lwmutex: bool,
    ) -> Lv2Dispatch {
        debug_assert!(!use_lwmutex, "lwmutex cond re-acquire not wired");
        // Waker is already off the cond; missing thread-table
        // entry strands it.
        let Some(waker_unit) = self.resolve_wake_thread(waker, "cond_reacquire_wake") else {
            return Lv2Dispatch::Immediate {
                code: 0u64,
                effects: vec![],
            };
        };
        match self.mutexes.try_acquire(mutex_id, waker) {
            Some(crate::sync_primitives::MutexAcquire::Acquired) => Lv2Dispatch::WakeAndReturn {
                code: 0u64,
                woken_unit_ids: vec![waker_unit],
                response_updates: vec![(waker_unit, PendingResponse::ReturnCode { code: 0u64 })],
                effects: vec![],
            },
            Some(crate::sync_primitives::MutexAcquire::Contended) => {
                // See `cond_signal_all` Contended: Err is state
                // corruption under the single-threaded commit
                // model; wake with ESRCH rather than strand.
                if let Err(err) = self.mutexes.enqueue_waiter(mutex_id, waker) {
                    self.record_invariant_break(
                        "cond_reacquire_wake.Contended.enqueue",
                        format_args!(
                            "enqueue_waiter failed for mutex {mutex_id} waker {waker:?}: \
                             {err:?}; waking with ESRCH to avoid stranding"
                        ),
                    );
                    return Lv2Dispatch::WakeAndReturn {
                        code: 0u64,
                        woken_unit_ids: vec![waker_unit],
                        response_updates: vec![(
                            waker_unit,
                            PendingResponse::ReturnCode {
                                code: crate::errno::CELL_ESRCH.into(),
                            },
                        )],
                        effects: vec![],
                    };
                }
                Lv2Dispatch::WakeAndReturn {
                    code: 0u64,
                    woken_unit_ids: vec![],
                    response_updates: vec![(
                        waker_unit,
                        PendingResponse::ReturnCode { code: 0u64 },
                    )],
                    effects: vec![],
                }
            }
            None => {
                // Destroyed mutex: wake the waker with ESRCH
                // rather than strand them. Signaler stays CELL_OK
                // so its errno does not leak waker presence.
                self.record_invariant_break(
                    "cond_reacquire_wake.DestroyedMutex",
                    format_args!("cond waiter {waker:?} references destroyed mutex {mutex_id}"),
                );
                Lv2Dispatch::WakeAndReturn {
                    code: 0u64,
                    woken_unit_ids: vec![waker_unit],
                    response_updates: vec![(
                        waker_unit,
                        PendingResponse::ReturnCode {
                            code: crate::errno::CELL_ESRCH.into(),
                        },
                    )],
                    effects: vec![],
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::test_support::{
        create_mutex_host, extract_write_u32, primary_attrs, seed_primary_ppu, FakeRuntime,
    };
    use crate::ppu_thread::PpuThreadId;
    use crate::request::Lv2Request;

    #[test]
    fn cond_create_writes_id_and_binds_mutex() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let mutex_id = create_mutex_host(&mut host, src, &rt);
        let created = host.dispatch(
            Lv2Request::CondCreate {
                id_ptr: 0x200,
                mutex_id,
                attr_ptr: 0,
            },
            src,
            &rt,
        );
        let cond_id = match &created {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        let entry = host.conds().lookup(cond_id).unwrap();
        assert_eq!(entry.mutex_id(), mutex_id);
        assert_eq!(entry.mutex_kind(), CondMutexKind::Mutex);
        assert!(entry.waiters().is_empty());
    }

    #[test]
    fn cond_create_unknown_mutex_returns_esrch() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(
            Lv2Request::CondCreate {
                id_ptr: 0x200,
                mutex_id: 0xDEAD,
                attr_ptr: 0,
            },
            src,
            &rt,
        );
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_ESRCH.into());
        assert!(host.conds().is_empty());
    }

    #[test]
    fn cond_destroy_empty_succeeds() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let mutex_id = create_mutex_host(&mut host, src, &rt);
        let created = host.dispatch(
            Lv2Request::CondCreate {
                id_ptr: 0x200,
                mutex_id,
                attr_ptr: 0,
            },
            src,
            &rt,
        );
        let cond_id = match &created {
            Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate, got {other:?}"),
        };
        let r = host.dispatch(Lv2Request::CondDestroy { id: cond_id }, src, &rt);
        assert!(matches!(
            r,
            Lv2Dispatch::Immediate {
                code: 0,
                effects: _,
            }
        ));
        assert!(host.conds().lookup(cond_id).is_none());
    }

    #[test]
    fn cond_destroy_unknown_returns_esrch() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(Lv2Request::CondDestroy { id: 0xDEAD }, src, &rt);
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_ESRCH.into());
    }

    #[test]
    fn cond_wait_releases_mutex_and_parks_caller() {
        // Caller holds the mutex, no mutex waiters. cond_wait must
        // drop the mutex (owner cleared) and park the caller on the
        // cond with a CondWakeReacquire pending response.
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let mutex_id = create_mutex_host(&mut host, src, &rt);
        // Acquire the mutex.
        host.dispatch(
            Lv2Request::MutexLock {
                mutex_id,
                timeout: 0,
            },
            src,
            &rt,
        );
        // Create the cond.
        let created = host.dispatch(
            Lv2Request::CondCreate {
                id_ptr: 0x200,
                mutex_id,
                attr_ptr: 0,
            },
            src,
            &rt,
        );
        let cond_id = match &created {
            Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate, got {other:?}"),
        };
        // Wait.
        let r = host.dispatch(
            Lv2Request::CondWait {
                id: cond_id,
                timeout: 0,
            },
            src,
            &rt,
        );
        match r {
            Lv2Dispatch::Block {
                reason, pending, ..
            } => {
                assert!(matches!(
                    reason,
                    crate::dispatch::Lv2BlockReason::Cond {
                        id,
                        mutex_id: m,
                        ..
                    } if id == cond_id && m == mutex_id
                ));
                assert!(matches!(
                    pending,
                    PendingResponse::CondWakeReacquire {
                        mutex_id: m,
                        mutex_kind: CondMutexKind::Mutex,
                    } if m == mutex_id
                ));
            }
            other => panic!("expected Block, got {other:?}"),
        }
        // Mutex is now unowned.
        assert_eq!(host.mutexes().lookup(mutex_id).unwrap().owner(), None);
        // Cond has the caller parked.
        assert_eq!(
            host.conds()
                .lookup(cond_id)
                .unwrap()
                .waiters()
                .iter()
                .collect::<Vec<_>>(),
            vec![PpuThreadId::PRIMARY],
        );
    }

    #[test]
    fn cond_wait_transfers_mutex_to_waiter_via_block_and_wake() {
        // Two threads contend on the mutex. When the owner calls
        // cond_wait, the mutex waiter inherits ownership and must
        // wake alongside the owner's cond park. The handler emits
        // BlockAndWake.
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let owner_unit = UnitId::new(0);
        let waiter_unit = UnitId::new(1);
        seed_primary_ppu(&mut host, owner_unit);
        let waiter_tid = host
            .ppu_threads_mut()
            .create(waiter_unit, primary_attrs())
            .expect("waiter create");
        let mutex_id = create_mutex_host(&mut host, owner_unit, &rt);
        host.dispatch(
            Lv2Request::MutexLock {
                mutex_id,
                timeout: 0,
            },
            owner_unit,
            &rt,
        );
        host.dispatch(
            Lv2Request::MutexLock {
                mutex_id,
                timeout: 0,
            },
            waiter_unit,
            &rt,
        );
        // waiter is now parked on the mutex.
        assert_eq!(
            host.mutexes()
                .lookup(mutex_id)
                .unwrap()
                .waiters()
                .iter()
                .collect::<Vec<_>>(),
            vec![waiter_tid],
        );
        let created = host.dispatch(
            Lv2Request::CondCreate {
                id_ptr: 0x200,
                mutex_id,
                attr_ptr: 0,
            },
            owner_unit,
            &rt,
        );
        let cond_id = match &created {
            Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate, got {other:?}"),
        };
        let r = host.dispatch(
            Lv2Request::CondWait {
                id: cond_id,
                timeout: 0,
            },
            owner_unit,
            &rt,
        );
        match r {
            Lv2Dispatch::BlockAndWake {
                reason,
                pending,
                woken_unit_ids,
                ..
            } => {
                assert!(matches!(
                    reason,
                    crate::dispatch::Lv2BlockReason::Cond { .. }
                ));
                assert!(matches!(pending, PendingResponse::CondWakeReacquire { .. }));
                assert_eq!(woken_unit_ids, vec![waiter_unit]);
            }
            other => panic!("expected BlockAndWake, got {other:?}"),
        }
        // Ownership transferred to the waiter.
        assert_eq!(
            host.mutexes().lookup(mutex_id).unwrap().owner(),
            Some(waiter_tid),
        );
        // Owner is now parked on the cond.
        assert_eq!(
            host.conds()
                .lookup(cond_id)
                .unwrap()
                .waiters()
                .iter()
                .collect::<Vec<_>>(),
            vec![PpuThreadId::PRIMARY],
        );
    }

    #[test]
    fn cond_wait_by_non_owner_returns_eperm() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let owner_unit = UnitId::new(0);
        let other_unit = UnitId::new(1);
        seed_primary_ppu(&mut host, owner_unit);
        host.ppu_threads_mut()
            .create(other_unit, primary_attrs())
            .expect("other create");
        let mutex_id = create_mutex_host(&mut host, owner_unit, &rt);
        host.dispatch(
            Lv2Request::MutexLock {
                mutex_id,
                timeout: 0,
            },
            owner_unit,
            &rt,
        );
        let created = host.dispatch(
            Lv2Request::CondCreate {
                id_ptr: 0x200,
                mutex_id,
                attr_ptr: 0,
            },
            owner_unit,
            &rt,
        );
        let cond_id = match &created {
            Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate, got {other:?}"),
        };
        // Non-owner attempts cond_wait; mutex release rejects with
        // NotOwner -> EPERM.
        let r = host.dispatch(
            Lv2Request::CondWait {
                id: cond_id,
                timeout: 0,
            },
            other_unit,
            &rt,
        );
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_EPERM.into());
        // Mutex ownership unchanged.
        assert_eq!(
            host.mutexes().lookup(mutex_id).unwrap().owner(),
            Some(PpuThreadId::PRIMARY),
        );
        // Cond is still empty.
        assert!(host.conds().lookup(cond_id).unwrap().waiters().is_empty());
    }

    #[test]
    fn cond_wait_unknown_id_returns_esrch() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(
            Lv2Request::CondWait {
                id: 0xDEAD,
                timeout: 0,
            },
            src,
            &rt,
        );
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_ESRCH.into());
    }

    #[test]
    fn cond_signal_no_waiter_is_observably_lost() {
        // Non-sticky: signal on a cond with no waiters returns
        // CELL_OK and does not record any pending state.
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let mutex_id = create_mutex_host(&mut host, src, &rt);
        let created = host.dispatch(
            Lv2Request::CondCreate {
                id_ptr: 0x200,
                mutex_id,
                attr_ptr: 0,
            },
            src,
            &rt,
        );
        let cond_id = match &created {
            Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate, got {other:?}"),
        };
        let r = host.dispatch(Lv2Request::CondSignal { id: cond_id }, src, &rt);
        assert!(matches!(
            r,
            Lv2Dispatch::Immediate {
                code: 0,
                effects: _,
            }
        ));
        // Cond stays empty; hash-level: same as a cond that never
        // received a signal (anchored by
        // state_hash_ignores_ephemeral_signal_attempts in cond
        // table tests).
        assert!(host.conds().lookup(cond_id).unwrap().waiters().is_empty());
    }

    #[test]
    fn cond_signal_unknown_id_returns_esrch() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(Lv2Request::CondSignal { id: 0xDEAD }, src, &rt);
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_ESRCH.into());
    }

    #[test]
    fn cond_signal_wakes_waiter_cleanly_when_mutex_free() {
        // Waiter parked via cond_wait, no other thread holds the
        // mutex. Signaler wakes the waiter; the waker acquires the
        // mutex and its pending response is swapped to
        // ReturnCode { 0 }.
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let waiter_unit = UnitId::new(0);
        let signaler_unit = UnitId::new(1);
        seed_primary_ppu(&mut host, waiter_unit);
        host.ppu_threads_mut()
            .create(signaler_unit, primary_attrs())
            .expect("signaler create");
        let mutex_id = create_mutex_host(&mut host, waiter_unit, &rt);
        host.dispatch(
            Lv2Request::MutexLock {
                mutex_id,
                timeout: 0,
            },
            waiter_unit,
            &rt,
        );
        let created = host.dispatch(
            Lv2Request::CondCreate {
                id_ptr: 0x200,
                mutex_id,
                attr_ptr: 0,
            },
            waiter_unit,
            &rt,
        );
        let cond_id = match &created {
            Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate, got {other:?}"),
        };
        host.dispatch(
            Lv2Request::CondWait {
                id: cond_id,
                timeout: 0,
            },
            waiter_unit,
            &rt,
        );
        // Mutex now free (waiter released on cond_wait).
        assert_eq!(host.mutexes().lookup(mutex_id).unwrap().owner(), None);
        let r = host.dispatch(Lv2Request::CondSignal { id: cond_id }, signaler_unit, &rt);
        match r {
            Lv2Dispatch::WakeAndReturn {
                code,
                woken_unit_ids,
                response_updates,
                ..
            } => {
                assert_eq!(code, 0);
                assert_eq!(woken_unit_ids, vec![waiter_unit]);
                assert_eq!(response_updates.len(), 1);
                assert_eq!(response_updates[0].0, waiter_unit);
                assert!(matches!(
                    response_updates[0].1,
                    PendingResponse::ReturnCode { code: 0 }
                ));
            }
            other => panic!("expected WakeAndReturn, got {other:?}"),
        }
        // Waker is now the mutex owner.
        assert_eq!(
            host.mutexes().lookup(mutex_id).unwrap().owner(),
            Some(PpuThreadId::PRIMARY),
        );
        // Cond is empty.
        assert!(host.conds().lookup(cond_id).unwrap().waiters().is_empty());
    }

    #[test]
    fn cond_signal_reparks_waiter_on_mutex_when_held() {
        // Waiter parked via cond_wait. Before signal, a THIRD
        // thread acquires the mutex. Signal fires but finds the
        // mutex held; the cond waiter transitions to the mutex
        // waiter list (its pending response swaps from
        // CondWakeReacquire to ReturnCode { 0 }). Signaler returns
        // CELL_OK with no wake.
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let waiter_unit = UnitId::new(0);
        let third_unit = UnitId::new(1);
        let signaler_unit = UnitId::new(2);
        seed_primary_ppu(&mut host, waiter_unit);
        let third_tid = host
            .ppu_threads_mut()
            .create(third_unit, primary_attrs())
            .expect("third create");
        host.ppu_threads_mut()
            .create(signaler_unit, primary_attrs())
            .expect("signaler create");
        let mutex_id = create_mutex_host(&mut host, waiter_unit, &rt);
        // Waiter takes the mutex.
        host.dispatch(
            Lv2Request::MutexLock {
                mutex_id,
                timeout: 0,
            },
            waiter_unit,
            &rt,
        );
        let created = host.dispatch(
            Lv2Request::CondCreate {
                id_ptr: 0x200,
                mutex_id,
                attr_ptr: 0,
            },
            waiter_unit,
            &rt,
        );
        let cond_id = match &created {
            Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate, got {other:?}"),
        };
        // Waiter calls cond_wait (releases mutex).
        host.dispatch(
            Lv2Request::CondWait {
                id: cond_id,
                timeout: 0,
            },
            waiter_unit,
            &rt,
        );
        // Third thread now takes the mutex.
        host.dispatch(
            Lv2Request::MutexLock {
                mutex_id,
                timeout: 0,
            },
            third_unit,
            &rt,
        );
        assert_eq!(
            host.mutexes().lookup(mutex_id).unwrap().owner(),
            Some(third_tid),
        );
        // Signaler fires. Waiter should re-park on mutex.
        let r = host.dispatch(Lv2Request::CondSignal { id: cond_id }, signaler_unit, &rt);
        match r {
            Lv2Dispatch::WakeAndReturn {
                code,
                woken_unit_ids,
                response_updates,
                ..
            } => {
                assert_eq!(code, 0);
                assert!(
                    woken_unit_ids.is_empty(),
                    "signal with mutex-held must not wake"
                );
                assert_eq!(response_updates.len(), 1);
                assert_eq!(response_updates[0].0, waiter_unit);
                assert!(matches!(
                    response_updates[0].1,
                    PendingResponse::ReturnCode { code: 0 }
                ));
            }
            other => panic!("expected WakeAndReturn with empty wake, got {other:?}"),
        }
        // Mutex owner unchanged (still the third thread).
        assert_eq!(
            host.mutexes().lookup(mutex_id).unwrap().owner(),
            Some(third_tid),
        );
        // Waiter is now parked on the mutex waiter list.
        assert_eq!(
            host.mutexes()
                .lookup(mutex_id)
                .unwrap()
                .waiters()
                .iter()
                .collect::<Vec<_>>(),
            vec![PpuThreadId::PRIMARY],
        );
        // Cond list is empty.
        assert!(host.conds().lookup(cond_id).unwrap().waiters().is_empty());
    }

    #[test]
    fn cond_signal_wakes_fifo_head_when_multiple_waiters() {
        // Two waiters parked in cond. First signal wakes the head;
        // second waiter stays parked.
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let w1_unit = UnitId::new(0);
        let w2_unit = UnitId::new(1);
        let signaler_unit = UnitId::new(2);
        seed_primary_ppu(&mut host, w1_unit);
        let w2_tid = host
            .ppu_threads_mut()
            .create(w2_unit, primary_attrs())
            .expect("w2 create");
        host.ppu_threads_mut()
            .create(signaler_unit, primary_attrs())
            .expect("signaler create");
        let mutex_id = create_mutex_host(&mut host, w1_unit, &rt);
        // Waiter 1 acquires mutex and parks on cond.
        host.dispatch(
            Lv2Request::MutexLock {
                mutex_id,
                timeout: 0,
            },
            w1_unit,
            &rt,
        );
        let created = host.dispatch(
            Lv2Request::CondCreate {
                id_ptr: 0x200,
                mutex_id,
                attr_ptr: 0,
            },
            w1_unit,
            &rt,
        );
        let cond_id = match &created {
            Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate, got {other:?}"),
        };
        host.dispatch(
            Lv2Request::CondWait {
                id: cond_id,
                timeout: 0,
            },
            w1_unit,
            &rt,
        );
        // Waiter 2 acquires mutex and parks on cond.
        host.dispatch(
            Lv2Request::MutexLock {
                mutex_id,
                timeout: 0,
            },
            w2_unit,
            &rt,
        );
        host.dispatch(
            Lv2Request::CondWait {
                id: cond_id,
                timeout: 0,
            },
            w2_unit,
            &rt,
        );
        // First signal wakes w1 (FIFO head).
        let r = host.dispatch(Lv2Request::CondSignal { id: cond_id }, signaler_unit, &rt);
        match r {
            Lv2Dispatch::WakeAndReturn { woken_unit_ids, .. } => {
                assert_eq!(woken_unit_ids, vec![w1_unit]);
            }
            other => panic!("expected WakeAndReturn, got {other:?}"),
        }
        // w2 still parked.
        assert_eq!(
            host.conds()
                .lookup(cond_id)
                .unwrap()
                .waiters()
                .iter()
                .collect::<Vec<_>>(),
            vec![w2_tid],
        );
    }

    #[test]
    fn cond_signal_all_wakes_first_reparks_rest_when_mutex_free() {
        // Three cond waiters parked. Mutex free at signal_all
        // time: first waiter acquires and wakes; second and third
        // re-park on the mutex waiter list. Order preserved (FIFO
        // from the cond list).
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let w1 = UnitId::new(0);
        let w2 = UnitId::new(1);
        let w3 = UnitId::new(2);
        let signaler = UnitId::new(3);
        seed_primary_ppu(&mut host, w1);
        let w2_tid = host
            .ppu_threads_mut()
            .create(w2, primary_attrs())
            .expect("w2 create");
        let w3_tid = host
            .ppu_threads_mut()
            .create(w3, primary_attrs())
            .expect("w3 create");
        host.ppu_threads_mut()
            .create(signaler, primary_attrs())
            .expect("signaler create");
        let mutex_id = create_mutex_host(&mut host, w1, &rt);
        let created = host.dispatch(
            Lv2Request::CondCreate {
                id_ptr: 0x200,
                mutex_id,
                attr_ptr: 0,
            },
            w1,
            &rt,
        );
        let cond_id = match &created {
            Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate, got {other:?}"),
        };
        for unit in [w1, w2, w3] {
            host.dispatch(
                Lv2Request::MutexLock {
                    mutex_id,
                    timeout: 0,
                },
                unit,
                &rt,
            );
            host.dispatch(
                Lv2Request::CondWait {
                    id: cond_id,
                    timeout: 0,
                },
                unit,
                &rt,
            );
        }
        // All three are now parked on cond; mutex free.
        assert_eq!(host.conds().lookup(cond_id).unwrap().waiters().len(), 3);
        assert_eq!(host.mutexes().lookup(mutex_id).unwrap().owner(), None);
        let r = host.dispatch(Lv2Request::CondSignalAll { id: cond_id }, signaler, &rt);
        match r {
            Lv2Dispatch::WakeAndReturn {
                woken_unit_ids,
                response_updates,
                ..
            } => {
                // Only w1 (head of FIFO) wakes cleanly.
                assert_eq!(woken_unit_ids, vec![w1]);
                // All three get response swapped.
                let updated_units: Vec<_> = response_updates.iter().map(|(u, _)| *u).collect();
                assert_eq!(updated_units, vec![w1, w2, w3]);
            }
            other => panic!("expected WakeAndReturn, got {other:?}"),
        }
        // w1 owns mutex; w2, w3 parked on mutex waiter list FIFO.
        assert_eq!(
            host.mutexes().lookup(mutex_id).unwrap().owner(),
            Some(PpuThreadId::PRIMARY),
        );
        assert_eq!(
            host.mutexes()
                .lookup(mutex_id)
                .unwrap()
                .waiters()
                .iter()
                .collect::<Vec<_>>(),
            vec![w2_tid, w3_tid],
        );
        assert!(host.conds().lookup(cond_id).unwrap().waiters().is_empty());
    }

    #[test]
    fn cond_signal_all_reparks_all_when_mutex_held() {
        // Three cond waiters parked, then a fourth thread takes
        // the mutex. signal_all: all three waiters re-park on the
        // mutex list; no one wakes.
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let w1 = UnitId::new(0);
        let w2 = UnitId::new(1);
        let w3 = UnitId::new(2);
        let holder = UnitId::new(3);
        let signaler = UnitId::new(4);
        seed_primary_ppu(&mut host, w1);
        let w2_tid = host
            .ppu_threads_mut()
            .create(w2, primary_attrs())
            .expect("w2 create");
        let w3_tid = host
            .ppu_threads_mut()
            .create(w3, primary_attrs())
            .expect("w3 create");
        let holder_tid = host
            .ppu_threads_mut()
            .create(holder, primary_attrs())
            .expect("holder create");
        host.ppu_threads_mut()
            .create(signaler, primary_attrs())
            .expect("signaler create");
        let mutex_id = create_mutex_host(&mut host, w1, &rt);
        let created = host.dispatch(
            Lv2Request::CondCreate {
                id_ptr: 0x200,
                mutex_id,
                attr_ptr: 0,
            },
            w1,
            &rt,
        );
        let cond_id = match &created {
            Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate, got {other:?}"),
        };
        for unit in [w1, w2, w3] {
            host.dispatch(
                Lv2Request::MutexLock {
                    mutex_id,
                    timeout: 0,
                },
                unit,
                &rt,
            );
            host.dispatch(
                Lv2Request::CondWait {
                    id: cond_id,
                    timeout: 0,
                },
                unit,
                &rt,
            );
        }
        // Holder takes the mutex.
        host.dispatch(
            Lv2Request::MutexLock {
                mutex_id,
                timeout: 0,
            },
            holder,
            &rt,
        );
        assert_eq!(
            host.mutexes().lookup(mutex_id).unwrap().owner(),
            Some(holder_tid),
        );
        let r = host.dispatch(Lv2Request::CondSignalAll { id: cond_id }, signaler, &rt);
        match r {
            Lv2Dispatch::WakeAndReturn {
                woken_unit_ids,
                response_updates,
                ..
            } => {
                assert!(woken_unit_ids.is_empty(), "nobody wakes when mutex is held");
                assert_eq!(response_updates.len(), 3);
            }
            other => panic!("expected WakeAndReturn, got {other:?}"),
        }
        // All three waiters parked on mutex in FIFO order.
        assert_eq!(
            host.mutexes()
                .lookup(mutex_id)
                .unwrap()
                .waiters()
                .iter()
                .collect::<Vec<_>>(),
            vec![PpuThreadId::PRIMARY, w2_tid, w3_tid],
        );
        assert!(host.conds().lookup(cond_id).unwrap().waiters().is_empty());
    }

    #[test]
    fn cond_signal_all_no_waiters_is_lost() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let mutex_id = create_mutex_host(&mut host, src, &rt);
        let created = host.dispatch(
            Lv2Request::CondCreate {
                id_ptr: 0x200,
                mutex_id,
                attr_ptr: 0,
            },
            src,
            &rt,
        );
        let cond_id = match &created {
            Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate, got {other:?}"),
        };
        let r = host.dispatch(Lv2Request::CondSignalAll { id: cond_id }, src, &rt);
        assert!(matches!(
            r,
            Lv2Dispatch::Immediate {
                code: 0,
                effects: _,
            }
        ));
    }

    #[test]
    fn cond_signal_all_unknown_id_returns_esrch() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(Lv2Request::CondSignalAll { id: 0xDEAD }, src, &rt);
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_ESRCH.into());
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn cond_signal_all_flags_invariant_break_on_double_parked_waker() {
        // The single-threaded commit model forecloses any
        // dispatch path that leaves a waiter on both the cond and
        // the mutex queue; this test seeds that state via direct
        // table manipulation to verify the release-mode
        // diagnostic path.
        //
        // Expected: record_invariant_break fires; the waker is
        // woken with CELL_ESRCH rather than stranded; the mutex
        // queue retains its single existing entry.
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let waker_unit = UnitId::new(0);
        let signaler_unit = UnitId::new(1);
        seed_primary_ppu(&mut host, waker_unit);
        host.ppu_threads_mut()
            .create(signaler_unit, primary_attrs())
            .expect("signaler create");
        let mutex_id = create_mutex_host(&mut host, waker_unit, &rt);
        host.dispatch(
            Lv2Request::MutexLock {
                mutex_id,
                timeout: 0,
            },
            signaler_unit,
            &rt,
        );
        let created = host.dispatch(
            Lv2Request::CondCreate {
                id_ptr: 0x200,
                mutex_id,
                attr_ptr: 0,
            },
            signaler_unit,
            &rt,
        );
        let cond_id = match &created {
            Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate, got {other:?}"),
        };
        host.conds_mut()
            .enqueue_waiter(cond_id, PpuThreadId::PRIMARY)
            .unwrap();
        host.mutexes_mut()
            .enqueue_waiter(mutex_id, PpuThreadId::PRIMARY)
            .unwrap();
        let breaks_before = host.invariant_break_count();
        let r = host.dispatch(
            Lv2Request::CondSignalAll { id: cond_id },
            signaler_unit,
            &rt,
        );
        match r {
            Lv2Dispatch::WakeAndReturn {
                woken_unit_ids,
                response_updates,
                ..
            } => {
                assert_eq!(woken_unit_ids, vec![waker_unit]);
                assert_eq!(response_updates.len(), 1);
                assert_eq!(response_updates[0].0, waker_unit);
                assert!(matches!(
                    response_updates[0].1,
                    PendingResponse::ReturnCode { code } if code == crate::errno::CELL_ESRCH.into()
                ));
            }
            other => panic!("expected WakeAndReturn, got {other:?}"),
        }
        assert!(host.invariant_break_count() > breaks_before);
        // Mutex queue retains the pre-existing entry; the
        // duplicate enqueue was rejected at the primitive layer.
        assert_eq!(
            host.mutexes()
                .lookup(mutex_id)
                .unwrap()
                .waiters()
                .iter()
                .collect::<Vec<_>>(),
            vec![PpuThreadId::PRIMARY],
        );
        assert!(host.conds().lookup(cond_id).unwrap().waiters().is_empty());
    }

    #[test]
    fn cond_signal_to_targets_specific_waiter_and_preserves_order() {
        // Three cond waiters parked (w1, w2, w3). signal_to(w2)
        // must wake exactly w2, leaving w1 and w3 parked in their
        // original relative order.
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let w1 = UnitId::new(0);
        let w2 = UnitId::new(1);
        let w3 = UnitId::new(2);
        let signaler = UnitId::new(3);
        seed_primary_ppu(&mut host, w1);
        let w2_tid = host
            .ppu_threads_mut()
            .create(w2, primary_attrs())
            .expect("w2 create");
        let w3_tid = host
            .ppu_threads_mut()
            .create(w3, primary_attrs())
            .expect("w3 create");
        host.ppu_threads_mut()
            .create(signaler, primary_attrs())
            .expect("signaler create");
        let mutex_id = create_mutex_host(&mut host, w1, &rt);
        let created = host.dispatch(
            Lv2Request::CondCreate {
                id_ptr: 0x200,
                mutex_id,
                attr_ptr: 0,
            },
            w1,
            &rt,
        );
        let cond_id = match &created {
            Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate, got {other:?}"),
        };
        for unit in [w1, w2, w3] {
            host.dispatch(
                Lv2Request::MutexLock {
                    mutex_id,
                    timeout: 0,
                },
                unit,
                &rt,
            );
            host.dispatch(
                Lv2Request::CondWait {
                    id: cond_id,
                    timeout: 0,
                },
                unit,
                &rt,
            );
        }
        // Mutex free; all three parked on cond.
        assert_eq!(host.mutexes().lookup(mutex_id).unwrap().owner(), None);
        assert_eq!(host.conds().lookup(cond_id).unwrap().waiters().len(), 3);
        // Signal specifically at w2.
        let r = host.dispatch(
            Lv2Request::CondSignalTo {
                id: cond_id,
                target_thread: w2_tid.raw() as u32,
            },
            signaler,
            &rt,
        );
        match r {
            Lv2Dispatch::WakeAndReturn {
                code,
                woken_unit_ids,
                response_updates,
                ..
            } => {
                assert_eq!(code, 0);
                assert_eq!(woken_unit_ids, vec![w2]);
                assert_eq!(response_updates.len(), 1);
                assert_eq!(response_updates[0].0, w2);
                assert!(matches!(
                    response_updates[0].1,
                    PendingResponse::ReturnCode { code: 0 }
                ));
            }
            other => panic!("expected WakeAndReturn, got {other:?}"),
        }
        // w2 now owns the mutex.
        assert_eq!(
            host.mutexes().lookup(mutex_id).unwrap().owner(),
            Some(w2_tid)
        );
        // Cond still has w1 then w3 (relative order preserved).
        assert_eq!(
            host.conds()
                .lookup(cond_id)
                .unwrap()
                .waiters()
                .iter()
                .collect::<Vec<_>>(),
            vec![PpuThreadId::PRIMARY, w3_tid],
        );
    }

    #[test]
    fn cond_signal_to_target_not_waiting_returns_eperm() {
        // target is a real thread but not parked on this cond. Per
        // RPCS3 sys_cond.cpp:391-396 the errno is EPERM, distinct
        // from the "unknown cond" ESRCH case.
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let w1 = UnitId::new(0);
        let other = UnitId::new(1);
        let signaler = UnitId::new(2);
        seed_primary_ppu(&mut host, w1);
        let other_tid = host
            .ppu_threads_mut()
            .create(other, primary_attrs())
            .expect("other create");
        host.ppu_threads_mut()
            .create(signaler, primary_attrs())
            .expect("signaler create");
        let mutex_id = create_mutex_host(&mut host, w1, &rt);
        let created = host.dispatch(
            Lv2Request::CondCreate {
                id_ptr: 0x200,
                mutex_id,
                attr_ptr: 0,
            },
            w1,
            &rt,
        );
        let cond_id = match &created {
            Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate, got {other:?}"),
        };
        // Park w1 on cond; do NOT park `other`.
        host.dispatch(
            Lv2Request::MutexLock {
                mutex_id,
                timeout: 0,
            },
            w1,
            &rt,
        );
        host.dispatch(
            Lv2Request::CondWait {
                id: cond_id,
                timeout: 0,
            },
            w1,
            &rt,
        );
        // signal_to at `other` (not parked) -> EPERM per RPCS3.
        let r = host.dispatch(
            Lv2Request::CondSignalTo {
                id: cond_id,
                target_thread: other_tid.raw() as u32,
            },
            signaler,
            &rt,
        );
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_EPERM.into());
        // w1 remains parked on cond.
        assert_eq!(
            host.conds()
                .lookup(cond_id)
                .unwrap()
                .waiters()
                .iter()
                .collect::<Vec<_>>(),
            vec![PpuThreadId::PRIMARY],
        );
    }

    #[test]
    fn cond_signal_to_unknown_cond_returns_esrch() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(
            Lv2Request::CondSignalTo {
                id: 0xDEAD,
                target_thread: PpuThreadId::PRIMARY.raw() as u32,
            },
            src,
            &rt,
        );
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_ESRCH.into());
    }

    #[test]
    fn cond_signal_to_reparks_target_on_mutex_when_held() {
        // Two cond waiters parked (w1, w2). A third thread (holder)
        // takes the mutex. signal_to(w1) must re-park w1 on the
        // mutex waiter list (no wake), leaving w2 on cond.
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let w1 = UnitId::new(0);
        let w2 = UnitId::new(1);
        let holder = UnitId::new(2);
        let signaler = UnitId::new(3);
        seed_primary_ppu(&mut host, w1);
        let w2_tid = host
            .ppu_threads_mut()
            .create(w2, primary_attrs())
            .expect("w2 create");
        let holder_tid = host
            .ppu_threads_mut()
            .create(holder, primary_attrs())
            .expect("holder create");
        host.ppu_threads_mut()
            .create(signaler, primary_attrs())
            .expect("signaler create");
        let mutex_id = create_mutex_host(&mut host, w1, &rt);
        let created = host.dispatch(
            Lv2Request::CondCreate {
                id_ptr: 0x200,
                mutex_id,
                attr_ptr: 0,
            },
            w1,
            &rt,
        );
        let cond_id = match &created {
            Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate, got {other:?}"),
        };
        for unit in [w1, w2] {
            host.dispatch(
                Lv2Request::MutexLock {
                    mutex_id,
                    timeout: 0,
                },
                unit,
                &rt,
            );
            host.dispatch(
                Lv2Request::CondWait {
                    id: cond_id,
                    timeout: 0,
                },
                unit,
                &rt,
            );
        }
        host.dispatch(
            Lv2Request::MutexLock {
                mutex_id,
                timeout: 0,
            },
            holder,
            &rt,
        );
        assert_eq!(
            host.mutexes().lookup(mutex_id).unwrap().owner(),
            Some(holder_tid),
        );
        let r = host.dispatch(
            Lv2Request::CondSignalTo {
                id: cond_id,
                target_thread: PpuThreadId::PRIMARY.raw() as u32,
            },
            signaler,
            &rt,
        );
        match r {
            Lv2Dispatch::WakeAndReturn {
                woken_unit_ids,
                response_updates,
                ..
            } => {
                assert!(woken_unit_ids.is_empty());
                assert_eq!(response_updates.len(), 1);
                assert_eq!(response_updates[0].0, w1);
            }
            other => panic!("expected WakeAndReturn, got {other:?}"),
        }
        // Mutex owner unchanged.
        assert_eq!(
            host.mutexes().lookup(mutex_id).unwrap().owner(),
            Some(holder_tid),
        );
        // w1 re-parked on mutex.
        assert_eq!(
            host.mutexes()
                .lookup(mutex_id)
                .unwrap()
                .waiters()
                .iter()
                .collect::<Vec<_>>(),
            vec![PpuThreadId::PRIMARY],
        );
        // w2 still parked on cond.
        assert_eq!(
            host.conds()
                .lookup(cond_id)
                .unwrap()
                .waiters()
                .iter()
                .collect::<Vec<_>>(),
            vec![w2_tid],
        );
    }

    #[test]
    fn cond_signal_before_wait_does_not_wake_subsequent_waiter() {
        // Non-sticky signal contract:
        //
        //   Thread A signals (signal / signal_all / signal_to) on
        //   a cond with no parked waiters. The signal is
        //   observably lost -- no pending-signal counter is
        //   maintained, no spurious wake token is buffered.
        //
        //   Thread B subsequently calls sys_cond_wait. B must
        //   block on the cond with PendingResponse::
        //   CondWakeReacquire, not wake spuriously with a
        //   CELL_OK from the earlier signal.
        //
        // This test covers all three signal variants
        // (signal / signal_all / signal_to) to prove none of them
        // introduces buffering. A regression in which the table
        // grew a "pending signal count" field (semaphore-style)
        // would fail this test: the subsequent cond_wait would
        // complete Immediate instead of Block.
        for variant in ["signal_one", "signal_all", "signal_to"] {
            let mut host = Lv2Host::new();
            let rt = FakeRuntime::new(0x10000);
            let waiter_unit = UnitId::new(0);
            let signaler_unit = UnitId::new(1);
            seed_primary_ppu(&mut host, waiter_unit);
            host.ppu_threads_mut()
                .create(signaler_unit, primary_attrs())
                .expect("signaler create");
            let mutex_id = create_mutex_host(&mut host, waiter_unit, &rt);
            let created = host.dispatch(
                Lv2Request::CondCreate {
                    id_ptr: 0x200,
                    mutex_id,
                    attr_ptr: 0,
                },
                waiter_unit,
                &rt,
            );
            let cond_id = match &created {
                Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
                other => panic!("expected Immediate, got {other:?}"),
            };
            // Fire the chosen signal variant on a cond that has
            // no parked waiter yet.
            let pre_signal = match variant {
                "signal_one" => {
                    host.dispatch(Lv2Request::CondSignal { id: cond_id }, signaler_unit, &rt)
                }
                "signal_all" => host.dispatch(
                    Lv2Request::CondSignalAll { id: cond_id },
                    signaler_unit,
                    &rt,
                ),
                "signal_to" => host.dispatch(
                    Lv2Request::CondSignalTo {
                        id: cond_id,
                        target_thread: PpuThreadId::PRIMARY.raw() as u32,
                    },
                    signaler_unit,
                    &rt,
                ),
                _ => unreachable!(),
            };
            // signal / signal_all return CELL_OK regardless of
            // waiter presence (non-sticky). signal_to returns
            // ESRCH because the specific target is not parked.
            // Neither outcome leaves observable state that a
            // later waiter could pick up.
            match variant {
                "signal_to" => {
                    let Lv2Dispatch::Immediate { code, .. } = pre_signal else {
                        panic!("{variant}: expected Immediate, got {pre_signal:?}");
                    };
                    assert_eq!(
                        code,
                        crate::errno::CELL_EPERM.into(),
                        "{variant}: signal_to on target-not-parked should EPERM per RPCS3",
                    );
                }
                _ => {
                    assert!(
                        matches!(
                            pre_signal,
                            Lv2Dispatch::Immediate {
                                code: 0,
                                effects: _,
                            }
                        ),
                        "{variant}: signal on no waiter should return CELL_OK",
                    );
                }
            }
            assert!(
                host.conds().lookup(cond_id).unwrap().waiters().is_empty(),
                "{variant}: cond waiter list must stay empty after lost signal",
            );
            assert_eq!(
                host.mutexes().lookup(mutex_id).unwrap().owner(),
                None,
                "{variant}: mutex must not be acquired by the lost signal",
            );
            // Waiter now locks mutex and cond_waits. It MUST
            // block -- not be satisfied by the earlier lost
            // signal.
            host.dispatch(
                Lv2Request::MutexLock {
                    mutex_id,
                    timeout: 0,
                },
                waiter_unit,
                &rt,
            );
            let wait_result = host.dispatch(
                Lv2Request::CondWait {
                    id: cond_id,
                    timeout: 0,
                },
                waiter_unit,
                &rt,
            );
            match wait_result {
                Lv2Dispatch::Block {
                    reason, pending, ..
                } => {
                    assert!(
                        matches!(reason, crate::dispatch::Lv2BlockReason::Cond { .. }),
                        "{variant}: wait must block on Cond reason",
                    );
                    assert!(
                        matches!(pending, PendingResponse::CondWakeReacquire { .. }),
                        "{variant}: wait must install CondWakeReacquire pending",
                    );
                }
                other => panic!("{variant}: expected Block after lost signal, got {other:?}",),
            }
            assert_eq!(
                host.conds()
                    .lookup(cond_id)
                    .unwrap()
                    .waiters()
                    .iter()
                    .collect::<Vec<_>>(),
                vec![PpuThreadId::PRIMARY],
                "{variant}: waiter must be parked on cond; no signal was buffered",
            );
        }
    }

    #[test]
    fn cond_many_lost_signals_do_not_accumulate() {
        // Fire 20 signals (alternating signal_one / signal_all)
        // against an empty cond, then cond_wait. The waiter must
        // still block. Anchors the "no pending count" invariant:
        // even N lost signals cannot produce a single wake.
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let waiter_unit = UnitId::new(0);
        let signaler_unit = UnitId::new(1);
        seed_primary_ppu(&mut host, waiter_unit);
        host.ppu_threads_mut()
            .create(signaler_unit, primary_attrs())
            .expect("signaler create");
        let mutex_id = create_mutex_host(&mut host, waiter_unit, &rt);
        let created = host.dispatch(
            Lv2Request::CondCreate {
                id_ptr: 0x200,
                mutex_id,
                attr_ptr: 0,
            },
            waiter_unit,
            &rt,
        );
        let cond_id = match &created {
            Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate, got {other:?}"),
        };
        for _ in 0..10 {
            host.dispatch(Lv2Request::CondSignal { id: cond_id }, signaler_unit, &rt);
            host.dispatch(
                Lv2Request::CondSignalAll { id: cond_id },
                signaler_unit,
                &rt,
            );
        }
        host.dispatch(
            Lv2Request::MutexLock {
                mutex_id,
                timeout: 0,
            },
            waiter_unit,
            &rt,
        );
        let wait_result = host.dispatch(
            Lv2Request::CondWait {
                id: cond_id,
                timeout: 0,
            },
            waiter_unit,
            &rt,
        );
        assert!(
            matches!(wait_result, Lv2Dispatch::Block { .. }),
            "20 lost signals must not wake a subsequent waiter",
        );
    }

    #[test]
    fn cond_destroy_with_waiter_returns_ebusy() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let mutex_id = create_mutex_host(&mut host, src, &rt);
        host.dispatch(
            Lv2Request::MutexLock {
                mutex_id,
                timeout: 0,
            },
            src,
            &rt,
        );
        let created = host.dispatch(
            Lv2Request::CondCreate {
                id_ptr: 0x200,
                mutex_id,
                attr_ptr: 0,
            },
            src,
            &rt,
        );
        let cond_id = match &created {
            Lv2Dispatch::Immediate { effects: e, .. } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate, got {other:?}"),
        };
        host.dispatch(
            Lv2Request::CondWait {
                id: cond_id,
                timeout: 0,
            },
            src,
            &rt,
        );
        let r = host.dispatch(Lv2Request::CondDestroy { id: cond_id }, src, &rt);
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_EBUSY.into());
    }
}
