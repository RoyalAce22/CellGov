//! LV2 dispatch for event flags.
//!
//! Waiters are FIFO. `set` delivers the observed bit pattern through
//! each woken waiter's recorded `result_ptr`; a missing thread-table
//! entry discards its wake.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;

use crate::dispatch::{Lv2Dispatch, PendingResponse};
use crate::host::guest_struct::GuestStruct;
use crate::host::{Lv2Host, Lv2Runtime};
use cellgov_time::GuestTicks;

impl Lv2Host {
    /// Decode a `sys_event_flag_wait` mode word; `None` when either
    /// nibble is out of range.
    ///
    /// [`CLEAR_ALL`] asks for every bit of the flag value. The wait
    /// modes `EventFlagTable` takes clear only the waiter's own bit
    /// pattern, so `CLEAR_ALL` decodes onto the same arm as [`CLEAR`].
    /// Both dispatch entries witness that collapse through
    /// `note_clear_all_collapse`.
    ///
    /// [`CLEAR`]: cellgov_ps3_abi::lv2::sync::event_flag_wait_mode::CLEAR
    /// [`CLEAR_ALL`]: cellgov_ps3_abi::lv2::sync::event_flag_wait_mode::CLEAR_ALL
    fn decode_event_flag_mode(raw: u32) -> Option<crate::ppu_thread::EventFlagWaitMode> {
        use cellgov_ps3_abi::lv2::sync::event_flag_wait_mode as wait_mode;
        let or_match = match raw & wait_mode::MATCH_MASK {
            wait_mode::AND => false,
            wait_mode::OR => true,
            _ => return None,
        };
        let clear = match raw & wait_mode::CLEAR_MASK {
            0 => false,
            wait_mode::CLEAR | wait_mode::CLEAR_ALL => true,
            _ => return None,
        };
        Some(match (or_match, clear) {
            (false, false) => crate::ppu_thread::EventFlagWaitMode::AndNoClear,
            (false, true) => crate::ppu_thread::EventFlagWaitMode::AndClear,
            (true, false) => crate::ppu_thread::EventFlagWaitMode::OrNoClear,
            (true, true) => crate::ppu_thread::EventFlagWaitMode::OrClear,
        })
    }

    /// Name the `CLEAR_ALL` wait the flag table cannot express.
    ///
    /// `decode_event_flag_mode` states the collapse. The mode is legal
    /// input, so the call still answers and this witness counts the
    /// divergence.
    fn note_clear_all_collapse(&mut self, site: &'static str, mode_raw: u32) {
        use cellgov_ps3_abi::lv2::sync::event_flag_wait_mode as wait_mode;
        if mode_raw & wait_mode::CLEAR_MASK != wait_mode::CLEAR_ALL {
            return;
        }
        self.log_invariant_break(
            site,
            format_args!(
                "sys_event_flag wait mode {mode_raw:#x} selects CLEAR_ALL; the model \
                 clears only the waiter's own bit pattern, so bits outside it survive \
                 the match"
            ),
        );
    }

    pub(super) fn dispatch_event_flag_create(
        &mut self,
        id_ptr: u32,
        attr_ptr: u32,
        init: u64,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if id_ptr == 0 || attr_ptr == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        // sys_event_flag_attribute_t: protocol@0 u32, pshared@4 u32,
        // ipc_key@8 u64, flags@16 s32, type@20 s32.
        let Some(attr) = GuestStruct::read(rt, attr_ptr as u64, 24) else {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        };
        let protocol = attr.u32_at(0);
        let kind = attr.u32_at(20);
        use cellgov_ps3_abi::lv2::sync::{
            SYS_SYNC_FIFO, SYS_SYNC_PRIORITY, SYS_SYNC_WAITER_MULTIPLE, SYS_SYNC_WAITER_SINGLE,
        };
        if protocol != SYS_SYNC_FIFO && protocol != SYS_SYNC_PRIORITY {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        if kind != SYS_SYNC_WAITER_SINGLE && kind != SYS_SYNC_WAITER_MULTIPLE {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        let id = self.alloc_id();
        if self.state.event_flags.create_with_id(id, init).is_err() {
            return Lv2Dispatch::immediate(errno::CELL_ENOMEM.into());
        }
        self.immediate_write_u32(id, id_ptr, requester, tick)
    }

    pub(super) fn dispatch_event_flag_destroy(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.state.event_flags.lookup(id) else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        if !entry.waiters().is_empty() {
            return Lv2Dispatch::immediate(errno::CELL_EBUSY.into());
        }
        self.state.event_flags.destroy(id);
        Lv2Dispatch::immediate(0)
    }

    pub(super) fn dispatch_event_flag_wait(
        &mut self,
        id: u32,
        bits: u64,
        mode_raw: u32,
        result_ptr: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let Some(caller) = self.state.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        // Every non-park exit stores through a non-null result
        // pointer: an error exit stores 0, and only a satisfied wait
        // stores the observed pattern. The order of mode validation
        // against the id lookup is unestablished. The hardware trace
        // in tests/ps3autotests/tests/lv2/sys_event_flag pins EINVAL
        // for a bad mode and ESRCH for a bad id, but never combines
        // the two. CellGov validates the mode first.
        let Some(mode) = Self::decode_event_flag_mode(mode_raw) else {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EINVAL.into(),
                effects: event_flag_result_write(result_ptr, 0, requester, tick),
            };
        };
        self.note_clear_all_collapse("event_flag.wait_clear_all_not_modeled", mode_raw);
        match self.state.event_flags.try_wait(id, bits, mode) {
            None => Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: event_flag_result_write(result_ptr, 0, requester, tick),
            },
            Some(crate::sync_primitives::EventFlagWait::Matched { observed }) => {
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: event_flag_result_write(result_ptr, observed, requester, tick),
                }
            }
            Some(crate::sync_primitives::EventFlagWait::NoMatch) => {
                // A finite timeout parks like any wait; the runtime's
                // timer-wake queue expires it with ETIMEDOUT at the
                // deadline.
                match self
                    .state
                    .event_flags
                    .enqueue_waiter(id, caller, bits, mode, result_ptr)
                {
                    Ok(()) => {}
                    Err(crate::sync_primitives::EventFlagEnqueueError::UnknownId) => {
                        return Lv2Dispatch::Immediate {
                            code: errno::CELL_ESRCH.into(),
                            effects: event_flag_result_write(result_ptr, 0, requester, tick),
                        };
                    }
                    Err(crate::sync_primitives::EventFlagEnqueueError::DuplicateWaiter) => {
                        // Host-side refusal, but the store-result
                        // contract above covers every non-park exit,
                        // so this one zeroes the result too.
                        return Lv2Dispatch::Immediate {
                            code: errno::CELL_EFAULT.into(),
                            effects: event_flag_result_write(result_ptr, 0, requester, tick),
                        };
                    }
                }
                Lv2Dispatch::Block {
                    reason: crate::dispatch::Lv2BlockReason::EventFlag { id },
                    pending: PendingResponse::EventFlagWake {
                        result_ptr,
                        observed: 0,
                    },
                    effects: vec![],
                }
            }
        }
    }

    pub(super) fn dispatch_event_flag_trywait(
        &mut self,
        id: u32,
        bits: u64,
        mode_raw: u32,
        result_ptr: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        // Every exit stores through a non-null result pointer:
        // EINVAL, ESRCH, and the no-match EBUSY all store 0; only a
        // match stores the observed pattern. As in the wait arm,
        // CellGov validates the mode before the id lookup and no
        // trace orders the two.
        let Some(mode) = Self::decode_event_flag_mode(mode_raw) else {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EINVAL.into(),
                effects: event_flag_result_write(result_ptr, 0, requester, tick),
            };
        };
        self.note_clear_all_collapse("event_flag.trywait_clear_all_not_modeled", mode_raw);
        match self.state.event_flags.try_wait(id, bits, mode) {
            None => Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: event_flag_result_write(result_ptr, 0, requester, tick),
            },
            Some(crate::sync_primitives::EventFlagWait::Matched { observed }) => {
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: event_flag_result_write(result_ptr, observed, requester, tick),
                }
            }
            Some(crate::sync_primitives::EventFlagWait::NoMatch) => Lv2Dispatch::Immediate {
                code: errno::CELL_EBUSY.into(),
                effects: event_flag_result_write(result_ptr, 0, requester, tick),
            },
        }
    }

    pub(super) fn dispatch_event_flag_set(&mut self, id: u32, bits: u64) -> Lv2Dispatch {
        let Some(woken) = self.state.event_flags.set_and_wake(id, bits) else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        if woken.is_empty() {
            return Lv2Dispatch::immediate(0);
        }
        let mut unit_ids: Vec<UnitId> = Vec::new();
        let mut updates: Vec<(UnitId, PendingResponse)> = Vec::new();
        for wake in woken {
            if let Some(unit) = self.resolve_wake_thread(wake.thread, "event_flag_set.waker") {
                unit_ids.push(unit);
                updates.push((
                    unit,
                    PendingResponse::EventFlagWake {
                        result_ptr: wake.result_ptr,
                        observed: wake.observed,
                    },
                ));
            }
        }
        Lv2Dispatch::WakeAndReturn {
            code: 0,
            woken_unit_ids: unit_ids,
            response_updates: updates,
            effects: vec![],
        }
    }

    pub(super) fn dispatch_event_flag_clear(&mut self, id: u32, bits: u64) -> Lv2Dispatch {
        if !self.state.event_flags.clear_bits(id, bits) {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        Lv2Dispatch::immediate(0)
    }

    pub(super) fn dispatch_event_flag_cancel(
        &mut self,
        id: u32,
        num_ptr: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        // Pattern snapshot before the drain: a cancelled waiter
        // reports the captured pattern through its own result
        // pointer, alongside ECANCELED. Cancel also stores 0 through
        // a non-null num pointer on the ESRCH exit.
        let Some(bits) = self.state.event_flags.lookup(id).map(|e| e.bits()) else {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: event_flag_count_write(num_ptr, 0, requester, tick),
            };
        };
        let Some(waiters) = self.state.event_flags.cancel_waiters(id) else {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: event_flag_count_write(num_ptr, 0, requester, tick),
            };
        };
        let count = waiters.len() as u32;
        let mut unit_ids: Vec<UnitId> = Vec::new();
        let mut updates: Vec<(UnitId, PendingResponse)> = Vec::new();
        let mut effects: Vec<Effect> = Vec::new();
        for w in waiters {
            if let Some(unit) = self.resolve_wake_thread(w.thread, "event_flag_cancel.waker") {
                unit_ids.push(unit);
                // Each result_ptr was decoded from the WAITER's
                // park-time syscall, so it must resolve in the
                // waiter's space; the canceller's effects commit into
                // the canceller's. Hence the store rides the wake
                // channel.
                updates.push((
                    unit,
                    PendingResponse::EventFlagCancelWake {
                        result_ptr: w.result_ptr,
                        observed: bits,
                    },
                ));
            }
        }
        effects.extend(event_flag_count_write(num_ptr, count, requester, tick));
        if unit_ids.is_empty() {
            return Lv2Dispatch::Immediate { code: 0, effects };
        }
        Lv2Dispatch::WakeAndReturn {
            code: 0,
            woken_unit_ids: unit_ids,
            response_updates: updates,
            effects,
        }
    }

    pub(super) fn dispatch_event_flag_get(
        &mut self,
        id: u32,
        flags_ptr: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let Some(entry) = self.state.event_flags.lookup(id) else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        if flags_ptr == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        let bits = entry.bits();
        let write = Effect::shared_write(
            ByteRange::contiguous_u32(flags_ptr, 8),
            WritePayload::from_slice(&bits.to_be_bytes()),
            requester,
            tick,
        );
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }
}

/// The observed-pattern write for a satisfied wait/trywait.
///
/// A null result pointer is legal: the caller passes 0 when it does
/// not want the pattern back, and nothing is stored. The trywait
/// helper in `tests/ps3autotests/tests/lv2/sys_event_flag` passes 0
/// and still returns CELL_OK on the satisfied call. The wait helper
/// there passes 0 too, but its call parks and a cancel wakes it, so
/// it never reaches this write.
fn event_flag_result_write(
    result_ptr: u32,
    observed: u64,
    requester: UnitId,
    tick: GuestTicks,
) -> Vec<Effect> {
    if result_ptr == 0 {
        return vec![];
    }
    vec![Effect::shared_write(
        ByteRange::contiguous_u32(result_ptr, 8),
        WritePayload::from_slice(&observed.to_be_bytes()),
        requester,
        tick,
    )]
}

/// The cancelled-waiter count write for `sys_event_flag_cancel`.
///
/// A null num pointer is legal: the caller passes 0 when it does not
/// want the woken-thread count back, and nothing is stored. The ESRCH
/// exit writes 0 through a non-null one.
fn event_flag_count_write(
    num_ptr: u32,
    count: u32,
    requester: UnitId,
    tick: GuestTicks,
) -> Vec<Effect> {
    if num_ptr == 0 {
        return vec![];
    }
    vec![Effect::shared_write(
        ByteRange::contiguous_u32(num_ptr, 4),
        WritePayload::from_slice(&count.to_be_bytes()),
        requester,
        tick,
    )]
}

#[cfg(test)]
#[path = "tests/event_flag_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/event_flag_clear_all_tests.rs"]
mod clear_all_tests;
