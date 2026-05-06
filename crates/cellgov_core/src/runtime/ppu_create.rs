//! PPU thread spawning: child-thread creation and callback-worker
//! spawning. Owns the runtime side of the host -> runtime cooperation
//! that materializes a worker after an HLE handler parks for a guest
//! callback.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
use cellgov_lv2::{
    host::CallbackError, Lv2Dispatch, PendingResponse, PpuThreadAttrs, PpuThreadInitState,
};
use cellgov_ps3_abi::cell_errors::{CELL_E2BIG, CELL_EAGAIN, CELL_EFAULT, CELL_ENOMEM};

use super::trace_bridge::MemoryView;
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

    #[cfg(test)]
    pub(crate) fn handle_callback_spawn_for_test(&mut self, dispatch: Lv2Dispatch) {
        self.handle_callback_spawn(dispatch);
    }

    /// Drain [`crate::hle::HleState::pending_callback_spawn`] and
    /// materialize the worker.
    ///
    /// # Cross-module contract
    /// Caller (`dispatch_hle`) MUST invoke this after every HLE
    /// dispatch, including unclaimed NIDs; otherwise a recorded
    /// park-for-callback intent leaks across syscalls and the parent
    /// stays `Runnable` while the host believes it is parked.
    ///
    /// # Errors
    /// On host failure the parent stays `Runnable` with r3 set to:
    /// - [`CallbackError::TooDeep`] -> [`CELL_EAGAIN`]
    /// - [`CallbackError::OpdReadFailed`] -> [`CELL_EFAULT`]
    /// - [`CallbackError::StackAllocFailed`] -> [`CELL_ENOMEM`]
    pub(crate) fn consume_pending_callback_spawn(&mut self, source: UnitId) {
        let Some(park) = self.hle.pending_callback_spawn.take() else {
            return;
        };
        let result = self.lv2_host.call_guest_callback_sync(
            source,
            park.opd_addr,
            park.args,
            park.stage,
            &MemoryView {
                memory: &self.memory,
                current_tick: self.time,
            },
        );
        match result {
            Ok(dispatch) => self.apply_callback_spawn(dispatch),
            Err(CallbackError::TooDeep) => {
                self.registry.set_syscall_return(source, CELL_EAGAIN.into());
            }
            Err(CallbackError::OpdReadFailed) => {
                self.registry.set_syscall_return(source, CELL_EFAULT.into());
            }
            Err(CallbackError::StackAllocFailed) => {
                self.registry.set_syscall_return(source, CELL_ENOMEM.into());
            }
        }
    }

    /// Public entry to [`Self::handle_callback_spawn`] for callers
    /// outside the runtime's LV2 dispatch path.
    pub fn apply_callback_spawn(&mut self, dispatch: Lv2Dispatch) {
        self.handle_callback_spawn(dispatch);
    }

    /// Spawn a detached callback worker and park the parent on the
    /// worker's trampoline return.
    ///
    /// # Invariants
    /// - `parent_pending` is a [`PendingResponse::CallbackReturn`];
    ///   any other variant wakes the parent with CELL_E2BIG.
    /// - The parent must not already have an entry in
    ///   `syscall_responses`; the parked response is inserted here.
    pub(super) fn handle_callback_spawn(&mut self, dispatch: Lv2Dispatch) {
        let (worker_init, parent, parent_pending, effects): (
            PpuThreadInitState,
            UnitId,
            PendingResponse,
            Vec<Effect>,
        ) = match dispatch {
            Lv2Dispatch::CallbackSpawn {
                worker_init,
                parent,
                parent_pending,
                effects,
                ..
            } => (worker_init, parent, parent_pending, effects),
            other => unreachable!("handle_callback_spawn called with {other:?}"),
        };
        self.apply_lv2_effects(&effects);

        let stage = match parent_pending {
            PendingResponse::CallbackReturn { stage, .. } => stage,
            ref other => {
                debug_assert!(
                    false,
                    "handle_callback_spawn: parent_pending is not CallbackReturn: {other:?}",
                );
                self.registry.set_syscall_return(parent, CELL_E2BIG.into());
                return;
            }
        };

        let Some(factory) = self.ppu_factory.as_ref() else {
            self.registry.set_syscall_return(parent, CELL_E2BIG.into());
            return;
        };
        let seed = worker_init.clone();
        let worker_unit_id = self
            .registry
            .register_dynamic(&|id| factory(id, seed.clone()));

        // Workers inherit the parent's TLS bindings; no fresh image.
        let attrs = PpuThreadAttrs {
            entry: worker_init.entry_code,
            arg: worker_init.arg,
            stack_base: 0,
            stack_size: 0x4000,
            priority: 0,
            tls_base: 0,
        };
        let Some(worker_thread_id) = self
            .lv2_host
            .ppu_threads_mut()
            .create(worker_unit_id, attrs)
        else {
            self.registry.set_syscall_return(parent, CELL_ENOMEM.into());
            return;
        };

        self.lv2_host
            .attach_callback_worker(worker_thread_id, parent, stage);

        // r3 is set by the trampoline-return wake, not here.
        let displaced = self.syscall_responses.insert(parent, parent_pending);
        debug_assert!(
            displaced.is_none(),
            "handle_callback_spawn: parent {parent:?} already had a pending response: \
             {displaced:?}",
        );
        self.registry
            .set_status_override(parent, UnitStatus::Blocked);
    }
}
