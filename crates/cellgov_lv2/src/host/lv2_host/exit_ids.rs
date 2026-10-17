//! The kernel ids an exited process held, and the objects behind them.

use std::collections::BTreeSet;

use cellgov_ps3_abi::lv2::kernel_id::{class_of, SHARED_MEMORY_CLASS, SHARED_MEMORY_EXT_CLASS};
use cellgov_ps3_abi::lv2::process::{
    SYS_COND_OBJECT, SYS_EVENT_FLAG_OBJECT, SYS_EVENT_PORT_OBJECT, SYS_EVENT_QUEUE_OBJECT,
    SYS_LWCOND_OBJECT, SYS_LWMUTEX_OBJECT, SYS_MUTEX_OBJECT, SYS_RWLOCK_OBJECT,
    SYS_SEMAPHORE_OBJECT, SYS_TIMER_OBJECT,
};

use crate::ppu_thread::PpuThreadId;

use super::model::Lv2Host;

impl Lv2Host {
    /// Release the kernel ids process `pid` held, which exited, and
    /// drop the object behind each. The console keeps one id table per
    /// process, so no id of an exited process stays live.
    ///
    /// An object another process can still reach keeps its id and
    /// stays: one registered under an IPC key, a shared-memory segment
    /// a window still holds busy, and an object a thread outside `pid`
    /// waits on or holds. Config and usbd handles stay too, since
    /// closing them notifies other processes. `process_exit_retained_ids`
    /// counts each kept id. What the console does with such
    /// an object is unestablished: a game process cannot run beside a
    /// second process of its own.
    pub(in crate::host) fn release_process_ids(&mut self, pid: u32) {
        let threads = self.threads_of_process(pid);
        for id in self.state.kernel_ids.held_by_owner(pid) {
            if self.drop_exited_object(id, &threads) {
                self.release_id(id);
            } else {
                self.obs.process_exit_retained_ids += 1;
            }
        }
    }

    /// Drop the object `id` names, which an exited process whose
    /// threads are `threads` created; `false` when it stays.
    ///
    /// Every create that mints one of these classes records its object,
    /// so a live id that names no object is an invariant break. The id
    /// is still released.
    fn drop_exited_object(&mut self, id: u32, threads: &BTreeSet<PpuThreadId>) -> bool {
        let state = &mut self.state;
        let derived = &mut self.derived;
        let found = match class_of(id) {
            SYS_MUTEX_OBJECT => {
                let held_outside = state.mutexes.lookup(id).is_some_and(|m| {
                    !m.waiters().is_empty() || m.owner().is_some_and(|o| !threads.contains(&o))
                });
                if held_outside {
                    return false;
                }
                state.mutexes.destroy_abandoned(id).is_some()
            }
            SYS_COND_OBJECT => {
                if derived.cond_ipc_keys.contains_key(&id)
                    || state
                        .conds
                        .lookup(id)
                        .is_some_and(|c| !c.waiters().is_empty())
                {
                    return false;
                }
                state.conds.destroy(id).is_some()
            }
            SYS_LWMUTEX_OBJECT => {
                if state
                    .lwmutexes
                    .lookup(id)
                    .is_some_and(|m| !m.waiters().is_empty())
                {
                    return false;
                }
                state.lwmutexes.destroy(id).is_some()
            }
            SYS_LWCOND_OBJECT => {
                if state
                    .lwconds
                    .lookup(id)
                    .is_some_and(|c| !c.waiters().is_empty())
                {
                    return false;
                }
                let found = state.lwconds.destroy(id).is_some();
                if found {
                    state.process_counts.lwcond_dec();
                }
                found
            }
            SYS_SEMAPHORE_OBJECT => {
                if state
                    .semaphores
                    .lookup(id)
                    .is_some_and(|s| !s.waiters().is_empty())
                {
                    return false;
                }
                state.semaphores.destroy(id).is_some()
            }
            SYS_EVENT_FLAG_OBJECT => {
                if state
                    .event_flags
                    .lookup(id)
                    .is_some_and(|f| !f.waiters().is_empty())
                {
                    return false;
                }
                state.event_flags.destroy(id).is_some()
            }
            SYS_EVENT_QUEUE_OBJECT => {
                if derived.event_queue_ipc_keys.contains_key(&id)
                    || state
                        .event_queues
                        .lookup(id)
                        .is_some_and(|q| !q.waiters().is_empty())
                {
                    return false;
                }
                let found = state.event_queues.destroy(id).is_some();
                state.event_ports.unbind_queue(id);
                state.groups.unbind_event_queue(id);
                found
            }
            SYS_EVENT_PORT_OBJECT => {
                let bound = state
                    .event_ports
                    .lookup(id)
                    .and_then(|p| p.queue())
                    .is_some();
                if bound && state.event_ports.disconnect(id).is_err() {
                    return false;
                }
                let found = state.event_ports.destroy(id).is_ok();
                if found {
                    state.process_counts.event_port_dec();
                }
                found
            }
            SYS_TIMER_OBJECT => state.process_counts.timer_remove(id),
            SYS_RWLOCK_OBJECT => state.process_counts.rwlock_remove(id),
            SHARED_MEMORY_CLASS | SHARED_MEMORY_EXT_CLASS => {
                if state.mmapper_ipc.iter().any(|(_, &mem_id)| mem_id == id) {
                    return false;
                }
                match state.mmapper_handles.get(id) {
                    Some(handle) => {
                        if self.mmapper_segment_busy(id, handle) {
                            return false;
                        }
                        self.state.mmapper_handles.remove(id);
                        true
                    }
                    None => false,
                }
            }
            _ => return false,
        };
        if !found {
            self.log_invariant_break(
                "process.exit_id_without_object",
                format_args!(
                    "exit released live kernel id {id:#x}, which names no object; \
                     the id table and the object tables diverged"
                ),
            );
        }
        true
    }
}

#[cfg(test)]
#[path = "tests/exit_ids_tests.rs"]
mod tests;
