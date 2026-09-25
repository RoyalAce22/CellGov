//! Process binding, exit, and the waiter purge an exit runs.

use super::model::Lv2Host;

impl Lv2Host {
    /// Bind `unit` to a spawned process; called by the runtime after
    /// registering a child's primary-thread unit.
    ///
    /// A pid outside the table or a rebind to a different pid is a
    /// runtime sequencing bug (spawn rollback racing registration);
    /// both are logged, and the binding still lands so the caller's
    /// view stays consistent with what it asked for.
    pub fn bind_unit_process(&mut self, unit: cellgov_event::UnitId, pid: u32) {
        if self.state.processes.get(pid).is_none() {
            self.log_invariant_break(
                "process.bind_to_unknown_pid",
                format_args!(
                    "bind_unit_process({unit:?}, {pid:#x}): pid not in the \
                     process table; binding recorded anyway"
                ),
            );
        }
        let prior = self
            .state
            .processes
            .unit_bindings()
            .find(|(u, _)| *u == unit)
            .map(|(_, p)| *p);
        if let Some(prev) = prior {
            if prev != pid {
                self.log_invariant_break(
                    "process.unit_rebound",
                    format_args!(
                        "bind_unit_process({unit:?}, {pid:#x}): unit already \
                         bound to {prev:#x}; rebinding"
                    ),
                );
            }
        }
        self.state.processes.bind_unit(unit, pid);
    }

    /// The pid `unit` belongs to (boot pid when unbound).
    pub fn process_of_unit(&self, unit: cellgov_event::UnitId) -> u32 {
        self.state.processes.process_of_unit(unit)
    }

    /// Units bound to `pid`, in unit-id order.
    pub fn units_of_process(&self, pid: u32) -> Vec<cellgov_event::UnitId> {
        self.state.processes.units_of(pid)
    }

    /// Remove every waiter record owned by `pid`'s threads from every
    /// LV2 waiter list: the six sync-primitive tables, per-thread
    /// join-waiter lists, the virtual UART's reader queue, and the USB
    /// driver's event readers.
    ///
    /// The runtime finishes every one of the pid's units at the same
    /// exit, so a grant handed to a parked thread of an exited process
    /// is a resource no thread will ever consume or release.
    ///
    /// Mutex ownership held by a dead thread is retained and witnessed
    /// in `process_exit_retained_mutex_owners`; reclaiming it needs
    /// creator attribution the shared object namespace does not
    /// record.
    fn purge_exited_process_waiters(&mut self, pid: u32) {
        let threads: std::collections::BTreeSet<crate::ppu_thread::PpuThreadId> = self
            .state
            .processes
            .units_of(pid)
            .into_iter()
            .filter_map(|unit| self.state.ppu_threads.thread_id_for_unit(unit))
            .collect();
        if threads.is_empty() {
            return;
        }
        let purges: [(&'static str, usize); 9] = [
            ("mutex", self.state.mutexes.purge_waiters_of(&threads).len()),
            (
                "lwmutex",
                self.state.lwmutexes.purge_waiters_of(&threads).len(),
            ),
            ("cond", self.state.conds.purge_waiters_of(&threads).len()),
            (
                "semaphore",
                self.state.semaphores.purge_waiters_of(&threads).len(),
            ),
            (
                "equeue",
                self.state.event_queues.purge_waiters_of(&threads).len(),
            ),
            (
                "eflag",
                self.state.event_flags.purge_waiters_of(&threads).len(),
            ),
            (
                "join",
                self.state.ppu_threads.purge_join_waiters_of(&threads).len(),
            ),
            ("uart", self.state.uart.purge_readers_of(&threads).len()),
            ("usbd", self.state.usbd.purge_waiters_of(&threads).len()),
        ];
        for (primitive, count) in purges {
            if count > 0 {
                *self
                    .obs
                    .process_exit_waiter_purges
                    .entry(primitive)
                    .or_insert(0) += count as u64;
            }
        }
        let orphaned = self.state.mutexes.ids_owned_by(&threads);
        if !orphaned.is_empty() {
            self.obs.process_exit_retained_mutex_owners += orphaned.len() as u64;
            // Guest-reachable (a process may exit holding a mutex), so
            // log-only: the witness names the ids without asserting.
            self.log_invariant_break(
                "process.exit_retained_mutex_owner",
                format_args!(
                    "pid {pid:#x} exited owning mutex ids {orphaned:?}; ownership retained \
                     until exit-time collection lands, survivors that lock these park \
                     forever"
                ),
            );
        }
    }

    /// Record `pid`'s exit; the entry is retained so later
    /// `sys_process_get_status` polls resolve deterministically.
    ///
    /// A process exits once; a second call for the same pid keeps the
    /// first status (overwriting it would retroactively change what
    /// `sys_process_get_status` and the sync-state hash already served)
    /// and is logged as a runtime sequencing bug.
    ///
    /// A fresh exit also purges the pid's threads from every LV2
    /// waiter list; see `purge_exited_process_waiters`.
    pub fn mark_process_exited(&mut self, pid: u32, status: i32) {
        match self.state.processes.get(pid).map(|entry| entry.exit_status) {
            None => self.log_invariant_break(
                "process.exit_of_unknown_pid",
                format_args!(
                    "mark_process_exited({pid:#x}, {status}): pid not in the \
                     process table; exit status dropped"
                ),
            ),
            Some(Some(prev)) => self.log_invariant_break(
                "process.double_exit",
                format_args!(
                    "mark_process_exited({pid:#x}, {status}): exit status \
                     {prev} already recorded; first status kept"
                ),
            ),
            Some(None) => {
                self.state
                    .processes
                    .get_mut(pid)
                    .expect("entry present just above")
                    .exit_status = Some(status);
                self.purge_exited_process_waiters(pid);
            }
        }
    }

    /// Spawn-failure rollback: drop the child entry minted by
    /// `dispatch_process_spawn` when the runtime's image load fails.
    pub fn unbind_spawned_process(&mut self, pid: u32) {
        if self.state.processes.remove_child(pid).is_none() {
            self.log_invariant_break(
                "process.rollback_of_unknown_pid",
                format_args!(
                    "unbind_spawned_process({pid:#x}): no child entry to \
                     remove (boot pid or never minted)"
                ),
            );
        }
    }

    /// `Some(status)` once `pid` has exited; `None` while alive or
    /// unknown.
    pub fn process_exit_status(&self, pid: u32) -> Option<i32> {
        self.state
            .processes
            .get(pid)
            .and_then(|entry| entry.exit_status)
    }
}
