//! PPU thread spawning: child-thread creation.

use cellgov_event::UnitId;
use cellgov_lv2::{Lv2Dispatch, PpuThreadAttrs, PpuThreadInitState};
use cellgov_ps3_abi::cell_errors::{CELL_E2BIG, CELL_ENOMEM};

use super::Runtime;

impl Runtime {
    pub(super) fn handle_ppu_thread_create(&mut self, source: UnitId, dispatch: Lv2Dispatch) {
        let (id_ptr, init, stack_base, stack_size, tls_bytes, priority) = match dispatch {
            Lv2Dispatch::PpuThreadCreate {
                id_ptr,
                init,
                stack_base,
                stack_size,
                tls_bytes,
                priority,
                effects,
            } => {
                self.apply_lv2_effects(&effects);
                (id_ptr, init, stack_base, stack_size, tls_bytes, priority)
            }
            other => unreachable!("handle_ppu_thread_create called with {other:?}"),
        };
        // Guards against committing a TLS image to guest address 0 if
        // the host-side CELL_EINVAL filter regresses.
        assert!(
            tls_bytes.is_empty() || init.tls_base != 0,
            "PpuThreadCreate: non-empty tls_bytes requires non-zero tls_base",
        );

        let Some(factory) = self.ppu_factory.as_ref() else {
            self.registry.set_syscall_return(source, CELL_E2BIG.into());
            return;
        };
        let seed: PpuThreadInitState = init.clone();
        let child_unit_id = self
            .registry
            .register_dynamic(&|id| factory(id, seed.clone()));

        if !tls_bytes.is_empty() {
            self.commit_bytes_at(init.tls_base, &tls_bytes);
        }

        let attrs = PpuThreadAttrs {
            entry: init.entry_code,
            arg: init.arg,
            stack_base: stack_base as u32,
            stack_size: stack_size as u32,
            priority,
            tls_base: init.tls_base as u32,
        };
        let Some(thread_id) = self.lv2_host.ppu_threads_mut().create(child_unit_id, attrs) else {
            self.registry.set_syscall_return(source, CELL_ENOMEM.into());
            return;
        };

        self.commit_bytes_at(id_ptr as u64, &thread_id.raw().to_be_bytes());
        self.registry.set_syscall_return(source, 0);
    }

    #[cfg(test)]
    pub(crate) fn handle_ppu_thread_create_for_test(
        &mut self,
        source: UnitId,
        dispatch: Lv2Dispatch,
    ) {
        self.handle_ppu_thread_create(source, dispatch);
    }
}
