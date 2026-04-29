//! LV2 dispatch for lightweight mutexes.
//!
//! Mirrors RPCS3's `lv2_lwmutex` model: the kernel side is a
//! signal flag plus a FIFO sleep queue, with no owner / recursion
//! tracking. PSL1GHT (and PS3 SDK static-libc) keeps owner /
//! recursion / waiter-count in the user-space `sys_lwmutex_t` and
//! only invokes the kernel for actual contention. `dispatch_lwmutex_lock`
//! consumes a pending signal or parks the caller; `dispatch_lwmutex_unlock`
//! wakes the head of the sleep queue or sets the signal for the
//! next acquirer.

use cellgov_event::UnitId;
use cellgov_ps3_abi::cell_errors as errno;

use crate::dispatch::{Lv2Dispatch, PendingResponse};
use crate::host::Lv2Host;

impl Lv2Host {
    pub(super) fn dispatch_lwmutex_create(
        &mut self,
        id_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        // Lwmutex uses a dedicated id allocator starting at 1.
        // id_ptr is not written on overflow.
        let Some(id) = self.lwmutexes.create() else {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_ENOMEM.into(),
                effects: vec![],
            };
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
            return Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        match self.lwmutexes.acquire_or_enqueue(id, caller) {
            crate::sync_primitives::LwMutexAcquireOrEnqueue::Unknown => Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: vec![],
            },
            crate::sync_primitives::LwMutexAcquireOrEnqueue::Acquired => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            crate::sync_primitives::LwMutexAcquireOrEnqueue::WouldDeadlock => {
                Lv2Dispatch::Immediate {
                    code: errno::CELL_EDEADLK.into(),
                    effects: vec![],
                }
            }
            crate::sync_primitives::LwMutexAcquireOrEnqueue::Enqueued => Lv2Dispatch::Block {
                reason: crate::dispatch::Lv2BlockReason::LwMutex { id },
                // On wake, the runtime claims user-space ownership
                // for the woken thread (decrement waiter, set
                // owner = caller, recursive_count = 1). For the
                // raw-syscall path with no user-space struct, the
                // wake degrades to a plain `r3 = 0`.
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
            return Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        match self.lwmutexes.try_acquire(id, caller) {
            None => Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: vec![],
            },
            Some(crate::sync_primitives::LwMutexAcquire::Acquired) => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            Some(crate::sync_primitives::LwMutexAcquire::Contended) => Lv2Dispatch::Immediate {
                code: errno::CELL_EBUSY.into(),
                effects: vec![],
            },
        }
    }

    pub(super) fn dispatch_lwmutex_unlock(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        match self.lwmutexes.release_and_wake_next(id, caller) {
            crate::sync_primitives::LwMutexRelease::Unknown => Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: vec![],
            },
            crate::sync_primitives::LwMutexRelease::Signaled => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            crate::sync_primitives::LwMutexRelease::Transferred { new_owner } => {
                // Wake the dequeued thread. A missing thread-table
                // entry strands the wake but the unlock still
                // returns OK.
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
        let Some(entry) = self.lwmutexes.lookup(id) else {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        // Only parked waiters block destroy. The signal flag does
        // not, since user-space ownership is invisible to us.
        if !entry.waiters().is_empty() {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EBUSY.into(),
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
        assert_eq!(code, errno::CELL_ESRCH.into());
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
        // Park a waiter directly so destroy hits the non-empty
        // sleep queue check.
        host.lwmutexes_mut()
            .enqueue_waiter(id, PpuThreadId::new(0x0100_0002))
            .unwrap();
        let r = host.dispatch(Lv2Request::LwMutexDestroy { id }, source, &rt);
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, errno::CELL_EBUSY.into());
        assert!(host.lwmutexes().lookup(id).is_some());
    }

    #[test]
    fn lwmutex_lock_unknown_id_returns_esrch() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(
            Lv2Request::LwMutexLock {
                id: 99,
                mutex_ptr: 0,
                timeout: 0,
            },
            src,
            &rt,
        );
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, errno::CELL_ESRCH.into());
    }

    #[test]
    fn lwmutex_lock_on_fresh_entry_parks_kernel_side() {
        // Kernel-side lock always parks (no signal pending). The
        // HLE wrapper covers the uncontended fast path; a direct
        // dispatch reaches here only via raw LV2 syscall, which the
        // tests below exercise.
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
        let r = host.dispatch(
            Lv2Request::LwMutexLock {
                id,
                mutex_ptr: 0,
                timeout: 0,
            },
            src,
            &rt,
        );
        match r {
            Lv2Dispatch::Block { .. } => {}
            other => panic!("expected Block, got {other:?}"),
        }
        let entry = host.lwmutexes().lookup(id).unwrap();
        assert!(!entry.signaled());
        assert_eq!(entry.waiters().len(), 1);
    }

    #[test]
    fn lwmutex_lock_contended_parks_caller_on_waiter_list() {
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
        // Both calls park (kernel always blocks; the HLE handles
        // the uncontended user-space fast path). After both, the
        // sleep queue contains both threads in FIFO order.
        host.dispatch(
            Lv2Request::LwMutexLock {
                id,
                mutex_ptr: 0,
                timeout: 0,
            },
            owner_unit,
            &rt,
        );
        let r = host.dispatch(
            Lv2Request::LwMutexLock {
                id,
                mutex_ptr: 0,
                timeout: 0,
            },
            waiter_unit,
            &rt,
        );
        match r {
            Lv2Dispatch::Block {
                reason: crate::dispatch::Lv2BlockReason::LwMutex { id: blocked_id },
                pending: PendingResponse::LwMutexWake { mutex_ptr: 0, .. },
                effects: _,
            } => {
                assert_eq!(blocked_id, id);
            }
            other => panic!("expected Block on LwMutex, got {other:?}"),
        }
        let entry = host.lwmutexes().lookup(id).unwrap();
        assert!(!entry.signaled());
        let seen: Vec<_> = entry.waiters().iter().collect();
        assert_eq!(seen.len(), 2);
        assert!(seen.contains(&waiter_tid));
    }

    #[test]
    fn lwmutex_trylock_on_fresh_entry_returns_ebusy_kernel_side() {
        // Kernel-side trylock has no signal pending on a fresh
        // entry, so it always reports `Contended` -> `CELL_EBUSY`.
        // The HLE wrapper handles uncontended `sys_lwmutex_trylock`
        // entirely in user space.
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
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, errno::CELL_EBUSY.into());
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
        host.dispatch(
            Lv2Request::LwMutexLock {
                id,
                mutex_ptr: 0,
                timeout: 0,
            },
            owner_unit,
            &rt,
        );
        let r = host.dispatch(Lv2Request::LwMutexTryLock { id }, other_unit, &rt);
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, errno::CELL_EBUSY.into());
        let entry = host.lwmutexes().lookup(id).unwrap();
        // The lock above parked owner_unit; trylock did not park.
        assert!(!entry.signaled());
        assert_eq!(entry.waiters().len(), 1);
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
        assert_eq!(code, errno::CELL_ESRCH.into());
    }

    #[test]
    fn lwmutex_unlock_without_waiters_signals() {
        // Kernel-side unlock with an empty sleep queue sets the
        // signal so the next contended lock can pass without
        // blocking. The HLE wrapper only invokes the kernel
        // unlock when the user-space waiter counter is non-zero,
        // but a direct dispatch (raw LV2 syscall path) can land
        // here.
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
        let r = host.dispatch(Lv2Request::LwMutexUnlock { id }, src, &rt);
        assert!(matches!(
            r,
            Lv2Dispatch::Immediate {
                code: 0,
                effects: _,
            }
        ));
        assert!(host.lwmutexes().lookup(id).unwrap().signaled());
    }

    #[test]
    fn lwmutex_unlock_with_waiters_transfers_to_head() {
        // Kernel-side unlock pops the FIFO head and reports it as
        // the wake target via `WakeAndReturn`. The unlocker's
        // identity is not consulted -- PSL1GHT enforces the owner
        // check in user space before invoking the kernel unlock.
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let unlocker = UnitId::new(0);
        let waiter_unit = UnitId::new(1);
        seed_primary_ppu(&mut host, unlocker);
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
            unlocker,
            &rt,
        );
        let id = match &created {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        // Park the waiter directly so the unlock has someone to
        // transfer to.
        host.lwmutexes_mut().enqueue_waiter(id, waiter_tid).unwrap();
        let r = host.dispatch(Lv2Request::LwMutexUnlock { id }, unlocker, &rt);
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
        let entry = host.lwmutexes().lookup(id).unwrap();
        assert!(!entry.signaled());
        assert!(entry.waiters().is_empty());
    }

    #[test]
    fn lwmutex_unlock_signals_when_only_blocked_caller_is_unlocker() {
        // The kernel does not validate the unlocker. PSL1GHT does
        // owner enforcement in user space before invoking unlock,
        // so this kernel-side path can fire from any unit and just
        // signals (because the queue now has the unlocker himself
        // as a waiter, dequeued and transferred).
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
        // Empty queue: unlock just signals.
        let r = host.dispatch(Lv2Request::LwMutexUnlock { id }, other_unit, &rt);
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, 0);
        assert!(host.lwmutexes().lookup(id).unwrap().signaled());
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
        assert_eq!(code, errno::CELL_ESRCH.into());
    }

    #[test]
    fn lwmutex_unlock_with_three_waiters_wakes_head_in_fifo_order() {
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
        // Park t1, t2, t3 directly. u0 will be the unlocker.
        let _ = (t1, t2, t3);
        host.lwmutexes_mut().enqueue_waiter(id, t1).unwrap();
        host.lwmutexes_mut().enqueue_waiter(id, t2).unwrap();
        host.lwmutexes_mut().enqueue_waiter(id, t3).unwrap();
        // Three unlocks transfer to u1, u2, u3 in FIFO order.
        match host.dispatch(Lv2Request::LwMutexUnlock { id }, u0, &rt) {
            Lv2Dispatch::WakeAndReturn { woken_unit_ids, .. } => {
                assert_eq!(woken_unit_ids, vec![u1]);
            }
            other => panic!("expected WakeAndReturn, got {other:?}"),
        }
        match host.dispatch(Lv2Request::LwMutexUnlock { id }, u0, &rt) {
            Lv2Dispatch::WakeAndReturn { woken_unit_ids, .. } => {
                assert_eq!(woken_unit_ids, vec![u2]);
            }
            other => panic!("expected WakeAndReturn, got {other:?}"),
        }
        match host.dispatch(Lv2Request::LwMutexUnlock { id }, u0, &rt) {
            Lv2Dispatch::WakeAndReturn { woken_unit_ids, .. } => {
                assert_eq!(woken_unit_ids, vec![u3]);
            }
            other => panic!("expected WakeAndReturn, got {other:?}"),
        }
        // Final unlock with empty queue signals.
        match host.dispatch(Lv2Request::LwMutexUnlock { id }, u0, &rt) {
            Lv2Dispatch::Immediate { code: 0, .. } => {}
            other => panic!("expected Immediate(0), got {other:?}"),
        }
        assert!(host.lwmutexes().lookup(id).unwrap().signaled());
        assert!(host.lwmutexes().lookup(id).unwrap().waiters().is_empty());
    }

    #[test]
    fn lwmutex_lock_duplicate_park_returns_edeadlk() {
        // Anchors the errno mapping: the table's duplicate-enqueue
        // rejection surfaces as EDEADLK (not ESRCH / EFAULT) at
        // the dispatch level.
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
        host.dispatch(
            Lv2Request::LwMutexLock {
                id,
                mutex_ptr: 0,
                timeout: 0,
            },
            owner_unit,
            &rt,
        );
        host.dispatch(
            Lv2Request::LwMutexLock {
                id,
                mutex_ptr: 0,
                timeout: 0,
            },
            waiter_unit,
            &rt,
        );
        // Already-parked caller tries to park again without a
        // prior wake.
        let r = host.dispatch(
            Lv2Request::LwMutexLock {
                id,
                mutex_ptr: 0,
                timeout: 0,
            },
            waiter_unit,
            &rt,
        );
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, errno::CELL_EDEADLK.into());
    }
}
