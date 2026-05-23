//! PPU thread lifecycle dispatch (create, exit, join).

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors as errno;

use crate::dispatch::{Lv2Dispatch, PendingResponse};
use crate::host::{Lv2Host, Lv2Runtime};
use crate::ppu_thread::{AddJoinWaiter, PpuThreadId};

impl Lv2Host {
    pub(super) fn dispatch_ppu_thread_join(
        &mut self,
        target: u64,
        status_out_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        let target_id = PpuThreadId::new(target);
        let Some(target_thread) = self.ppu_threads.get(target_id) else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        if matches!(
            target_thread.state,
            crate::ppu_thread::PpuThreadState::Finished
        ) {
            let exit_value = target_thread.exit_value.unwrap_or(0);
            let write = Effect::SharedWriteIntent {
                range: ByteRange::contiguous_u32(status_out_ptr, 8),
                bytes: WritePayload::from_slice(&exit_value.to_be_bytes()),
                ordering: PriorityClass::Normal,
                source: requester,
                source_time: self.current_tick,
            };
            return Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![write],
            };
        }
        // No fallback to PRIMARY: that would route the exit-wake to
        // the wrong unit when the caller is not in the table.
        let Some(caller_thread_id) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        match self
            .ppu_threads
            .add_join_waiter(target_id, caller_thread_id)
        {
            AddJoinWaiter::Parked => Lv2Dispatch::Block {
                reason: crate::dispatch::Lv2BlockReason::PpuThreadJoin { target },
                pending: PendingResponse::PpuThreadJoin {
                    target,
                    status_out_ptr,
                },
                effects: vec![],
            },
            AddJoinWaiter::SelfJoin => Lv2Dispatch::immediate(errno::CELL_EDEADLK.into()),
            AddJoinWaiter::TargetDetached => Lv2Dispatch::immediate(errno::CELL_ESRCH.into()),
            // Pre-checks above eliminate both variants; reaching here
            // means the table mutated mid-call.
            AddJoinWaiter::UnknownTarget | AddJoinWaiter::TargetAlreadyFinished => {
                self.record_invariant_break(
                    "ppu_thread_join.add_join_waiter_unreachable",
                    format_args!(
                        "add_join_waiter returned an outcome the upstream checks ruled out \
                         for target {target_id:?}"
                    ),
                );
                Lv2Dispatch::immediate(errno::CELL_ESRCH.into())
            }
        }
    }

    pub(super) fn dispatch_ppu_thread_create(
        &mut self,
        id_ptr: u32,
        param_ptr: u32,
        arg: u64,
        priority: u32,
        stacksize: u64,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        // syscall 52 (`_sys_ppu_thread_create`) takes a
        // `ppu_thread_param_t *` in r4 (here `param_ptr`), NOT an OPD
        // pointer. Layout: `{ u32 entry_opd_ptr; u32 tls; }`. First
        // deref gives the OPD address; the OPD itself is the standard
        // 8-byte PS3 `{ u32 code; u32 toc; }` (not PowerOpen 24).
        // Either deref failing routes to CELL_EFAULT before any side
        // effects (stack alloc, TLS commit).
        let param_bytes: [u8; 8] = match rt
            .read_committed(param_ptr as u64, 8)
            .and_then(|bytes| bytes.first_chunk::<8>().copied())
        {
            Some(arr) => arr,
            None => {
                return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
            }
        };
        let entry_opd_ptr = u32::from_be_bytes([
            param_bytes[0],
            param_bytes[1],
            param_bytes[2],
            param_bytes[3],
        ]);
        // `param->tls` is a TLS-template pointer the title pre-built;
        // CellGov's TLS-template path already covers per-thread init,
        // so the field is captured here for future use but not yet
        // routed into PpuThreadInitState.
        let _param_tls = u32::from_be_bytes([
            param_bytes[4],
            param_bytes[5],
            param_bytes[6],
            param_bytes[7],
        ]);

        let opd_bytes: [u8; 8] = match rt
            .read_committed(entry_opd_ptr as u64, 8)
            .and_then(|bytes| bytes.first_chunk::<8>().copied())
        {
            Some(arr) => arr,
            None => {
                return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
            }
        };
        let entry_code =
            u32::from_be_bytes([opd_bytes[0], opd_bytes[1], opd_bytes[2], opd_bytes[3]]) as u64;
        let entry_toc =
            u32::from_be_bytes([opd_bytes[4], opd_bytes[5], opd_bytes[6], opd_bytes[7]]) as u64;

        // 0x4000 floor covers the ABI back-chain + register save area.
        let size = stacksize.max(0x4000);
        let stack = match self.allocate_child_stack(size, 0x10) {
            Some(s) => s,
            None => {
                return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
            }
        };

        // Empty template encodes "no TLS, r13 = 0"; non-empty places
        // the slot 16-aligned above the stack.
        let tls_bytes = self.tls_template.instantiate();
        let tls_base = if tls_bytes.is_empty() {
            0
        } else {
            (stack.end() + 0xF) & !0xF
        };

        // Non-empty TLS at guest 0 would alias the empty-template sentinel.
        if !tls_bytes.is_empty() && tls_base == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }

        Lv2Dispatch::PpuThreadCreate {
            id_ptr,
            init: crate::dispatch::PpuThreadInitState {
                entry_code,
                entry_toc,
                arg,
                extra_args: [0; 7],
                stack_top: stack.initial_sp(),
                tls_base,
                // Guests exit via sys_ppu_thread_exit; LR=0 traps a
                // fallthrough return.
                lr_sentinel: 0,
            },
            stack_base: stack.base,
            stack_size: stack.size,
            tls_bytes,
            priority,
            effects: vec![],
        }
    }

    pub(super) fn dispatch_ppu_thread_exit(
        &mut self,
        exit_value: u64,
        requester: UnitId,
    ) -> Lv2Dispatch {
        // Abnormal exit (or any path that skips the HLE unlock wrapper)
        // would otherwise leak the hold count forever.
        if let Some(tid) = self.ppu_threads.thread_id_for_unit(requester) {
            self.lwmutex_holds_clear(tid);
        }
        let waiters_unit_ids = match self.ppu_threads.thread_id_for_unit(requester) {
            Some(tid) => {
                // resolve_wake_thread filters waiters whose table
                // entry vanished between join and wake.
                let waiter_thread_ids = self.ppu_threads.mark_finished(tid, exit_value);
                waiter_thread_ids
                    .into_iter()
                    .filter_map(|wtid| self.resolve_wake_thread(wtid, "ppu_thread_exit.joiner"))
                    .collect()
            }
            None => {
                // Empty table is a legitimate pre-seed (testkit);
                // non-empty + absent caller would strand joiners.
                if !self.ppu_threads.is_empty() {
                    self.record_invariant_break(
                        "ppu_thread_exit.unknown_caller",
                        format_args!(
                            "sys_ppu_thread_exit from UnitId {requester:?} not in \
                             PpuThreadTable (table non-empty); joiners (if any) will \
                             not wake"
                        ),
                    );
                }
                Vec::new()
            }
        };
        let lwmutex_inheritors = self.release_held_lwmutexes_on_exit();
        Lv2Dispatch::PpuThreadExit {
            exit_value,
            woken_unit_ids: waiters_unit_ids,
            lwmutex_inheritors,
            effects: vec![],
        }
    }

    /// Transfer one waiter from each non-empty kernel lwmutex queue.
    ///
    /// Kernel lwmutex entries carry no owner record, so on thread
    /// exit we cannot identify the held set; we wake one waiter per
    /// non-empty queue and rely on each `LwMutexWake` pending
    /// response to fix up user-space owner / waiter / recursive_count.
    fn release_held_lwmutexes_on_exit(&mut self) -> Vec<UnitId> {
        let ids: Vec<u32> = self
            .lwmutexes
            .iter_ids()
            .filter(|id| {
                self.lwmutexes
                    .lookup(*id)
                    .map(|e| !e.waiters().is_empty())
                    .unwrap_or(false)
            })
            .collect();
        let mut inheritors = Vec::new();
        for id in ids {
            if let crate::sync_primitives::LwMutexRelease::Transferred { new_owner } = self
                .lwmutexes
                .release_and_wake_next(id, PpuThreadId::PRIMARY)
            {
                if let Some(unit) =
                    self.resolve_wake_thread(new_owner, "ppu_thread_exit.lwmutex_transfer")
                {
                    inheritors.push(unit);
                }
            }
        }
        inheritors
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::test_support::{opd_runtime, primary_attrs, FakeRuntime};
    use crate::request::Lv2Request;

    #[test]
    fn ppu_thread_exit_marks_thread_finished_with_exit_value() {
        let mut host = Lv2Host::new();
        host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(
            Lv2Request::PpuThreadExit {
                exit_value: 0xDEAD_BEEF,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::PpuThreadExit {
                exit_value,
                woken_unit_ids,
                effects,
                ..
            } => {
                assert_eq!(exit_value, 0xDEAD_BEEF);
                assert!(woken_unit_ids.is_empty());
                assert!(effects.is_empty());
            }
            other => panic!("expected PpuThreadExit dispatch, got {other:?}"),
        }
        let primary = host.ppu_thread_for_unit(UnitId::new(0)).unwrap();
        assert_eq!(primary.state, crate::ppu_thread::PpuThreadState::Finished);
        assert_eq!(primary.exit_value, Some(0xDEAD_BEEF));
    }

    #[test]
    fn ppu_thread_exit_unseeded_thread_still_returns_dispatch() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(
            Lv2Request::PpuThreadExit { exit_value: 7 },
            UnitId::new(99),
            &rt,
        );
        match result {
            Lv2Dispatch::PpuThreadExit {
                exit_value,
                woken_unit_ids,
                ..
            } => {
                assert_eq!(exit_value, 7);
                assert!(woken_unit_ids.is_empty());
            }
            other => panic!("expected PpuThreadExit, got {other:?}"),
        }
    }

    #[test]
    fn ppu_thread_exit_wakes_join_waiters() {
        let mut host = Lv2Host::new();
        host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
        let child_tid = host
            .ppu_threads_mut()
            .create(UnitId::new(1), primary_attrs())
            .expect("child create");
        host.ppu_threads_mut()
            .add_join_waiter(child_tid, crate::ppu_thread::PpuThreadId::PRIMARY);
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(
            Lv2Request::PpuThreadExit { exit_value: 5 },
            UnitId::new(1),
            &rt,
        );
        match result {
            Lv2Dispatch::PpuThreadExit {
                exit_value,
                woken_unit_ids,
                ..
            } => {
                assert_eq!(exit_value, 5);
                assert_eq!(woken_unit_ids, vec![UnitId::new(0)]);
            }
            other => panic!("expected PpuThreadExit, got {other:?}"),
        }
    }

    #[test]
    fn ppu_thread_join_finished_target_returns_immediate_with_exit_value() {
        let mut host = Lv2Host::new();
        host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
        let child = host
            .ppu_threads_mut()
            .create(UnitId::new(1), primary_attrs())
            .expect("child create");
        host.ppu_threads_mut().mark_finished(child, 0xFEED_FACE);
        let rt = FakeRuntime::new(0x10000);
        let result = host.dispatch(
            Lv2Request::PpuThreadJoin {
                target: child.raw(),
                status_out_ptr: 0x500,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, 0);
                assert_eq!(effects.len(), 1);
                if let Effect::SharedWriteIntent { range, bytes, .. } = &effects[0] {
                    assert_eq!(range.start().raw(), 0x500);
                    assert_eq!(range.length(), 8);
                    assert_eq!(bytes.bytes(), &0xFEED_FACE_u64.to_be_bytes());
                } else {
                    panic!("expected SharedWriteIntent");
                }
            }
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    #[test]
    fn ppu_thread_join_running_target_blocks_and_records_waiter() {
        let mut host = Lv2Host::new();
        host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
        let child = host
            .ppu_threads_mut()
            .create(UnitId::new(1), primary_attrs())
            .expect("child create");
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(
            Lv2Request::PpuThreadJoin {
                target: child.raw(),
                status_out_ptr: 0x500,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::Block {
                reason, pending, ..
            } => {
                assert!(matches!(
                    reason,
                    crate::dispatch::Lv2BlockReason::PpuThreadJoin { target } if target == child.raw()
                ));
                assert!(matches!(
                    pending,
                    PendingResponse::PpuThreadJoin {
                        status_out_ptr: 0x500,
                        ..
                    }
                ));
            }
            other => panic!("expected Block, got {other:?}"),
        }
        assert_eq!(
            host.ppu_threads().get(child).unwrap().join_waiters,
            vec![crate::ppu_thread::PpuThreadId::PRIMARY],
        );
    }

    #[test]
    fn ppu_thread_join_unknown_target_returns_esrch() {
        let mut host = Lv2Host::new();
        host.seed_primary_ppu_thread(UnitId::new(0), primary_attrs());
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(
            Lv2Request::PpuThreadJoin {
                target: 0xDEAD_BEEF,
                status_out_ptr: 0x500,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::Immediate { code, effects } => {
                assert_eq!(code, errno::CELL_ESRCH.into());
                assert!(effects.is_empty());
            }
            other => panic!("expected Immediate with ESRCH, got {other:?}"),
        }
    }

    #[test]
    fn ppu_thread_create_returns_dispatch_with_allocated_stack_and_tls() {
        let mut host = Lv2Host::new();
        host.set_tls_template(crate::ppu_thread::TlsTemplate::new(
            vec![0xAB, 0xCD, 0xEF],
            0x100,
            0x10,
            0x89_5cd0,
        ));
        let rt = opd_runtime(0x200, 0x10_0000, 0x10_0100);
        let result = host.dispatch(
            Lv2Request::PpuThreadCreate {
                id_ptr: 0x1000,
                param_ptr: 0x200,
                arg: 0xDEAD_BEEF,
                priority: 1500,
                stacksize: 0x10_000,
                flags: 0,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::PpuThreadCreate {
                id_ptr,
                init,
                stack_base,
                stack_size,
                tls_bytes,
                priority,
                effects,
            } => {
                assert_eq!(id_ptr, 0x1000);
                assert_eq!(init.entry_code, 0x10_0000);
                assert_eq!(init.entry_toc, 0x10_0100);
                assert_eq!(init.arg, 0xDEAD_BEEF);
                assert_eq!(priority, 1500);
                assert_eq!(stack_base, 0xD010_0000);
                assert_eq!(stack_size, 0x10_000);
                assert_eq!(init.stack_top, 0xD011_0000 - 0x10);
                assert!(init.tls_base >= stack_base + stack_size);
                assert_eq!(tls_bytes.len(), 0x100);
                assert_eq!(&tls_bytes[..3], &[0xAB, 0xCD, 0xEF]);
                assert!(tls_bytes[3..].iter().all(|&b| b == 0));
                assert!(effects.is_empty());
            }
            other => panic!("expected PpuThreadCreate, got {other:?}"),
        }
    }

    #[test]
    fn ppu_thread_create_with_empty_template_has_no_tls() {
        let mut host = Lv2Host::new();
        let rt = opd_runtime(0x200, 0, 0);
        let result = host.dispatch(
            Lv2Request::PpuThreadCreate {
                id_ptr: 0x1000,
                param_ptr: 0x200,
                arg: 0,
                priority: 1000,
                stacksize: 0x8000,
                flags: 0,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::PpuThreadCreate {
                init, tls_bytes, ..
            } => {
                assert_eq!(init.tls_base, 0);
                assert!(tls_bytes.is_empty());
            }
            other => panic!("expected PpuThreadCreate, got {other:?}"),
        }
    }

    #[test]
    fn ppu_thread_create_enforces_minimum_stack_size() {
        let mut host = Lv2Host::new();
        let rt = opd_runtime(0x200, 0, 0);
        let result = host.dispatch(
            Lv2Request::PpuThreadCreate {
                id_ptr: 0x1000,
                param_ptr: 0x200,
                arg: 0,
                priority: 1000,
                stacksize: 0x100,
                flags: 0,
            },
            UnitId::new(0),
            &rt,
        );
        match result {
            Lv2Dispatch::PpuThreadCreate { stack_size, .. } => {
                assert_eq!(stack_size, 0x4000);
            }
            other => panic!("expected PpuThreadCreate, got {other:?}"),
        }
    }

    #[test]
    fn ppu_thread_create_bad_opd_returns_efault() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(0x100);
        let result = host.dispatch(
            Lv2Request::PpuThreadCreate {
                id_ptr: 0x10,
                param_ptr: 0xDEAD_BEEF,
                arg: 0,
                priority: 1000,
                stacksize: 0x4000,
                flags: 0,
            },
            UnitId::new(0),
            &rt,
        );
        assert_eq!(result, Lv2Dispatch::immediate(errno::CELL_EFAULT.into()));
    }

    #[test]
    fn ppu_thread_create_bad_opd_via_param_returns_efault() {
        // First deref succeeds (param sits in bounds) but the
        // entry_opd_ptr it carries is unmapped, so the second
        // deref must surface CELL_EFAULT.
        let mut mem = cellgov_mem::GuestMemory::new(0x1_0000);
        let mut param_bytes = [0u8; 8];
        param_bytes[0..4].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
        let param_range =
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0x200), 8).unwrap();
        mem.apply_commit(param_range, &param_bytes).unwrap();
        let rt = crate::host::test_support::FakeRuntime::with_memory(mem);

        let mut host = Lv2Host::new();
        let result = host.dispatch(
            Lv2Request::PpuThreadCreate {
                id_ptr: 0x10,
                param_ptr: 0x200,
                arg: 0,
                priority: 1000,
                stacksize: 0x4000,
                flags: 0,
            },
            UnitId::new(0),
            &rt,
        );
        assert_eq!(result, Lv2Dispatch::immediate(errno::CELL_EFAULT.into()));
    }

    #[test]
    fn ppu_thread_yield_returns_ok_with_no_effects() {
        let mut host = Lv2Host::new();
        let rt = FakeRuntime::new(256);
        let result = host.dispatch(Lv2Request::PpuThreadYield, UnitId::new(0), &rt);
        assert_eq!(result, Lv2Dispatch::immediate(0));
    }
}
