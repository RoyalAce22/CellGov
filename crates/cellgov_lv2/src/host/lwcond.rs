//! LV2 dispatch for lightweight condition variables.
//!
//! An lwcond is bound to one lwmutex. A wait releases that lwmutex in
//! the kernel and parks; a signal hands the woken waiter the lwmutex in
//! the way its `mode` names. The owner word and waiter count live in the
//! user-space `sys_lwmutex_t`, which liblv2.prx's wrappers keep.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::process::SYS_LWCOND_OBJECT;
use cellgov_ps3_abi::lv2::sync::{lwcond_signal_mode, LWCOND_SIGNAL_ANY_THREAD};
use cellgov_time::GuestTicks;

use crate::dispatch::{Lv2BlockReason, Lv2Dispatch, PendingResponse};
use crate::host::Lv2Host;
use crate::ppu_thread::PpuThreadId;
use crate::sync_primitives::LwMutexRelease;

impl Lv2Host {
    /// `_sys_lwcond_create` (111): bind a new lwcond to the lwmutex
    /// `lwmutex_id` and write its id to `*id_ptr`.
    ///
    /// # Errors
    ///
    /// Listed in the order they fire.
    ///
    /// - `CELL_ESRCH` when no lwmutex has the id.
    /// - `CELL_EFAULT` when `id_ptr` is null. The gate is CellGov's own.
    /// - `CELL_EAGAIN` when the process holds the most kernel ids it may.
    pub(super) fn dispatch_lwcond_create(
        &mut self,
        id_ptr: u32,
        lwmutex_id: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if self.state.lwmutexes.lookup(lwmutex_id).is_none() {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        if let Some(d) = self.efault_if_null(&[id_ptr]) {
            return d;
        }
        let Some(id) = self.alloc_id(requester, SYS_LWCOND_OBJECT) else {
            return Lv2Dispatch::immediate(errno::CELL_EAGAIN.into());
        };
        self.state.lwconds.create_with_id(id, lwmutex_id);
        self.state.process_counts.lwcond_inc();
        self.immediate_write_u32(id, id_ptr, requester, tick)
    }

    /// `_sys_lwcond_destroy` (112).
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` when no lwcond has the id.
    /// - `CELL_EBUSY` while a thread waits on the lwcond.
    pub(super) fn dispatch_lwcond_destroy(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.state.lwconds.lookup(id) else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        if !entry.waiters().is_empty() {
            return Lv2Dispatch::immediate(errno::CELL_EBUSY.into());
        }
        self.state.lwconds.destroy(id);
        self.release_id(id);
        self.state.process_counts.lwcond_dec();
        Lv2Dispatch::immediate(0)
    }

    /// `_sys_lwcond_queue_wait` (113): park the caller on the lwcond,
    /// then release the lwmutex as `sys_lwmutex_unlock` does, which wakes
    /// the head of its sleep queue or sets its signal.
    ///
    /// The parked caller later returns one of these:
    ///
    /// - 0 when a signal gives it the lwmutex.
    /// - `CELL_EBUSY` when the signaller could not take the lwmutex.
    /// - `CELL_ETIMEDOUT` when the timeout ends the wait.
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` when the caller is no PPU thread, or when no
    ///   lwcond or no lwmutex has the id.
    /// - `CELL_EDEADLK` when the caller already waits on the lwcond,
    ///   recorded as an invariant break.
    pub(super) fn dispatch_lwcond_queue_wait(
        &mut self,
        id: u32,
        lwmutex_id: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        let Some(caller) = self.state.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        if self.state.lwconds.lookup(id).is_none()
            || self.state.lwmutexes.lookup(lwmutex_id).is_none()
        {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        if !self.state.lwconds.enqueue_waiter(id, caller) {
            self.record_invariant_break(
                "lwcond_queue_wait.duplicate_waiter",
                format_args!("lwcond {id}: caller {caller:?} already on its queue"),
            );
            return Lv2Dispatch::immediate(errno::CELL_EDEADLK.into());
        }
        let reason = Lv2BlockReason::LwCond { id, lwmutex_id };
        // Every wake that gives the caller the lwmutex resolves this
        // response; a refused or timed-out wait overrides it.
        let pending = PendingResponse::LwMutexWake {
            mutex_ptr: 0,
            caller: caller.raw() as u32,
        };
        match self
            .state
            .lwmutexes
            .release_and_wake_next(lwmutex_id, caller)
        {
            LwMutexRelease::Transferred { new_owner } => {
                let woken_unit_ids = self
                    .resolve_wake_thread(new_owner, "lwcond_queue_wait.Transferred")
                    .into_iter()
                    .collect();
                Lv2Dispatch::BlockAndWake {
                    reason,
                    pending,
                    woken_unit_ids,
                    response_updates: vec![],
                    effects: vec![],
                }
            }
            LwMutexRelease::Signaled | LwMutexRelease::Unknown => Lv2Dispatch::Block {
                reason,
                pending,
                effects: vec![],
            },
        }
    }

    /// `_sys_lwcond_signal` (115): wake the waiter `ppu_thread_id` names,
    /// or the head of the waiter list for [`LWCOND_SIGNAL_ANY_THREAD`], and
    /// hand it the lwmutex the way [`lwcond_signal_mode`] `mode` names.
    ///
    /// With no waiter to wake, the call answers:
    ///
    /// - 0 for [`lwcond_signal_mode::NOT_OWNED`] with
    ///   [`LWCOND_SIGNAL_ANY_THREAD`].
    /// - `CELL_ENOENT` for [`lwcond_signal_mode::FORCED`] with
    ///   [`LWCOND_SIGNAL_ANY_THREAD`].
    /// - `CELL_EPERM` for [`lwcond_signal_mode::OWNED`] with
    ///   [`LWCOND_SIGNAL_ANY_THREAD`], or for a named thread that does not
    ///   wait.
    ///
    /// liblv2.prx's wrapper reads each of these answers as "nothing woken".
    ///
    /// # Errors
    ///
    /// Listed in the order they fire.
    ///
    /// - `CELL_EINVAL` for a `mode` outside the three. The oracle stops
    ///   on it instead; the answer is CellGov's own.
    /// - `CELL_ESRCH` in these cases:
    ///   - No lwcond has the id.
    ///   - `ppu_thread_id` names no thread.
    ///   - `mode` is not `NOT_OWNED` and no lwmutex has `lwmutex_id`.
    pub(super) fn dispatch_lwcond_signal(
        &mut self,
        id: u32,
        lwmutex_id: u32,
        ppu_thread_id: u32,
        mode: u32,
    ) -> Lv2Dispatch {
        if !(lwcond_signal_mode::OWNED..=lwcond_signal_mode::FORCED).contains(&mode) {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        if self.state.lwconds.lookup(id).is_none() {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        let target = (ppu_thread_id != LWCOND_SIGNAL_ANY_THREAD)
            .then(|| PpuThreadId::new(u64::from(ppu_thread_id)));
        if target.is_some_and(|t| self.state.ppu_threads.get(t).is_none()) {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        if mode != lwcond_signal_mode::NOT_OWNED
            && self.state.lwmutexes.lookup(lwmutex_id).is_none()
        {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        let woken = match target {
            Some(t) => self.state.lwconds.remove_waiter(id, t).then_some(t),
            None => self.state.lwconds.dequeue_one(id),
        };
        let Some(waiter) = woken else {
            let code = match (target, mode) {
                (None, lwcond_signal_mode::NOT_OWNED) => 0,
                (None, lwcond_signal_mode::FORCED) => errno::CELL_ENOENT.into(),
                _ => errno::CELL_EPERM.into(),
            };
            return Lv2Dispatch::immediate(code);
        };
        let Some(unit) = self.resolve_wake_thread(waiter, "lwcond_signal.waiter") else {
            return Lv2Dispatch::immediate(0);
        };
        match mode {
            lwcond_signal_mode::NOT_OWNED => coded_wake(vec![unit], errno::CELL_EBUSY.into(), 0),
            lwcond_signal_mode::OWNED => {
                if self.lwcond_move_to_lwmutex(waiter, lwmutex_id) {
                    Lv2Dispatch::immediate(0)
                } else {
                    coded_wake(vec![unit], errno::CELL_ESRCH.into(), 0)
                }
            }
            _ => self.lwcond_forced_hand_off(waiter, unit, lwmutex_id),
        }
    }

    /// `_sys_lwcond_signal_all` (116): wake every waiter the way
    /// [`lwcond_signal_mode`] `mode` names.
    ///
    /// For [`lwcond_signal_mode::OWNED`] the call returns the number of
    /// waiters it moved onto the lwmutex's sleep queue.
    ///
    /// # Errors
    ///
    /// Listed in the order they fire.
    ///
    /// - `CELL_EINVAL` for a `mode` other than `OWNED` or `NOT_OWNED`.
    ///   The oracle stops on it instead; the answer is CellGov's own.
    /// - `CELL_ESRCH` in these cases:
    ///   - No lwcond has the id.
    ///   - `mode` is `OWNED` and no lwmutex has `lwmutex_id`.
    pub(super) fn dispatch_lwcond_signal_all(
        &mut self,
        id: u32,
        lwmutex_id: u32,
        mode: u32,
    ) -> Lv2Dispatch {
        if mode != lwcond_signal_mode::OWNED && mode != lwcond_signal_mode::NOT_OWNED {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        if self.state.lwconds.lookup(id).is_none()
            || (mode == lwcond_signal_mode::OWNED
                && self.state.lwmutexes.lookup(lwmutex_id).is_none())
        {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        let waiters = self.state.lwconds.drain_all(id).unwrap_or_default();
        let waiters = waiters.into_iter().map(|waiter| (waiter, waiter));
        let units = self.collect_wakes("lwcond_signal_all.waiter", waiters);
        if mode == lwcond_signal_mode::NOT_OWNED {
            let units = units.into_iter().map(|(unit, _)| unit).collect();
            return coded_wake(units, errno::CELL_EBUSY.into(), 0);
        }
        let moved = units.len() as u64;
        let stranded: Vec<UnitId> = units
            .into_iter()
            .filter(|&(_, waiter)| !self.lwcond_move_to_lwmutex(waiter, lwmutex_id))
            .map(|(unit, _)| unit)
            .collect();
        if stranded.is_empty() {
            return Lv2Dispatch::immediate(moved);
        }
        coded_wake(stranded, errno::CELL_ESRCH.into(), moved)
    }

    /// Park `waiter` on the lwmutex's sleep queue: the signaller owns
    /// the lwmutex, and its unlock wakes the waiter as the new owner.
    ///
    /// When the enqueue fails, this records an invariant break and
    /// returns `false`.
    fn lwcond_move_to_lwmutex(&mut self, waiter: PpuThreadId, lwmutex_id: u32) -> bool {
        let Err(err) = self.state.lwmutexes.enqueue_waiter(lwmutex_id, waiter) else {
            return true;
        };
        self.record_invariant_break(
            "lwcond_signal.lwmutex_enqueue",
            format_args!(
                "enqueue on lwmutex {lwmutex_id} failed for {waiter:?}: {err:?}; \
                 waking with ESRCH to avoid stranding"
            ),
        );
        false
    }

    /// Hand off the lwmutex the signaller took for a forced signal.
    ///
    /// - With a sleeper, the lwmutex passes to its oldest sleeper and the
    ///   waiter queues behind it.
    /// - With no sleeper, the lwmutex passes to the waiter.
    fn lwcond_forced_hand_off(
        &mut self,
        waiter: PpuThreadId,
        unit: UnitId,
        lwmutex_id: u32,
    ) -> Lv2Dispatch {
        let has_sleepers = self
            .state
            .lwmutexes
            .lookup(lwmutex_id)
            .is_some_and(|m| !m.waiters().is_empty());
        if !has_sleepers {
            return Lv2Dispatch::WakeAndReturn {
                code: 0,
                woken_unit_ids: vec![unit],
                response_updates: vec![],
                effects: vec![],
            };
        }
        if !self.lwcond_move_to_lwmutex(waiter, lwmutex_id) {
            return coded_wake(vec![unit], errno::CELL_ESRCH.into(), 0);
        }
        let woken_unit_ids = match self
            .state
            .lwmutexes
            .release_and_wake_next(lwmutex_id, waiter)
        {
            LwMutexRelease::Transferred { new_owner } => self
                .resolve_wake_thread(new_owner, "lwcond_signal.forced_hand_off")
                .into_iter()
                .collect(),
            LwMutexRelease::Signaled | LwMutexRelease::Unknown => vec![],
        };
        Lv2Dispatch::WakeAndReturn {
            code: 0,
            woken_unit_ids,
            response_updates: vec![],
            effects: vec![],
        }
    }
}

/// Wake each of `units` with the return code `waiter_code`; the
/// signaller returns `code`.
///
/// `CELL_EBUSY` tells a waiter that the signaller could not take the
/// lwmutex, so the waiter locks it itself.
fn coded_wake(units: Vec<UnitId>, waiter_code: u64, code: u64) -> Lv2Dispatch {
    let response_updates = units
        .iter()
        .map(|&unit| (unit, PendingResponse::ReturnCode { code: waiter_code }))
        .collect();
    Lv2Dispatch::WakeAndReturn {
        code,
        woken_unit_ids: units,
        response_updates,
        effects: vec![],
    }
}

#[cfg(test)]
#[path = "tests/lwcond_tests.rs"]
mod tests;
