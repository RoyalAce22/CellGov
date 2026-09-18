//! PPU thread lifecycle dispatch (create, exit, join).

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;

use crate::dispatch::{Lv2Dispatch, PendingResponse};
use crate::host::guest_struct::GuestStruct;
use crate::host::{Lv2Host, Lv2Runtime};
use crate::ppu_thread::{AddJoinWaiter, PpuThreadId};
use cellgov_time::GuestTicks;

impl Lv2Host {
    /// `sys_ppu_thread_join`: writes a finished target's exit value to
    /// `status_out_ptr` at once. Otherwise it parks the caller on the
    /// target until `sys_ppu_thread_exit` wakes it.
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` when `target` names no thread, or the caller has
    ///   no thread-table entry.
    /// - `CELL_EFAULT` when the target is finished and
    ///   `status_out_ptr` is null; the arm drops the exit value.
    /// - `CELL_EDEADLK` on a self-join.
    /// - `CELL_EINVAL` when the target is detached.
    pub(super) fn dispatch_ppu_thread_join(
        &mut self,
        target: u64,
        status_out_ptr: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let target_id = PpuThreadId::new(target);
        let Some(target_thread) = self.state.ppu_threads.get(target_id) else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        if target_thread.state.is_finished() {
            let exit_value = target_thread.exit_value.unwrap_or(0);
            // The join completes, then the status pointer is checked:
            // a null pointer is EFAULT and the exit status is
            // dropped, not written. Both codes are defined for this
            // call; that the join runs first is a CellGov choice,
            // unestablished against the console.
            if status_out_ptr == 0 {
                return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
            }
            let write = Effect::shared_write(
                ByteRange::contiguous_u32(status_out_ptr, 8),
                WritePayload::from_slice(&exit_value.to_be_bytes()),
                requester,
                tick,
            );
            return Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![write],
            };
        }
        let Some(caller_thread_id) = self.state.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        match self
            .state
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
            // A join against a target that is not joinable is EINVAL.
            // ESRCH is reserved for ids that name no thread or an
            // already-reaped one.
            AddJoinWaiter::TargetDetached => Lv2Dispatch::immediate(errno::CELL_EINVAL.into()),
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

    /// `sys_ppu_thread_create`: reads the thread param and its entry
    /// descriptor, takes a stack block from the child-stack arena, and
    /// hands the runtime a seeded init state. The runtime installs the
    /// block as a region in the creator's address space and registers
    /// the unit through the PPU factory. A create the runtime refuses
    /// returns the block to the arena.
    ///
    /// The arena is a deterministic bump allocator: the arm floors the
    /// requested size at 0x4000, and two fresh hosts hand out the same
    /// blocks in the same order.
    ///
    /// # Errors
    ///
    /// Listed in the order they fire.
    ///
    /// - `CELL_EFAULT` when the param block is unreadable or its entry
    ///   descriptor pointer is null.
    /// - `CELL_EINVAL` when `priority` is outside the window
    ///   `sys_ppu_thread_set_priority` enforces.
    /// - `CELL_EFAULT` when the entry descriptor is unreadable.
    /// - `CELL_ENOMEM` when the arena is exhausted.
    pub(super) fn dispatch_ppu_thread_create(
        &mut self,
        id_ptr: u32,
        param_ptr: u32,
        arg: u64,
        priority: u32,
        stacksize: u64,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        use cellgov_ps3_abi::format::elf::function_descriptor;
        use cellgov_ps3_abi::lv2::ppu_thread::thread_param;
        let Some(param) = GuestStruct::read(rt, param_ptr as u64, thread_param::SIZE as usize)
        else {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        };
        let entry_opd_ptr = param.u32_at(thread_param::ENTRY_OFFSET);
        let param_tls = param.u32_at(thread_param::TLS_OFFSET);

        // A null entry descriptor is EFAULT. The check runs before
        // the priority range test; that order is a CellGov choice,
        // unestablished against the console.
        if entry_opd_ptr == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }

        // `sys_ppu_thread_set_priority` (47) enforces the same window.
        use cellgov_ps3_abi::lv2::ppu_thread::{
            PPU_THREAD_PRIORITY_MAX, PPU_THREAD_PRIORITY_MIN, PPU_THREAD_PRIORITY_MIN_ROOT,
        };
        let prio = priority as i32;
        let prio_floor = if self.debug_or_root() {
            PPU_THREAD_PRIORITY_MIN_ROOT
        } else {
            PPU_THREAD_PRIORITY_MIN
        };
        if !(prio_floor..=PPU_THREAD_PRIORITY_MAX).contains(&prio) {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }

        let Some(opd) = GuestStruct::read(rt, entry_opd_ptr as u64, function_descriptor::SIZE)
        else {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        };
        let entry_code = u64::from(opd.u32_at(function_descriptor::CODE_OFFSET));
        let entry_toc = u64::from(opd.u32_at(function_descriptor::TOC_OFFSET));

        // 0x4000 floor covers the ABI back-chain + register save area.
        let size = stacksize.max(0x4000);
        let stack = match self.allocate_child_stack(size, 0x10) {
            Some(s) => s,
            None => {
                return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
            }
        };

        Lv2Dispatch::PpuThreadCreate {
            id_ptr,
            init: crate::dispatch::PpuThreadInitState {
                entry_code,
                entry_toc,
                arg,
                extra_args: [0; 7],
                stack_top: stack.initial_sp(),
                // r13 is the PPU thread's TLS base: liblv2 reaches a
                // thread's own id through an r13-relative TLS slot in
                // sys_lwmutex_lock. The kernel installs the param's
                // tls field there verbatim and unvalidated; the
                // caller's wrapper allocated and initialized the
                // block before the syscall.
                tls_base: param_tls as u64,
                // LR=0 traps a fallthrough return; guests exit via
                // sys_ppu_thread_exit.
                lr_sentinel: 0,
            },
            stack_base: stack.base,
            stack_size: stack.size,
            priority,
            effects: vec![],
        }
    }

    /// `sys_ppu_thread_exit`: marks the caller's thread Finished with
    /// `exit_value` and names every joiner to wake with it.
    ///
    /// The arm also clears the caller's lwmutex hold count and hands
    /// every waited-on kernel lwmutex to its next waiter (see
    /// [`Self::release_held_lwmutexes_on_exit`]).
    pub(super) fn dispatch_ppu_thread_exit(
        &mut self,
        exit_value: u64,
        requester: UnitId,
    ) -> Lv2Dispatch {
        // Abnormal exit paths skip the HLE unlock wrapper, so clear
        // the hold count here.
        if let Some(tid) = self.state.ppu_threads.thread_id_for_unit(requester) {
            self.lwmutex_holds_clear(tid);
        }
        let waiters_unit_ids = match self.state.ppu_threads.thread_id_for_unit(requester) {
            Some(tid) => {
                let waiter_thread_ids = self.state.ppu_threads.mark_finished(tid, exit_value);
                waiter_thread_ids
                    .into_iter()
                    .filter_map(|wtid| self.resolve_wake_thread(wtid, "ppu_thread_exit.joiner"))
                    .collect()
            }
            None => {
                // Empty table is a legitimate testkit pre-seed; a
                // non-empty table with no caller entry would strand
                // joiners.
                if !self.state.ppu_threads.is_empty() {
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
            .state
            .lwmutexes
            .iter_ids()
            .filter(|id| {
                self.state
                    .lwmutexes
                    .lookup(*id)
                    .map(|e| !e.waiters().is_empty())
                    .unwrap_or(false)
            })
            .collect();
        let mut inheritors = Vec::new();
        for id in ids {
            if let crate::sync_primitives::LwMutexRelease::Transferred { new_owner } = self
                .state
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
