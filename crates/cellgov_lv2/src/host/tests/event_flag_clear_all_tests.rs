//! The `CLEAR_ALL` wait mode, the mode with both clear bits, and the
//! event flag object count answer the `lv2_event_flag_clear_all` probe
//! as the reference console does, word for word.

use super::*;
use crate::host::test_support::{
    console_payload, fake_runtime_with_valid_sync_attr, payload_u32, seed_primary_ppu, FakeRuntime,
    VALID_SYNC_ATTR_PTR,
};
use crate::request::Lv2Request;
use cellgov_ps3_abi::lv2::process::SYS_EVENT_FLAG_OBJECT;
use cellgov_ps3_abi::lv2::sync::event_flag_wait_mode as wait_mode;

const PROBE: &str = "lv2_event_flag_clear_all";

/// The probe prefills every u64 the kernel writes with all ones.
const FILL: u64 = u64::MAX;

const OUT_PTR: u32 = 0x300;

struct Probe {
    host: Lv2Host,
    rt: FakeRuntime,
    src: UnitId,
    id: u32,
    words: Vec<u32>,
}

impl Probe {
    fn call(&mut self, req: Lv2Request) -> Lv2Dispatch {
        self.host.dispatch(req, self.src, &self.rt)
    }

    /// The u64 at `OUT_PTR` after `d`: the written bytes over the
    /// all-ones fill, as the probe reads it.
    fn out_u64(d: &Lv2Dispatch) -> u64 {
        let mut word = FILL.to_be_bytes();
        if let Lv2Dispatch::Immediate { effects, .. } = d {
            for effect in effects {
                if let cellgov_effects::Effect::SharedWriteIntent { range, bytes, .. } = effect {
                    let off = (range.start().raw() - u64::from(OUT_PTR)) as usize;
                    let b = bytes.bytes();
                    word[off..off + b.len()].copy_from_slice(b);
                }
            }
        }
        u64::from_be_bytes(word)
    }

    fn code(d: &Lv2Dispatch) -> u32 {
        match d {
            Lv2Dispatch::Immediate { code, .. } => *code as u32,
            other => panic!("expected Immediate, got {other:?}"),
        }
    }

    fn push64(&mut self, v: u64) {
        self.words.push((v >> 32) as u32);
        self.words.push(v as u32);
    }

    fn count(&mut self) {
        let d = self.call(Lv2Request::ProcessGetNumberOfObject {
            class_id: SYS_EVENT_FLAG_OBJECT,
            count_out_ptr: OUT_PTR,
        });
        self.push64(Self::out_u64(&d));
        self.words.push(Self::code(&d));
    }

    fn set_all(&mut self) {
        let d = self.call(Lv2Request::EventFlagSet {
            id: self.id,
            bits: 0xFF,
        });
        self.words.push(Self::code(&d));
    }

    /// One wait (timeout 0) or trywait, its result, and the bits after.
    fn one_wait(&mut self, wait: bool, bits: u64, mode: u32) {
        let id = self.id;
        let d = self.call(if wait {
            Lv2Request::EventFlagWait {
                id,
                bits,
                mode,
                result_ptr: OUT_PTR,
                timeout: 0,
            }
        } else {
            Lv2Request::EventFlagTryWait {
                id,
                bits,
                mode,
                result_ptr: OUT_PTR,
            }
        });
        self.words.push(Self::code(&d));
        self.push64(Self::out_u64(&d));
        let got = self.call(Lv2Request::EventFlagGet {
            id,
            flags_ptr: OUT_PTR,
        });
        self.push64(Self::out_u64(&got));
    }
}

/// CellGov's words 2..=34 of the payload.
fn cellgov_words() -> Vec<u32> {
    let mut host = Lv2Host::new();
    let rt = fake_runtime_with_valid_sync_attr(0x10000);
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let mut p = Probe {
        host,
        rt,
        src,
        id: 0,
        words: Vec::new(),
    };
    p.count();
    let created = p.call(Lv2Request::EventFlagCreate {
        id_ptr: 0x100,
        attr_ptr: VALID_SYNC_ATTR_PTR,
        init: 0xFF,
    });
    p.id = match &created {
        Lv2Dispatch::Immediate { code: 0, effects } => {
            crate::host::test_support::extract_write_u32(&effects[0])
        }
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    p.count();
    p.one_wait(false, 1, wait_mode::OR | wait_mode::CLEAR_ALL);
    p.set_all();
    p.one_wait(
        false,
        1,
        wait_mode::OR | wait_mode::CLEAR | wait_mode::CLEAR_ALL,
    );
    p.set_all();
    p.one_wait(false, 3, wait_mode::AND | wait_mode::CLEAR_ALL);
    p.set_all();
    p.one_wait(true, 1, wait_mode::OR | wait_mode::CLEAR_ALL);
    let id = p.id;
    let destroyed = p.call(Lv2Request::EventFlagDestroy { id });
    p.words.push(Probe::code(&destroyed));
    p.count();
    p.words
}

const WAKE_PROBE: &str = "lv2_event_flag_clear_all_wake";

/// CellGov's words 2..=13 of the wake probe: A parks on OR|CLEAR_ALL
/// mask 1, B parks behind it on OR mask 2, and main sets 0b11.
fn cellgov_wake_words() -> Vec<u32> {
    use cellgov_ps3_abi::lv2::sync::{
        event_flag_attribute as layout, SYS_SYNC_FIFO, SYS_SYNC_NOT_PROCESS_SHARED,
        SYS_SYNC_WAITER_MULTIPLE,
    };
    let (main, a, b) = (UnitId::new(0), UnitId::new(1), UnitId::new(2));
    let mut mem = cellgov_mem::GuestMemory::new(0x10000);
    for (off, word) in [
        (layout::PROTOCOL_OFFSET, SYS_SYNC_FIFO),
        (4, SYS_SYNC_NOT_PROCESS_SHARED),
        (layout::TYPE_OFFSET, SYS_SYNC_WAITER_MULTIPLE),
    ] {
        let at = 0x800 + off as u64;
        mem.apply_commit(
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(at), 4).unwrap(),
            &word.to_be_bytes(),
        )
        .unwrap();
    }
    let rt = FakeRuntime::with_memory(mem);
    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, main);
    for (i, unit) in [a, b].into_iter().enumerate() {
        host.ppu_threads_mut()
            .create(
                unit,
                crate::ppu_thread::PpuThreadAttrs {
                    entry: 0x10_1000,
                    arg: 0,
                    stack_base: 0xD001_0000 + 0x1_0000 * i as u32,
                    stack_size: 0x4000,
                    priority: 1000,
                    tls_base: 0,
                },
            )
            .unwrap();
    }
    let id = match host.dispatch(
        Lv2Request::EventFlagCreate {
            id_ptr: 0x100,
            attr_ptr: 0x800,
            init: 0,
        },
        main,
        &rt,
    ) {
        Lv2Dispatch::Immediate { code: 0, effects } => {
            crate::host::test_support::extract_write_u32(&effects[0])
        }
        other => panic!("expected Immediate(0), got {other:?}"),
    };
    let mut park = |unit: UnitId, bits: u64, mode: u32, result_ptr: u32| match host.dispatch(
        Lv2Request::EventFlagWait {
            id,
            bits,
            mode,
            result_ptr,
            timeout: 200_000,
        },
        unit,
        &rt,
    ) {
        Lv2Dispatch::Block { reason, .. } => reason,
        other => panic!("expected Block, got {other:?}"),
    };
    let _reason_a = park(a, 1, wait_mode::OR | wait_mode::CLEAR_ALL, 0x300);
    let reason_b = park(b, 2, wait_mode::OR, 0x308);

    let set = host.dispatch(Lv2Request::EventFlagSet { id, bits: 0b11 }, main, &rt);
    let (set_rc, woken, updates) = match set {
        Lv2Dispatch::WakeAndReturn {
            code,
            woken_unit_ids,
            response_updates,
            ..
        } => (code as u32, woken_unit_ids, response_updates),
        Lv2Dispatch::Immediate { code, .. } => (code as u32, vec![], vec![]),
        other => panic!("expected an answer, got {other:?}"),
    };
    let bits = host.event_flags().lookup(id).unwrap().bits();
    let observed = |unit: UnitId| {
        updates.iter().find_map(|(u, r)| match r {
            PendingResponse::EventFlagWake { observed, .. } if *u == unit => Some(*observed),
            _ => None,
        })
    };
    let a_result = observed(a).expect("A woke with its pattern");
    let b_expired = host.expire_wait(reason_b, b, GuestTicks::ZERO);
    let b_rc = match b_expired.response_updates.as_slice() {
        [(unit, PendingResponse::ReturnCode { code })] if *unit == b => *code as u32,
        other => panic!("one ReturnCode for B, got {other:?}"),
    };
    let b_result = match b_expired.effects.as_slice() {
        [cellgov_effects::Effect::SharedWriteIntent { bytes, .. }] => {
            u64::from_be_bytes(bytes.bytes().try_into().expect("8 bytes"))
        }
        other => panic!("one result write for B, got {other:?}"),
    };
    let destroy = match host.dispatch(Lv2Request::EventFlagDestroy { id }, main, &rt) {
        Lv2Dispatch::Immediate { code, .. } => code as u32,
        other => panic!("expected Immediate, got {other:?}"),
    };
    vec![
        set_rc,
        (bits >> 32) as u32,
        bits as u32,
        u32::from(woken.contains(&a)),
        u32::from(woken.contains(&b)),
        0,
        (a_result >> 32) as u32,
        a_result as u32,
        b_rc,
        (b_result >> 32) as u32,
        b_result as u32,
        destroy,
    ]
}

#[test]
fn a_set_that_wakes_a_clear_all_waiter_answers_as_the_console_does() {
    let payload = console_payload(WAKE_PROBE);
    assert_eq!(
        (payload_u32(&payload, 0), payload_u32(&payload, 4)),
        (0, 0),
        "the probe ran every step"
    );
    let console: Vec<u32> = (2..=13).map(|i| payload_u32(&payload, 4 * i)).collect();
    assert_eq!(cellgov_wake_words(), console, "payload words 2..=13");
}

#[test]
fn clear_all_and_the_object_count_answer_as_the_console_does() {
    let payload = console_payload(PROBE);
    assert_eq!(
        (payload_u32(&payload, 0), payload_u32(&payload, 4)),
        (0, 0),
        "the probe created its flag"
    );
    let console: Vec<u32> = (2..=34).map(|i| payload_u32(&payload, 4 * i)).collect();
    assert_eq!(cellgov_words(), console, "payload words 2..=34");
}
