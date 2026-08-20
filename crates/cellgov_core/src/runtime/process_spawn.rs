//! Process spawning and child-process exit.
//!
//! `handle_process_spawn` builds the child: a fresh address space,
//! the image installed through the caller-provided
//! [`ProcessSpawnLoader`], a primary-thread unit from the PPU
//! factory, and the unit/space/pid bindings. `handle_process_exit_child`
//! finishes exactly the exiting process's units; the run continues.
//!
//! [`ProcessSpawnLoader`]: crate::runtime::types::ProcessSpawnLoader

use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
use cellgov_lv2::{Lv2Dispatch, PpuThreadAttrs, PpuThreadInitState};
use cellgov_ps3_abi::cell_errors::{CELL_EFAULT, CELL_ENOMEM, CELL_ENOSYS};

use super::spaces::AddressSpaceId;
use super::Runtime;

impl Runtime {
    pub(super) fn handle_process_spawn(&mut self, source: UnitId, dispatch: Lv2Dispatch) {
        let caller_space = self.spaces.space_of(source);
        let (pid, pid_out_ptr, prio, path, elf_bytes) = match dispatch {
            Lv2Dispatch::ProcessSpawn {
                pid,
                pid_out_ptr,
                prio,
                path,
                elf_bytes,
                effects,
            } => {
                self.apply_lv2_effects(&effects, caller_space);
                (pid, pid_out_ptr, prio, path, elf_bytes)
            }
            other => unreachable!("handle_process_spawn called with {other:?}"),
        };

        let fail = |rt: &mut Self, pid: u32, code: u64| {
            rt.lv2_host.unbind_spawned_process(pid);
            rt.registry.set_syscall_return(source, code);
        };

        if self.process_spawn_loader.is_none() || self.ppu_factory.is_none() {
            self.lv2_host.log_invariant_break(
                "runtime.process_spawn_loader_missing",
                format_args!(
                    "spawn of {:?} dispatched without a process spawn loader / PPU factory; \
                     install both via set_process_spawn_loader / set_ppu_factory",
                    String::from_utf8_lossy(&path),
                ),
            );
            return fail(self, pid, CELL_ENOSYS.into());
        }

        // Deterministic: one space per spawned process, ids dense
        // from 1 in spawn order.
        let next_space_raw = self
            .spaces
            .extra
            .keys()
            .next_back()
            .map_or(Some(1), |s| s.raw().checked_add(1));
        let Some(next_space_raw) = next_space_raw else {
            // Reachable via `create_address_space(u32::MAX)`; a wrap
            // here would alias space 0 (the boot space).
            self.lv2_host.log_invariant_break(
                "runtime.process_spawn_space_ids_exhausted",
                format_args!(
                    "spawn of {:?} found the maximum address-space id already in use; \
                     no child space can be allocated",
                    String::from_utf8_lossy(&path),
                ),
            );
            return fail(self, pid, CELL_ENOMEM.into());
        };
        let space = AddressSpaceId::new(next_space_raw);
        self.create_address_space(space)
            .expect("space id chosen past the maximum existing id");

        let image = {
            let loader = self
                .process_spawn_loader
                .as_ref()
                .expect("checked is_some above");
            let mem = self
                .spaces
                .extra
                .get_mut(&space)
                .expect("space created above");
            loader(&elf_bytes, mem)
        };
        let image = match image {
            Ok(image) => image,
            Err(reason) => {
                self.spaces.extra.remove(&space);
                self.spaces.extra_reservations.remove(&space);
                self.lv2_host.log_invariant_break(
                    "runtime.process_spawn_image_load_failed",
                    format_args!(
                        "spawn loader rejected {:?}: {reason}",
                        String::from_utf8_lossy(&path),
                    ),
                );
                return fail(self, pid, CELL_EFAULT.into());
            }
        };

        let init = PpuThreadInitState {
            entry_code: image.entry_code,
            entry_toc: image.entry_toc,
            arg: 0,
            extra_args: [0; 7],
            stack_top: image.stack_top,
            // The child's CRT0 owns main-thread TLS (the
            // sys_initialize_tls path); the kernel seeds no block.
            tls_base: 0,
            lr_sentinel: image.lr_sentinel,
        };
        let factory = self.ppu_factory.as_ref().expect("checked is_some above");
        let child_unit = self
            .registry
            .register_dynamic(&|id| factory(id, init.clone()));
        self.assign_unit_space(child_unit, space)
            .expect("space created above");
        self.lv2_host.bind_unit_process(child_unit, pid);

        // PPU thread creation rejects a priority outside 0..=3071
        // with EINVAL for a retail-authority caller (RPCS3
        // sys_ppu_thread.cpp _sys_ppu_thread_create); whether this
        // syscall does the same for the child's primary thread is
        // undecoded (RPCS3 todo-stubs it), so an out-of-range value
        // is witnessed instead of silently normalized.
        if !(0..=3071).contains(&prio) {
            self.lv2_host.log_invariant_break(
                "runtime.process_spawn_prio_out_of_range",
                format_args!(
                    "spawn of {:?} carries primary-thread prio {prio} outside 0..=3071; \
                     using {} for the child's primary thread",
                    String::from_utf8_lossy(&path),
                    prio.max(0),
                ),
            );
        }
        let attrs = PpuThreadAttrs {
            entry: image.entry_code,
            arg: 0,
            stack_base: 0,
            stack_size: 0,
            priority: prio.max(0) as u32,
            tls_base: 0,
        };
        if self
            .lv2_host
            .ppu_threads_mut()
            .create(child_unit, attrs)
            .is_none()
        {
            // Mirror the loader-failure rollback: the child space and
            // the unit's space tag feed metadata_hash /
            // committed_memory_hash and must not outlive the failed
            // spawn. The registered unit cannot be removed; it stays
            // Finished (inert) in the boot space.
            self.registry
                .set_status_override(child_unit, UnitStatus::Finished);
            self.spaces.unit_spaces.remove(&child_unit);
            self.spaces.extra.remove(&space);
            self.spaces.extra_reservations.remove(&space);
            self.lv2_host.log_invariant_break(
                "runtime.process_spawn_thread_ids_exhausted",
                format_args!(
                    "spawn of {:?}: PPU thread-id allocator exhausted; \
                     child unit {child_unit:?} left registered as Finished",
                    String::from_utf8_lossy(&path),
                ),
            );
            return fail(self, pid, CELL_ENOMEM.into());
        }

        // Pid writeback lands in the CALLER's space.
        // Lv2Host::dispatch_process_spawn already rejected an
        // unwritable pid_out_ptr with CELL_EFAULT, so a failure inside
        // commit_bytes_at is host-state corruption.
        self.commit_bytes_at(caller_space, u64::from(pid_out_ptr), &pid.to_be_bytes());
        self.step_woke_others = true;
        self.registry.set_syscall_return(source, 0);
    }

    /// Finish exactly `pid`'s units and record its exit status; the
    /// rest of the system keeps running. Parent observation is by
    /// `sys_process_get_status` polling -- no wake is owed.
    pub(super) fn handle_process_exit_child(
        &mut self,
        source: UnitId,
        pid: u32,
        code: i32,
        effects: Vec<cellgov_effects::Effect>,
    ) {
        let caller_space = self.spaces.space_of(source);
        self.apply_lv2_effects(&effects, caller_space);
        let units = self.lv2_host.units_of_process(pid);
        debug_assert!(
            units.contains(&source),
            "child exit from {source:?} for pid {pid:#x} whose unit set {units:?} \
             does not contain the caller; unit/process bindings are corrupt",
        );
        if !units.contains(&source) {
            // Release-build fallback for the corrupt-bindings case the
            // debug_assert names: the caller invoked sys_process_exit
            // and must not run past it, even when the per-pid sweep
            // below cannot see it.
            self.lv2_host.log_invariant_break(
                "runtime.process_exit_child_unbound_caller",
                format_args!(
                    "exit of pid {pid:#x} from {source:?} which is not in the pid's \
                     unit set {units:?}; finishing the caller anyway",
                ),
            );
            self.registry
                .set_status_override(source, UnitStatus::Finished);
            self.timer_wakes.cancel(source);
            let _ = self.syscall_responses.try_take(source);
        }
        for uid in units {
            self.registry.set_status_override(uid, UnitStatus::Finished);
            self.timer_wakes.cancel(uid);
            let _ = self.syscall_responses.try_take(uid);
        }
        self.lv2_host.mark_process_exited(pid, code);
    }
}

#[cfg(test)]
#[path = "tests/process_spawn_tests.rs"]
mod tests;
