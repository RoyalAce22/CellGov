//! LV2 dispatch for heavy mutexes.
//!
//! `acquire_or_enqueue` is atomic: recursive-lock-by-owner
//! (EDEADLK) and contention (park on FIFO waiter list) are
//! distinguished in one call. Splitting into try-then-enqueue
//! would race with a concurrent unlock.

use cellgov_event::UnitId;
use cellgov_ps3_abi::cell_errors as errno;

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
        // `sys_mutex_attribute_t`: first three big-endian u32s
        // carry protocol, recursive flag, and pshared. Remaining
        // fields (adaptive, name) are not surfaced here.
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
            // IdCollision is a host-invariant break; surface
            // ENOMEM so the guest cannot use the bad id.
            return Lv2Dispatch::Immediate {
                code: errno::CELL_ENOMEM.into(),
                effects: vec![],
            };
        }
        self.immediate_write_u32(id, id_ptr, requester)
    }

    pub(super) fn dispatch_mutex_destroy(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.mutexes.lookup(id) else {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        if entry.owner().is_some() || !entry.waiters().is_empty() {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EBUSY.into(),
                effects: vec![],
            };
        }
        self.mutexes.destroy(id);
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    }

    pub(super) fn dispatch_mutex_lock(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        match self.mutexes.acquire_or_enqueue(id, caller) {
            crate::sync_primitives::MutexAcquireOrEnqueue::Unknown => Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: vec![],
            },
            crate::sync_primitives::MutexAcquireOrEnqueue::Acquired => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            crate::sync_primitives::MutexAcquireOrEnqueue::WouldDeadlock => {
                Lv2Dispatch::Immediate {
                    code: errno::CELL_EDEADLK.into(),
                    effects: vec![],
                }
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
            return Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        match self.mutexes.try_acquire(id, caller) {
            None => Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: vec![],
            },
            Some(crate::sync_primitives::MutexAcquire::Acquired) => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            Some(crate::sync_primitives::MutexAcquire::Contended) => Lv2Dispatch::Immediate {
                code: errno::CELL_EBUSY.into(),
                effects: vec![],
            },
        }
    }

    pub(super) fn dispatch_mutex_unlock(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        match self.mutexes.release_and_wake_next(id, caller) {
            crate::sync_primitives::MutexRelease::Unknown => Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: vec![],
            },
            crate::sync_primitives::MutexRelease::NotOwner => Lv2Dispatch::Immediate {
                code: errno::CELL_EPERM.into(),
                effects: vec![],
            },
            crate::sync_primitives::MutexRelease::Freed => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            crate::sync_primitives::MutexRelease::Transferred { new_owner } => {
                // Ownership has already transferred; missing
                // thread-table entry leaves the mutex naming an
                // owner the runtime cannot wake.
                match self.resolve_wake_thread(new_owner, "mutex_unlock.Transferred") {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::test_support::{extract_write_u32, seed_primary_ppu, FakeRuntime};
    use crate::ppu_thread::{PpuThreadAttrs, PpuThreadId};
    use crate::request::Lv2Request;

    #[test]
    fn mutex_create_allocates_monotonic_ids() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let source = UnitId::new(0);

        let r1 = host.dispatch(
            Lv2Request::MutexCreate {
                id_ptr: 0x100,
                attr_ptr: 0x200,
            },
            source,
            &rt,
        );
        let r2 = host.dispatch(
            Lv2Request::MutexCreate {
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
        assert_ne!(id1, id2, "IDs should be monotonically different");
        assert!(id1 > 0 && id2 > 0, "IDs should be non-zero");
    }

    #[test]
    fn mutex_lock_on_unknown_id_returns_esrch() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        seed_primary_ppu(&mut host, UnitId::new(0));
        let r = host.dispatch(
            Lv2Request::MutexLock {
                mutex_id: 99,
                timeout: 0,
            },
            UnitId::new(0),
            &rt,
        );
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, errno::CELL_ESRCH.into());
    }

    #[test]
    fn mutex_lock_unowned_acquires_and_unlock_frees() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let created = host.dispatch(
            Lv2Request::MutexCreate {
                id_ptr: 0x100,
                attr_ptr: 0,
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
        let lock = host.dispatch(
            Lv2Request::MutexLock {
                mutex_id: id,
                timeout: 0,
            },
            src,
            &rt,
        );
        assert!(matches!(
            lock,
            Lv2Dispatch::Immediate {
                code: 0,
                effects: _,
            }
        ));
        assert_eq!(
            host.mutexes().lookup(id).unwrap().owner(),
            Some(PpuThreadId::PRIMARY),
        );
        let unlock = host.dispatch(Lv2Request::MutexUnlock { mutex_id: id }, src, &rt);
        assert!(matches!(
            unlock,
            Lv2Dispatch::Immediate {
                code: 0,
                effects: _,
            }
        ));
        assert_eq!(host.mutexes().lookup(id).unwrap().owner(), None);
    }

    #[test]
    fn mutex_lock_contended_blocks_and_unlock_wakes() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
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
            Lv2Request::MutexCreate {
                id_ptr: 0x100,
                attr_ptr: 0,
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
            Lv2Request::MutexLock {
                mutex_id: id,
                timeout: 0,
            },
            owner_unit,
            &rt,
        );
        let block = host.dispatch(
            Lv2Request::MutexLock {
                mutex_id: id,
                timeout: 0,
            },
            waiter_unit,
            &rt,
        );
        match block {
            Lv2Dispatch::Block {
                reason: crate::dispatch::Lv2BlockReason::Mutex { id: blocked_id },
                pending: PendingResponse::ReturnCode { code: 0 },
                ..
            } => {
                assert_eq!(blocked_id, id);
            }
            other => panic!("expected Block on Mutex, got {other:?}"),
        }
        let wake = host.dispatch(Lv2Request::MutexUnlock { mutex_id: id }, owner_unit, &rt);
        match wake {
            Lv2Dispatch::WakeAndReturn {
                code: 0,
                woken_unit_ids,
                ..
            } => assert_eq!(woken_unit_ids, vec![waiter_unit]),
            other => panic!("expected WakeAndReturn, got {other:?}"),
        }
        assert_eq!(host.mutexes().lookup(id).unwrap().owner(), Some(waiter_tid));
    }

    #[test]
    fn mutex_create_default_attrs_when_attr_ptr_zero() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let created = host.dispatch(
            Lv2Request::MutexCreate {
                id_ptr: 0x100,
                attr_ptr: 0,
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
        assert_eq!(
            host.mutexes().lookup(id).unwrap().attrs(),
            crate::sync_primitives::MutexAttrs::default()
        );
    }

    #[test]
    fn mutex_create_decodes_attr_ptr() {
        let mut mem = cellgov_mem::GuestMemory::new(0x10000);
        let attr_bytes = [
            0x00, 0x00, 0x00, 0x20, // protocol = 0x20 (PRIORITY_INHERIT)
            0x00, 0x00, 0x00, 0x11, // recursive = 0x11 (RECURSIVE)
            0x00, 0x00, 0x00, 0x00, // pshared (ignored)
        ];
        mem.apply_commit(
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0x200), 12).unwrap(),
            &attr_bytes,
        )
        .unwrap();
        let rt = FakeRuntime::with_memory(mem);
        let mut host = Lv2Host::new();
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let created = host.dispatch(
            Lv2Request::MutexCreate {
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
        let attrs = host.mutexes().lookup(id).unwrap().attrs();
        assert_eq!(attrs.protocol, 0x20);
        assert_eq!(attrs.priority_policy, 0x20);
        assert!(attrs.recursive);
    }

    #[test]
    fn mutex_trylock_unowned_acquires() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let created = host.dispatch(
            Lv2Request::MutexCreate {
                id_ptr: 0x100,
                attr_ptr: 0,
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
        let r = host.dispatch(Lv2Request::MutexTryLock { mutex_id: id }, src, &rt);
        assert!(matches!(
            r,
            Lv2Dispatch::Immediate {
                code: 0,
                effects: _,
            }
        ));
        assert_eq!(
            host.mutexes().lookup(id).unwrap().owner(),
            Some(PpuThreadId::PRIMARY),
        );
    }

    #[test]
    fn mutex_trylock_contended_returns_ebusy_and_does_not_park() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
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
            Lv2Request::MutexCreate {
                id_ptr: 0x100,
                attr_ptr: 0,
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
            Lv2Request::MutexLock {
                mutex_id: id,
                timeout: 0,
            },
            owner_unit,
            &rt,
        );
        let r = host.dispatch(Lv2Request::MutexTryLock { mutex_id: id }, other_unit, &rt);
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, errno::CELL_EBUSY.into());
        assert_eq!(
            host.mutexes().lookup(id).unwrap().owner(),
            Some(PpuThreadId::PRIMARY),
        );
        assert!(host.mutexes().lookup(id).unwrap().waiters().is_empty());
    }

    #[test]
    fn mutex_trylock_unknown_returns_esrch() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(Lv2Request::MutexTryLock { mutex_id: 77 }, src, &rt);
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, errno::CELL_ESRCH.into());
    }

    #[test]
    fn mutex_unlock_non_owner_returns_eperm() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
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
            Lv2Request::MutexCreate {
                id_ptr: 0x100,
                attr_ptr: 0,
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
            Lv2Request::MutexLock {
                mutex_id: id,
                timeout: 0,
            },
            owner_unit,
            &rt,
        );
        let r = host.dispatch(Lv2Request::MutexUnlock { mutex_id: id }, other_unit, &rt);
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, errno::CELL_EPERM.into());
    }
}
