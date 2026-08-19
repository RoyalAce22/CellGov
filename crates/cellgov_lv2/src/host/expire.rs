//! Timed-wait expiry: unqueue a parked waiter and deliver
//! `CELL_ETIMEDOUT`, per primitive.
//!
//! The runtime calls [`Lv2Host::expire_wait`] when a timed wait's
//! guest-tick deadline fires. The wake-vs-timeout race cannot occur
//! here: any wake or response restage cancels the unit's timer entry
//! first, so a missing waiter record at expiry time is a host
//! invariant break, not a lost race (contrast RPCS3's `unqueue`
//! self-removal in `rpcs3/Emu/Cell/lv2/lv2.cpp`, where a failed
//! unqueue means a signal won).

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors;
use cellgov_time::GuestTicks;

use crate::dispatch::{CondMutexKind, ExpiredWait, Lv2BlockReason, PendingResponse};
use crate::host::Lv2Host;
use crate::ppu_thread::PpuThreadId;

impl Lv2Host {
    /// Expire `requester`'s timed wait on the primitive named by
    /// `reason`, removing its waiter record and staging
    /// `CELL_ETIMEDOUT` delivery.
    ///
    /// Per-primitive semantics follow real LV2 via the RPCS3 oracle:
    /// mutex family takes no ownership, the event queue writes
    /// nothing, the event flag writes the current pattern, and
    /// cond re-acquires the companion mutex -- contended re-acquire
    /// re-parks the unit UNTIMED with r3 already staged, so it
    /// returns ETIMEDOUT holding the mutex when the unlock arrives.
    pub fn expire_wait(
        &mut self,
        reason: Lv2BlockReason,
        requester: UnitId,
        tick: GuestTicks,
    ) -> ExpiredWait {
        let Some(thread) = self.state.ppu_threads.thread_id_for_unit(requester) else {
            self.record_invariant_break(
                "expire_wait.unknown_unit",
                format_args!("timed wait expired for {requester:?} with no PPU thread record"),
            );
            return ExpiredWait::default();
        };
        match reason {
            // RPCS3 sys_mutex.cpp sys_mutex_lock: timeout unqueues the
            // sleeper without taking ownership.
            Lv2BlockReason::Mutex { id } => {
                if !self.state.mutexes.remove_waiter(id, thread) {
                    return self.expire_missing_waiter("mutex", id, requester);
                }
                self.note_expiry("mutex");
                timed_out_wake(requester)
            }
            // RPCS3 sys_lwmutex.cpp _sys_lwmutex_lock: same shape; the
            // syscall side writes no sys_lwmutex_t fields on timeout --
            // the guest's lock routine owns its own bookkeeping on the
            // ETIMEDOUT return path.
            Lv2BlockReason::LwMutex { id } => {
                if !self.state.lwmutexes.remove_waiter(id, thread) {
                    return self.expire_missing_waiter("lwmutex", id, requester);
                }
                self.note_expiry("lwmutex");
                timed_out_wake(requester)
            }
            // RPCS3 sys_semaphore.cpp sys_semaphore_wait: unqueue +
            // ETIMEDOUT. RPCS3 also repairs its negative count; CellGov's
            // count never decrements for waiters, so nothing to repair.
            Lv2BlockReason::Semaphore { id } => {
                if !self.state.semaphores.remove_waiter(id, thread) {
                    return self.expire_missing_waiter("semaphore", id, requester);
                }
                self.note_expiry("semaphore");
                timed_out_wake(requester)
            }
            // RPCS3 sys_event.cpp sys_event_queue_receive: timeout
            // writes nothing; event data only ever returns in r4-r7 on
            // success.
            Lv2BlockReason::EventQueue { id } => {
                if self.state.event_queues.remove_waiter(id, thread).is_none() {
                    return self.expire_missing_waiter("equeue", id, requester);
                }
                self.note_expiry("equeue");
                timed_out_wake(requester)
            }
            // RPCS3 sys_event_flag.cpp sys_event_flag_wait: timeout
            // writes the flag's current pattern through the result
            // pointer (cancel does the same alongside ECANCELED).
            Lv2BlockReason::EventFlag { id } => {
                // Bits first: a missing flag entry implies a missing
                // waiter record, so it routes to the same refusal.
                let Some(bits) = self.state.event_flags.lookup(id).map(|e| e.bits()) else {
                    return self.expire_missing_waiter("eflag", id, requester);
                };
                let Some(waiter) = self.state.event_flags.remove_waiter(id, thread) else {
                    return self.expire_missing_waiter("eflag", id, requester);
                };
                self.note_expiry("eflag");
                let mut out = timed_out_wake(requester);
                // RPCS3 sys_event_flag.cpp sys_event_store_result:
                // the result is written only through a non-null
                // pointer; a null result pointer is legal and skipped.
                if waiter.result_ptr != 0 {
                    out.effects.push(Effect::SharedWriteIntent {
                        range: ByteRange::contiguous_u32(waiter.result_ptr, 8),
                        bytes: WritePayload::from_slice(&bits.to_be_bytes()),
                        ordering: PriorityClass::Normal,
                        source: requester,
                        source_time: tick,
                    });
                }
                out
            }
            Lv2BlockReason::Cond {
                id,
                mutex_id,
                mutex_kind,
            } => self.expire_cond_wait(id, mutex_id, mutex_kind, requester, thread),
            Lv2BlockReason::ThreadGroupJoin { .. } | Lv2BlockReason::PpuThreadJoin { .. } => {
                self.record_invariant_break(
                    "expire_wait.untimed_reason",
                    format_args!(
                        "timer entry fired for join-family reason {reason:?} on {requester:?}; \
                         join requests carry no timeout and must never register a deadline"
                    ),
                );
                ExpiredWait::default()
            }
        }
    }

    /// RPCS3 sys_cond.cpp sys_cond_wait: timeout unqueues from the
    /// cond, stages ETIMEDOUT, then tries to own the mutex; if held,
    /// the thread re-parks on the mutex untimed and returns ETIMEDOUT
    /// holding the mutex once the unlock transfers ownership.
    fn expire_cond_wait(
        &mut self,
        id: u32,
        mutex_id: u32,
        mutex_kind: CondMutexKind,
        requester: UnitId,
        thread: PpuThreadId,
    ) -> ExpiredWait {
        if mutex_kind == CondMutexKind::LwMutex {
            // Cond-over-lwmutex is rejected with EPERM at every
            // dispatch site, so no waiter can carry this kind.
            self.record_invariant_break(
                "expire_wait.cond_lwmutex",
                format_args!("cond {id} timed out with lwmutex companion (unreachable kind)"),
            );
            return ExpiredWait::default();
        }
        if self.state.conds.signal_to(id, thread).is_err() {
            return self.expire_missing_waiter("cond", id, requester);
        }
        self.note_expiry("cond");
        match self.state.mutexes.try_acquire(mutex_id, thread) {
            Some(crate::sync_primitives::MutexAcquire::Acquired) => timed_out_wake(requester),
            Some(crate::sync_primitives::MutexAcquire::Contended) => {
                if let Err(err) = self.state.mutexes.enqueue_waiter(mutex_id, thread) {
                    self.record_invariant_break(
                        "expire_wait.cond_reenqueue",
                        format_args!(
                            "enqueue_waiter failed for mutex {mutex_id} thread {thread:?}: \
                             {err:?}; waking with ESRCH to avoid stranding"
                        ),
                    );
                    return coded_wake(requester, cell_errors::CELL_ESRCH.into());
                }
                // Response staged, no wake: the next mutex unlock
                // resolves it through the ordinary sync-wake path.
                ExpiredWait {
                    woken_unit_ids: vec![],
                    response_updates: vec![(
                        requester,
                        PendingResponse::ReturnCode {
                            code: cell_errors::CELL_ETIMEDOUT.into(),
                        },
                    )],
                    effects: vec![],
                }
            }
            None => {
                self.record_invariant_break(
                    "expire_wait.cond_destroyed_mutex",
                    format_args!("cond {id} timeout references destroyed mutex {mutex_id}"),
                );
                coded_wake(requester, cell_errors::CELL_ESRCH.into())
            }
        }
    }

    /// The cancel invariant failed: the unit's timer entry outlived
    /// its waiter record. Refuse the expiry so a resolved wait is not
    /// clobbered; the unit's real wake already happened or is staged.
    fn expire_missing_waiter(
        &mut self,
        primitive: &'static str,
        id: u32,
        requester: UnitId,
    ) -> ExpiredWait {
        self.record_invariant_break(
            "expire_wait.missing_waiter",
            format_args!(
                "timed {primitive} wait expired for {requester:?} but no waiter record \
                 exists on id {id}; a wake path resolved this wait without cancelling \
                 its timer entry"
            ),
        );
        ExpiredWait::default()
    }

    fn note_expiry(&mut self, primitive: &'static str) {
        *self.obs.wait_timeout_expiries.entry(primitive).or_insert(0) += 1;
    }
}

fn timed_out_wake(unit: UnitId) -> ExpiredWait {
    coded_wake(unit, cell_errors::CELL_ETIMEDOUT.into())
}

fn coded_wake(unit: UnitId, code: u64) -> ExpiredWait {
    ExpiredWait {
        woken_unit_ids: vec![unit],
        response_updates: vec![(unit, PendingResponse::ReturnCode { code })],
        effects: vec![],
    }
}

#[cfg(test)]
#[path = "tests/expire_tests.rs"]
mod tests;
