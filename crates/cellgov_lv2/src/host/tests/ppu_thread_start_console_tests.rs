//! `sys_ppu_thread_start` held to the `lv2_ppu_thread_answers` and
//! `lv2_ppu_thread_start_edges` captures from the reference console. The
//! classifier reads each call from the probe's literal syscall number and
//! arguments, and the test compares its code with the captured word.

use cellgov_event::UnitId;

use crate::dispatch::Lv2Dispatch;
use crate::host::test_support::{
    console_payload, payload_u32, primary_attrs, seed_primary_ppu, FakeRuntime,
};
use crate::host::Lv2Host;
use crate::ppu_thread::PpuThreadId;
use crate::request::classify;

const CALLER: UnitId = UnitId::new(0);
const CHILD: UnitId = UnitId::new(1);
/// The SELF control flags every probe EBOOT carries: root permission.
const PROBE_CONTROL_FLAGS1: u32 = 0x4000_0000;
const NO_SUCH_ID: u64 = 0xDEAD_BEEF;
const PRIO_PTR: u64 = 0x2000;
const VALUE_PTR: u64 = 0x2010;
/// The priority the start-edges probe moves its waiting thread to.
const D_PRIO: u64 = 1500;

struct Probe {
    host: Lv2Host,
    rt: FakeRuntime,
}

impl Probe {
    fn new() -> Self {
        let mut host = Lv2Host::new();
        host.set_control_flags1(PROBE_CONTROL_FLAGS1);
        seed_primary_ppu(&mut host, CALLER);
        Self {
            host,
            rt: FakeRuntime::new(0x10000),
        }
    }

    fn syscall(&mut self, number: u64, a: u64, b: u64) -> Lv2Dispatch {
        let request = classify(number, &[a, b, 0, 0, 0, 0, 0, 0]);
        self.host.dispatch(request, CALLER, &self.rt)
    }

    /// The code a call answers. A start the host accepts answers 0
    /// once the runtime releases the thread.
    fn code(&mut self, number: u64, a: u64, b: u64) -> u32 {
        match self.syscall(number, a, b) {
            Lv2Dispatch::Immediate { code, .. } => code as u32,
            Lv2Dispatch::PpuThreadStart { .. } => 0,
            other => panic!("syscall {number}: unexpected dispatch {other:?}"),
        }
    }

    /// A thread `_sys_ppu_thread_create` made, as the runtime enters it.
    fn created_thread(&mut self) -> u64 {
        self.host
            .ppu_threads_mut()
            .create_unstarted(CHILD, primary_attrs())
            .expect("child create")
            .raw()
    }
}

#[test]
fn a_start_answers_as_on_the_console() {
    let answers = console_payload("lv2_ppu_thread_answers");
    let edges = console_payload("lv2_ppu_thread_start_edges");
    let edge = |w: usize| payload_u32(&edges, 4 * w);
    assert_eq!((edge(0), edge(1)), (0, 0), "the probe ran every step");

    let mut p = Probe::new();
    let me = PpuThreadId::PRIMARY.raw();
    assert_eq!(
        p.code(53, NO_SUCH_ID, 0),
        edge(2),
        "w2: 53 of an unknown id"
    );
    assert_eq!(p.code(53, me, 0), edge(3), "w3: 53 of the caller");

    let c = p.created_thread();
    assert_eq!(
        p.syscall(53, c, 0),
        Lv2Dispatch::PpuThreadStart { unit: CHILD },
        "the first 53 releases the thread's unit"
    );
    assert_eq!(edge(5), 0, "w5: the console's first 53");
    assert_eq!(
        p.code(53, c, 0),
        payload_u32(&answers, 4 * 26),
        "a second 53 on a running thread"
    );
    p.host
        .ppu_threads_mut()
        .mark_finished(PpuThreadId::new(c), u64::from(edge(8)));
    assert_eq!(
        p.code(53, c, 0),
        payload_u32(&answers, 4 * 37),
        "a 53 on a thread that has exited"
    );
    assert_eq!(p.code(44, c, VALUE_PTR), edge(7), "w7: the join");
    assert_eq!(p.code(53, c, 0), edge(9), "w9: a 53 after the join");
}

#[test]
fn a_thread_waiting_for_its_start_answers_priority_calls_as_on_the_console() {
    let edges = console_payload("lv2_ppu_thread_start_edges");
    let edge = |w: usize| payload_u32(&edges, 4 * w);
    let mut p = Probe::new();
    let d = p.created_thread();

    let read_priority = |p: &mut Probe| match p.syscall(48, d, PRIO_PTR) {
        Lv2Dispatch::Immediate { code: 0, effects } => {
            crate::host::test_support::extract_write_u32(&effects[0])
        }
        other => panic!("48: unexpected dispatch {other:?}"),
    };
    assert_eq!(edge(11), 0, "w11: the console's 48");
    assert_eq!(read_priority(&mut p), edge(12), "w12: the created priority");
    assert_eq!(p.code(47, d, D_PRIO), edge(13), "w13: 47");
    assert_eq!(read_priority(&mut p), edge(14), "w14: the moved priority");
    assert_eq!(p.code(53, d, 0), edge(16), "w16: the start");
}
