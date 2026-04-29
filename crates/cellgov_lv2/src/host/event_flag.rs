//! LV2 dispatch for event flags.
//!
//! Waiters are FIFO. `set` delivers the observed bit pattern back
//! through each woken waiter's recorded `result_ptr`, so a missing
//! thread-table entry discards its wake without merging into a
//! parked response.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::{PriorityClass, UnitId};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::cell_errors as errno;

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
        // NULL-pointer checks come first: real LV2 returns EFAULT
        // before inspecting any attribute fields.
        if id_ptr == 0 || attr_ptr == 0 {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EFAULT.into(),
                effects: vec![],
            };
        }
        // sys_event_flag_attribute_t layout:
        //   +0  u32 protocol
        //   +4  u32 pshared
        //   +8  u64 ipc_key
        //   +16 s32 flags
        //   +20 s32 type
        // Both protocol and type must be valid sync constants;
        // memset-zero attrs are rejected with EINVAL on real LV2.
        let Some(attr_bytes) = rt.read_committed(attr_ptr as u64, 24) else {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EFAULT.into(),
                effects: vec![],
            };
        };
        let protocol =
            u32::from_be_bytes([attr_bytes[0], attr_bytes[1], attr_bytes[2], attr_bytes[3]]);
        let kind = u32::from_be_bytes([
            attr_bytes[20],
            attr_bytes[21],
            attr_bytes[22],
            attr_bytes[23],
        ]);
        const SYS_SYNC_FIFO: u32 = 0x1;
        const SYS_SYNC_PRIORITY: u32 = 0x2;
        const SYS_SYNC_WAITER_SINGLE: u32 = 0x10000;
        const SYS_SYNC_WAITER_MULTIPLE: u32 = 0x20000;
        if protocol != SYS_SYNC_FIFO && protocol != SYS_SYNC_PRIORITY {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EINVAL.into(),
                effects: vec![],
            };
        }
        if kind != SYS_SYNC_WAITER_SINGLE && kind != SYS_SYNC_WAITER_MULTIPLE {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EINVAL.into(),
                effects: vec![],
            };
        }
        let id = self.alloc_id();
        if self.event_flags.create_with_id(id, init).is_err() {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_ENOMEM.into(),
                effects: vec![],
            };
        }
        self.immediate_write_u32(id, id_ptr, requester)
    }

    pub(super) fn dispatch_event_flag_destroy(&mut self, id: u32) -> Lv2Dispatch {
        let Some(entry) = self.event_flags.lookup(id) else {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        if !entry.waiters().is_empty() {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EBUSY.into(),
                effects: vec![],
            };
        }
        self.event_flags.destroy(id);
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
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
            return Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        let Some(mode) = Self::decode_event_flag_mode(mode_raw) else {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EINVAL.into(),
                effects: vec![],
            };
        };
        match self.event_flags.try_wait(id, bits, mode) {
            None => Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: vec![],
            },
            Some(crate::sync_primitives::EventFlagWait::Matched { observed }) => {
                let write = Effect::SharedWriteIntent {
                    range: ByteRange::new(GuestAddr::new(result_ptr as u64), 8).unwrap(),
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
                // Finite timeout, no peer that could set/clear the
                // bits: trip ETIMEDOUT immediately. With peers
                // alive, block and let the upcoming set wake us.
                if timeout != 0 && !self.ppu_threads.has_other_alive_thread(caller) {
                    return Lv2Dispatch::Immediate {
                        code: errno::CELL_ETIMEDOUT.into(),
                        effects: vec![],
                    };
                }
                match self
                    .event_flags
                    .enqueue_waiter(id, caller, bits, mode, result_ptr)
                {
                    Ok(()) => {}
                    Err(crate::sync_primitives::EventFlagEnqueueError::UnknownId) => {
                        return Lv2Dispatch::Immediate {
                            code: errno::CELL_ESRCH.into(),
                            effects: vec![],
                        };
                    }
                    Err(crate::sync_primitives::EventFlagEnqueueError::DuplicateWaiter) => {
                        return Lv2Dispatch::Immediate {
                            code: errno::CELL_EFAULT.into(),
                            effects: vec![],
                        };
                    }
                }
                // set-side replaces this with an EventFlagWake
                // carrying the observed bits; the result_ptr
                // recorded on the waiter entry makes that wake
                // complete without reading the parked response.
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
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EINVAL.into(),
                effects: vec![],
            };
        };
        match self.event_flags.try_wait(id, bits, mode) {
            None => Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: vec![],
            },
            Some(crate::sync_primitives::EventFlagWait::Matched { observed }) => {
                let write = Effect::SharedWriteIntent {
                    range: ByteRange::new(GuestAddr::new(result_ptr as u64), 8).unwrap(),
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
            Some(crate::sync_primitives::EventFlagWait::NoMatch) => Lv2Dispatch::Immediate {
                code: errno::CELL_EBUSY.into(),
                effects: vec![],
            },
        }
    }

    pub(super) fn dispatch_event_flag_set(&mut self, id: u32, bits: u64) -> Lv2Dispatch {
        let Some(woken) = self.event_flags.set_and_wake(id, bits) else {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        if woken.is_empty() {
            return Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            };
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
            return Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        }
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        }
    }

    pub(super) fn dispatch_event_flag_cancel(
        &mut self,
        id: u32,
        num_ptr: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        // Real LV2 wakes every parked waiter with `CELL_ECANCELED`
        // and writes the count to `num_ptr`.
        let Some(waiters) = self.event_flags.cancel_waiters(id) else {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: vec![],
            };
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
                        code: errno::CELL_ECANCELED.into(),
                    },
                ));
            }
        }
        let mut effects: Vec<Effect> = Vec::new();
        if num_ptr != 0 {
            effects.push(Effect::SharedWriteIntent {
                range: ByteRange::new(GuestAddr::new(num_ptr as u64), 4).unwrap(),
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
        // Real LV2 (matching RPCS3 sys_event_flag_get): unknown id ->
        // ESRCH (and writes 0 through `flags_ptr` if non-NULL); known
        // id + NULL `flags_ptr` -> EFAULT; otherwise writes the
        // current pattern and returns OK.
        let Some(entry) = self.event_flags.lookup(id) else {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_ESRCH.into(),
                effects: vec![],
            };
        };
        if flags_ptr == 0 {
            return Lv2Dispatch::Immediate {
                code: errno::CELL_EFAULT.into(),
                effects: vec![],
            };
        }
        let bits = entry.bits();
        let write = Effect::SharedWriteIntent {
            range: ByteRange::new(GuestAddr::new(flags_ptr as u64), 8).unwrap(),
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
mod tests {
    use super::*;
    use crate::host::test_support::{
        extract_write_u32, fake_runtime_with_valid_sync_attr, seed_primary_ppu, FakeRuntime,
        VALID_SYNC_ATTR_PTR,
    };
    use crate::ppu_thread::PpuThreadAttrs;
    use crate::request::Lv2Request;

    #[test]
    fn event_flag_create_null_id_ptr_returns_efault() {
        let mut host = Lv2Host::new();
        let rt = fake_runtime_with_valid_sync_attr(0x10000);
        let r = host.dispatch(
            Lv2Request::EventFlagCreate {
                id_ptr: 0,
                attr_ptr: VALID_SYNC_ATTR_PTR,
                init: 0,
            },
            UnitId::new(0),
            &rt,
        );
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, errno::CELL_EFAULT.into());
        assert!(host.event_flags().is_empty());
    }

    #[test]
    fn event_flag_create_null_attr_ptr_returns_efault() {
        let mut host = Lv2Host::new();
        let rt = fake_runtime_with_valid_sync_attr(0x10000);
        let r = host.dispatch(
            Lv2Request::EventFlagCreate {
                id_ptr: 0x100,
                attr_ptr: 0,
                init: 0,
            },
            UnitId::new(0),
            &rt,
        );
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, errno::CELL_EFAULT.into());
    }

    #[test]
    fn event_flag_create_zeroed_attr_returns_einval() {
        // Memset-zero attr (protocol=0, type=0): real LV2 rejects
        // with EINVAL because protocol must be FIFO or PRIORITY.
        let mut host = Lv2Host::new();
        // FakeRuntime::new gives zero memory at the attr_ptr; that's
        // the in-memory shape of `memset(&attr, 0, ...)`.
        let rt = FakeRuntime::new(0x10000);
        let r = host.dispatch(
            Lv2Request::EventFlagCreate {
                id_ptr: 0x100,
                attr_ptr: 0x800,
                init: 0,
            },
            UnitId::new(0),
            &rt,
        );
        let Lv2Dispatch::Immediate { code, .. } = r else {
            panic!("expected Immediate, got {r:?}");
        };
        assert_eq!(code, errno::CELL_EINVAL.into());
    }

    #[test]
    fn event_flag_create_stores_init_bits() {
        let mut host = Lv2Host::new();
        let rt = fake_runtime_with_valid_sync_attr(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(
            Lv2Request::EventFlagCreate {
                id_ptr: 0x100,
                attr_ptr: VALID_SYNC_ATTR_PTR,
                init: 0x1234,
            },
            src,
            &rt,
        );
        let id = match &r {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        assert_eq!(host.event_flags().lookup(id).unwrap().bits(), 0x1234);
    }

    #[test]
    fn event_flag_wait_and_mode_mask_match_returns_observed_bits() {
        let mut host = Lv2Host::new();
        let rt = fake_runtime_with_valid_sync_attr(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(
            Lv2Request::EventFlagCreate {
                id_ptr: 0x100,
                attr_ptr: VALID_SYNC_ATTR_PTR,
                init: 0b1111,
            },
            src,
            &rt,
        );
        let id = match &r {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        // mode 0x01 = AND + NO-CLEAR.
        let w = host.dispatch(
            Lv2Request::EventFlagWait {
                id,
                bits: 0b0011,
                mode: 0x01,
                result_ptr: 0x200,
                timeout: 0,
            },
            src,
            &rt,
        );
        match w {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => {
                assert_eq!(e.len(), 1);
            }
            other => panic!("expected Immediate(0), got {other:?}"),
        }
        assert_eq!(host.event_flags().lookup(id).unwrap().bits(), 0b1111);
    }

    #[test]
    fn event_flag_wait_no_match_parks_caller() {
        let mut host = Lv2Host::new();
        let rt = fake_runtime_with_valid_sync_attr(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(
            Lv2Request::EventFlagCreate {
                id_ptr: 0x100,
                attr_ptr: VALID_SYNC_ATTR_PTR,
                init: 0,
            },
            src,
            &rt,
        );
        let id = match &r {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        let w = host.dispatch(
            Lv2Request::EventFlagWait {
                id,
                bits: 0b0010,
                mode: 0x01, // AND + NO-CLEAR
                result_ptr: 0x200,
                timeout: 0,
            },
            src,
            &rt,
        );
        match w {
            Lv2Dispatch::Block {
                reason: crate::dispatch::Lv2BlockReason::EventFlag { id: fid },
                pending:
                    PendingResponse::EventFlagWake {
                        result_ptr,
                        observed,
                    },
                ..
            } => {
                assert_eq!(fid, id);
                assert_eq!(result_ptr, 0x200);
                assert_eq!(observed, 0);
            }
            other => panic!("expected Block on EventFlag, got {other:?}"),
        }
        assert_eq!(host.event_flags().lookup(id).unwrap().waiters().len(), 1);
    }

    #[test]
    fn event_flag_set_wakes_matching_waiters_in_fifo_order() {
        let mut host = Lv2Host::new();
        let rt = fake_runtime_with_valid_sync_attr(0x10000);
        let u1 = UnitId::new(0);
        let u2 = UnitId::new(1);
        let u3 = UnitId::new(2);
        seed_primary_ppu(&mut host, u1);
        let _t2 = host
            .ppu_threads_mut()
            .create(
                u2,
                PpuThreadAttrs {
                    entry: 0,
                    arg: 0,
                    stack_base: 0,
                    stack_size: 0,
                    priority: 0,
                    tls_base: 0,
                },
            )
            .unwrap();
        let t3 = host
            .ppu_threads_mut()
            .create(
                u3,
                PpuThreadAttrs {
                    entry: 0,
                    arg: 0,
                    stack_base: 0,
                    stack_size: 0,
                    priority: 0,
                    tls_base: 0,
                },
            )
            .unwrap();
        let r = host.dispatch(
            Lv2Request::EventFlagCreate {
                id_ptr: 0x100,
                attr_ptr: VALID_SYNC_ATTR_PTR,
                init: 0,
            },
            u1,
            &rt,
        );
        let id = match &r {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        host.dispatch(
            Lv2Request::EventFlagWait {
                id,
                bits: 0b0001,
                mode: 0x01,
                result_ptr: 0x200,
                timeout: 0,
            },
            u1,
            &rt,
        );
        host.dispatch(
            Lv2Request::EventFlagWait {
                id,
                bits: 0b0010,
                mode: 0x01,
                result_ptr: 0x210,
                timeout: 0,
            },
            u2,
            &rt,
        );
        // u3 waits on 0b1000; the set below won't match it.
        host.dispatch(
            Lv2Request::EventFlagWait {
                id,
                bits: 0b1000,
                mode: 0x01,
                result_ptr: 0x220,
                timeout: 0,
            },
            u3,
            &rt,
        );
        let s = host.dispatch(Lv2Request::EventFlagSet { id, bits: 0b0011 }, u1, &rt);
        match s {
            Lv2Dispatch::WakeAndReturn {
                code: 0,
                woken_unit_ids,
                response_updates,
                ..
            } => {
                assert_eq!(woken_unit_ids, vec![u1, u2]);
                assert_eq!(response_updates.len(), 2);
                for (_, resp) in &response_updates {
                    match resp {
                        PendingResponse::EventFlagWake { observed, .. } => {
                            assert_eq!(*observed, 0b0011);
                        }
                        other => panic!("expected EventFlagWake, got {other:?}"),
                    }
                }
            }
            other => panic!("expected WakeAndReturn, got {other:?}"),
        }
        let remaining: Vec<_> = host
            .event_flags()
            .lookup(id)
            .unwrap()
            .waiters()
            .iter()
            .map(|w| w.thread)
            .collect();
        assert_eq!(remaining, vec![t3]);
    }

    #[test]
    fn event_flag_clear_does_not_wake_anyone() {
        let mut host = Lv2Host::new();
        let rt = fake_runtime_with_valid_sync_attr(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(
            Lv2Request::EventFlagCreate {
                id_ptr: 0x100,
                attr_ptr: VALID_SYNC_ATTR_PTR,
                init: 0b1111,
            },
            src,
            &rt,
        );
        let id = match &r {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        let c = host.dispatch(Lv2Request::EventFlagClear { id, bits: 0b0101 }, src, &rt);
        assert!(matches!(
            c,
            Lv2Dispatch::Immediate {
                code: 0,
                effects: _,
            }
        ));
        // sys_event_flag_clear masks AND: bits in the mask survive,
        // bits outside drop. 0b1111 & 0b0101 -> 0b0101.
        assert_eq!(host.event_flags().lookup(id).unwrap().bits(), 0b0101);
    }

    #[test]
    fn event_flag_trywait_no_match_returns_ebusy() {
        let mut host = Lv2Host::new();
        let rt = fake_runtime_with_valid_sync_attr(0x10000);
        let src = UnitId::new(0);
        seed_primary_ppu(&mut host, src);
        let r = host.dispatch(
            Lv2Request::EventFlagCreate {
                id_ptr: 0x100,
                attr_ptr: VALID_SYNC_ATTR_PTR,
                init: 0,
            },
            src,
            &rt,
        );
        let id = match &r {
            Lv2Dispatch::Immediate {
                code: 0,
                effects: e,
            } => extract_write_u32(&e[0]),
            other => panic!("expected Immediate(0), got {other:?}"),
        };
        let w = host.dispatch(
            Lv2Request::EventFlagTryWait {
                id,
                bits: 0b1,
                mode: 0x01,
                result_ptr: 0x200,
            },
            src,
            &rt,
        );
        let Lv2Dispatch::Immediate { code, .. } = w else {
            panic!("expected Immediate, got {w:?}");
        };
        assert_eq!(code, errno::CELL_EBUSY.into());
    }
}
