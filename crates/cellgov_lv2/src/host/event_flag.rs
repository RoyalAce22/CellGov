//! LV2 dispatch for event flags.
//!
//! Waiters are FIFO. `set` delivers the observed bit pattern through
//! each woken waiter's recorded `result_ptr`; a missing thread-table
//! entry discards its wake.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::cell_errors;

use crate::dispatch::{Lv2Dispatch, PendingResponse};
use crate::host::{Lv2Host, Lv2Runtime};

impl Lv2Host {
    // `sys_event_flag_wait_mode` bit layout:
    //   bit 0 (0x01): AND match
    //   bit 1 (0x02): OR  match (exactly one of AND / OR must be set)
    //   bit 4 (0x10): CLEAR on match
    //   bit 5 (0x20): CLEAR_ALL on match (CLEAR and CLEAR_ALL are
    //                                     mutually exclusive)
    // Returns `None` if the low or high nibble is out of range.
    fn decode_event_flag_mode(raw: u32) -> Option<crate::ppu_thread::EventFlagWaitMode> {
        let or_match = match raw & 0x0F {
            0x01 => false, // AND
            0x02 => true,  // OR
            _ => return None,
        };
        let clear = match raw & 0xF0 {
            0x00 => false,
            0x10 | 0x20 => true,
            _ => return None,
        };
        Some(match (or_match, clear) {
            (false, false) => crate::ppu_thread::EventFlagWaitMode::AndNoClear,
            (false, true) => crate::ppu_thread::EventFlagWaitMode::AndClear,
            (true, false) => crate::ppu_thread::EventFlagWaitMode::OrNoClear,
            (true, true) => crate::ppu_thread::EventFlagWaitMode::OrClear,
        })
    }

    pub(super) fn dispatch_event_flag_create(
        &mut self,
        id_ptr: u32,
        attr_ptr: u32,
        init: u64,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        if id_ptr == 0 || attr_ptr == 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        // sys_event_flag_attribute_t: protocol@0 u32, pshared@4 u32,
        // ipc_key@8 u64, flags@16 s32, type@20 s32.
        let Some(attr_bytes) = rt.read_committed(attr_ptr as u64, 24) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        };
        let protocol =
            u32::from_be_bytes([attr_bytes[0], attr_bytes[1], attr_bytes[2], attr_bytes[3]]);
        let kind = u32::from_be_bytes([
            attr_bytes[20],
            attr_bytes[21],
            attr_bytes[22],
            attr_bytes[23],
        ]);
        use cellgov_ps3_abi::sys_sync::{
            SYS_SYNC_FIFO, SYS_SYNC_PRIORITY, SYS_SYNC_WAITER_MULTIPLE, SYS_SYNC_WAITER_SINGLE,
        };
        if protocol != SYS_SYNC_FIFO && protocol != SYS_SYNC_PRIORITY {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        if kind != SYS_SYNC_WAITER_SINGLE && kind != SYS_SYNC_WAITER_MULTIPLE {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        }
        let id = self.alloc_id();
        if self.event_flags.create_with_id(id, init).is_err() {
            return Lv2Dispatch::immediate(cell_errors::CELL_ENOMEM.into());
        }
        self.immediate_write_u32(id, id_ptr, requester)
    }

    pub(super) fn dispatch_event_flag_destroy(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.event_flags.lookup(id) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        if !entry.waiters().is_empty() {
            return Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into());
        }
        self.event_flags.destroy(id);
        Lv2Dispatch::immediate(0)
    }

    pub(super) fn dispatch_event_flag_wait(
        &mut self,
        id: u32,
        bits: u64,
        mode_raw: u32,
        result_ptr: u32,
        timeout: u64,
        requester: UnitId,
    ) -> Lv2Dispatch {
        let Some(caller) = self.ppu_threads.thread_id_for_unit(requester) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        let Some(mode) = Self::decode_event_flag_mode(mode_raw) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        };
        match self.event_flags.try_wait(id, bits, mode) {
            None => Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into()),
            Some(crate::sync_primitives::EventFlagWait::Matched { observed }) => {
                let write = Effect::SharedWriteIntent {
                    range: ByteRange::contiguous_u32(result_ptr, 8),
                    bytes: WritePayload::from_slice(&observed.to_be_bytes()),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: self.current_tick,
                };
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![write],
                }
            }
            Some(crate::sync_primitives::EventFlagWait::NoMatch) => {
                // Finite timeout with no peer that could set/clear:
                // ETIMEDOUT immediately.
                if timeout != 0 && !self.ppu_threads.has_other_alive_thread(caller) {
                    return Lv2Dispatch::immediate(cell_errors::CELL_ETIMEDOUT.into());
                }
                match self
                    .event_flags
                    .enqueue_waiter(id, caller, bits, mode, result_ptr)
                {
                    Ok(()) => {}
                    Err(crate::sync_primitives::EventFlagEnqueueError::UnknownId) => {
                        return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
                    }
                    Err(crate::sync_primitives::EventFlagEnqueueError::DuplicateWaiter) => {
                        return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
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
    ) -> Lv2Dispatch {
        let Some(mode) = Self::decode_event_flag_mode(mode_raw) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_EINVAL.into());
        };
        match self.event_flags.try_wait(id, bits, mode) {
            None => Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into()),
            Some(crate::sync_primitives::EventFlagWait::Matched { observed }) => {
                let write = Effect::SharedWriteIntent {
                    range: ByteRange::contiguous_u32(result_ptr, 8),
                    bytes: WritePayload::from_slice(&observed.to_be_bytes()),
                    ordering: PriorityClass::Normal,
                    source: requester,
                    source_time: self.current_tick,
                };
                Lv2Dispatch::Immediate {
                    code: 0,
                    effects: vec![write],
                }
            }
            Some(crate::sync_primitives::EventFlagWait::NoMatch) => {
                Lv2Dispatch::immediate(cell_errors::CELL_EBUSY.into())
            }
        }
    }

    pub(super) fn dispatch_event_flag_set(&mut self, id: u32, bits: u64) -> Lv2Dispatch {
        let Some(woken) = self.event_flags.set_and_wake(id, bits) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
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
        if !self.event_flags.clear_bits(id, bits) {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        }
        Lv2Dispatch::immediate(0)
    }

    pub(super) fn dispatch_event_flag_cancel(
        &mut self,
        id: u32,
        num_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        let Some(waiters) = self.event_flags.cancel_waiters(id) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        let count = waiters.len() as u32;
        let mut unit_ids: Vec<UnitId> = Vec::new();
        let mut updates: Vec<(UnitId, PendingResponse)> = Vec::new();
        for w in waiters {
            if let Some(unit) = self.resolve_wake_thread(w.thread, "event_flag_cancel.waker") {
                unit_ids.push(unit);
                updates.push((
                    unit,
                    PendingResponse::ReturnCode {
                        code: cell_errors::CELL_ECANCELED.into(),
                    },
                ));
            }
        }
        let mut effects: Vec<Effect> = Vec::new();
        if num_ptr != 0 {
            effects.push(Effect::SharedWriteIntent {
                range: ByteRange::contiguous_u32(num_ptr, 4),
                bytes: WritePayload::from_slice(&count.to_be_bytes()),
                ordering: PriorityClass::Normal,
                source: requester,
                source_time: self.current_tick,
            });
        }
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
    ) -> Lv2Dispatch {
        let Some(entry) = self.event_flags.lookup(id) else {
            return Lv2Dispatch::immediate(cell_errors::CELL_ESRCH.into());
        };
        if flags_ptr == 0 {
            return Lv2Dispatch::immediate(cell_errors::CELL_EFAULT.into());
        }
        let bits = entry.bits();
        let write = Effect::SharedWriteIntent {
            range: ByteRange::contiguous_u32(flags_ptr, 8),
            bytes: WritePayload::from_slice(&bits.to_be_bytes()),
            ordering: PriorityClass::Normal,
            source: requester,
            source_time: self.current_tick,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![write],
        }
    }
}

#[cfg(test)]
#[path = "tests/event_flag_tests.rs"]
mod tests;
