//! PPU thread creation handler extracted from `runtime.rs`.
//!
//! The host resolved the OPD already; this handler walks:
//!
//! 1. Construct the child execution unit via the installed PPU
//!    factory and commit its initial TLS image.
//! 2. Register the child in the LV2 PPU thread table.
//! 3. Write the minted thread id into the caller's out pointer and
//!    return CELL_OK.

use cellgov_event::UnitId;
use cellgov_lv2::errno::{CELL_E2BIG, CELL_ENOMEM};
use cellgov_lv2::{Lv2Dispatch, PpuThreadAttrs, PpuThreadInitState};

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
        // Defense in depth: the host rejects this combination with
        // CELL_EINVAL in `dispatch_ppu_thread_create`, so a
        // malformed dispatch reaching here is a host regression,
        // not a guest-triggerable error. Unconditional assert
        // (not `debug_assert`) so a release build fails loudly
        // rather than committing the TLS image to guest address 0.
        assert!(
            tls_bytes.is_empty() || init.tls_base != 0,
            "PpuThreadCreate: non-empty tls_bytes requires non-zero tls_base \
             (host-side guard in dispatch_ppu_thread_create bypassed -- host regression)",
        );

        // Register the child unit via the PPU factory. Without a
        // factory we cannot construct a concrete PpuExecutionUnit
        // here (cellgov_core does not depend on cellgov_ppu), so
        // thread creation fails with CELL_E2BIG (the shipped
        // value). Preserved verbatim -- changing it would shift
        // foundation-title baselines.
        let Some(factory) = self.ppu_factory.as_ref() else {
            self.registry.set_syscall_return(source, CELL_E2BIG.into());
            return;
        };
        let seed: PpuThreadInitState = init.clone();
        let child_unit_id = self
            .registry
            .register_dynamic(&|id| factory(id, seed.clone()));

        // Commit TLS bytes into guest memory at tls_base.
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
