//! The PPU thread priority, create and join refusals held to the
//! `lv2_ppu_thread_answers`, `lv2_ppu_thread_priority_bounds` and
//! `lv2_ppu_thread_priority_ceiling` captures from the reference
//! console. Every probe runs as a root process, so the host carries the
//! root control flag. Each call goes through the dispatch with the
//! probe's arguments, and the test compares its code with the captured
//! word.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::syscall;

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{
    console_payload, extract_write_u32, opd_runtime, payload_u32, seed_primary_ppu, FakeRuntime,
};
use crate::host::Lv2Host;
use crate::ppu_thread::PpuThreadId;
use crate::request::Lv2Request;

const CALLER: UnitId = UnitId::new(0);
/// The SELF control flags every probe EBOOT carries: root permission.
const PROBE_CONTROL_FLAGS1: u32 = 0x4000_0000;
const NO_SUCH_ID: u64 = 0xDEAD_BEEF;
/// A thread-init block naming a readable entry descriptor.
const PARAM: u32 = 0x200;
/// A thread-init block whose entry word is 0.
const PARAM_NO_ENTRY: u32 = 0x300;
const ID_PTR: u32 = 0x1000;
const PRIO_PTR: u32 = 0x2000;
const VALUE_PTR: u32 = 0x2010;
const JOINABLE: u64 = 1;
const JOINABLE_AND_INTERRUPT: u64 = 3;
/// The priority the probes create their threads at.
const CHILD_PRIO: i32 = 1000;

struct Probe {
    host: Lv2Host,
    rt: FakeRuntime,
}

fn code(d: &Lv2Dispatch) -> u32 {
    match d {
        Lv2Dispatch::Immediate { code, .. } => *code as u32,
        Lv2Dispatch::PpuThreadCreate { .. } => 0,
        other => panic!("unexpected dispatch {other:?}"),
    }
}

impl Probe {
    fn new() -> Self {
        let mut host = Lv2Host::new();
        host.set_control_flags1(PROBE_CONTROL_FLAGS1);
        seed_primary_ppu(&mut host, CALLER);
        // Fresh memory is zero, so the block at PARAM_NO_ENTRY names no
        // entry.
        let rt = opd_runtime(PARAM, 0x10_0000, 0x10_0100);
        Self { host, rt }
    }

    fn syscall(&mut self, number: u64, a: u64, b: u64) -> Lv2Dispatch {
        let mut args = [0u64; 8];
        args[0] = a;
        args[1] = b;
        self.host
            .dispatch(Lv2Request::Unsupported { number, args }, CALLER, &self.rt)
    }

    fn get_priority(&mut self, thread_id: u64, ptr: u32) -> u32 {
        code(&self.syscall(syscall::PPU_THREAD_GET_PRIORITY, thread_id, u64::from(ptr)))
    }

    fn set_priority(&mut self, thread_id: u64, prio: i32) -> u32 {
        code(&self.syscall(
            syscall::PPU_THREAD_SET_PRIORITY,
            thread_id,
            prio as i64 as u64,
        ))
    }

    fn create(&mut self, id_ptr: u32, param_ptr: u32, priority: i32, flags: u64) -> Lv2Dispatch {
        self.host.dispatch(
            Lv2Request::PpuThreadCreate {
                id_ptr,
                param_ptr,
                arg: 0,
                unk: 0,
                priority,
                stacksize: 0x4000,
                flags,
                threadname_ptr: 0,
            },
            CALLER,
            &self.rt,
        )
    }

    fn join(&mut self, target: u64) -> u32 {
        code(&self.host.dispatch(
            Lv2Request::PpuThreadJoin {
                target,
                status_out_ptr: VALUE_PTR,
            },
            CALLER,
            &self.rt,
        ))
    }
}

#[test]
fn priority_create_and_join_refusals_answer_as_on_the_console() {
    let payload = console_payload("lv2_ppu_thread_answers");
    let word = |w: usize| payload_u32(&payload, 4 * w);
    assert_eq!((word(0), word(1)), (0, 0), "the probe ran every step");

    let mut p = Probe::new();
    let me = PpuThreadId::PRIMARY.raw();
    // The process starts at the priority its parameter block names.
    assert_eq!(p.set_priority(me, word(6) as i32), 0);

    let mut got = vec![0u32; 2];
    got.push(p.get_priority(NO_SUCH_ID, 0));
    got.push(p.get_priority(NO_SUCH_ID, PRIO_PTR));
    got.push(p.get_priority(me, 0));
    let d = p.syscall(syscall::PPU_THREAD_GET_PRIORITY, me, u64::from(PRIO_PTR));
    got.push(code(&d));
    let Lv2Dispatch::Immediate { effects, .. } = &d else {
        panic!("expected Immediate");
    };
    got.push(extract_write_u32(&effects[0]));
    got.push(p.set_priority(NO_SUCH_ID, 5000));
    got.push(p.set_priority(NO_SUCH_ID, 100));
    for prio in [5000, -1, -512, -513, 3071, 3072] {
        got.push(p.set_priority(me, prio));
    }
    got.push(p.set_priority(me, word(6) as i32));

    got.push(code(&p.create(ID_PTR, 0, 9999, 0)));
    got.push(code(&p.create(ID_PTR, PARAM_NO_ENTRY, 9999, 0)));
    got.push(code(&p.create(
        ID_PTR,
        PARAM,
        CHILD_PRIO,
        JOINABLE_AND_INTERRUPT,
    )));
    got.push(code(&p.create(ID_PTR, PARAM, 9999, JOINABLE_AND_INTERRUPT)));
    got.push(code(&p.create(
        0,
        PARAM,
        CHILD_PRIO,
        JOINABLE_AND_INTERRUPT,
    )));
    got.push(code(&p.create(ID_PTR, PARAM, 9999, 0)));
    got.push(code(&p.create(ID_PTR, PARAM, -1, 0)));
    got.push(code(&p.create(0, PARAM, CHILD_PRIO, 0)));
    got.push(code(&p.create(ID_PTR, PARAM, CHILD_PRIO, JOINABLE)));
    assert_eq!(got.len(), 25);
    for (w, &value) in got.iter().enumerate() {
        assert_eq!(
            value,
            word(w),
            "w{w}: 0x{value:08x} vs console 0x{:08x}",
            word(w)
        );
    }

    assert_eq!(p.join(NO_SUCH_ID), word(40), "join of an unknown id");
    assert_eq!(p.join(me), word(41), "join of the caller");
}

#[test]
fn set_priority_window_and_created_priorities_answer_as_on_the_console() {
    let bounds = console_payload("lv2_ppu_thread_priority_bounds");
    let ceiling = console_payload("lv2_ppu_thread_priority_ceiling");
    for payload in [&bounds, &ceiling] {
        assert_eq!(
            (payload_u32(payload, 0), payload_u32(payload, 4)),
            (0, 0),
            "the probe ran every step"
        );
    }
    let mut p = Probe::new();
    let me = PpuThreadId::PRIMARY.raw();
    for (payload, values) in [
        (
            &bounds,
            [
                3072,
                3200,
                3583,
                3584,
                3839,
                3840,
                4095,
                4096,
                4500,
                4999,
                0x7FFF_FFFF,
                -1000,
            ],
        ),
        (
            &ceiling,
            [
                3071, 3072, 3073, 3074, 3080, 3100, 3127, 3128, 3135, 3136, 3150, 3199,
            ],
        ),
    ] {
        for (i, prio) in values.into_iter().enumerate() {
            assert_eq!(
                p.set_priority(me, prio),
                payload_u32(payload, 4 * (2 + i)),
                "47({prio})"
            );
            assert_eq!(p.set_priority(me, CHILD_PRIO), 0);
        }
    }

    for (i, prio) in [3072, 4096, 9999, 65535, 0x7FFF_FFFF, -1000]
        .into_iter()
        .enumerate()
    {
        let w = 16 + 5 * i;
        match p.create(ID_PTR, PARAM, prio, JOINABLE) {
            Lv2Dispatch::PpuThreadCreate { priority, .. } => {
                assert_eq!(payload_u32(&bounds, 4 * w), 0, "52({prio})");
                assert_eq!(
                    priority,
                    payload_u32(&bounds, 4 * (w + 2)),
                    "52({prio}) priority"
                );
            }
            other => panic!("52({prio}): expected PpuThreadCreate, got {other:?}"),
        }
    }
}

/// A join of a thread that already finished collects it too, so a
/// second join answers CELL_ESRCH, as the console's second join of a
/// collected thread does in the `lv2_ppu_thread_answers` capture.
#[test]
fn a_join_of_a_finished_thread_collects_it() {
    let payload = console_payload("lv2_ppu_thread_answers");
    let mut p = Probe::new();
    let child = p
        .host
        .ppu_threads_mut()
        .create(UnitId::new(1), crate::host::test_support::primary_attrs())
        .expect("child create");
    p.host.ppu_threads_mut().mark_finished(child, 0x55);
    assert_eq!(p.join(child.raw()), 0, "the first join collects the thread");
    assert_eq!(
        p.join(child.raw()),
        payload_u32(&payload, 4 * 29),
        "a second join answers as the console's second join"
    );
}
