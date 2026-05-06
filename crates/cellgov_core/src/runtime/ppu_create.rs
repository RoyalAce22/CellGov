//! PPU thread creation: construct the child unit via the installed PPU
//! factory, commit its initial TLS image, register it in the LV2 PPU
//! thread table, and write the minted thread id into the caller's out
//! pointer. The host has already resolved the OPD.

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
        // The host rejects `tls_bytes && tls_base == 0` with CELL_EINVAL
        // in `dispatch_ppu_thread_create`; reaching here is a host
        // regression. Unconditional assert so release builds fail loudly
        // rather than committing the TLS image to guest address 0.
        assert!(
            tls_bytes.is_empty() || init.tls_base != 0,
            "PpuThreadCreate: non-empty tls_bytes requires non-zero tls_base \
             (host-side guard in dispatch_ppu_thread_create bypassed -- host regression)",
        );

        // cellgov_core does not depend on cellgov_ppu; without a
        // factory we cannot construct a concrete PpuExecutionUnit.
        // CELL_E2BIG is the shipped error value.
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
    /// Called after every [`Self::dispatch_hle`] call (whether or not
    /// any handler claimed the NID); a no-op when no handler recorded
    /// a park-for-callback intent.
    ///
    /// On success the parent unit transitions to `Blocked` and a fresh
    /// worker PPU is registered (see [`Self::handle_callback_spawn`]).
    /// On failure the parent is left `Runnable` with a kernel error
    /// code in r3:
    /// - [`CallbackError::TooDeep`] -> [`CELL_EAGAIN`] (recursion cap
    ///   would be exceeded).
    /// - [`CallbackError::OpdReadFailed`] -> [`CELL_EFAULT`] (title
    ///   handed us an unmapped OPD pointer).
    /// - [`CallbackError::StackAllocFailed`] -> [`CELL_ENOMEM`] (the
    ///   worker stack arena is exhausted).
    pub(crate) fn consume_pending_callback_spawn(&mut self, source: UnitId) {
        let Some(park) = self.hle.pending_callback_spawn.take() else {
            return;
        };
        // Split borrow: lv2_host is mutable, memory is immutable.
        // The view's borrow ends at the end of this statement so the
        // subsequent `apply_callback_spawn` (which needs `&mut self`)
        // composes without an explicit drop.
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

    /// Public entry point for the callback-dispatch spawn handler.
    ///
    /// Mirrors [`Self::handle_callback_spawn`] for callers that
    /// construct a [`Lv2Dispatch::CallbackSpawn`] outside the
    /// runtime's LV2 dispatch path (integration tests that
    /// synthesize spawns directly, and HLE handlers that build a
    /// spawn via [`cellgov_lv2::Lv2Host::call_guest_callback_sync`]
    /// and inject it via this entry).
    pub fn apply_callback_spawn(&mut self, dispatch: Lv2Dispatch) {
        self.handle_callback_spawn(dispatch);
    }

    /// Spawn a callback worker AND park the parent on the worker's
    /// trampoline return. Mirrors `handle_ppu_thread_create` but
    /// (a) does not write a thread id to a guest pointer (callbacks
    /// are detached -- no caller-visible id), (b) extracts the
    /// `CallbackReturnStage` from `parent_pending` to feed
    /// `Lv2Host::attach_callback_worker`, (c) parks `parent` in
    /// `Blocked` with `parent_pending` stored in
    /// `syscall_responses`.
    ///
    /// # Invariants
    /// - `parent_pending` is a [`PendingResponse::CallbackReturn`].
    /// - The PPU factory is installed; without it, callback dispatch
    ///   cannot run and the parent wakes immediately with CELL_E2BIG.
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

        // Recover the stage from the parent's pending response. The
        // host built `parent_pending` in `call_guest_callback_sync`,
        // so this match is total in current callers; debug-asserted
        // for forward-compat.
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

        // No TLS image for callback workers -- they execute the
        // title's existing code, which has whatever TLS bindings
        // the parent already established.

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

        // Park the parent. Mirrors `handle_block` shape: insert
        // pending response, flip status to Blocked. No r3 write
        // here -- the trampoline-return wake will set it.
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
