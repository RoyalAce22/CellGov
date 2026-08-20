//! PPU thread spawning: child-thread creation.

use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
use cellgov_lv2::{Lv2Dispatch, PpuThreadAttrs, PpuThreadInitState};
use cellgov_ps3_abi::cell_errors::{CELL_E2BIG, CELL_ENOMEM};

use super::spaces::AddressSpaceId;
use super::Runtime;

impl Runtime {
    pub(super) fn handle_ppu_thread_create(&mut self, source: UnitId, dispatch: Lv2Dispatch) {
        let caller_space = self.spaces.space_of(source);
        let (id_ptr, init, stack_base, stack_size, priority) = match dispatch {
            Lv2Dispatch::PpuThreadCreate {
                id_ptr,
                init,
                stack_base,
                stack_size,
                priority,
                effects,
            } => {
                self.apply_lv2_effects(&effects, caller_space);
                (id_ptr, init, stack_base, stack_size, priority)
            }
            other => unreachable!("handle_ppu_thread_create called with {other:?}"),
        };

        let Some(factory) = self.ppu_factory.as_ref() else {
            // Foundation-title baselines pin CELL_E2BIG on this path.
            self.lv2_host.log_invariant_break(
                "runtime.ppu_thread_create_factory_missing",
                format_args!(
                    "sys_ppu_thread_create dispatched without a PPU factory; \
                     install one via set_ppu_factory. No thread created",
                ),
            );
            self.registry.set_syscall_return(source, CELL_E2BIG.into());
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
        let Some(thread_id) = self.lv2_host.ppu_threads_mut().create(child_unit_id, attrs) else {
            // The registered unit cannot be removed, so it must not
            // stay schedulable behind the ENOMEM the guest sees. It
            // stays Finished (inert) in the boot space; its space tag
            // and pid binding are only assigned below, after this
            // point, so a failed create leaves neither behind
            // (ProcessTable contract: units_of names exactly the
            // pid's live units).
            self.registry
                .set_status_override(child_unit_id, UnitStatus::Finished);
            self.lv2_host.log_invariant_break(
                "runtime.ppu_thread_create_thread_ids_exhausted",
                format_args!(
                    "sys_ppu_thread_create: PPU thread-id allocator exhausted; \
                     child unit {child_unit_id:?} left registered as Finished",
                ),
            );
            self.registry.set_syscall_return(source, CELL_ENOMEM.into());
            return;
        };

        // A thread belongs to its creator's process: same address
        // space, same pid binding (ProcessTable contract: the runtime
        // binds every unit it registers for a spawned child). Boot
        // callers keep the untagged default.
        if caller_space != AddressSpaceId::BOOT {
            self.assign_unit_space(child_unit_id, caller_space)
                .expect("caller's space exists while the caller runs in it");
        }
        let caller_pid = self.lv2_host.process_of_unit(source);
        if caller_pid != cellgov_ps3_abi::sys_process::BOOT_PROCESS_PID {
            self.lv2_host.bind_unit_process(child_unit_id, caller_pid);
        }

        // The tid writeback lands in the CREATOR's space, like every
        // other syscall out-parameter.
        self.commit_bytes_at(
            caller_space,
            u64::from(id_ptr),
            &thread_id.raw().to_be_bytes(),
        );
        self.registry.set_syscall_return(source, 0);
    }
}
