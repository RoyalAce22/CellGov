//! LV2 dispatch for event flags.
//!
//! Waiters are FIFO. A waiter names a bit pattern, an AND or OR match
//! rule, and whether a match clears its pattern. `set` delivers the
//! observed bit pattern through each woken waiter's recorded
//! `result_ptr`; a missing thread-table entry discards its wake.
//!
//! The hardware trace in `tests/ps3autotests/tests/lv2/sys_event_flag`
//! pins a refusal code on every arm here, the set / clear arithmetic,
//! and the count cancel reports. It never presents two faults at once.
//! The console capture of `tests/micro/lv2_event_flag_check_order`
//! does: wait and trywait check the id before the mode. The console
//! capture of `tests/micro/lv2_destroy_parked` settles destroy's
//! `CELL_EBUSY` with a parked waiter, who stays parked.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::process::SYS_EVENT_FLAG_OBJECT;

use crate::dispatch::{Lv2Dispatch, PendingResponse};
use crate::host::guest_struct::GuestStruct;
use crate::host::{Lv2Host, Lv2Runtime};
use cellgov_time::GuestTicks;

impl Lv2Host {
    /// Decode a `sys_event_flag_wait` mode word; `None` when either
    /// nibble is out of range, both clear bits included.
    ///
    /// [`CLEAR_ALL`] clears every bit of the flag on a match, where
    /// [`CLEAR`] clears only the waited bits. The console capture of
    /// `tests/micro/lv2_event_flag_clear_all` shows the `CLEAR_ALL`
    /// arms, and EINVAL for the two clear bits together.
    ///
    /// [`CLEAR`]: cellgov_ps3_abi::lv2::sync::event_flag_wait_mode::CLEAR
    /// [`CLEAR_ALL`]: cellgov_ps3_abi::lv2::sync::event_flag_wait_mode::CLEAR_ALL
    fn decode_event_flag_mode(raw: u32) -> Option<crate::ppu_thread::EventFlagWaitMode> {
        use crate::ppu_thread::EventFlagWaitMode as Mode;
        use cellgov_ps3_abi::lv2::sync::event_flag_wait_mode as wait_mode;
        let or_match = match raw & wait_mode::MATCH_MASK {
            wait_mode::AND => false,
            wait_mode::OR => true,
            _ => return None,
        };
        Some(match (or_match, raw & wait_mode::CLEAR_MASK) {
            (false, 0) => Mode::AndNoClear,
            (false, wait_mode::CLEAR) => Mode::AndClear,
            (false, wait_mode::CLEAR_ALL) => Mode::AndClearAll,
            (true, 0) => Mode::OrNoClear,
            (true, wait_mode::CLEAR) => Mode::OrClear,
            (true, wait_mode::CLEAR_ALL) => Mode::OrClearAll,
            _ => return None,
        })
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
        use cellgov_ps3_abi::lv2::sync::event_flag_attribute as attr_layout;
        let Some(attr) = GuestStruct::read(rt, attr_ptr as u64, attr_layout::SIZE as usize) else {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        };
        let protocol = attr.u32_at(attr_layout::PROTOCOL_OFFSET);
        let kind = attr.u32_at(attr_layout::TYPE_OFFSET);
        use cellgov_ps3_abi::lv2::sync::{
            SYS_SYNC_FIFO, SYS_SYNC_PRIORITY, SYS_SYNC_WAITER_MULTIPLE, SYS_SYNC_WAITER_SINGLE,
        };
        if protocol != SYS_SYNC_FIFO && protocol != SYS_SYNC_PRIORITY {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        if kind != SYS_SYNC_WAITER_SINGLE && kind != SYS_SYNC_WAITER_MULTIPLE {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        let Some(id) = self.alloc_id(SYS_EVENT_FLAG_OBJECT) else {
            return Lv2Dispatch::immediate(errno::CELL_EAGAIN.into());
        };
        if self.state.event_flags.create_with_id(id, init).is_err() {
            self.release_id(id);
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
        self.release_id(id);
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
        // stores the observed pattern. The arm checks the id before the
        // mode, so an unknown id answers ESRCH whatever the mode; the
        // console capture of tests/micro/lv2_event_flag_check_order
        // shows this order.
        if self.state.event_flags.lookup(id).is_none() {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: event_flag_result_write(result_ptr, 0, requester, tick),
            };
        }
        let Some(mode) = Self::decode_event_flag_mode(mode_raw) else {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EINVAL.into(),
                effects: event_flag_result_write(result_ptr, 0, requester, tick),
            };
        };
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
        // match stores the observed pattern. As in the wait arm, this
        // arm checks the id before the mode.
        if self.state.event_flags.lookup(id).is_none() {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: event_flag_result_write(result_ptr, 0, requester, tick),
            };
        }
        let Some(mode) = Self::decode_event_flag_mode(mode_raw) else {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EINVAL.into(),
                effects: event_flag_result_write(result_ptr, 0, requester, tick),
            };
        };
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

    /// `sys_event_flag_clear` (118).
    ///
    /// The flag keeps only the bits `bits` names; no waiter wakes.
    pub(super) fn dispatch_event_flag_clear(&mut self, id: u32, bits: u64) -> Lv2Dispatch {
        if !self.state.event_flags.clear_bits(id, bits) {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        Lv2Dispatch::immediate(0)
    }

    /// `sys_event_flag_cancel` (132): wakes every parked waiter with
    /// `CELL_ECANCELED` and reports the count through `num_ptr`.
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

    /// `sys_event_flag_get` (139): writes the flag's current bits
    /// through `flags_ptr` as a big-endian u64.
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` for an unknown id.
    /// - `CELL_EFAULT` for a null `flags_ptr`; the id gate fires first.
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

#[cfg(test)]
#[path = "tests/event_flag_console_tests.rs"]
mod console_tests;
