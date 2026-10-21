//! `sys_ppu_thread_create`: child PPU thread registration, and
//! `sys_ppu_thread_start`: its release.

use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
use cellgov_lv2::{Lv2Dispatch, PpuThreadAttrs, PpuThreadInitState};
use cellgov_ps3_abi::lv2::errno::{CELL_E2BIG, CELL_ENOMEM};
use cellgov_trace::HostWriter;

use super::spaces::AddressSpaceId;
use super::Runtime;

/// Label of the region a child thread's stack installs as in a spawned
/// process's address space.
const CHILD_STACK_LABEL: &str = "child_stack";

impl Runtime {
    pub(super) fn handle_ppu_thread_create(&mut self, source: UnitId, dispatch: Lv2Dispatch) {
        let caller_space = self.spaces.space_of(source);
        let (id_ptr, init, stack_base, stack_size, priority, joinable) = match dispatch {
            Lv2Dispatch::PpuThreadCreate {
                id_ptr,
                init,
                stack_base,
                stack_size,
                priority,
                joinable,
                effects,
            } => {
                self.apply_lv2_effects(&effects, caller_space);
                (id_ptr, init, stack_base, stack_size, priority, joinable)
            }
            other => unreachable!("handle_ppu_thread_create called with {other:?}"),
        };

        let Some(factory) = self.ppu_factory.as_ref() else {
            // Foundation-title baselines pin CELL_E2BIG on this path. No
            // thread holds the stack block, so it is free again.
            self.lv2_host.log_invariant_break(
                "runtime.ppu_thread_create_factory_missing",
                format_args!(
                    "sys_ppu_thread_create dispatched without a PPU factory; \
                     install one via set_ppu_factory. No thread created",
                ),
            );
            self.deliver_syscall_return(source, CELL_E2BIG.into());
            return;
        };
        let seed: PpuThreadInitState = init.clone();
        let child_unit_id = self
            .registry
            .register_dynamic(&|id| factory(id, seed.clone()));

        let attrs = PpuThreadAttrs {
            entry: init.entry_code,
            arg: init.arg,
            stack_base: stack_base as u32,
            stack_size: stack_size as u32,
            priority,
            tls_base: init.tls_base as u32,
        };
        let Some(thread_id) =
            self.lv2_host
                .ppu_threads_mut()
                .create_unstarted(child_unit_id, attrs, joinable)
        else {
            // The registered unit cannot be removed, so it must not
            // stay schedulable behind the ENOMEM the guest sees. It
            // stays Finished (inert) in the boot space; its space tag
            // and pid binding are only assigned below, after this
            // point, so a failed create leaves neither behind
            // (ProcessTable contract: units_of names exactly the
            // pid's live units). No thread holds the stack block.
            self.registry
                .set_status_override(child_unit_id, UnitStatus::Finished);
            self.lv2_host.log_invariant_break(
                "runtime.ppu_thread_create_thread_ids_exhausted",
                format_args!(
                    "sys_ppu_thread_create: PPU thread-id allocator exhausted; \
                     child unit {child_unit_id:?} left registered as Finished",
                ),
            );
            self.deliver_syscall_return(source, CELL_ENOMEM.into());
            return;
        };

        // A thread belongs to its creator's process: same address
        // space, same pid binding (ProcessTable contract: the runtime
        // binds every unit it registers for a spawned child). Boot
        // callers keep the untagged default.
        if caller_space != AddressSpaceId::BOOT {
            // The arena hands out addresses inside the boot layout's
            // child-stacks window; a child space has no region there
            // until this install, so without it the new thread's
            // first stack store faults. Boot callers skip it: the
            // boot pipeline pre-installs the whole window.
            let mem = super::spaces::resolve_space_memory_for_write(
                &mut self.memory,
                &mut self.spaces,
                caller_space,
            );
            // A collected thread's stack region stays in the space until
            // its range is placed again. No live thread holds any part
            // of the new block, so every stack region it overlaps is a
            // collected thread's.
            let stale: Vec<(u64, usize)> = mem
                .regions()
                .filter(|r| {
                    r.label() == CHILD_STACK_LABEL
                        && r.base() < stack_base + stack_size
                        && stack_base < r.base() + r.size()
                })
                .map(|r| (r.base(), r.size() as usize))
                .collect();
            for (base, size) in stale {
                if let Err(err) = mem.remove_region(base, size) {
                    self.lv2_host.log_invariant_break(
                        "runtime.ppu_thread_create_stale_stack_remove",
                        format_args!(
                            "collected stack region 0x{base:x}+0x{size:x} in space {} \
                             would not remove: {err}",
                            caller_space.raw(),
                        ),
                    );
                }
            }
            if let Err(err) = mem.install_region(
                stack_base,
                stack_size as usize,
                CHILD_STACK_LABEL,
                cellgov_mem::PageSize::Page4K,
            ) {
                // Guest-reachable only when the child's image layout
                // occupies the child-stacks window; the kernel would
                // have failed the stack allocation itself. PPU thread
                // creation reports CELL_ENOMEM when it cannot get the
                // user stack area, which is the case here.
                self.registry
                    .set_status_override(child_unit_id, UnitStatus::Finished);
                let stranded = self.lv2_host.ppu_threads_mut().mark_finished(thread_id, 0);
                debug_assert!(
                    stranded.is_empty(),
                    "a thread refused inside its own create cannot have joiners yet; \
                     {stranded:?} would never wake",
                );
                // The guest never gets the id, so no join collects the
                // thread: drop it here, which frees its stack block.
                self.lv2_host.ppu_threads_mut().reap(thread_id);
                self.lv2_host.log_invariant_break(
                    "runtime.ppu_thread_create_stack_install_overlap",
                    format_args!(
                        "child stack region 0x{stack_base:x}+0x{stack_size:x} overlaps \
                         space {}'s existing layout: {err}; thread refused with ENOMEM",
                        caller_space.raw(),
                    ),
                );
                self.deliver_syscall_return(source, CELL_ENOMEM.into());
                return;
            }
            self.assign_unit_space(child_unit_id, caller_space)
                .expect("caller's space exists while the caller runs in it");
        }
        let caller_pid = self.lv2_host.process_of_unit(source);
        if caller_pid != cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID {
            self.lv2_host.bind_unit_process(child_unit_id, caller_pid);
        }
        // The thread runs once `sys_ppu_thread_start` names it.
        self.registry.set_suspended(child_unit_id, true);

        // The tid writeback lands in the CREATOR's space, like every
        // other syscall out-parameter.
        self.commit_bytes_at(
            HostWriter::SyscallOutParam,
            source,
            u64::from(id_ptr),
            &thread_id.raw().to_be_bytes(),
        );
        self.deliver_syscall_return(source, 0);
    }

    /// Release the unit of a thread the host just marked started.
    pub(super) fn handle_ppu_thread_start(&mut self, source: UnitId, unit: UnitId) {
        self.registry.set_suspended(unit, false);
        self.deliver_syscall_return(source, 0);
    }
}
