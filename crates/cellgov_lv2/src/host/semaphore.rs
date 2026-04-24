//! LV2 dispatch for counting semaphores.
//!
//! `post_and_wake` preserves the count invariant: a post with a
//! parked waiter hands off directly (no increment), otherwise the
//! count grows up to `max`. Over-max post with no waiter is EINVAL,
//! so the count can never exceed `max`.

use cellgov_event::UnitId;

use crate::dispatch::{Lv2Dispatch, PendingResponse};
use crate::host::Lv2Host;

impl Lv2Host {
    pub(super) fn dispatch_semaphore_create(
        &mut self,
        id_ptr: u32,
        initial: i32,
        max: i32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        // Reject bad bounds before alloc_id so an invalid request
        // does not burn a kernel id.
        if initial > max || initial < 0 || max < 0 {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_EINVAL.into(),
                effects: vec![],
            };
        }
        let id = self.alloc_id();
        match self.semaphores.create_with_id(id, initial, max) {
            Ok(()) => {}
            Err(crate::sync_primitives::SemaphoreCreateError::IdCollision) => {
                // Host-invariant break; ENOMEM is a best-effort
                // errno since no Cell OS code maps to "allocator
                // handed me a live id".
                return Lv2Dispatch::Immediate {
                    code: crate::errno::CELL_ENOMEM.into(),
                    effects: vec![],
                };
            }
            Err(crate::sync_primitives::SemaphoreCreateError::InvalidBounds) => {
                return Lv2Dispatch::Immediate {
                    code: crate::errno::CELL_EINVAL.into(),
                    effects: vec![],
                };
            }
        }
        self.immediate_write_u32(id, id_ptr, requester)
    }

    pub(super) fn dispatch_semaphore_destroy(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.semaphores.lookup(id) else {
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
        self.semaphores.destroy(id);
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    }

    pub(super) fn dispatch_semaphore_wait(&mut self, id: u32, requester: UnitId) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        match self.semaphores.try_wait(id) {
            None => Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            },
            Some(crate::sync_primitives::SemaphoreWait::Acquired) => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            Some(crate::sync_primitives::SemaphoreWait::Empty) => {
                match self.semaphores.enqueue_waiter(id, caller) {
                    Ok(()) => {}
                    // Both errors are host-invariant breaks here
                    // (try_wait just confirmed the id; a blocked
                    // caller cannot re-enter wait). Real
                    // sys_semaphore_wait never returns EDEADLK,
                    // so ESRCH is the safest surface.
                    Err(
                        crate::sync_primitives::SemaphoreEnqueueError::UnknownId
                        | crate::sync_primitives::SemaphoreEnqueueError::DuplicateWaiter,
                    ) => {
                        return Lv2Dispatch::Immediate {
                            code: crate::errno::CELL_ESRCH.into(),
                            effects: vec![],
                        };
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
            None => Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            },
            Some(crate::sync_primitives::SemaphoreWait::Acquired) => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            Some(crate::sync_primitives::SemaphoreWait::Empty) => Lv2Dispatch::Immediate {
                code: crate::errno::CELL_EBUSY.into(),
                effects: vec![],
            },
        }
    }

    pub(super) fn dispatch_semaphore_get_value(
        &mut self,
        id: u32,
        out_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        let Some(entry) = self.semaphores.lookup(id) else {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        let count = entry.count() as u32;
        self.immediate_write_u32(count, out_ptr, requester)
    }

    pub(super) fn dispatch_semaphore_post(&mut self, id: u32, val: i32) -> Lv2Dispatch {
        // val != 1 would require waking multiple waiters in one
        // WakeAndReturn; not wired.
        if val != 1 {
            return Lv2Dispatch::Immediate {
                code: crate::errno::CELL_EINVAL.into(),
                effects: vec![],
            };
        }
        match self.semaphores.post_and_wake(id) {
            crate::sync_primitives::SemaphorePost::Unknown => Lv2Dispatch::Immediate {
                code: crate::errno::CELL_ESRCH.into(),
                effects: vec![],
            },
            crate::sync_primitives::SemaphorePost::OverMax => Lv2Dispatch::Immediate {
                code: crate::errno::CELL_EINVAL.into(),
                effects: vec![],
            },
            crate::sync_primitives::SemaphorePost::Incremented => Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            },
            crate::sync_primitives::SemaphorePost::Woke { new_owner } => {
                // Waiter is already off the list; releaser
                // returns CELL_OK even if resolve_wake_thread
                // fires the host-invariant break, since the
                // waiter is already stranded at that point.
                match self.resolve_wake_thread(new_owner, "semaphore_post.Woke") {
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
    fn semaphore_create_writes_id_and_stores_entry() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(
            Lv2Request::SemaphoreCreate {
                id_ptr: 0x100,
                attr_ptr: 0,
                initial: 2,
                max: 10,
            },
            src,
            &rt,
        );
        let id = match &r {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        let entry = host.semaphores().lookup(id).unwrap();
        assert_eq!(entry.count(), 2);
        assert_eq!(entry.max(), 10);
    }

    #[test]
    fn semaphore_create_rejects_initial_above_max() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let r = host.dispatch(
            Lv2Request::SemaphoreCreate {
                id_ptr: 0x100,
                attr_ptr: 0,
                initial: 11,
                max: 10,
            },
            UnitId::new(0),
            &rt,
        );
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_EINVAL.into());
    }

    #[test]
    fn semaphore_destroy_unknown_returns_esrch() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let r = host.dispatch(Lv2Request::SemaphoreDestroy { id: 77 }, UnitId::new(0), &rt);
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_ESRCH.into());
    }

    #[test]
    fn semaphore_destroy_with_waiter_returns_ebusy() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(
            Lv2Request::SemaphoreCreate {
                id_ptr: 0x100,
                attr_ptr: 0,
                initial: 0,
                max: 10,
            },
            src,
            &rt,
        );
        let id = match &r {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        host.semaphores_mut()
            .enqueue_waiter(id, PpuThreadId::PRIMARY)
            .unwrap();
        let d = host.dispatch(Lv2Request::SemaphoreDestroy { id }, src, &rt);
        let Lv2Dispatch::Immediate { code, .. } = d else {
            panic!("expected Immediate, got {d:?}");
        };
        assert_eq!(code, crate::errno::CELL_EBUSY.into());
    }

    #[test]
    fn semaphore_wait_with_positive_count_decrements_and_returns_ok() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(
            Lv2Request::SemaphoreCreate {
                id_ptr: 0x100,
                attr_ptr: 0,
                initial: 1,
                max: 10,
            },
            src,
            &rt,
        );
        let id = match &r {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        let w = host.dispatch(Lv2Request::SemaphoreWait { id, timeout: 0 }, src, &rt);
        assert!(matches!(
            w,
            Lv2Dispatch::Immediate {
                code: 0,
                effects: _,
            }
        ));
        assert_eq!(host.semaphores().lookup(id).unwrap().count(), 0);
    }

    #[test]
    fn semaphore_wait_with_zero_count_parks_caller() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(
            Lv2Request::SemaphoreCreate {
                id_ptr: 0x100,
                attr_ptr: 0,
                initial: 0,
                max: 10,
            },
            src,
            &rt,
        );
        let id = match &r {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        let w = host.dispatch(Lv2Request::SemaphoreWait { id, timeout: 0 }, src, &rt);
        match w {
            Lv2Dispatch::Block {
                reason: crate::dispatch::Lv2BlockReason::Semaphore { id: sid },
                pending: PendingResponse::ReturnCode { code: 0 },
                ..
            } => {
                assert_eq!(sid, id);
            }
            other => panic!("expected Block on Semaphore, got {other:?}"),
        }
        let waiters: Vec<_> = host
            .semaphores()
            .lookup(id)
            .unwrap()
            .waiters()
            .iter()
            .collect();
        assert_eq!(waiters, vec![PpuThreadId::PRIMARY]);
    }

    #[test]
    fn semaphore_trywait_with_positive_count_acquires() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(
            Lv2Request::SemaphoreCreate {
                id_ptr: 0x100,
                attr_ptr: 0,
                initial: 1,
                max: 10,
            },
            src,
            &rt,
        );
        let id = match &r {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        let w = host.dispatch(Lv2Request::SemaphoreTryWait { id }, src, &rt);
        assert!(matches!(
            w,
            Lv2Dispatch::Immediate {
                code: 0,
                effects: _,
            }
        ));
        assert_eq!(host.semaphores().lookup(id).unwrap().count(), 0);
    }

    #[test]
    fn semaphore_trywait_with_zero_count_returns_ebusy() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(
            Lv2Request::SemaphoreCreate {
                id_ptr: 0x100,
                attr_ptr: 0,
                initial: 0,
                max: 10,
            },
            src,
            &rt,
        );
        let id = match &r {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        let w = host.dispatch(Lv2Request::SemaphoreTryWait { id }, src, &rt);
        let Lv2Dispatch::Immediate { code, .. } = w else {
            panic!("expected Immediate, got {w:?}");
        };
        assert_eq!(code, crate::errno::CELL_EBUSY.into());
        assert_eq!(host.semaphores().lookup(id).unwrap().count(), 0);
        assert!(host.semaphores().lookup(id).unwrap().waiters().is_empty());
    }

    #[test]
    fn semaphore_trywait_unknown_returns_esrch() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let r = host.dispatch(Lv2Request::SemaphoreTryWait { id: 99 }, UnitId::new(0), &rt);
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_ESRCH.into());
    }

    #[test]
    fn semaphore_get_value_writes_current_count() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(
            Lv2Request::SemaphoreCreate {
                id_ptr: 0x100,
                attr_ptr: 0,
                initial: 5,
                max: 10,
            },
            src,
            &rt,
        );
        let id = match &r {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        let g = host.dispatch(
            Lv2Request::SemaphoreGetValue { id, out_ptr: 0x200 },
            src,
            &rt,
        );
        match &g {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => {
                assert_eq!(extract_write_u32(&e[0]), 5);
            }
            other => panic!("expected Immediate(0), got {other:?}"),
        }
    }

    #[test]
    fn semaphore_get_value_unknown_returns_esrch() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let r = host.dispatch(
            Lv2Request::SemaphoreGetValue {
                id: 99,
                out_ptr: 0x200,
            },
            UnitId::new(0),
            &rt,
        );
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_ESRCH.into());
    }

    #[test]
    fn semaphore_post_unknown_returns_esrch() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let r = host.dispatch(
            Lv2Request::SemaphorePost { id: 99, val: 1 },
            UnitId::new(0),
            &rt,
        );
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_ESRCH.into());
    }

    #[test]
    fn semaphore_post_val_not_one_returns_einval() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let r = host.dispatch(
            Lv2Request::SemaphorePost { id: 1, val: 2 },
            UnitId::new(0),
            &rt,
        );
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_EINVAL.into());
    }

    #[test]
    fn semaphore_post_with_no_waiters_increments() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(
            Lv2Request::SemaphoreCreate {
                id_ptr: 0x100,
                attr_ptr: 0,
                initial: 0,
                max: 10,
            },
            src,
            &rt,
        );
        let id = match &r {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        let post = host.dispatch(Lv2Request::SemaphorePost { id, val: 1 }, src, &rt);
        assert!(matches!(
            post,
            Lv2Dispatch::Immediate {
                code: 0,
                effects: _,
            }
        ));
        assert_eq!(host.semaphores().lookup(id).unwrap().count(), 1);
    }

    #[test]
    fn semaphore_post_wakes_parked_waiter_without_incrementing() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let poster_unit = UnitId::new(0);
        let waiter_unit = UnitId::new(1);
        seed_primary_ppu(&mut host, poster_unit);
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
        let r = host.dispatch(
            Lv2Request::SemaphoreCreate {
                id_ptr: 0x100,
                attr_ptr: 0,
                initial: 0,
                max: 10,
            },
            poster_unit,
            &rt,
        );
        let id = match &r {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        host.dispatch(
            Lv2Request::SemaphoreWait { id, timeout: 0 },
            waiter_unit,
            &rt,
        );
        let post = host.dispatch(Lv2Request::SemaphorePost { id, val: 1 }, poster_unit, &rt);
        match post {
            Lv2Dispatch::WakeAndReturn {
                code: 0,
                woken_unit_ids,
                ..
            } => assert_eq!(woken_unit_ids, vec![waiter_unit]),
            other => panic!("expected WakeAndReturn, got {other:?}"),
        }
        assert_eq!(host.semaphores().lookup(id).unwrap().count(), 0);
        assert!(host.semaphores().lookup(id).unwrap().waiters().is_empty());
    }

    #[test]
    fn semaphore_post_past_max_with_no_waiters_returns_einval() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(
            Lv2Request::SemaphoreCreate {
                id_ptr: 0x100,
                attr_ptr: 0,
                initial: 3,
                max: 3,
            },
            src,
            &rt,
        );
        let id = match &r {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        let post = host.dispatch(Lv2Request::SemaphorePost { id, val: 1 }, src, &rt);
        let Lv2Dispatch::Immediate { code, .. } = post else {
            panic!("expected Immediate, got {post:?}");
        };
        assert_eq!(code, crate::errno::CELL_EINVAL.into());
        assert_eq!(host.semaphores().lookup(id).unwrap().count(), 3);
    }

    #[test]
    fn semaphore_wait_unknown_id_returns_esrch() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(Lv2Request::SemaphoreWait { id: 99, timeout: 0 }, src, &rt);
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, crate::errno::CELL_ESRCH.into());
    }
}
