//! Lightweight mutex LV2 dispatch: create, destroy, lock, trylock, unlock.

use cellgov_event::UnitId;

use crate::dispatch::{Lv2Dispatch, PendingResponse};
use crate::host::Lv2Host;

impl Lv2Host {
    pub(super) fn dispatch_lwmutex_create(
        &mut self,
        id_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        // Lwmutex has its own id allocator (starts at 1); out
        // pointer is not written on overflow.
        let Some(id) = self.lwmutexes.create() else {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ENOMEM.into(),
                effects: vec![],
            };
        };
        self.immediate_write_u32(id, id_ptr, requester)
    }

    pub(super) fn dispatch_lwmutex_lock(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        // See `dispatch_mutex_lock` for the acquire_or_enqueue
        // TOCTOU rationale.
        match self.lwmutexes.acquire_or_enqueue(id, caller) {
            crate::sync_primitives::LwMutexAcquireOrEnqueue::Unknown => Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            },
            crate::sync_primitives::LwMutexAcquireOrEnqueue::Acquired => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            crate::sync_primitives::LwMutexAcquireOrEnqueue::WouldDeadlock => {
                Lv2Dispatch::Immediate {
                    code: crate::errno::CELL_EDEADLK.into(),
                    effects: vec![],
                }
            }
            crate::sync_primitives::LwMutexAcquireOrEnqueue::Enqueued => Lv2Dispatch::Block {
                reason: crate::dispatch::Lv2BlockReason::LwMutex { id },
                pending: PendingResponse::ReturnCode { code: 0 },
                effects: vec![],
            },
        }
    }

    pub(super) fn dispatch_lwmutex_trylock(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        match self.lwmutexes.try_acquire(id, caller) {
            None => Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            },
            Some(crate::sync_primitives::LwMutexAcquire::Acquired) => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            Some(crate::sync_primitives::LwMutexAcquire::Contended) => Lv2Dispatch::Immediate {
                code: crate::errno::CELL_EBUSY.into(),
                effects: vec![],
            },
        }
    }

    pub(super) fn dispatch_lwmutex_unlock(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        match self.lwmutexes.release_and_wake_next(id, caller) {
            crate::sync_primitives::LwMutexRelease::Unknown => Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            },
            crate::sync_primitives::LwMutexRelease::NotOwner => Lv2Dispatch::Immediate {
                code: crate::errno::CELL_EPERM.into(),
                effects: vec![],
            },
            crate::sync_primitives::LwMutexRelease::Freed => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            crate::sync_primitives::LwMutexRelease::Transferred { new_owner } => {
                // See `dispatch_mutex_unlock.Transferred`.
                match self.resolve_wake_thread(new_owner, "lwmutex_unlock.Transferred") {
                    Some(unit) => Lv2Dispatch::WakeAndReturn {
                        code: 0,
                        woken_unit_ids: vec![unit],
                        response_updates: vec![],
                        effects: vec![],
                    },
                    None => Lv2Dispatch::Immediate {
                        code: 0,
                        effects: vec![],
                    },
                }
            }
        }
    }

    pub(super) fn dispatch_lwmutex_destroy(&mut self, id: u32) -> Lv2Dispatch {
        // Destroy-with-waiters would strand them -- EBUSY.
        let Some(entry) = self.lwmutexes.lookup(id) else {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        if !entry.waiters().is_empty() || entry.owner().is_some() {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_EBUSY.into(),
                effects: vec![],
            };
        }
        self.lwmutexes.destroy(id);
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::test_support::{extract_write_u32, seed_primary_ppu, FakeRuntime};
    use crate::ppu_thread::{PpuThreadAttrs, PpuThreadId};
    use crate::request::Lv2Request;

    #[test]
    fn lwmutex_create_allocates_monotonic_ids_starting_at_one() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let source = UnitId::new(0);
        let r1 = host.dispatch(
            Lv2Request::LwMutexCreate {
                id_ptr: 0x100,
                attr_ptr: 0x200,
            },
            source,
            &rt,
        );
        let r2 = host.dispatch(
            Lv2Request::LwMutexCreate {
                id_ptr: 0x104,
                attr_ptr: 0x200,
            },
            source,
            &rt,
        );
        let id1 = match &r1 {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        let id2 = match &r2 {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(host.lwmutexes().len(), 2);
    }

    #[test]
    fn lwmutex_destroy_unknown_id_returns_esrch() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let r = host.dispatch(Lv2Request::LwMutexDestroy { id: 42 }, UnitId::new(0), &rt);
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_ESRCH.into());
    }

    #[test]
    fn lwmutex_create_destroy_roundtrip() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let source = UnitId::new(0);
        let created = host.dispatch(
            Lv2Request::LwMutexCreate {
                id_ptr: 0x100,
                attr_ptr: 0x200,
            },
            source,
            &rt,
        );
        let id = match &created {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        let destroyed = host.dispatch(Lv2Request::LwMutexDestroy { id }, source, &rt);
        assert!(matches!(
            destroyed,
            Lv2Dispatch::Immediate {
                code: 0,
                effects: _,
            }
        ));
        assert!(host.lwmutexes().lookup(id).is_none());
    }

    #[test]
    fn lwmutex_destroy_with_waiter_returns_ebusy() {
        // Preload the table: create an lwmutex, set an owner, and
        // enqueue a waiter. Destroy must reject with EBUSY without
        // tearing down state.
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let source = UnitId::new(0);
        let created = host.dispatch(
            Lv2Request::LwMutexCreate {
                id_ptr: 0x100,
                attr_ptr: 0x200,
            },
            source,
            &rt,
        );
        let id = match &created {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        host.lwmutexes_mut()
            .try_acquire(id, PpuThreadId::new(0x0100_0001));
        let r = host.dispatch(Lv2Request::LwMutexDestroy { id }, source, &rt);
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_EBUSY.into());
        assert!(host.lwmutexes().lookup(id).is_some());
    }

    #[test]
    fn lwmutex_lock_unknown_id_returns_esrch() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(Lv2Request::LwMutexLock { id: 99, timeout: 0 }, src, &rt);
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_ESRCH.into());
    }

    #[test]
    fn lwmutex_lock_unowned_acquires_immediately() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let created = host.dispatch(
            Lv2Request::LwMutexCreate {
                id_ptr: 0x100,
                attr_ptr: 0x200,
            },
            src,
            &rt,
        );
        let id = match &created {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        let r = host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, src, &rt);
        assert!(matches!(
            r,
            Lv2Dispatch::Immediate {
                code: 0,
                effects: _,
            }
        ));
        assert_eq!(
            host.lwmutexes().lookup(id).unwrap().owner(),
            Some(PpuThreadId::PRIMARY),
        );
        assert!(host.lwmutexes().lookup(id).unwrap().waiters().is_empty());
    }

    #[test]
    fn lwmutex_lock_contended_parks_caller_on_waiter_list() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let owner_unit = UnitId::new(0);
        let waiter_unit = UnitId::new(1);
        seed_primary_ppu(&mut host, owner_unit);
        // Register a second PPU thread so the waiter has a
        // distinct thread id.
        let waiter_tid = host
            .ppu_threads_mut()
            .create(
                waiter_unit,
                PpuThreadAttrs {
                    entry: 0,
                    arg: 0,
                    stack_base: 0,
                    stack_size: 0,
                    priority: 0,
                    tls_base: 0,
                },
            )
            .unwrap();
        // Owner creates and acquires.
        let created = host.dispatch(
            Lv2Request::LwMutexCreate {
                id_ptr: 0x100,
                attr_ptr: 0x200,
            },
            owner_unit,
            &rt,
        );
        let id = match &created {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, owner_unit, &rt);
        // Waiter tries to acquire and blocks.
        let r = host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, waiter_unit, &rt);
        match r {
            Lv2Dispatch::Block {
                reason: crate::dispatch::Lv2BlockReason::LwMutex { id: blocked_id },
                pending: PendingResponse::ReturnCode { code: 0 },
                effects: _,
            } => {
                assert_eq!(blocked_id, id);
            }
            other => panic!("expected Block on LwMutex, got {other:?}"),
        }
        // Owner unchanged; waiter enqueued.
        let entry = host.lwmutexes().lookup(id).unwrap();
        assert_eq!(entry.owner(), Some(PpuThreadId::PRIMARY));
        let seen: Vec<_> = entry.waiters().iter().collect();
        assert_eq!(seen, vec![waiter_tid]);
    }

    #[test]
    fn lwmutex_trylock_unowned_acquires() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let created = host.dispatch(
            Lv2Request::LwMutexCreate {
                id_ptr: 0x100,
                attr_ptr: 0x200,
            },
            src,
            &rt,
        );
        let id = match &created {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        let r = host.dispatch(Lv2Request::LwMutexTryLock { id }, src, &rt);
        assert!(matches!(
            r,
            Lv2Dispatch::Immediate {
                code: 0,
                effects: _,
            }
        ));
        assert_eq!(
            host.lwmutexes().lookup(id).unwrap().owner(),
            Some(PpuThreadId::PRIMARY),
        );
    }

    #[test]
    fn lwmutex_trylock_contended_returns_ebusy_and_does_not_park() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let owner_unit = UnitId::new(0);
        let other_unit = UnitId::new(1);
        seed_primary_ppu(&mut host, owner_unit);
        host.ppu_threads_mut()
            .create(
                other_unit,
                PpuThreadAttrs {
                    entry: 0,
                    arg: 0,
                    stack_base: 0,
                    stack_size: 0,
                    priority: 0,
                    tls_base: 0,
                },
            )
            .unwrap();
        let created = host.dispatch(
            Lv2Request::LwMutexCreate {
                id_ptr: 0x100,
                attr_ptr: 0x200,
            },
            owner_unit,
            &rt,
        );
        let id = match &created {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, owner_unit, &rt);
        // Other thread tries non-blockingly.
        let r = host.dispatch(Lv2Request::LwMutexTryLock { id }, other_unit, &rt);
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_EBUSY.into());
        // Owner unchanged; waiter list untouched.
        let entry = host.lwmutexes().lookup(id).unwrap();
        assert_eq!(entry.owner(), Some(PpuThreadId::PRIMARY));
        assert!(entry.waiters().is_empty());
    }

    #[test]
    fn lwmutex_trylock_unknown_returns_esrch() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(Lv2Request::LwMutexTryLock { id: 77 }, src, &rt);
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_ESRCH.into());
    }

    #[test]
    fn lwmutex_unlock_without_waiters_frees() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let created = host.dispatch(
            Lv2Request::LwMutexCreate {
                id_ptr: 0x100,
                attr_ptr: 0x200,
            },
            src,
            &rt,
        );
        let id = match &created {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, src, &rt);
        let r = host.dispatch(Lv2Request::LwMutexUnlock { id }, src, &rt);
        assert!(matches!(
            r,
            Lv2Dispatch::Immediate {
                code: 0,
                effects: _,
            }
        ));
        assert_eq!(host.lwmutexes().lookup(id).unwrap().owner(), None);
    }

    #[test]
    fn lwmutex_unlock_with_waiters_transfers_and_reports_wake_target() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let owner_unit = UnitId::new(0);
        let waiter_unit = UnitId::new(1);
        seed_primary_ppu(&mut host, owner_unit);
        let waiter_tid = host
            .ppu_threads_mut()
            .create(
                waiter_unit,
                PpuThreadAttrs {
                    entry: 0,
                    arg: 0,
                    stack_base: 0,
                    stack_size: 0,
                    priority: 0,
                    tls_base: 0,
                },
            )
            .unwrap();
        let created = host.dispatch(
            Lv2Request::LwMutexCreate {
                id_ptr: 0x100,
                attr_ptr: 0x200,
            },
            owner_unit,
            &rt,
        );
        let id = match &created {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, owner_unit, &rt);
        host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, waiter_unit, &rt);
        // Owner unlocks.
        let r = host.dispatch(Lv2Request::LwMutexUnlock { id }, owner_unit, &rt);
        match r {
            Lv2Dispatch::WakeAndReturn {
                code: 0,
                woken_unit_ids,
                effects,
                ..
            } => {
                assert_eq!(woken_unit_ids, vec![waiter_unit]);
                assert!(effects.is_empty());
            }
            other => panic!("expected WakeAndReturn, got {other:?}"),
        }
        // Ownership transferred to the waiter.
        let entry = host.lwmutexes().lookup(id).unwrap();
        assert_eq!(entry.owner(), Some(waiter_tid));
        assert!(entry.waiters().is_empty());
    }

    #[test]
    fn lwmutex_unlock_not_owner_returns_eperm() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let owner_unit = UnitId::new(0);
        let other_unit = UnitId::new(1);
        seed_primary_ppu(&mut host, owner_unit);
        host.ppu_threads_mut()
            .create(
                other_unit,
                PpuThreadAttrs {
                    entry: 0,
                    arg: 0,
                    stack_base: 0,
                    stack_size: 0,
                    priority: 0,
                    tls_base: 0,
                },
            )
            .unwrap();
        let created = host.dispatch(
            Lv2Request::LwMutexCreate {
                id_ptr: 0x100,
                attr_ptr: 0x200,
            },
            owner_unit,
            &rt,
        );
        let id = match &created {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, owner_unit, &rt);
        // Non-owner tries to unlock.
        let r = host.dispatch(Lv2Request::LwMutexUnlock { id }, other_unit, &rt);
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_EPERM.into());
        // Owner unchanged.
        assert_eq!(
            host.lwmutexes().lookup(id).unwrap().owner(),
            Some(PpuThreadId::PRIMARY),
        );
    }

    #[test]
    fn lwmutex_unlock_unknown_returns_esrch() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(Lv2Request::LwMutexUnlock { id: 99 }, src, &rt);
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_ESRCH.into());
    }

    #[test]
    fn lwmutex_unlock_with_three_waiters_wakes_head_in_fifo_order() {
        // Waiters parked in order w1, w2, w3. First unlock wakes w1.
        // w1 unlocks -> wakes w2. w2 unlocks -> wakes w3. w3 unlocks
        // -> mutex free.
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let u0 = UnitId::new(0);
        let u1 = UnitId::new(1);
        let u2 = UnitId::new(2);
        let u3 = UnitId::new(3);
        seed_primary_ppu(&mut host, u0);
        let t1 = host
            .ppu_threads_mut()
            .create(
                u1,
                PpuThreadAttrs {
                    entry: 0,
                    arg: 0,
                    stack_base: 0,
                    stack_size: 0,
                    priority: 0,
                    tls_base: 0,
                },
            )
            .unwrap();
        let t2 = host
            .ppu_threads_mut()
            .create(
                u2,
                PpuThreadAttrs {
                    entry: 0,
                    arg: 0,
                    stack_base: 0,
                    stack_size: 0,
                    priority: 0,
                    tls_base: 0,
                },
            )
            .unwrap();
        let t3 = host
            .ppu_threads_mut()
            .create(
                u3,
                PpuThreadAttrs {
                    entry: 0,
                    arg: 0,
                    stack_base: 0,
                    stack_size: 0,
                    priority: 0,
                    tls_base: 0,
                },
            )
            .unwrap();
        let created = host.dispatch(
            Lv2Request::LwMutexCreate {
                id_ptr: 0x100,
                attr_ptr: 0x200,
            },
            u0,
            &rt,
        );
        let id = match &created {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, u0, &rt);
        host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, u1, &rt);
        host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, u2, &rt);
        host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, u3, &rt);
        // u0 unlocks -> u1 gets it.
        match host.dispatch(Lv2Request::LwMutexUnlock { id }, u0, &rt) {
            Lv2Dispatch::WakeAndReturn { woken_unit_ids, .. } => {
                assert_eq!(woken_unit_ids, vec![u1]);
            }
            other => panic!("expected WakeAndReturn, got {other:?}"),
        }
        assert_eq!(host.lwmutexes().lookup(id).unwrap().owner(), Some(t1));
        // u1 unlocks -> u2 gets it.
        match host.dispatch(Lv2Request::LwMutexUnlock { id }, u1, &rt) {
            Lv2Dispatch::WakeAndReturn { woken_unit_ids, .. } => {
                assert_eq!(woken_unit_ids, vec![u2]);
            }
            other => panic!("expected WakeAndReturn, got {other:?}"),
        }
        assert_eq!(host.lwmutexes().lookup(id).unwrap().owner(), Some(t2));
        // u2 unlocks -> u3 gets it.
        match host.dispatch(Lv2Request::LwMutexUnlock { id }, u2, &rt) {
            Lv2Dispatch::WakeAndReturn { woken_unit_ids, .. } => {
                assert_eq!(woken_unit_ids, vec![u3]);
            }
            other => panic!("expected WakeAndReturn, got {other:?}"),
        }
        assert_eq!(host.lwmutexes().lookup(id).unwrap().owner(), Some(t3));
        // u3 unlocks -> mutex free.
        match host.dispatch(Lv2Request::LwMutexUnlock { id }, u3, &rt) {
            Lv2Dispatch::Immediate { code: 0, .. } => {}
            other => panic!("expected Immediate(0), got {other:?}"),
        }
        assert_eq!(host.lwmutexes().lookup(id).unwrap().owner(), None);
    }

    #[test]
    fn lwmutex_lock_duplicate_park_returns_edeadlk() {
        // A caller that is already parked on the same mutex cannot
        // park again. The table's duplicate-enqueue rejection
        // surfaces as EDEADLK at the dispatch level.
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let owner_unit = UnitId::new(0);
        let waiter_unit = UnitId::new(1);
        seed_primary_ppu(&mut host, owner_unit);
        host.ppu_threads_mut()
            .create(
                waiter_unit,
                PpuThreadAttrs {
                    entry: 0,
                    arg: 0,
                    stack_base: 0,
                    stack_size: 0,
                    priority: 0,
                    tls_base: 0,
                },
            )
            .unwrap();
        let created = host.dispatch(
            Lv2Request::LwMutexCreate {
                id_ptr: 0x100,
                attr_ptr: 0x200,
            },
            owner_unit,
            &rt,
        );
        let id = match &created {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, owner_unit, &rt);
        host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, waiter_unit, &rt);
        // Second block attempt from the same waiter without a prior
        // wake.
        let r = host.dispatch(Lv2Request::LwMutexLock { id, timeout: 0 }, waiter_unit, &rt);
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_EDEADLK.into());
    }
}
