//! PPU thread lifecycle dispatch (create, exit, join).

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors;

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
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        if target_thread.state.is_finished() {
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
        let Some(caller_thread_id) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
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
            AddJoinWaiter::SelfJoin => Lv2Dispatch::immediate(cell_errors::CELL_EDEADLK.into()),
            AddJoinWaiter::TargetDetached => Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into()),
            AddJoinWaiter::UnknownTarget | AddJoinWaiter::TargetAlreadyFinished => {
                self.record_invariant_break(
                    "ppu_thread_join.add_join_waiter_unreachable",
                    format_args!(
                        "add_join_waiter returned an outcome the upstream checks ruled out \
                         for target {target_id:?}"
                    ),
                );
                Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into())
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
        // r4 is a `ppu_thread_param_t *`: `{ u32 entry_opd_ptr; u32
        // tls; }`. The OPD it points to is `{ u32 code; u32 toc; }`
        // (8 bytes, not PowerOpen 24).
        let param_bytes: [u8; 8] = match rt
            .read_committed(param_ptr as u64, 8)
            .and_then(|bytes| bytes.first_chunk::<8>().copied())
        {
            Some(arr) => arr,
            None => {
                return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
            }
        };
        let entry_opd_ptr = u32::from_be_bytes([
            param_bytes[0],
            param_bytes[1],
            param_bytes[2],
            param_bytes[3],
        ]);
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
                return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
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
                return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
            }
        };

        let tls_bytes = self.tls_template.instantiate();
        // Empty template encodes "no TLS, r13 = 0".
        let tls_base = if tls_bytes.is_empty() {
            0
        } else {
            (stack.end() + 0xF) & !0xF
        };

        if !tls_bytes.is_empty() && tls_base == 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
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
                // LR=0 traps a fallthrough return; guests exit via
                // sys_ppu_thread_exit.
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
        // Abnormal exit paths skip the HLE unlock wrapper, so clear
        // the hold count here.
        if let Some(tid) = self.ppu_threads.thread_id_for_unit(requester) {
            self.lwmutex_holds_clear(tid);
        }
        let waiters_unit_ids = match self.ppu_threads.thread_id_for_unit(requester) {
            Some(tid) => {
                let waiter_thread_ids = self.ppu_threads.mark_finished(tid, exit_value);
                waiter_thread_ids
                    .into_iter()
                    .filter_map(|wtid| self.resolve_wake_thread(wtid, "ppu_thread_exit.joiner"))
                    .collect()
            }
            None => {
                // Empty table is a legitimate testkit pre-seed; a
                // non-empty table with no caller entry would strand
                // joiners.
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
    /// Kernel lwmutex entries carry no owner record; each woken thread
    /// fixes up user-space owner / waiter / recursive_count via its
    /// `LwMutexWake` pending response.
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
#[path = "tests/ppu_thread_tests.rs"]
mod tests;
